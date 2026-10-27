use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use crossbeam_channel::{Receiver, Sender, bounded};
use teto_llm_protocol::{
    ClauseEvent, ClausePlayed, FinalizedTurn, OpenConfig, PlaybackStop, ReceiptSource, Request,
    Response, Sensitivity, TurnId, UserInput, policy,
};

use crate::{
    ClientError, WorkerLaunch,
    transport::{Delivery, Transport},
};

/// Per-operation deadlines and optional shutdown flag.
#[derive(Debug, Clone)]
pub struct ClientOptions {
    /// Open deadline in microseconds.
    pub open_us: u64,
    /// Start deadline in microseconds.
    pub start_us: u64,
    /// Pull deadline in microseconds.
    pub pull_us: u64,
    /// Finish and acknowledgment deadline in microseconds.
    pub finish_us: u64,
    /// Close deadline in microseconds.
    pub close_us: u64,
    /// Shutdown poll interval in microseconds.
    pub shutdown_poll_us: u64,
    /// Optional process shutdown flag.
    pub shutdown: Option<Arc<AtomicBool>>,
}

impl Default for ClientOptions {
    fn default() -> Self {
        let seconds = |value: u64| value.saturating_mul(1_000_000);
        Self {
            open_us: seconds(policy::STARTUP_SECONDS),
            start_us: seconds(policy::STARTUP_SECONDS),
            pull_us: seconds(policy::STARTUP_SECONDS),
            finish_us: seconds(policy::SHUTDOWN_SECONDS),
            close_us: seconds(policy::SHUTDOWN_SECONDS),
            shutdown_poll_us: 25_000,
            shutdown: None,
        }
    }
}

/// A cross-thread cancellation handle for one active turn.
#[derive(Clone)]
pub struct CancellationToken {
    sender: Sender<TurnId>,
}

impl CancellationToken {
    /// Request cancellation of one turn.
    pub fn cancel(&self, turn: TurnId) {
        let _ = self.sender.try_send(turn);
    }
}

/// One active turn over one worker process.
pub struct Client {
    transport: Transport,
    options: ClientOptions,
    active: Option<TurnId>,
    cancel_sender: Sender<TurnId>,
    cancel_receiver: Receiver<TurnId>,
    closed: bool,
}

impl Client {
    /// Spawn a worker and open one model.
    pub fn open(
        launch: WorkerLaunch,
        config: OpenConfig,
        options: ClientOptions,
    ) -> Result<Self, ClientError> {
        let transport = Transport::spawn(launch)?;
        let (cancel_sender, cancel_receiver) = bounded(1);
        let mut client = Self {
            transport,
            options,
            active: None,
            cancel_sender,
            cancel_receiver,
            closed: false,
        };
        client.request(
            Request::Open(config),
            Sensitivity::Secret,
            "open",
            client.options.open_us,
            false,
        )?;
        Ok(client)
    }

    /// Start one turn.
    pub fn start(&mut self, input: UserInput) -> Result<TurnId, ClientError> {
        match self.request(
            Request::Start(input),
            Sensitivity::Public,
            "start",
            self.options.start_us,
            true,
        )? {
            Response::Started(turn) => {
                self.active = Some(turn);
                Ok(turn)
            }
            _ => Err(ClientError::Protocol),
        }
    }

    /// Pull one clause event.
    pub fn pull(&mut self, turn: TurnId) -> Result<ClauseEvent, ClientError> {
        match self.request(
            Request::Pull(turn),
            Sensitivity::Public,
            "pull",
            self.options.pull_us,
            false,
        )? {
            Response::Event(event) => Ok(event),
            _ => Err(ClientError::Protocol),
        }
    }

    /// Acknowledge one whole clause.
    pub fn acknowledge(&mut self, receipt: ClausePlayed) -> Result<(), ClientError> {
        match self.request(
            Request::Played(receipt),
            Sensitivity::Public,
            "acknowledge",
            self.options.finish_us,
            false,
        )? {
            Response::Acknowledged => Ok(()),
            _ => Err(ClientError::Protocol),
        }
    }

    /// Request cancellation without waiting for a response.
    pub fn cancel(&mut self, turn: TurnId) -> Result<(), ClientError> {
        if self.active == Some(turn) {
            let seq = self.transport.next_seq()?;
            self.transport
                .send_nowait(seq, Sensitivity::Public, Request::Cancel(turn));
        }
        Ok(())
    }

    /// Seal the active turn.
    pub fn finish(
        &mut self,
        turn: TurnId,
        playback: PlaybackStop,
        source: ReceiptSource,
    ) -> Result<FinalizedTurn, ClientError> {
        match self.request(
            Request::Finish {
                turn,
                playback,
                source,
            },
            Sensitivity::Public,
            "finish",
            self.options.finish_us,
            true,
        )? {
            Response::Finished(record) if record.turn == turn => {
                self.active = None;
                Ok(*record)
            }
            _ => Err(ClientError::Protocol),
        }
    }

    /// Reset the conversation.
    pub fn reset(&mut self) -> Result<(), ClientError> {
        match self.request(
            Request::Reset,
            Sensitivity::Public,
            "reset",
            self.options.finish_us,
            false,
        )? {
            Response::Reset => Ok(()),
            _ => Err(ClientError::Protocol),
        }
    }

    /// Retry the pending journal record.
    pub fn retry(&mut self) -> Result<FinalizedTurn, ClientError> {
        match self.request(
            Request::Retry,
            Sensitivity::Public,
            "retry",
            self.options.finish_us,
            true,
        )? {
            Response::Finished(record) => {
                self.active = None;
                Ok(*record)
            }
            _ => Err(ClientError::Protocol),
        }
    }

    /// Close the worker. A domain error keeps the client usable.
    pub fn close(&mut self) -> Result<(), ClientError> {
        if self.closed {
            return Ok(());
        }
        let response = self.request(
            Request::Close,
            Sensitivity::Public,
            "close",
            self.options.close_us,
            true,
        )?;
        if !matches!(response, Response::Closed) {
            return Err(ClientError::Protocol);
        }
        self.transport.close_writer();
        let result = self.transport.wait_exit(duration(self.options.close_us));
        self.closed = true;
        result
    }

    /// Return a cross-thread cancellation handle.
    pub fn cancellation(&self) -> CancellationToken {
        CancellationToken {
            sender: self.cancel_sender.clone(),
        }
    }

    fn request(
        &mut self,
        request: Request,
        sensitivity: Sensitivity,
        operation: &'static str,
        timeout_us: u64,
        uncertain: bool,
    ) -> Result<Response, ClientError> {
        if self.closed {
            return Err(ClientError::Closed);
        }
        let timeout = duration(timeout_us);
        let seq = self.transport.next_seq()?;
        if let Err(error) = self.transport.send(seq, sensitivity, request, timeout) {
            return Err(match error {
                ClientError::Timeout { .. } | ClientError::Transport(_) if uncertain => {
                    ClientError::OutcomeUnknown { operation }
                }
                other => other,
            });
        }
        let deadline = Instant::now() + timeout;
        let mut cancelled = false;
        loop {
            if !cancelled && let Some(turn) = self.pending_cancel() {
                let cancel_seq = self.transport.next_seq()?;
                self.transport
                    .send_nowait(cancel_seq, Sensitivity::Public, Request::Cancel(turn));
                cancelled = true;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                self.transport.fail();
                return Err(if uncertain {
                    ClientError::OutcomeUnknown { operation }
                } else {
                    ClientError::Timeout { operation }
                });
            }
            let wait = remaining.min(duration(self.options.shutdown_poll_us));
            match self.transport.receive(wait)? {
                None => continue,
                Some(Delivery::Transport(error)) => {
                    self.transport.fail();
                    return Err(error);
                }
                Some(Delivery::Response { seq: got, response }) => {
                    if got < seq {
                        continue;
                    }
                    if got > seq {
                        self.transport.fail();
                        return Err(ClientError::Protocol);
                    }
                    return match response {
                        Response::Error(error) => Err(ClientError::Remote(error)),
                        other => Ok(other),
                    };
                }
            }
        }
    }

    fn pending_cancel(&mut self) -> Option<TurnId> {
        while let Ok(turn) = self.cancel_receiver.try_recv() {
            if self.active == Some(turn) {
                return Some(turn);
            }
        }
        let shutdown = self
            .options
            .shutdown
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Acquire));
        if shutdown { self.active } else { None }
    }
}

fn duration(microseconds: u64) -> Duration {
    Duration::from_micros(microseconds.max(1))
}
