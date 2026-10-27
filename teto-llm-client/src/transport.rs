use std::{
    io,
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use crossbeam_channel::{Receiver, Sender, bounded};
use teto_llm_protocol::{Error, ErrorKind, Request, Response, Sensitivity, read_ipc, write_ipc};

use crate::{ClientError, WorkerLaunch};

pub(crate) const POLL: Duration = Duration::from_millis(25);

struct Outgoing {
    seq: u64,
    sensitivity: Sensitivity,
    request: Request,
    ack: Option<Sender<Result<(), ClientError>>>,
}

pub(crate) enum Delivery {
    Response { seq: u64, response: Response },
    Transport(ClientError),
}

/// One worker process and its framed pipes.
pub(crate) struct Transport {
    child: Child,
    outgoing: Sender<Outgoing>,
    incoming: Option<Receiver<Delivery>>,
    writer: Option<JoinHandle<()>>,
    reader: Option<JoinHandle<()>>,
    next_seq: u64,
    unusable: bool,
}

impl Transport {
    pub fn spawn(launch: WorkerLaunch) -> Result<Self, ClientError> {
        let mut child = Command::new(&launch.program)
            .args(&launch.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| ClientError::Io {
                operation: "spawn",
                kind: error.kind(),
            })?;
        let input = child.stdin.take().ok_or(ClientError::Io {
            operation: "stdin",
            kind: io::ErrorKind::BrokenPipe,
        })?;
        let output = child.stdout.take().ok_or(ClientError::Io {
            operation: "stdout",
            kind: io::ErrorKind::BrokenPipe,
        })?;
        let (outgoing, outbox) = bounded(4);
        let (inbox, incoming) = bounded(4);
        let writer = thread::Builder::new()
            .name("teto-llm-client-writer".into())
            .spawn(move || write_loop(input, outbox))
            .map_err(|error| ClientError::Io {
                operation: "writer thread",
                kind: error.kind(),
            })?;
        let reader = thread::Builder::new()
            .name("teto-llm-client-reader".into())
            .spawn(move || read_loop(output, inbox))
            .map_err(|error| ClientError::Io {
                operation: "reader thread",
                kind: error.kind(),
            })?;
        Ok(Self {
            child,
            outgoing,
            incoming: Some(incoming),
            writer: Some(writer),
            reader: Some(reader),
            next_seq: 1,
            unusable: false,
        })
    }

    pub fn next_seq(&mut self) -> Result<u64, ClientError> {
        let seq = self.next_seq;
        self.next_seq = self
            .next_seq
            .checked_add(1)
            .ok_or(ClientError::SequenceExhausted)?;
        Ok(seq)
    }

    pub fn send(
        &mut self,
        seq: u64,
        sensitivity: Sensitivity,
        request: Request,
        timeout: Duration,
    ) -> Result<(), ClientError> {
        if self.unusable {
            return Err(ClientError::Closed);
        }
        let (ack_tx, ack_rx) = bounded(1);
        let outgoing = Outgoing {
            seq,
            sensitivity,
            request,
            ack: Some(ack_tx),
        };
        if self.outgoing.send_timeout(outgoing, timeout).is_err() {
            self.fail();
            return Err(ClientError::Timeout {
                operation: "write queue",
            });
        }
        match ack_rx.recv_timeout(timeout) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => {
                self.fail();
                Err(error)
            }
            Err(_) => {
                self.fail();
                Err(ClientError::Timeout { operation: "write" })
            }
        }
    }

    pub fn send_nowait(&mut self, seq: u64, sensitivity: Sensitivity, request: Request) {
        let _ = self.outgoing.try_send(Outgoing {
            seq,
            sensitivity,
            request,
            ack: None,
        });
    }

    pub fn receive(&self, timeout: Duration) -> Result<Option<Delivery>, ClientError> {
        let incoming = self.incoming.as_ref().ok_or(ClientError::Closed)?;
        match incoming.recv_timeout(timeout) {
            Ok(delivery) => Ok(Some(delivery)),
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => Ok(None),
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => Err(ClientError::Closed),
        }
    }

    pub fn request(
        &mut self,
        request: Request,
        sensitivity: Sensitivity,
        timeout: Duration,
    ) -> Result<Response, ClientError> {
        let seq = self.next_seq()?;
        self.send(seq, sensitivity, request, timeout)?;
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                self.fail();
                return Err(ClientError::Timeout {
                    operation: "response",
                });
            }
            match self.receive(remaining.min(POLL))? {
                None => continue,
                Some(Delivery::Transport(error)) => {
                    self.fail();
                    return Err(error);
                }
                Some(Delivery::Response { seq: got, response }) => {
                    if got < seq {
                        continue;
                    }
                    if got > seq {
                        self.fail();
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

    pub fn fail(&mut self) {
        self.unusable = true;
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    pub fn close_writer(&mut self) {
        let (sender, _receiver) = bounded(1);
        drop(std::mem::replace(&mut self.outgoing, sender));
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
    }

    pub fn wait_exit(&mut self, timeout: Duration) -> Result<(), ClientError> {
        let started = Instant::now();
        loop {
            let status = self.child.try_wait().map_err(|error| ClientError::Io {
                operation: "wait",
                kind: error.kind(),
            })?;
            if let Some(status) = status {
                if status.success() {
                    return Ok(());
                }
                return Err(ClientError::Exited {
                    status: status.code(),
                });
            }
            if started.elapsed() >= timeout {
                return Err(ClientError::Timeout {
                    operation: "worker exit",
                });
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Transport {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.close_writer();
        drop(self.incoming.take());
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

fn write_loop(mut input: ChildStdin, outbox: Receiver<Outgoing>) {
    while let Ok(outgoing) = outbox.recv() {
        let result = write_ipc(
            &mut input,
            outgoing.seq,
            outgoing.sensitivity,
            &outgoing.request,
        )
        .map_err(write_error);
        let failed = result.is_err();
        if let Some(ack) = outgoing.ack {
            let _ = ack.send(result);
        }
        if failed {
            break;
        }
    }
}

fn read_loop(mut output: ChildStdout, inbox: Sender<Delivery>) {
    loop {
        match read_ipc::<Response>(&mut output) {
            Ok((seq, response)) => {
                if inbox.send(Delivery::Response { seq, response }).is_err() {
                    break;
                }
            }
            Err(error) => {
                let _ = inbox.send(Delivery::Transport(ClientError::Transport(error)));
                break;
            }
        }
    }
}

fn write_error(error: Error) -> ClientError {
    match error.kind() {
        ErrorKind::Protocol => ClientError::Protocol,
        _ => ClientError::Io {
            operation: "write",
            kind: io::ErrorKind::BrokenPipe,
        },
    }
}
