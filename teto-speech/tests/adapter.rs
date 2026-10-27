#![cfg(feature = "llm-adapter")]

use teto_llm_protocol::{Clause, ClauseIndex, SpeakableText, TurnId};
use teto_speech::adapter::llm::from_clause;

#[test]
fn adapter_preserves_clause_identity() {
    let turn = TurnId::new(4).unwrap();
    let index = ClauseIndex::new(2).unwrap();
    let clause = Clause::new(
        turn,
        index,
        SpeakableText::try_from("Hello world.".to_owned()).unwrap(),
    )
    .unwrap();
    let input = from_clause(&clause, 9).unwrap();
    assert_eq!(input.key().epoch, 9);
    assert_eq!(input.key().turn.get(), 4);
    assert_eq!(input.key().clause, 2);
    assert_eq!(input.text(), "Hello world.");
}
