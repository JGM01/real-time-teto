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
