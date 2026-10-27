use crate::*;
use llama_cpp_2::{context::LlamaContext, token::LlamaToken};

/// Token IDs describe only tokens successfully evaluated in this one context.
pub(crate) struct PrefixCache {
    tokens: Vec<LlamaToken>,
    mode: CacheMode,
    invalidated: Option<CacheReason>,
}
impl PrefixCache {
    pub fn new(mode: CacheMode) -> Self {
        Self {
            tokens: Vec::with_capacity(policy::CONTEXT_TOKENS as usize),
            mode,
            invalidated: None,
        }
    }
    pub fn invalidate(&mut self, reason: CacheReason) {
        self.tokens.clear();
        self.invalidated = Some(reason);
    }
    pub fn prepare(
        &mut self,
        context: &mut LlamaContext<'_>,
        target: &[LlamaToken],
        metrics: &mut TurnMetrics,
    ) -> usize {
        assert!(!target.is_empty());
        metrics.cache_mode = self.mode;
        let prefix = common_prefix(&self.tokens, target);
        let invalidated = self.invalidated.take();
        let forced = if metrics.evicted_turns > 0 {
            Some(CacheReason::Eviction)
        } else if invalidated.is_some() {
            invalidated
        } else if self.mode == CacheMode::FullRebuild {
            Some(CacheReason::Disabled)
        } else {
            None
        };
        let reusable = forced.is_none() && prefix > 0;
        let removed = reusable
            && context
                .clear_kv_cache_seq(Some(0), Some(prefix as u32), None)
                .unwrap_or(false);
        let keep = if removed { prefix } else { 0 };
        metrics.cache_reason = forced.unwrap_or({
            if reusable && !removed {
                CacheReason::RemovalFailed
            } else if self.tokens.is_empty() {
                CacheReason::Cold
            } else if prefix < self.tokens.len() {
                CacheReason::CanonicalSuffix
            } else {
                CacheReason::Reused
            }
        });
        if keep == 0 {
            context.clear_kv_cache();
        }
        self.tokens.truncate(keep);
        metrics.cached_prefix_tokens = keep as u32;
        metrics.uncached_prompt_tokens = target.len() as u32 - keep as u32;
        keep
    }
    pub fn remember(&mut self, tokens: &[LlamaToken]) {
        assert!(self.tokens.len() + tokens.len() <= policy::CONTEXT_TOKENS as usize);
        self.tokens.extend_from_slice(tokens);
    }
}
fn common_prefix(cached: &[LlamaToken], target: &[LlamaToken]) -> usize {
    // Re-evaluate the last target token when the entire target is cached, to obtain logits.
    cached
        .iter()
        .zip(target)
        .take_while(|(a, b)| a == b)
        .count()
        .min(target.len().saturating_sub(1))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prefix_tracks_only_evaluated_tokens() {
        let mut cache = PrefixCache::new(CacheMode::VerifiedPrefixReuse);
        cache.remember(&[LlamaToken(1), LlamaToken(2)]);
        assert_eq!(
            common_prefix(&cache.tokens, &[LlamaToken(1), LlamaToken(3)]),
            1
        );
        assert_eq!(
            common_prefix(&cache.tokens, &[LlamaToken(1), LlamaToken(2)]),
            1
        );
        cache.invalidate(CacheReason::Reset);
        assert!(cache.tokens.is_empty());
    }
    #[test]
    fn exact_profile_cache_evidence() {
        let report: serde_json::Value =
            serde_json::from_str(include_str!("../qualification/cache-m3-pro.json")).unwrap();
        assert_eq!(report["model_sha256"], policy::MODEL_SHA256);
        assert_eq!(report["runtime"], policy::RUNTIME);
        assert_eq!(report["device"], policy::PREFIX_REUSE_DEVICE);
        assert_eq!(report["os"], policy::PREFIX_REUSE_OS);
        assert_eq!(report["cases"].as_array().unwrap().len(), 6);
        for case in report["cases"].as_array().unwrap() {
            assert_eq!(case["continuation_equal"], true);
        }
    }
}
