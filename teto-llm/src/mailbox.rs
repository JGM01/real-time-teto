use crate::*;
use std::{
    sync::{Condvar, Mutex, MutexGuard},
    time::Instant,
};

/// The lifecycle of one turn.
enum Phase {
    Streaming,
    Ending {
        stop: GenerationStop,
        failure: Option<Error>,
    },
    Sealed,
}

/// The result of one non-blocking state step.
enum Poll {
    Event(Result<ClauseEvent>),
    Wait,
}

/// The mailbox owns the clause queue, the played prefix, and the turn outcome.
pub(crate) struct Mailbox {
    pub id: TurnId,
    state: Mutex<State>,
    changed: Condvar,
    started: Instant,
}

struct State {
    ready: [Option<Clause>; policy::CLAUSE_QUEUE],
    ready_len: usize,
    delivered: [Option<Clause>; 2],
    delivered_len: usize,
    window: usize,
    played: Vec<String>,
    published: u32,
    bytes: usize,
    characters: u32,
    phase: Phase,
    cancel: Option<Instant>,
    consumer_saw_terminal: bool,
    metrics: TurnMetrics,
    #[cfg(test)]
    producer_waiting: bool,
    #[cfg(test)]
    consumer_waiting: bool,
}

impl State {
    fn new(window: usize) -> Self {
        debug_assert!((1..=2).contains(&window));
        Self {
            ready: [None, None],
            ready_len: 0,
            delivered: [None, None],
            delivered_len: 0,
            window,
            played: Vec::with_capacity(policy::TURN_CLAUSES),
            published: 0,
            bytes: 0,
            characters: 0,
            phase: Phase::Streaming,
            cancel: None,
            consumer_saw_terminal: false,
            metrics: TurnMetrics::default(),
            #[cfg(test)]
            producer_waiting: false,
            #[cfg(test)]
            consumer_waiting: false,
        }
    }

    fn streaming(&self) -> bool {
        matches!(self.phase, Phase::Streaming)
    }

    fn push(&mut self, clause: Clause) -> Result<()> {
        if !self.streaming() {
            return Err(failure(ErrorKind::Worker));
        }
        let text = clause.text();
        let bytes = self
            .bytes
            .checked_add(text.len())
            .ok_or_else(|| failure(ErrorKind::Normalization))?;
        if bytes > policy::TURN_BYTES {
            return Err(failure(ErrorKind::Normalization));
        }
        debug_assert_eq!(clause.index().get(), self.published);
        debug_assert!(self.ready_len < policy::CLAUSE_QUEUE);
        self.bytes = bytes;
        self.characters = self.characters.saturating_add(text.chars().count() as u32);
        self.ready[self.ready_len] = Some(clause);
        self.ready_len += 1;
        self.published += 1;
        self.metrics.queue_high_water = self.metrics.queue_high_water.max(self.ready_len as u32);
        Ok(())
    }

    fn pop_ready(&mut self) -> Option<Clause> {
        if self.ready_len == 0 {
            return None;
        }
        let clause = self.ready[0].take();
        if self.ready_len == 2 {
            self.ready[0] = self.ready[1].take();
        }
        self.ready_len -= 1;
        clause
    }

    fn poll_next(&mut self, turn: TurnId, elapsed_us: u64) -> Poll {
        if self.consumer_saw_terminal {
            return Poll::Event(Err(failure(ErrorKind::Exhausted)));
        }
        if let Phase::Ending {
            failure: Some(_), ..
        } = &self.phase
        {
            self.consumer_saw_terminal = true;
            if let Phase::Ending { failure, .. } = &mut self.phase {
                let error = failure.take().expect("checked worker failure");
                return Poll::Event(Err(error));
            }
        }
        if self.cancel.is_some() {
            self.consumer_saw_terminal = true;
            return Poll::Event(Ok(ClauseEvent::Cancelled { turn }));
        }
        if self.delivered_len == self.window {
            return Poll::Wait;
        }
        if let Some(clause) = self.pop_ready() {
            self.metrics.first_clause_us.get_or_insert(elapsed_us);
            self.metrics.last_clause_us = Some(elapsed_us);
            self.delivered[self.delivered_len] = Some(clause.clone());
            self.delivered_len += 1;
            return Poll::Event(Ok(ClauseEvent::Clause(clause)));
        }
        if self.delivered_len == 0
            && let Phase::Ending { stop, .. } = &self.phase
        {
            self.consumer_saw_terminal = true;
            return Poll::Event(Ok(ClauseEvent::End {
                turn,
                reason: *stop,
            }));
        }
        Poll::Wait
    }

    fn acknowledge(&mut self, receipt: ClausePlayed, turn: TurnId) -> Result<()> {
        if receipt.turn != turn || matches!(self.phase, Phase::Sealed) {
            return Err(failure(ErrorKind::Receipt));
        }
        let index = receipt.index.get() as usize;
        if index < self.played.len() {
            return Ok(());
        }
        if index != self.played.len() || self.delivered_len == 0 {
            return Err(failure(ErrorKind::Receipt));
        }
        let clause = self.delivered[0]
            .take()
            .ok_or_else(|| failure(ErrorKind::Receipt))?;
        if clause.index() != receipt.index {
            self.delivered[0] = Some(clause);
            return Err(failure(ErrorKind::Receipt));
        }
        if self.delivered_len == 2 {
            self.delivered[0] = self.delivered[1].take();
        }
        self.delivered_len -= 1;
        self.played.push(clause.into_text());
        Ok(())
    }

    fn finish(&mut self, stop: GenerationStop, failure: Option<Error>, mut metrics: TurnMetrics) {
        if !self.streaming() {
            return;
        }
        metrics.queue_wait_us = self.metrics.queue_wait_us;
        metrics.queue_high_water = self.metrics.queue_high_water;
        metrics.first_clause_us = self.metrics.first_clause_us;
        metrics.last_clause_us = self.metrics.last_clause_us;
        metrics.decode_us = metrics.decode_us.saturating_sub(metrics.queue_wait_us);
        metrics.cancel_us = self.cancel.map(|start| elapsed_us(start.elapsed()));
        metrics.clauses = self.published;
        metrics.characters = self.characters;
        debug_assert!(
            metrics.evaluated_prompt_tokens
                <= metrics
                    .prompt_tokens
                    .saturating_sub(metrics.cached_prefix_tokens)
        );
        self.metrics = metrics;
        self.phase = Phase::Ending { stop, failure };
    }

    fn seal(
        &mut self,
        playback: PlaybackStop,
    ) -> Result<(Vec<String>, GenerationStop, TurnMetrics)> {
        let stop = match &self.phase {
            Phase::Sealed => return Err(failure(ErrorKind::Receipt)),
            Phase::Streaming => return Err(failure(ErrorKind::PlaybackPending)),
            Phase::Ending { stop, .. } => *stop,
        };
        if playback == PlaybackStop::Complete
            && (!self.consumer_saw_terminal || self.played.len() != self.published as usize)
        {
            return Err(failure(ErrorKind::PlaybackPending));
        }
        self.ready = [None, None];
        self.ready_len = 0;
        self.delivered = [None, None];
        self.delivered_len = 0;
        self.phase = Phase::Sealed;
        Ok((std::mem::take(&mut self.played), stop, self.metrics.clone()))
    }
}

fn elapsed_us(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

impl Mailbox {
    pub fn new(id: TurnId, window: DeliveryWindow) -> Self {
        Self {
            id,
            state: Mutex::new(State::new(window.get())),
            changed: Condvar::new(),
            started: Instant::now(),
        }
    }

    pub fn started(&self) -> Instant {
        self.started
    }

    pub fn worker_failed(&self) {
        let mut state = self.lock();
        state.ready = [None, None];
        state.ready_len = 0;
        state.phase = Phase::Ending {
            stop: GenerationStop::Failed,
            failure: Some(failure(ErrorKind::Worker)),
        };
        state.cancel = None;
        self.changed.notify_all();
    }

    pub fn cancel(&self) {
        let mut state = self.lock();
        if let Phase::Ending {
            failure: Some(error),
            ..
        } = &state.phase
            && error.kind() == ErrorKind::Worker
        {
            return;
        }
        if !matches!(state.phase, Phase::Sealed) && state.cancel.is_none() {
            state.cancel = Some(Instant::now());
            state.ready = [None, None];
            state.ready_len = 0;
        }
        self.changed.notify_all();
    }

    pub fn cancelled(&self) -> bool {
        self.lock().cancel.is_some()
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn capacity(&self) -> Result<MutexGuard<'_, State>> {
        let started = Instant::now();
        let mut state = self.lock();
        let mut waited = false;
        while state.ready_len == policy::CLAUSE_QUEUE && state.cancel.is_none() && state.streaming()
        {
            waited = true;
            #[cfg(test)]
            {
                state.producer_waiting = true;
                self.changed.notify_all();
            }
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            #[cfg(test)]
            {
                state.producer_waiting = false;
            }
        }
        if waited {
            state.metrics.queue_wait_us = state
                .metrics
                .queue_wait_us
                .saturating_add(elapsed_us(started.elapsed()));
        }
        if state.cancel.is_some() {
            return Err(failure(ErrorKind::Cancelled));
        }
        if !state.streaming() {
            return Err(failure(ErrorKind::Worker));
        }
        Ok(state)
    }

    pub fn wait_for_capacity(&self) -> Result<()> {
        drop(self.capacity()?);
        Ok(())
    }

    pub fn publish(&self, clause: Clause) -> Result<()> {
        let mut state = self.capacity()?;
        state.push(clause)?;
        self.changed.notify_all();
        Ok(())
    }

    pub fn next(&self) -> Result<ClauseEvent> {
        let mut state = self.lock();
        loop {
            let elapsed = elapsed_us(self.started.elapsed());
            match state.poll_next(self.id, elapsed) {
                Poll::Event(event) => {
                    self.changed.notify_all();
                    return event;
                }
                Poll::Wait => {
                    #[cfg(test)]
                    {
                        state.consumer_waiting = true;
                        self.changed.notify_all();
                    }
                    state = self
                        .changed
                        .wait(state)
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    #[cfg(test)]
                    {
                        state.consumer_waiting = false;
                    }
                }
            }
        }
    }

    pub fn acknowledge(&self, receipt: ClausePlayed) -> Result<()> {
        let mut state = self.lock();
        state.acknowledge(receipt, self.id)?;
        self.changed.notify_all();
        Ok(())
    }

    pub fn end(&self, stop: GenerationStop, error: Option<Error>, metrics: TurnMetrics) {
        let mut state = self.lock();
        state.finish(stop, error, metrics);
        self.changed.notify_all();
    }

    pub fn seal(
        &self,
        playback: PlaybackStop,
    ) -> Result<(Vec<String>, GenerationStop, TurnMetrics)> {
        let mut state = self.lock();
        let result = state.seal(playback);
        self.changed.notify_all();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{Arc, Barrier},
        thread,
    };

    fn mailbox() -> Arc<Mailbox> {
        mailbox_with(DeliveryWindow::One)
    }

    fn mailbox_with(window: DeliveryWindow) -> Arc<Mailbox> {
        Arc::new(Mailbox::new(TurnId::new(1).unwrap(), window))
    }

    fn clause(index: u32) -> Clause {
        Clause::new(
            TurnId::new(1).unwrap(),
            ClauseIndex::new(index).unwrap(),
            format!(
                "Clause {}.",
                ["zero", "one", "two", "three"][index as usize]
            )
            .try_into()
            .unwrap(),
        )
        .unwrap()
    }

    fn receipt(index: u32) -> ClausePlayed {
        ClausePlayed {
            turn: TurnId::new(1).unwrap(),
            index: ClauseIndex::new(index).unwrap(),
        }
    }

    fn end(mailbox: &Mailbox, stop: GenerationStop) {
        mailbox.end(stop, None, TurnMetrics::default());
    }

    #[test]
    fn ordered_clauses_carry_turn() {
        let mailbox = mailbox();
        mailbox.publish(clause(0)).unwrap();
        mailbox.publish(clause(1)).unwrap();
        for index in 0..2 {
            let ClauseEvent::Clause(clause) = mailbox.next().unwrap() else {
                panic!("expected clause");
            };
            assert_eq!(clause.index().get(), index);
            assert_eq!(clause.turn().get(), 1);
            mailbox.acknowledge(receipt(index)).unwrap();
        }
    }

    #[test]
    fn terminal_is_single_use() {
        let mailbox = mailbox();
        end(&mailbox, GenerationStop::Eos);
        assert!(matches!(mailbox.next().unwrap(), ClauseEvent::End { .. }));
        assert_eq!(mailbox.next().unwrap_err().kind(), ErrorKind::Exhausted);
    }

    #[test]
    fn cancel_is_idempotent_and_not_failure() {
        let mailbox = mailbox();
        mailbox.publish(clause(0)).unwrap();
        mailbox.cancel();
        mailbox.cancel();
        assert!(matches!(
            mailbox.next().unwrap(),
            ClauseEvent::Cancelled { .. }
        ));
        assert_eq!(mailbox.next().unwrap_err().kind(), ErrorKind::Exhausted);
    }

    #[test]
    fn receipts_are_contiguous_and_idempotent() {
        let mailbox = mailbox();
        mailbox.publish(clause(0)).unwrap();
        assert!(mailbox.acknowledge(receipt(0)).is_err());
        let _ = mailbox.next().unwrap();
        assert!(mailbox.acknowledge(receipt(1)).is_err());
        mailbox.acknowledge(receipt(0)).unwrap();
        mailbox.acknowledge(receipt(0)).unwrap();
        end(&mailbox, GenerationStop::TokenLimit);
        let (played, stop, _) = mailbox.seal(PlaybackStop::Interrupted).unwrap();
        assert_eq!(played, ["Clause zero."]);
        assert_eq!(stop, GenerationStop::TokenLimit);
    }

    #[test]
    fn complete_seal_requires_receipt_fence() {
        let mailbox = mailbox();
        mailbox.publish(clause(0)).unwrap();
        let _ = mailbox.next().unwrap();
        end(&mailbox, GenerationStop::Eos);
        assert!(mailbox.seal(PlaybackStop::Complete).is_err());
        mailbox.acknowledge(receipt(0)).unwrap();
        let _ = mailbox.next().unwrap();
        mailbox.seal(PlaybackStop::Complete).unwrap();
    }

    #[test]
    fn sealed_turn_rejects_receipts() {
        let mailbox = mailbox();
        end(&mailbox, GenerationStop::Cancelled);
        mailbox.seal(PlaybackStop::Interrupted).unwrap();
        assert!(mailbox.acknowledge(receipt(0)).is_err());
    }

    #[test]
    fn cancel_keeps_played_prefix() {
        let mailbox = mailbox();
        mailbox.publish(clause(0)).unwrap();
        let _ = mailbox.next().unwrap();
        mailbox.acknowledge(receipt(0)).unwrap();
        mailbox.publish(clause(1)).unwrap();
        mailbox.cancel();
        end(&mailbox, GenerationStop::Cancelled);
        let (played, stop, _) = mailbox.seal(PlaybackStop::Interrupted).unwrap();
        assert_eq!(played, ["Clause zero."]);
        assert_eq!(stop, GenerationStop::Cancelled);
    }

    #[test]
    fn worker_failure_wakes_consumer() {
        let mailbox = mailbox();
        mailbox.worker_failed();
        mailbox.cancel();
        assert_eq!(mailbox.next().unwrap_err().kind(), ErrorKind::Worker);
        assert_eq!(mailbox.next().unwrap_err().kind(), ErrorKind::Exhausted);
    }

    #[test]
    fn slow_consumer_blocks_producer() {
        let mailbox = mailbox();
        mailbox.publish(clause(0)).unwrap();
        mailbox.publish(clause(1)).unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let producer = Arc::clone(&mailbox);
        let entered = Arc::clone(&barrier);
        let thread = thread::spawn(move || {
            entered.wait();
            producer.publish(clause(2))
        });
        barrier.wait();
        wait_for(&mailbox, true);
        assert_eq!(
            mailbox.state.lock().unwrap().ready_len,
            policy::CLAUSE_QUEUE
        );
        mailbox.cancel();
        assert_eq!(
            thread.join().unwrap().unwrap_err().kind(),
            ErrorKind::Cancelled
        );
    }

    #[test]
    fn cross_thread_cancel_wakes_consumer() {
        let mailbox = mailbox();
        let consumer = Arc::clone(&mailbox);
        let barrier = Arc::new(Barrier::new(2));
        let entered = Arc::clone(&barrier);
        let thread = thread::spawn(move || {
            entered.wait();
            consumer.next()
        });
        barrier.wait();
        wait_for(&mailbox, false);
        mailbox.cancel();
        assert!(matches!(
            thread.join().unwrap().unwrap(),
            ClauseEvent::Cancelled { .. }
        ));
    }

    #[test]
    fn receipt_during_and_after_generation() {
        let mailbox = mailbox();
        mailbox.publish(clause(0)).unwrap();
        let _ = mailbox.next().unwrap();
        mailbox.acknowledge(receipt(0)).unwrap();
        mailbox.publish(clause(1)).unwrap();
        end(&mailbox, GenerationStop::Eos);
        let _ = mailbox.next().unwrap();
        mailbox.acknowledge(receipt(1)).unwrap();
        let _ = mailbox.next().unwrap();
        assert_eq!(mailbox.seal(PlaybackStop::Complete).unwrap().0.len(), 2);
    }

    #[test]
    fn receipt_racing_seal_has_one_linearized_result() {
        let mailbox = mailbox();
        mailbox.publish(clause(0)).unwrap();
        let _ = mailbox.next().unwrap();
        end(&mailbox, GenerationStop::Eos);
        let gate = Arc::new(Barrier::new(3));
        let ack_mailbox = Arc::clone(&mailbox);
        let ack_gate = Arc::clone(&gate);
        let ack = thread::spawn(move || {
            ack_gate.wait();
            ack_mailbox.acknowledge(receipt(0))
        });
        let seal_mailbox = Arc::clone(&mailbox);
        let seal_gate = Arc::clone(&gate);
        let seal = thread::spawn(move || {
            seal_gate.wait();
            seal_mailbox.seal(PlaybackStop::Interrupted).unwrap().0
        });
        gate.wait();
        let acknowledged = ack.join().unwrap();
        let committed = seal.join().unwrap();
        assert_eq!(committed.len(), usize::from(acknowledged.is_ok()));
        if let Err(error) = acknowledged {
            assert_eq!(error.kind(), ErrorKind::Receipt);
        }
        assert!(mailbox.acknowledge(receipt(0)).is_err());
    }

    fn wait_for(mailbox: &Mailbox, producer: bool) {
        let mut state = mailbox.state.lock().unwrap();
        while if producer {
            !state.producer_waiting
        } else {
            !state.consumer_waiting
        } {
            let (next, timeout) = mailbox
                .changed
                .wait_timeout(state, std::time::Duration::from_secs(2))
                .unwrap();
            assert!(
                !timeout.timed_out(),
                "participant never reached its blocking wait"
            );
            state = next;
        }
    }

    #[test]
    fn two_clause_window_allows_prefetch() {
        let mailbox = mailbox_with(DeliveryWindow::Two);
        mailbox.publish(clause(0)).unwrap();
        mailbox.publish(clause(1)).unwrap();
        let ClauseEvent::Clause(first) = mailbox.next().unwrap() else {
            panic!("expected first clause");
        };
        let ClauseEvent::Clause(second) = mailbox.next().unwrap() else {
            panic!("expected second clause");
        };
        assert_eq!(first.index().get(), 0);
        assert_eq!(second.index().get(), 1);
        mailbox.acknowledge(receipt(0)).unwrap();
        mailbox.publish(clause(2)).unwrap();
        let ClauseEvent::Clause(third) = mailbox.next().unwrap() else {
            panic!("expected third clause");
        };
        assert_eq!(third.index().get(), 2);
        mailbox.acknowledge(receipt(1)).unwrap();
        mailbox.acknowledge(receipt(2)).unwrap();
        end(&mailbox, GenerationStop::Eos);
        assert!(matches!(mailbox.next().unwrap(), ClauseEvent::End { .. }));
        assert_eq!(mailbox.seal(PlaybackStop::Complete).unwrap().0.len(), 3);
    }

    #[test]
    fn two_clause_window_keeps_receipts_contiguous() {
        let mailbox = mailbox_with(DeliveryWindow::Two);
        mailbox.publish(clause(0)).unwrap();
        mailbox.publish(clause(1)).unwrap();
        let _ = mailbox.next().unwrap();
        let _ = mailbox.next().unwrap();
        assert_eq!(
            mailbox.acknowledge(receipt(1)).unwrap_err().kind(),
            ErrorKind::Receipt
        );
        mailbox.acknowledge(receipt(0)).unwrap();
        mailbox.acknowledge(receipt(1)).unwrap();
    }

    #[test]
    fn two_clause_window_waits_for_complete_fence() {
        let mailbox = mailbox_with(DeliveryWindow::Two);
        mailbox.publish(clause(0)).unwrap();
        mailbox.publish(clause(1)).unwrap();
        let _ = mailbox.next().unwrap();
        let _ = mailbox.next().unwrap();
        end(&mailbox, GenerationStop::Eos);
        assert!(mailbox.seal(PlaybackStop::Complete).is_err());
        mailbox.acknowledge(receipt(0)).unwrap();
        mailbox.acknowledge(receipt(1)).unwrap();
        assert!(matches!(mailbox.next().unwrap(), ClauseEvent::End { .. }));
        mailbox.seal(PlaybackStop::Complete).unwrap();
    }
}
