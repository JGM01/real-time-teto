use super::*;
use proptest::prelude::*;

fn render(chunks: &[&[u8]], eos: bool) -> Result<Vec<Clause>> {
    let mut text = TextPipeline::new(TurnId::new(1).unwrap());
    let mut clauses = Vec::new();
    let mut emit = |clause| {
        clauses.push(clause);
        Ok(())
    };
    for bytes in chunks {
        text.push(bytes, &mut emit)?;
    }
    text.finish(eos, &mut emit)?;
    Ok(clauses)
}
fn strings(clauses: &[Clause]) -> Vec<&str> {
    clauses.iter().map(Clause::text).collect()
}
#[test]
fn normalization_v1() {
    let clauses = render(
        &[b"# **Hello**, Dr. Smith. I have 12 apples and 42 pears."],
        true,
    )
    .unwrap();
    assert_eq!(
        strings(&clauses).join(" "),
        "Hello, Doctor Smith. I have twelve apples and four two pears."
    );
    assert!(render(&[b"It costs $20."], true).is_err());
    assert!(render(&[b"It is 3.14."], true).is_err());
    assert!(render(&[b"The 21st."], true).is_err());
    assert!(render(&[b"On 2026-10-27."], true).is_err());
    assert!(render(&[b"It weighs 10kg."], true).is_err());
    assert!(render(&[b"It weighs 10 kg."], true).is_err());
}
#[test]
fn reasoning_and_control_suppressed() {
    let input = b"<|channel>thought\nSECRET hidden analysis.<channel|>Hello there. <|tool_call>SECRET tool<tool_call|>Goodbye.";
    for split in 0..=input.len() {
        let clauses = render(&[&input[..split], &input[split..]], true).unwrap();
        assert_eq!(strings(&clauses), ["Hello there.", "Goodbye."]);
    }
    assert!(render(&[b"<|channel>unknown\nsecret"], true).is_err());
    assert!(render(&[b"<|channel>thought\nnever closed"], true).is_err());
    assert!(render(&[b"<unknown>secret"], true).is_err());
}
#[test]
fn clause_order() {
    let result = render(&[b"Hello. One, two and three. Goodbye."], true).unwrap();
    for (index, clause) in result.iter().enumerate() {
        assert_eq!(clause.index().get() as usize, index);
    }
}
#[test]
fn clause_indices() {
    clause_order();
}
#[test]
fn every_clause_has_turn() {
    for clause in render(&[b"Hello. Goodbye."], true).unwrap() {
        assert_eq!(clause.turn().get(), 1);
    }
}
#[test]
fn output_token_bound() {
    let clauses = render(&[b"A complete sentence. An unfinished tail"], false).unwrap();
    assert_eq!(strings(&clauses), ["A complete sentence."]);
}
#[test]
fn token_limit_with_context_headroom() {
    output_token_bound();
}
#[test]
fn normalization_omits_code_tables_urls_emoji() {
    let input = "Hello.\n```rust\nsecret code\n```\n| secret | table |\nVisit https://example.com now. 🙂 ❤️ [Read this](https://example.com).";
    let clauses = render(&[input.as_bytes()], true).unwrap();
    let output = strings(&clauses).join(" ");
    assert!(!output.contains("secret"));
    assert!(!output.contains("http"));
    assert!(output.contains("Read this"));
    for split in 0..=input.len() {
        let other = render(
            &[&input.as_bytes()[..split], &input.as_bytes()[split..]],
            true,
        )
        .unwrap();
        assert_eq!(strings(&other).join(" "), output);
    }
}
proptest! {
    #[test]
    fn chunking_is_invariant(words in prop::collection::vec("[a-z]{1,10}", 1..100), chunk in 1usize..40) {
        let input = format!("{}.", words.join(" "));
        let chunks: Vec<_> = input.as_bytes().chunks(chunk).collect();
        match (render(&chunks, true), render(&[input.as_bytes()], true)) {
            (Ok(a), Ok(b)) => {
                prop_assert_eq!(strings(&a), strings(&b));
                for clause in &a {
                    let limit = if clause.index().get() == 0 { policy::FIRST_CHARS } else { policy::LATER_CHARS };
                    prop_assert!(clause.text().chars().count() <= limit);
                }
            }
            (Err(a), Err(b)) => prop_assert_eq!(a.kind(), b.kind()),
            _ => prop_assert!(false, "chunking changed acceptance"),
        }
    }
}

#[test]
fn unicode_prose_keeps_scalar_boundaries() {
    let input = "Renée visits a café. She says ‘Hello’…";
    let expected = "Renée visits a café. She says 'Hello'.";
    for split in 0..=input.len() {
        let clauses = render(
            &[&input.as_bytes()[..split], &input.as_bytes()[split..]],
            true,
        )
        .unwrap();
        assert_eq!(strings(&clauses).join(" "), expected);
    }
}

#[test]
fn fresh_clause_rechecks_minimum_preference() {
    let input = format!("Hello. {} a b However, this continues.", "a".repeat(230));
    let clauses = render(&[input.as_bytes()], true).unwrap();
    assert_eq!(clauses.last().unwrap().text(), "However, this continues.");
}

#[test]
fn abbreviation_wrappers_preserve_punctuation() {
    let result = render(&[b"Ask (Dr.) Smith; e.g., say hello."], true).unwrap();
    assert_eq!(
        strings(&result).join(" "),
        "Ask (Doctor) Smith; for example, say hello."
    );
}
