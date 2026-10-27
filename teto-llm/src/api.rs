use crate::mailbox::Mailbox;
use crate::*;
use crossbeam_channel::{Receiver, Sender, bounded};
use std::{
    cell::Cell,
    marker::PhantomData,
    sync::Arc,
    thread::{self, JoinHandle},
    time::Duration,
};

pub(crate) type Reply<T> = Sender<Result<T>>;

/// Commands accepted by the inference thread.
pub(crate) enum Command {
    Start(UserInput, Arc<Mailbox>, Reply<()>),
    Finish(TurnId, PlaybackStop, ReceiptSource, Reply<FinalizedTurn>),
    Reset(Reply<()>),
    Retry(Reply<FinalizedTurn>),
    Close(Reply<()>),
}

fn wait_for<T>(receiver: Receiver<Result<T>>, seconds: u64) -> Result<T> {
    match receiver.recv_timeout(Duration::from_secs(seconds)) {
        Ok(result) => result,
        Err(_) => Err(failure(ErrorKind::Worker)),
    }
}

/// Cancels a turn from any thread.
#[derive(Clone)]
pub struct TurnControl {
    mailbox: Arc<Mailbox>,
}
impl TurnControl {
    pub fn id(&self) -> TurnId {
        self.mailbox.id
    }
    pub fn cancel(&self) {
        self.mailbox.cancel();
    }
    pub fn acknowledge(&self, receipt: ClausePlayed) -> Result<()> {
        self.mailbox.acknowledge(receipt)
    }
}

/// Receives clauses for one turn.
pub struct Turn {
    mailbox: Arc<Mailbox>,
    _not_sync: PhantomData<Cell<()>>,
}
impl Turn {
    pub fn id(&self) -> TurnId {
        self.mailbox.id
    }
    pub fn next_clause(&mut self) -> Result<ClauseEvent> {
        self.mailbox.next()
    }
    pub fn control(&self) -> TurnControl {
        TurnControl {
            mailbox: Arc::clone(&self.mailbox),
        }
    }
    pub fn acknowledge(&mut self, receipt: ClausePlayed) -> Result<()> {
        self.mailbox.acknowledge(receipt)
    }
}
impl Drop for Turn {
    fn drop(&mut self) {
        self.mailbox.cancel();
    }
}

/// One model, one context, one active turn.
pub struct Llm {
    commands: Sender<Command>,
    worker: Option<JoinHandle<()>>,
    active: Option<Arc<Mailbox>>,
    next_id: Option<TurnId>,
    delivery_window: DeliveryWindow,
    _not_sync: PhantomData<Cell<()>>,
}
impl Llm {
    /// Start the inference thread and wait for the model and resources.
    ///
    /// # Errors
    /// Returns `Worker` when the thread does not report readiness in time.
    /// Returns the startup failure reported by the thread.
    pub fn open(config: OpenConfig) -> Result<Self> {
        Profile::validate_config(&config)?;
        let delivery_window = config.delivery_window;
        let (commands, receiver) = bounded(policy::CONTROL_QUEUE);
        let (ready, startup) = bounded(1);
        let worker = thread::Builder::new()
            .name("teto-llm-inference".into())
            .spawn(move || {
                if let Err(error) = crate::worker::run(config, receiver, ready.clone()) {
                    let _ = ready.send(Err(error));
                }
            })
            .map_err(|_| failure(ErrorKind::Worker))?;
        match startup.recv_timeout(Duration::from_secs(policy::STARTUP_SECONDS)) {
            Ok(Ok(())) => Ok(Self {
                commands,
                worker: Some(worker),
                active: None,
                next_id: Some(TurnId::new(1)?),
                delivery_window,
                _not_sync: PhantomData,
            }),
            Ok(Err(error)) => {
                let _ = worker.join();
                Err(error)
            }
            Err(_) => {
                drop(worker);
                Err(failure(ErrorKind::Worker))
            }
        }
    }

    /// Cancel any active turn and admit one new turn.
    ///
    /// # Errors
    /// Returns `PlaybackPending` when a previous turn is not sealed.
    pub fn start_turn(&mut self, input: UserInput) -> Result<Turn> {
        if let Some(active) = &self.active {
            active.cancel();
            return Err(failure(ErrorKind::PlaybackPending));
        }
        let id = self.next_id.ok_or_else(|| failure(ErrorKind::Worker))?;
        self.next_id = id.next().ok();
        let mailbox = Arc::new(Mailbox::new(id, self.delivery_window));
        let (reply, receive) = bounded(1);
        self.commands
            .send(Command::Start(input, Arc::clone(&mailbox), reply))
            .map_err(|_| failure(ErrorKind::Worker))?;
        wait_for(receive, policy::SHUTDOWN_SECONDS)?;
        self.active = Some(Arc::clone(&mailbox));
        Ok(Turn {
            mailbox,
            _not_sync: PhantomData,
        })
    }

    /// Seal the turn owned by `turn`.
    pub fn finish_turn(
        &mut self,
        turn: &mut Turn,
        playback: PlaybackStop,
        source: ReceiptSource,
    ) -> Result<FinalizedTurn> {
        if self
            .active
            .as_ref()
            .is_none_or(|active| !Arc::ptr_eq(active, &turn.mailbox))
        {
            return Err(failure(ErrorKind::Receipt));
        }
        self.finish_playback(turn.id(), playback, source)
    }

    /// Seal a turn after its consumer handle is dropped.
    pub fn finish_playback(
        &mut self,
        id: TurnId,
        playback: PlaybackStop,
        source: ReceiptSource,
    ) -> Result<FinalizedTurn> {
        let active = self
            .active
            .as_ref()
            .filter(|active| active.id == id)
            .ok_or_else(|| failure(ErrorKind::Receipt))?;
        if playback != PlaybackStop::Complete {
            active.cancel();
        }
        let (reply, receive) = bounded(1);
        self.commands
            .send(Command::Finish(id, playback, source, reply))
            .map_err(|_| failure(ErrorKind::Worker))?;
        let record = wait_for(receive, policy::SHUTDOWN_SECONDS)?;
        self.active = None;
        Ok(record)
    }

    /// Drop all history and clear the model cache.
    pub fn reset_conversation(&mut self) -> Result<()> {
        if let Some(active) = &self.active {
            active.cancel();
            return Err(failure(ErrorKind::PlaybackPending));
        }
        let (reply, receive) = bounded(1);
        self.commands
            .send(Command::Reset(reply))
            .map_err(|_| failure(ErrorKind::Worker))?;
        wait_for(receive, policy::SHUTDOWN_SECONDS)
    }

    /// Retry the pending journal record.
    pub fn retry_pending(&mut self) -> Result<FinalizedTurn> {
        let (reply, receive) = bounded(1);
        self.commands
            .send(Command::Retry(reply))
            .map_err(|_| failure(ErrorKind::Worker))?;
        let record = wait_for(receive, policy::SHUTDOWN_SECONDS)?;
        self.active = None;
        Ok(record)
    }

    /// Stop the inference thread.
    pub fn close(&mut self) -> Result<()> {
        if let Some(active) = &self.active {
            active.cancel();
            return Err(failure(ErrorKind::PlaybackPending));
        }
        let (reply, receive) = bounded(1);
        self.commands
            .send(Command::Close(reply))
            .map_err(|_| failure(ErrorKind::Worker))?;
        wait_for(receive, policy::SHUTDOWN_SECONDS)?;
        self.join()
    }

    fn join(&mut self) -> Result<()> {
        if let Some(worker) = self.worker.take() {
            worker.join().map_err(|_| failure(ErrorKind::Worker))?;
        }
        Ok(())
    }
}
impl Drop for Llm {
    fn drop(&mut self) {
        if let Some(active) = &self.active {
            active.cancel();
        }
        let (reply, _) = bounded(1);
        let _ = self.commands.send(Command::Close(reply));
        let _ = self.join();
    }
}
