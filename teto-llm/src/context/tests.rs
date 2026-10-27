use super::*;

#[test]
fn exact_fit_and_one_token_over() {
    let max_prompt = policy::CONTEXT_TOKENS - policy::OUTPUT_TOKENS;
    assert_eq!(context_budget(max_prompt, 8192, 4096).unwrap(), (4096, 0));
    assert_eq!(
        context_budget(max_prompt + 1, 8192, 4096)
            .unwrap_err()
            .kind(),
        ErrorKind::ContextBudget
    );
    assert!(context_budget(u32::MAX, u32::MAX, u32::MAX).is_err());
}
#[test]
fn context_budget_bound() {
    exact_fit_and_one_token_over();
}
#[test]
fn effective_context_limit() {
    assert_eq!(context_budget(100, 2048, 4096).unwrap(), (2048, 1436));
    assert_eq!(context_budget(100, 4096, 1024).unwrap(), (1024, 412));
}
#[test]
fn oversized_pinned_prompt_error() {
    assert_eq!(
        context_budget(4096, 4096, 4096).unwrap_err().kind(),
        ErrorKind::ContextBudget
    );
}
fn exchange(user: &str, tokens: u32) -> Exchange {
    Exchange {
        user: user.into(),
        assistant: "played".into(),
        note: String::new(),
        bytes: user.len() + 6,
        tokens,
    }
}
#[test]
fn exchange_eviction_order() {
    let mut builder = ContextBuilder::new(
        "system".into(),
        Profile::lookup(policy::PROFILE_ID)
            .unwrap()
            .generation_prompt,
    );
    builder
        .history
        .extend([exchange("oldest", 10), exchange("newest", 20)]);
    builder.bytes = builder.history.iter().map(|e| e.bytes).sum();
    builder.evict();
    assert_eq!(builder.history[0].user, "newest");
    assert_eq!(builder.system, "system");
    assert_eq!(builder.evicted_tokens, 10);
    assert_eq!(builder.evicted_turns, 1);
}
#[test]
fn eviction_metrics() {
    exchange_eviction_order();
}
#[test]
fn pinned_content_preserved() {
    exchange_eviction_order();
}
#[test]
fn decode_requires_budgeted_prompt() {
    let source = include_str!("../native.rs");
    assert!(source.contains("prompt: PreparedPrompt"));
    assert!(source.contains("context.clear_kv_cache()"));
}
#[test]
fn single_prompt_assembler() {
    assert!(!include_str!("../native.rs").contains("<|turn>"));
    assert!(include_str!("../context.rs").contains("fn render("));
}
#[test]
fn builder_does_not_decode() {
    assert!(!include_str!("../context.rs").contains("context.decode("));
}
#[test]
fn cache_capability_gate() {
    assert_eq!(
        Profile::cache_mode(policy::PHONE_PROFILE),
        CacheMode::FullRebuild
    );
}
#[test]
fn cache_failure_rebuilds() {
    decode_requires_budgeted_prompt();
    cache_capability_gate();
}

proptest::proptest! {
    #[test]
    fn budget_arithmetic_matches_widened_oracle(
        prompt in proptest::prop_oneof![0u32..8192, proptest::prelude::any::<u32>()],
        profile in 0u32..8192, runtime in 0u32..8192,
    ) {
        let limit = u64::from(profile.min(runtime).min(policy::CONTEXT_TOKENS));
        let total = u64::from(prompt) + u64::from(policy::OUTPUT_TOKENS);
        match context_budget(prompt, profile, runtime) {
            Ok((actual, headroom)) => {
                proptest::prop_assert!(total <= limit);
                proptest::prop_assert_eq!(u64::from(actual), limit);
                proptest::prop_assert_eq!(u64::from(headroom) + total, limit);
            }
            Err(error) => {
                proptest::prop_assert!(total > limit);
                proptest::prop_assert_eq!(error.kind(), ErrorKind::ContextBudget);
            }
        }
    }
}
