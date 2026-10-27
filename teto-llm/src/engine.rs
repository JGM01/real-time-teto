use crate::*;
use crate::{context::ContextBuilder, mailbox::Mailbox};
use crossbeam_channel::{Sender, bounded};
use std::{
    cell::Cell,
    marker::PhantomData,
    sync::Arc,
    thread::{self, JoinHandle},
    time::Instant,
};

pub(crate) type Reply<T> = Sender<Result<T>>;
pub(crate) enum Command {
    Start(UserInput, Arc<Mailbox>, Reply<()>),
    Finish(TurnId, PlaybackStop, ReceiptSource, Reply<FinalizedTurn>),
    Reset(Reply<()>),
    Retry(Reply<FinalizedTurn>),
    Close(Reply<()>),
}
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

pub struct Llm {
    commands: Sender<Command>,
    worker: Option<JoinHandle<()>>,
    active: Option<Arc<Mailbox>>,
    next_id: Option<TurnId>,
    _not_sync: PhantomData<Cell<()>>,
}
impl Llm {
    pub fn open(config: OpenConfig) -> Result<Self> {
        Profile::validate_config(&config)?;
        let (commands, receiver) = bounded(policy::CONTROL_QUEUE);
        let (ready, startup) = bounded(1);
        let worker = thread::Builder::new()
            .name("teto-llm-inference".into())
            .spawn(move || {
                if let Err(error) = crate::native::run(config, receiver, ready.clone()) {
                    let _ = ready.send(Err(error));
                }
            })
            .map_err(|_| failure(ErrorKind::Worker))?;
        match startup.recv().map_err(|_| failure(ErrorKind::Worker))? {
            Ok(()) => Ok(Self {
                commands,
                worker: Some(worker),
                active: None,
                next_id: Some(TurnId::new(1)?),
                _not_sync: PhantomData,
            }),
            Err(error) => {
                let _ = worker.join();
                Err(error)
            }
        }
    }
    pub fn start_turn(&mut self, input: UserInput) -> Result<Turn> {
        if let Some(active) = &self.active {
            active.cancel();
            return Err(failure(ErrorKind::PlaybackPending));
        }
        let id = self.next_id.ok_or_else(|| failure(ErrorKind::Worker))?;
        self.next_id = id.next().ok();
        let mailbox = Arc::new(Mailbox::new(id));
        let (reply, receive) = bounded(1);
        self.commands
            .send(Command::Start(input, Arc::clone(&mailbox), reply))
            .map_err(|_| failure(ErrorKind::Worker))?;
        receive.recv().map_err(|_| failure(ErrorKind::Worker))??;
        self.active = Some(Arc::clone(&mailbox));
        Ok(Turn {
            mailbox,
            _not_sync: PhantomData,
        })
    }
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

    /// Seal an abandoned/dropped consumer after the orchestrator drains its receipt fence.
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
        let record = receive.recv().map_err(|_| failure(ErrorKind::Worker))??;
        self.active = None;
        Ok(record)
    }
    pub fn reset_conversation(&mut self) -> Result<()> {
        if let Some(active) = &self.active {
            active.cancel();
            return Err(failure(ErrorKind::PlaybackPending));
        }
        let (reply, receive) = bounded(1);
        self.commands
            .send(Command::Reset(reply))
            .map_err(|_| failure(ErrorKind::Worker))?;
        receive.recv().map_err(|_| failure(ErrorKind::Worker))?
    }
    pub fn retry_pending(&mut self) -> Result<FinalizedTurn> {
        let (reply, receive) = bounded(1);
        self.commands
            .send(Command::Retry(reply))
            .map_err(|_| failure(ErrorKind::Worker))?;
        let record = receive.recv().map_err(|_| failure(ErrorKind::Worker))??;
        self.active = None;
        Ok(record)
    }
    pub fn close(&mut self) -> Result<()> {
        if let Some(active) = &self.active {
            active.cancel();
            return Err(failure(ErrorKind::PlaybackPending));
        }
        let (reply, receive) = bounded(1);
        self.commands
            .send(Command::Close(reply))
            .map_err(|_| failure(ErrorKind::Worker))?;
        receive.recv().map_err(|_| failure(ErrorKind::Worker))??;
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

pub(crate) struct Active {
    pub input: UserInput,
    pub mailbox: Arc<Mailbox>,
}
pub(crate) enum PendingRecord {
    Unwritten(FinalizedTurn),
    Durable(FinalizedTurn),
}
impl PendingRecord {
    fn record(&self) -> &FinalizedTurn {
        match self {
            Self::Unwritten(r) | Self::Durable(r) => r,
        }
    }
}

pub(crate) struct Session {
    pub context: ContextBuilder,
    pub active: Option<Active>,
    pub pending: Option<PendingRecord>,
    pub journal: Option<journal::Journal>,
    pub session: [u8; 16],
    pub profile: &'static str,
}
impl Session {
    pub fn finalize(
        &mut self,
        id: TurnId,
        playback: PlaybackStop,
        source: ReceiptSource,
        model: &llama_cpp_2::model::LlamaModel,
    ) -> Result<FinalizedTurn> {
        if source == ReceiptSource::TestSink && self.journal.is_some() {
            return Err(failure(ErrorKind::Receipt));
        }
        if self.pending.is_some() {
            return Err(failure(ErrorKind::Persistence));
        }
        let active = self
            .active
            .as_ref()
            .ok_or_else(|| failure(ErrorKind::Receipt))?;
        if active.mailbox.id != id {
            return Err(failure(ErrorKind::Receipt));
        }
        let (assistant, generation_stop, metrics) = active.mailbox.seal(playback)?;
        let active = self.active.take().expect("validated active turn");
        self.pending = Some(PendingRecord::Unwritten(FinalizedTurn {
            schema_version: 1,
            policy_version: policy::VERSION,
            session: self.session,
            turn: id,
            profile: self.profile.into(),
            user: active.input,
            assistant,
            generation_stop,
            playback_stop: playback,
            partial: generation_stop != GenerationStop::Eos || playback != PlaybackStop::Complete,
            receipt_source: source,
            metrics,
        }));
        self.persist()?;
        self.commit(model)
    }
    pub fn persist(&mut self) -> Result<()> {
        let record = self
            .pending
            .as_ref()
            .ok_or_else(|| failure(ErrorKind::Persistence))?;
        if matches!(record, PendingRecord::Durable(_)) {
            return Ok(());
        }
        let started = Instant::now();
        if let Some(journal) = &mut self.journal {
            journal.retry_record(record.record())?;
        }
        let pending = self.pending.take().expect("validated pending record");
        if let PendingRecord::Unwritten(mut record) = pending {
            record.metrics.journal_ms = started.elapsed().as_secs_f64() * 1000.0;
            self.pending = Some(PendingRecord::Durable(record));
        }
        Ok(())
    }
    pub fn commit(&mut self, model: &llama_cpp_2::model::LlamaModel) -> Result<FinalizedTurn> {
        let record = self
            .pending
            .as_ref()
            .ok_or_else(|| failure(ErrorKind::Persistence))?;
        if !matches!(record, PendingRecord::Durable(_)) {
            return Err(failure(ErrorKind::Persistence));
        }
        self.context.commit(model, record.record())?;
        match self.pending.take().expect("validated pending record") {
            PendingRecord::Durable(record) => Ok(record),
            PendingRecord::Unwritten(_) => unreachable!("commit requires durable acknowledgment"),
        }
    }
}
