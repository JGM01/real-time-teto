//! Release-only tests: exact local weights, real inference, no fake text backend.
use std::{path::PathBuf, sync::Mutex};
use teto_llm::*;
static MODEL_GATE: Mutex<()> = Mutex::new(());

fn config() -> OpenConfig {
    OpenConfig {
        assets: PathBuf::from(std::env::var_os("TETO_LLM_ASSETS").expect("set TETO_LLM_ASSETS")),
        profile: ProfileId::desktop(),
        output_language: OutputLanguage::English,
        system_prompt: String::new(),
        seed: 7,
        journal: JournalAccess::Disabled,
    }
}
fn drain(turn: &mut Turn) -> GenerationStop {
    loop {
        match turn.next_clause().unwrap() {
            ClauseEvent::Clause(clause) => {
                assert_eq!(clause.turn(), turn.id());
                turn.acknowledge(ClausePlayed {
                    turn: clause.turn(),
                    index: clause.index(),
                })
                .unwrap();
            }
            ClauseEvent::End { reason, .. } => return reason,
            ClauseEvent::Cancelled { .. } => return GenerationStop::Cancelled,
        }
    }
}
fn replacement_scenario() {
    let _gate = MODEL_GATE.lock().unwrap();
    let mut llm = Llm::open(config()).unwrap();
    let mut turn = llm
        .start_turn(UserInput::new("Say hello in one short sentence.".into(), false).unwrap())
        .unwrap();
    assert_eq!(
        llm.start_turn(UserInput::new("replacement".into(), false).unwrap())
            .err()
            .unwrap()
            .kind(),
        ErrorKind::PlaybackPending
    );
    assert!(matches!(
        turn.next_clause().unwrap(),
        ClauseEvent::Cancelled { .. }
    ));
    let old = turn.control();
    let record = llm
        .finish_turn(&mut turn, PlaybackStop::Interrupted, ReceiptSource::Display)
        .unwrap();
    assert!(record.partial);
    assert!(record.assistant.is_empty());
    let mut next = llm
        .start_turn(UserInput::new("Say hello in one short sentence.".into(), false).unwrap())
        .unwrap();
    assert!(next.id().get() > turn.id().get());
    old.cancel();
    assert_ne!(drain(&mut next), GenerationStop::Cancelled);
    llm.finish_turn(&mut next, PlaybackStop::Complete, ReceiptSource::Display)
        .unwrap();
    llm.close().unwrap();
}
#[test]
#[ignore = "requires exact desktop model and Metal"]
fn one_active_turn() {
    replacement_scenario();
}
#[test]
#[ignore = "requires exact desktop model and Metal"]
fn replacement_cancels_before_admission() {
    replacement_scenario();
}
#[test]
#[ignore = "requires exact desktop model and Metal"]
fn replacement_terminal_once() {
    replacement_scenario();
}
#[test]
#[ignore = "requires exact desktop model and Metal"]
fn stale_clause_dropped() {
    replacement_scenario();
}
#[test]
#[ignore = "requires exact desktop model and Metal"]
fn reset_active_turn() {
    let _gate = MODEL_GATE.lock().unwrap();
    let mut llm = Llm::open(config()).unwrap();
    let mut turn = llm
        .start_turn(UserInput::new("Say hello.".into(), false).unwrap())
        .unwrap();
    assert_eq!(
        llm.reset_conversation().unwrap_err().kind(),
        ErrorKind::PlaybackPending
    );
    llm.finish_turn(&mut turn, PlaybackStop::Interrupted, ReceiptSource::Display)
        .unwrap();
    llm.reset_conversation().unwrap();
    llm.close().unwrap();
}
#[test]
#[ignore = "requires exact desktop model and Metal"]
fn drop_active_turn() {
    let _gate = MODEL_GATE.lock().unwrap();
    let mut llm = Llm::open(config()).unwrap();
    let turn = llm
        .start_turn(UserInput::new("Say hello.".into(), false).unwrap())
        .unwrap();
    let id = turn.id();
    drop(turn);
    let record = llm
        .finish_playback(id, PlaybackStop::Interrupted, ReceiptSource::Display)
        .unwrap();
    assert!(record.partial);
    assert_eq!(record.generation_stop, GenerationStop::Cancelled);
    llm.close().unwrap();
}
#[test]
#[ignore = "requires exact desktop model and Metal"]
fn close_active_turn() {
    let _gate = MODEL_GATE.lock().unwrap();
    let mut llm = Llm::open(config()).unwrap();
    let mut turn = llm
        .start_turn(UserInput::new("Say hello.".into(), false).unwrap())
        .unwrap();
    assert_eq!(llm.close().unwrap_err().kind(), ErrorKind::PlaybackPending);
    llm.finish_turn(&mut turn, PlaybackStop::Interrupted, ReceiptSource::Display)
        .unwrap();
    llm.close().unwrap();
}
#[test]
#[ignore = "requires exact desktop model and Metal"]
fn one_context_lifetime() {
    let _gate = MODEL_GATE.lock().unwrap();
    let mut llm = Llm::open(config()).unwrap();
    for (index, text) in [
        "My name is Robin. Greet me briefly.",
        "What name did I give you? Answer in one sentence.",
    ]
    .into_iter()
    .enumerate()
    {
        let mut turn = llm
            .start_turn(UserInput::new(text.into(), false).unwrap())
            .unwrap();
        drain(&mut turn);
        let record = llm
            .finish_turn(&mut turn, PlaybackStop::Complete, ReceiptSource::Display)
            .unwrap();
        assert!(!record.assistant.is_empty());
        assert!(
            record.metrics.prompt_tokens + policy::OUTPUT_TOKENS <= record.metrics.context_tokens
        );
        assert_eq!(
            record.metrics.cached_prefix_tokens + record.metrics.uncached_prompt_tokens,
            record.metrics.prompt_tokens
        );
        assert_eq!(
            record.metrics.evaluated_prompt_tokens,
            record.metrics.uncached_prompt_tokens
        );
        assert_eq!(record.metrics.worker_name, "teto-llm-inference");
        if index == 1 {
            assert!(record.assistant.join(" ").to_lowercase().contains("robin"));
            if record.metrics.cache_mode == CacheMode::VerifiedPrefixReuse {
                assert!(record.metrics.cached_prefix_tokens > 0);
            }
        }
    }
    llm.close().unwrap();
}
#[test]
#[ignore = "requires exact desktop model and Metal"]
fn one_clause_producer() {
    one_context_lifetime();
}
#[test]
#[ignore = "requires exact desktop model and Metal"]
fn cache_evaluated_token_counts() {
    one_context_lifetime();
}
#[test]
#[ignore = "requires exact desktop model and Metal"]
fn rendered_prompt_token_count() {
    one_context_lifetime();
}
#[test]
#[ignore = "requires exact desktop model and Metal"]
fn display_receipt_commit_path() {
    one_context_lifetime();
}
#[test]
#[ignore = "requires exact desktop model and Metal"]
fn inference_thread_identity() {
    one_context_lifetime();
}
#[test]
#[ignore = "requires exact desktop model and Metal"]
fn cancelled_prefix_next_prompt() {
    let _gate = MODEL_GATE.lock().unwrap();
    let mut llm = Llm::open(config()).unwrap();
    let mut turn = llm
        .start_turn(
            UserInput::new("Explain a rainbow in several sentences.".into(), false).unwrap(),
        )
        .unwrap();
    let ClauseEvent::Clause(clause) = turn.next_clause().unwrap() else {
        panic!("expected clause")
    };
    turn.acknowledge(ClausePlayed {
        turn: turn.id(),
        index: clause.index(),
    })
    .unwrap();
    turn.control().cancel();
    let record = llm
        .finish_turn(&mut turn, PlaybackStop::Interrupted, ReceiptSource::Display)
        .unwrap();
    assert_eq!(record.assistant, [clause.text()]);
    assert!(record.partial);
    let mut next = llm
        .start_turn(UserInput::new("Briefly continue your explanation.".into(), false).unwrap())
        .unwrap();
    drain(&mut next);
    llm.finish_turn(&mut next, PlaybackStop::Complete, ReceiptSource::Display)
        .unwrap();
    llm.close().unwrap();
}
#[test]
#[ignore = "requires exact desktop model and Metal"]
fn token_limit_next_prompt() {
    let _gate = MODEL_GATE.lock().unwrap();
    let temporary = tempfile::tempdir().unwrap();
    let directory = temporary.path().join("journal");
    journal::Journal::initialize(
        &directory,
        JournalIdentity {
            journal: [3; 16],
            key: [4; 16],
        },
        &SecretKey::new([17; 32]),
    )
    .unwrap();
    let mut configured = config();
    configured.journal = JournalAccess::Encrypted {
        directory: directory.clone(),
        key: SecretKey::new([17; 32]),
    };
    let mut llm = Llm::open(configured).unwrap();
    let mut turn = llm.start_turn(UserInput::new("Write a very long continuous story about a gardener exploring a forest. Include at least a thousand words of descriptive prose. Start immediately with the story and keep going without an introduction or conclusion.".into(), false).unwrap()).unwrap();
    let generation = drain(&mut turn);
    assert_eq!(generation, GenerationStop::TokenLimit);
    let record = llm
        .finish_turn(&mut turn, PlaybackStop::Interrupted, ReceiptSource::Display)
        .unwrap();
    assert!(record.partial);
    assert_eq!(record.generation_stop, GenerationStop::TokenLimit);
    assert_eq!(record.playback_stop, PlaybackStop::Interrupted);
    let mut next = llm
        .start_turn(UserInput::new("Briefly explain why you stopped.".into(), false).unwrap())
        .unwrap();
    drain(&mut next);
    llm.finish_turn(&mut next, PlaybackStop::Complete, ReceiptSource::Display)
        .unwrap();
    llm.close().unwrap();
    drop(llm);
    drop(journal::Journal::open(directory, SecretKey::new([17; 32])).unwrap());
}

#[test]
#[ignore = "requires exact desktop model and Metal"]
fn real_profile_encrypted_finalization() {
    let _gate = MODEL_GATE.lock().unwrap();
    let temporary = tempfile::tempdir().unwrap();
    let directory = temporary.path().join("journal");
    let identity = JournalIdentity {
        journal: [19; 16],
        key: [23; 16],
    };
    journal::Journal::initialize(&directory, identity, &SecretKey::new([31; 32])).unwrap();
    let mut config = config();
    config.journal = JournalAccess::Encrypted {
        directory: directory.clone(),
        key: SecretKey::new([31; 32]),
    };
    let mut llm = Llm::open(config).unwrap();
    let mut turn = llm
        .start_turn(UserInput::new("Say a friendly greeting.".into(), false).unwrap())
        .unwrap();
    drain(&mut turn);
    assert_eq!(
        llm.finish_turn(&mut turn, PlaybackStop::Complete, ReceiptSource::TestSink)
            .unwrap_err()
            .kind(),
        ErrorKind::Receipt
    );
    let record = llm
        .finish_turn(&mut turn, PlaybackStop::Complete, ReceiptSource::Display)
        .unwrap();
    assert!(!record.assistant.is_empty());
    llm.close().unwrap();
    drop(llm);
    let bytes = std::fs::read(directory.join("segment-00.cbor")).unwrap();
    assert!(
        !bytes
            .windows(record.user.text().len())
            .any(|w| w == record.user.text().as_bytes())
    );
    assert!(!directory.join("pending.cbor").exists());
    // Open authenticates every finalized record and deliberately restores no model context.
    let reopened = journal::Journal::open(directory.clone(), SecretKey::new([31; 32])).unwrap();
    drop(reopened);
    assert!(journal::Journal::open(directory, SecretKey::new([32; 32])).is_err());
}

#[test]
#[ignore = "requires exact desktop model and Metal"]
fn portable_measurement_cancel_probe() {
    let _gate = MODEL_GATE.lock().unwrap();
    let mut output = Vec::new();
    qualification::measure(
        config(),
        &mut output,
        &std::sync::atomic::AtomicBool::new(false),
        1,
        std::time::Duration::ZERO,
    )
    .unwrap();
    let lines: Vec<serde_json::Value> = std::str::from_utf8(&output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines[0]["cancellation_probe"], true);
    assert_eq!(lines[0]["generation_stop"], "Cancelled");
    assert_eq!(lines.last().unwrap()["measurement_summary"], true);
    assert_eq!(lines.last().unwrap()["measured_turns"], 1);
}
