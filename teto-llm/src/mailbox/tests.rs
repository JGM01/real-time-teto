use super::*;
use std::{
    sync::{Arc, Barrier},
    thread,
};

fn mailbox() -> Arc<Mailbox> {
    Arc::new(Mailbox::new(TurnId::new(1).unwrap()))
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
        Boundary::Sentence,
    )
    .unwrap()
}
fn receipt(index: u32) -> ClausePlayed {
    ClausePlayed {
        turn: TurnId::new(1).unwrap(),
        index: ClauseIndex::new(index).unwrap(),
    }
}
#[test]
fn receive_after_terminal_errors() {
    let m = mailbox();
    m.end(GenerationStop::Eos, None, TurnMetrics::default());
    assert!(matches!(m.next().unwrap(), ClauseEvent::End { .. }));
    assert_eq!(m.next().unwrap_err().kind(), ErrorKind::Exhausted);
}
#[test]
fn no_clause_after_terminal() {
    receive_after_terminal_errors();
}
#[test]
fn repeated_cancel() {
    let m = mailbox();
    m.publish(clause(0)).unwrap();
    m.cancel();
    m.cancel();
    assert!(matches!(m.next().unwrap(), ClauseEvent::Cancelled { .. }));
    assert!(m.next().is_err());
}
#[test]
fn cancel_is_not_failure() {
    repeated_cancel();
}
#[test]
fn slow_consumer_bound() {
    let m = mailbox();
    m.publish(clause(0)).unwrap();
    m.publish(clause(1)).unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let producer = Arc::clone(&m);
    let entered = Arc::clone(&barrier);
    let thread = thread::spawn(move || {
        entered.wait();
        producer.publish(clause(2))
    });
    barrier.wait();
    wait_for(&m, true);
    assert_eq!(m.state.lock().unwrap().queue.len(), policy::CLAUSE_QUEUE);
    m.cancel();
    assert_eq!(
        thread.join().unwrap().unwrap_err().kind(),
        ErrorKind::Cancelled
    );
}
#[test]
fn cross_thread_cancel() {
    let m = mailbox();
    let consumer = Arc::clone(&m);
    let barrier = Arc::new(Barrier::new(2));
    let entered = Arc::clone(&barrier);
    let thread = thread::spawn(move || {
        entered.wait();
        consumer.next()
    });
    barrier.wait();
    wait_for(&m, false);
    m.cancel();
    assert!(matches!(
        thread.join().unwrap().unwrap(),
        ClauseEvent::Cancelled { .. }
    ));
}
#[test]
fn receive_waits_and_wakes() {
    cross_thread_cancel();
}
#[test]
fn ack_during_decode_and_after_end() {
    let m = mailbox();
    m.publish(clause(0)).unwrap();
    m.next().unwrap();
    m.acknowledge(receipt(0)).unwrap();
    m.publish(clause(1)).unwrap();
    m.end(GenerationStop::Eos, None, TurnMetrics::default());
    m.next().unwrap();
    m.acknowledge(receipt(1)).unwrap();
    m.next().unwrap();
    assert_eq!(m.seal(PlaybackStop::Complete).unwrap().0.len(), 2);
}
#[test]
fn receipt_authorizes_commit() {
    let m = mailbox();
    m.publish(clause(0)).unwrap();
    assert!(m.acknowledge(receipt(0)).is_err());
    m.next().unwrap();
    assert!(m.acknowledge(receipt(1)).is_err());
    m.acknowledge(receipt(0)).unwrap();
    m.acknowledge(receipt(0)).unwrap();
    m.end(GenerationStop::TokenLimit, None, TurnMetrics::default());
    assert_eq!(m.seal(PlaybackStop::Interrupted).unwrap().0.len(), 1);
}
#[test]
fn finish_waits_for_receipt_fence() {
    let m = mailbox();
    m.publish(clause(0)).unwrap();
    m.next().unwrap();
    m.end(GenerationStop::Eos, None, TurnMetrics::default());
    assert!(m.seal(PlaybackStop::Complete).is_err());
    m.acknowledge(receipt(0)).unwrap();
    m.next().unwrap();
    m.seal(PlaybackStop::Complete).unwrap();
}
#[test]
fn late_receipt_rejected() {
    let m = mailbox();
    m.end(GenerationStop::Cancelled, None, TurnMetrics::default());
    m.seal(PlaybackStop::Interrupted).unwrap();
    assert!(m.acknowledge(receipt(0)).is_err());
}
#[test]
fn cancelled_played_prefix() {
    let m = mailbox();
    m.publish(clause(0)).unwrap();
    m.next().unwrap();
    m.acknowledge(receipt(0)).unwrap();
    m.publish(clause(1)).unwrap();
    m.cancel();
    m.end(GenerationStop::Cancelled, None, TurnMetrics::default());
    let (text, stop, _) = m.seal(PlaybackStop::Interrupted).unwrap();
    assert_eq!(text, ["Clause zero."]);
    assert_eq!(stop, GenerationStop::Cancelled);
}
#[test]
fn unacknowledged_not_in_history() {
    cancelled_played_prefix();
}
#[test]
fn unplayed_sentinel_absent() {
    cancelled_played_prefix();
}
#[test]
fn cancelled_never_complete() {
    let m = mailbox();
    m.cancel();
    m.end(GenerationStop::Cancelled, None, TurnMetrics::default());
    assert!(m.seal(PlaybackStop::Complete).is_err());
}
#[test]
fn token_limit_then_playback_interrupted() {
    let m = mailbox();
    m.end(GenerationStop::TokenLimit, None, TurnMetrics::default());
    let (_, generation, _) = m.seal(PlaybackStop::Interrupted).unwrap();
    assert_eq!(generation, GenerationStop::TokenLimit);
}
#[test]
fn answer_and_clause_timestamps() {
    let m = mailbox();
    m.publish(clause(0)).unwrap();
    m.next().unwrap();
    m.acknowledge(receipt(0)).unwrap();
    m.end(
        GenerationStop::Eos,
        None,
        TurnMetrics {
            first_answer_ms: Some(0.0),
            ..Default::default()
        },
    );
    m.next().unwrap();
    let (_, _, metrics) = m.seal(PlaybackStop::Complete).unwrap();
    assert!(metrics.first_clause_ms.unwrap() >= metrics.first_answer_ms.unwrap());
}

#[test]
fn worker_failure_wakes_receiver_as_failure() {
    let mailbox = mailbox();
    mailbox.worker_failed();
    mailbox.cancel();
    assert_eq!(mailbox.next().unwrap_err().kind(), ErrorKind::Worker);
    assert_eq!(mailbox.next().unwrap_err().kind(), ErrorKind::Exhausted);
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
fn receipt_racing_seal_has_one_linearized_result() {
    let m = mailbox();
    m.publish(clause(0)).unwrap();
    m.next().unwrap();
    m.end(GenerationStop::Eos, None, TurnMetrics::default());
    let gate = Arc::new(Barrier::new(3));
    let ack_mailbox = Arc::clone(&m);
    let ack_gate = Arc::clone(&gate);
    let ack = std::thread::spawn(move || {
        ack_gate.wait();
        ack_mailbox.acknowledge(receipt(0))
    });
    let seal_mailbox = Arc::clone(&m);
    let seal_gate = Arc::clone(&gate);
    let seal = std::thread::spawn(move || {
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
    assert!(m.acknowledge(receipt(0)).is_err());
}
