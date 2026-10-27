use std::io::{self, Read};
use teto_llm_protocol::*;

#[test]
fn policy_numbers_match_document() {
    let document = include_str!("../../llm-subsystem.md");
    for expected in [
        format!("| Context | {} tokens", policy::CONTEXT_TOKENS),
        format!(
            "| Output reserve | {} generated tokens",
            policy::OUTPUT_TOKENS
        ),
        format!("| Frame / record | {} bytes", policy::FRAME_BYTES),
        format!("| Journal capacity | {} segments", policy::SEGMENTS),
        format!("| Prompt batch | {} tokens", policy::PREFILL_CHUNK),
        format!(
            "| History low water | {} percent",
            policy::HISTORY_LOW_WATER_PERCENT
        ),
    ] {
        assert!(
            document.contains(&expected),
            "missing policy row: {expected}"
        );
    }
}

#[test]
fn protocol_surface() {
    let input = Request::Start(UserInput::new("hello".into(), true).unwrap());
    let mut frame = Vec::new();
    write_ipc(&mut frame, 7, Sensitivity::Public, &input).unwrap();
    let (seq, request) = read_ipc::<Request>(&mut frame.as_slice()).unwrap();
    assert_eq!(seq, 7);
    assert!(matches!(request, Request::Start(input) if input.truncated()));
    for name in ["Audio", "Phoneme", "TokenId", "Retrieve", "Compact"] {
        let invalid = encode(&name).unwrap();
        assert!(ciborium::from_reader::<Request, _>(invalid.as_slice()).is_err());
    }
}

#[test]
fn turn_id_reset_and_overflow() {
    assert!(TurnId::new(0).is_err());
    assert_eq!(TurnId::new(1).unwrap().next().unwrap().get(), 2);
    assert!(TurnId::new(u64::MAX).unwrap().next().is_err());
    assert!(ClauseIndex::new(policy::TURN_CLAUSES as u32).is_err());
}

#[test]
fn oversized_input_not_truncated() {
    assert!(UserInput::new("a".repeat(policy::INPUT_BYTES + 1), false).is_err());
    assert!(UserInput::new(" ".into(), false).is_err());
    assert!(UserInput::new("hello\nworld".into(), false).is_err());
    let text = "a".repeat(policy::INPUT_BYTES);
    assert_eq!(UserInput::new(text.clone(), true).unwrap().text(), text);
}

#[test]
fn response_sentinel_not_logged() {
    let user = UserInput::new("SENSITIVE_RESPONSE".into(), false).unwrap();
    assert!(!format!("{user:?}").contains("SENSITIVE_RESPONSE"));
    assert!(!format!("{:?}", SecretKey::new([42; 32])).contains("42"));
}

#[test]
fn clause_ownership_utf8() {
    let turn = TurnId::new(1).unwrap();
    let clause = Clause::new(
        turn,
        ClauseIndex::new(0).unwrap(),
        "Hello, world.".to_owned().try_into().unwrap(),
    )
    .unwrap();
    assert_eq!(clause.text(), "Hello, world.");
    assert_eq!(clause.turn(), turn);
    assert!(clause.is_sentence());
    assert_eq!(clause.into_text(), "Hello, world.");
}

#[test]
fn speakable_text_rejects_markup_and_controls() {
    for text in [
        "```code```",
        "<|channel>",
        "https://example.com",
        "🙂",
        "42",
        "a  b",
        "\n",
        "...",
    ] {
        assert!(SpeakableText::try_from(text.to_owned()).is_err(), "{text}");
    }
}

#[test]
fn clause_size_bounds() {
    for (index, length) in [(0, policy::FIRST_CHARS), (1, policy::LATER_CHARS)] {
        let index = ClauseIndex::new(index).unwrap();
        assert!(
            Clause::new(
                TurnId::new(1).unwrap(),
                index,
                "a".repeat(length).try_into().unwrap()
            )
            .is_ok()
        );
        assert!(
            Clause::new(
                TurnId::new(1).unwrap(),
                index,
                "a".repeat(length + 1).try_into().unwrap()
            )
            .is_err()
        );
    }
}

struct Fragmented<'a>(&'a [u8]);
impl Read for Fragmented<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if self.0.is_empty() {
            return Ok(0);
        }
        bytes[0] = self.0[0];
        self.0 = &self.0[1..];
        Ok(1)
    }
}

#[test]
fn ipc_limits_backpressure_and_child_crash() {
    let mut bytes = Vec::new();
    write_ipc(&mut bytes, 1, Sensitivity::Public, &Request::Close).unwrap();
    assert!(matches!(
        read_ipc::<Request>(&mut Fragmented(&bytes)).unwrap().1,
        Request::Close
    ));
    assert!(read_ipc::<Request>(&mut &bytes[..bytes.len() - 1]).is_err());
    assert!(read_ipc::<Request>(&mut &(policy::FRAME_BYTES as u32 + 1).to_be_bytes()[..]).is_err());
    assert!(encode(&"x".repeat(policy::FRAME_BYTES + 1)).is_err());
}

#[test]
fn unsupported_memory_operations_rejected() {
    assert!(ciborium::from_reader::<Request, _>(encode(&"Retrieve").unwrap().as_slice()).is_err());
}
