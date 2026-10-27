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
fn eviction_removes_the_oldest_exchange() {
    let mut builder = ContextBuilder::new(
        "system".into(),
        Profile::lookup(ProfileId::Desktop)
            .unwrap()
            .generation_prompt,
    );
    builder
        .history
        .extend([exchange("oldest", 10), exchange("newest", 20)]);
    builder.bytes = builder.history.iter().map(|item| item.bytes).sum();
    builder.history_tokens = 30;
    let old = builder.evict();
    assert_eq!(old.user, "oldest");
    assert_eq!(builder.history[0].user, "newest");
    assert_eq!(builder.history_tokens, 20);
    assert_eq!(builder.bytes, "newest".len() + 6);
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
