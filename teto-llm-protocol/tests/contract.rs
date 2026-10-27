use std::io::{self, Read};
use teto_llm_protocol::*;

#[test]
fn policy_and_traceability_complete() {
    let document = include_str!("../../llm-subsystem.md");
    let table = document
        .split("<!-- POLICY:BEGIN -->\n")
        .nth(1)
        .unwrap()
        .split("<!-- POLICY:END -->")
        .next()
        .unwrap();
    assert_eq!(table, policy::markdown());
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let mut sources = String::new();
    for package in ["teto-cli", "teto-core", "teto-llm", "teto-llm-protocol"] {
        source_text(&root.join(package), &mut sources);
    }
    assert_eq!(
        document
            .lines()
            .filter(|line| line.starts_with("| LLM-"))
            .count(),
        104
    );
    let deferred = [49, 54, 55, 70, 72, 73, 77];
    for number in 1..=104 {
        let id = format!("| LLM-{number:03} |");
        assert_eq!(document.matches(&id).count(), 1, "{id}");
        let row = document.lines().find(|line| line.starts_with(&id)).unwrap();
        assert!(row.split('|').count() >= 7);
        let columns: Vec<_> = row.split('|').collect();
        assert!(!columns[3].trim_matches([' ', '`']).is_empty());
        let test = columns[4].trim_matches([' ', '`']);
        assert!(!test.is_empty());
        assert_eq!(
            row.contains("Deferred"),
            deferred.contains(&number),
            "unmarked or unauthorized deferral: {id}"
        );
        assert!(row.contains(if deferred.contains(&number) {
            "activation gate"
        } else {
            "Required"
        }));
        if !row.contains("Deferred") {
            assert!(sources.contains(&format!("fn {test}(")), "missing {test}");
        }
    }
}
fn source_text(directory: &std::path::Path, output: &mut String) {
    let mut directories = vec![directory.to_owned()];
    let mut visited = 0;
    while let Some(directory) = directories.pop() {
        visited += 1;
        assert!(visited <= 128, "source inventory exceeds its bound");
        for entry in std::fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if entry.file_type().unwrap().is_symlink()
                || path.file_name().is_some_and(|name| name == "target")
            {
                continue;
            }
            if path.is_dir() {
                directories.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                output.push_str(&std::fs::read_to_string(path).unwrap());
            }
        }
    }
}
#[test]
fn protocol_surface() {
    let input = Request::Start(UserInput::new("hello".into(), true).unwrap());
    let mut frame = Vec::new();
    write_frame(&mut frame, &input).unwrap();
    assert!(
        matches!(read_frame::<Request>(&mut frame.as_slice()).unwrap(), Request::Start(input) if input.truncated())
    );
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
        Boundary::Sentence,
    )
    .unwrap();
    assert_eq!(clause.text(), "Hello, world.");
    assert_eq!(clause.turn(), turn);
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
    assert!(std::mem::size_of::<Clause>() <= 48);
}
#[test]
fn clause_size_bounds() {
    for (index, length) in [(0, policy::FIRST_CHARS), (1, policy::LATER_CHARS)] {
        let index = ClauseIndex::new(index).unwrap();
        assert!(
            Clause::new(
                TurnId::new(1).unwrap(),
                index,
                "a".repeat(length).try_into().unwrap(),
                Boundary::Clause
            )
            .is_ok()
        );
        assert!(
            Clause::new(
                TurnId::new(1).unwrap(),
                index,
                "a".repeat(length + 1).try_into().unwrap(),
                Boundary::Clause
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
    write_frame(&mut bytes, &Request::Close).unwrap();
    assert!(matches!(
        read_frame::<Request>(&mut Fragmented(&bytes)).unwrap(),
        Request::Close
    ));
    assert!(read_frame::<Request>(&mut &bytes[..bytes.len() - 1]).is_err());
    assert!(
        read_frame::<Request>(&mut &(policy::FRAME_BYTES as u32 + 1).to_be_bytes()[..]).is_err()
    );
    assert!(encode(&"x".repeat(policy::FRAME_BYTES + 1)).is_err());
}
#[test]
fn unsupported_memory_operations_rejected() {
    assert!(ciborium::from_reader::<Request, _>(encode(&"Retrieve").unwrap().as_slice()).is_err());
}
