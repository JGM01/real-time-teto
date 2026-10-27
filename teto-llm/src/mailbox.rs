use crate::*;
use std::{
    collections::VecDeque,
    sync::{Condvar, Mutex, MutexGuard},
    time::Instant,
};

pub(crate) struct Mailbox {
    pub id: TurnId,
    state: Mutex<State>,
    changed: Condvar,
    started: Instant,
}
struct State {
    queue: VecDeque<Clause>,
    ledger: Vec<String>,
    bytes: usize,
    delivered: usize,
    acknowledged: usize,
    stop: Option<GenerationStop>,
    failure: Option<Error>,
    cancel: Option<Instant>,
    terminal_seen: bool,
    sealed: bool,
    metrics: TurnMetrics,
    #[cfg(test)]
    producer_waiting: bool,
    #[cfg(test)]
    consumer_waiting: bool,
}
impl Mailbox {
    pub fn started(&self) -> Instant {
        self.started
    }
    pub fn worker_failed(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.queue.clear();
        state.stop = Some(GenerationStop::Failed);
        state.failure = Some(failure(ErrorKind::Worker));
        state.cancel = None;
        self.state.clear_poison();
        self.changed.notify_all();
    }
    pub fn new(id: TurnId) -> Self {
        Self {
            id,
            state: Mutex::new(State {
                queue: VecDeque::with_capacity(policy::CLAUSE_QUEUE),
                ledger: Vec::new(),
                bytes: 0,
                delivered: 0,
                acknowledged: 0,
                stop: None,
                failure: None,
                cancel: None,
                terminal_seen: false,
                sealed: false,
                metrics: TurnMetrics::default(),
                #[cfg(test)]
                producer_waiting: false,
                #[cfg(test)]
                consumer_waiting: false,
            }),
            changed: Condvar::new(),
            started: Instant::now(),
        }
    }
    pub fn cancel(&self) {
        let mut state = self.state.lock().expect("mailbox lock");
        if state
            .failure
            .as_ref()
            .is_some_and(|error| error.kind() == ErrorKind::Worker)
        {
            return;
        }
        if !state.sealed && state.cancel.is_none() {
            state.cancel = Some(Instant::now());
            state.queue.clear();
        }
        self.changed.notify_all();
    }
    pub fn cancelled(&self) -> bool {
        self.state.lock().expect("mailbox lock").cancel.is_some()
    }
    fn capacity(&self) -> Result<MutexGuard<'_, State>> {
        let started = Instant::now();
        let mut state = self.state.lock().expect("mailbox lock");
        let waiting = state.queue.len() == policy::CLAUSE_QUEUE;
        while state.queue.len() == policy::CLAUSE_QUEUE && state.cancel.is_none() {
            #[cfg(test)]
            {
                state.producer_waiting = true;
                self.changed.notify_all();
            }
            state = self.changed.wait(state).expect("mailbox wait");
            #[cfg(test)]
            {
                state.producer_waiting = false;
            }
        }
        if waiting {
            state.metrics.queue_wait_ms += started.elapsed().as_secs_f64() * 1000.0;
        }
        if state.cancel.is_some() {
            return Err(failure(ErrorKind::Cancelled));
        }
        Ok(state)
    }
    pub fn wait_for_capacity(&self) -> Result<()> {
        drop(self.capacity()?);
        Ok(())
    }
    pub fn publish(&self, clause: Clause) -> Result<()> {
        let mut state = self.capacity()?;
        assert!(state.stop.is_none() && !state.sealed);
        assert_eq!(clause.index().get() as usize, state.ledger.len());
        let bytes = state
            .bytes
            .checked_add(clause.text().len())
            .ok_or_else(|| failure(ErrorKind::Normalization))?;
        if bytes > policy::TURN_BYTES {
            return Err(failure(ErrorKind::Normalization));
        }
        state.bytes = bytes;
        state.ledger.push(clause.text().into());
        state.queue.push_back(clause);
        state.metrics.queue_high_water = state.metrics.queue_high_water.max(state.queue.len());
        self.changed.notify_all();
        Ok(())
    }
    pub fn next(&self) -> Result<ClauseEvent> {
        let mut state = self.state.lock().expect("mailbox lock");
        if state.terminal_seen {
            return Err(failure(ErrorKind::Exhausted));
        }
        loop {
            if state
                .failure
                .as_ref()
                .is_some_and(|error| error.kind() == ErrorKind::Worker)
            {
                state.terminal_seen = true;
                return Err(state.failure.take().expect("worker failure"));
            }
            if state.cancel.is_some() {
                state.terminal_seen = true;
                return Ok(ClauseEvent::Cancelled { turn: self.id });
            }
            if state.delivered == state.acknowledged {
                if let Some(clause) = state.queue.pop_front() {
                    let elapsed = self.started.elapsed().as_secs_f64() * 1000.0;
                    state.metrics.first_clause_ms.get_or_insert(elapsed);
                    state.metrics.last_clause_ms = Some(elapsed);
                    state.delivered += 1;
                    self.changed.notify_all();
                    return Ok(ClauseEvent::Clause(clause));
                }
                if let Some(reason) = state.stop {
                    state.terminal_seen = true;
                    if let Some(error) = state.failure.take() {
                        return Err(error);
                    }
                    return Ok(ClauseEvent::End {
                        turn: self.id,
                        reason,
                    });
                }
            }
            #[cfg(test)]
            {
                state.consumer_waiting = true;
                self.changed.notify_all();
            }
            state = self.changed.wait(state).expect("mailbox wait");
            #[cfg(test)]
            {
                state.consumer_waiting = false;
            }
        }
    }
    pub fn acknowledge(&self, receipt: ClausePlayed) -> Result<()> {
        let mut state = self.state.lock().expect("mailbox lock");
        let index = receipt.index.get() as usize;
        if receipt.turn != self.id || state.sealed || index >= state.delivered {
            return Err(failure(ErrorKind::Receipt));
        }
        if index < state.acknowledged {
            return Ok(());
        }
        if index != state.acknowledged {
            return Err(failure(ErrorKind::Receipt));
        }
        state.acknowledged += 1;
        self.changed.notify_all();
        Ok(())
    }
    pub fn end(&self, stop: GenerationStop, failure: Option<Error>, mut metrics: TurnMetrics) {
        let mut state = self.state.lock().expect("mailbox lock");
        assert!(state.stop.is_none());
        metrics.queue_wait_ms = state.metrics.queue_wait_ms;
        metrics.worker_name = std::thread::current()
            .name()
            .unwrap_or("unnamed")
            .to_owned();
        metrics.queue_high_water = state.metrics.queue_high_water;
        metrics.first_clause_ms = state.metrics.first_clause_ms;
        metrics.last_clause_ms = state.metrics.last_clause_ms;
        metrics.generation_ms = (metrics.generation_ms - metrics.queue_wait_ms).max(0.0);
        metrics.cancel_ms = state
            .cancel
            .map(|start| start.elapsed().as_secs_f64() * 1000.0);
        metrics.clauses = state.ledger.len() as u32;
        metrics.characters = state.ledger.iter().map(|s| s.chars().count() as u32).sum();
        state.metrics = metrics;
        state.stop = Some(stop);
        state.failure = failure;
        self.changed.notify_all();
    }
    pub fn seal(
        &self,
        playback: PlaybackStop,
    ) -> Result<(Vec<String>, GenerationStop, TurnMetrics)> {
        let mut state = self.state.lock().expect("mailbox lock");
        if state.sealed {
            return Err(failure(ErrorKind::Receipt));
        }
        let stop = state
            .stop
            .ok_or_else(|| failure(ErrorKind::PlaybackPending))?;
        if playback == PlaybackStop::Complete
            && (!state.terminal_seen || state.acknowledged != state.ledger.len())
        {
            return Err(failure(ErrorKind::PlaybackPending));
        }
        state.sealed = true;
        let acknowledged = state.acknowledged;
        let mut text = std::mem::take(&mut state.ledger);
        text.truncate(acknowledged);
        state.queue.clear();
        Ok((text, stop, state.metrics.clone()))
    }
}

#[cfg(test)]
mod tests;
