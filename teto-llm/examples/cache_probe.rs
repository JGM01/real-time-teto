//! Explicit model/runtime gate, using one context and deterministic greedy comparison.
use llama_cpp_2::{
    context::{LlamaContext, params::LlamaContextParams},
    llama_backend::LlamaBackend,
    llama_batch::LlamaBatch,
    model::{AddBos, LlamaModel, params::LlamaModelParams},
    sampling::LlamaSampler,
    token::LlamaToken,
};
use std::{num::NonZeroU32, time::Instant};
use teto_llm::{Profile, policy};

const SYSTEM: &str = "<|turn>system\nAnswer briefly in plain English.<turn|>\n";
const NEXT: &str =
    "<turn|>\n<|turn>user\nBriefly explain.<turn|>\n<|turn>model\n<|channel>thought\n<channel|>";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root =
        std::path::PathBuf::from(std::env::var_os("TETO_LLM_ASSETS").ok_or("set TETO_LLM_ASSETS")?);
    Profile::validate_assets(&root)?;
    let mut backend = LlamaBackend::init()?;
    backend.void_logs();
    let model = LlamaModel::load_from_file(
        &backend,
        root.join(policy::MODEL_FILE),
        &LlamaModelParams::default().with_n_gpu_layers(u32::MAX),
    )?;
    let params = LlamaContextParams::default()
        .with_n_ctx(NonZeroU32::new(policy::CONTEXT_TOKENS))
        .with_n_batch(policy::PREFILL_BATCH as u32)
        .with_n_ubatch(policy::PREFILL_BATCH as u32)
        .with_swa_full(true);
    let mut context = model.new_context(&backend, params)?;
    let started = Instant::now();
    let mut results = Vec::with_capacity(6);
    for (name, before, after, forced_failure) in cases() {
        assert!(
            started.elapsed().as_secs() < 4 * 60 * 60,
            "cache gate exceeded four hours"
        );
        results.push(probe(
            &model,
            &mut context,
            name,
            before,
            after,
            forced_failure,
        )?);
    }
    println!(
        "{}",
        serde_json::json!({"runtime": policy::RUNTIME, "model_sha256": policy::MODEL_SHA256,
        "swa_full": true, "context": policy::CONTEXT_TOKENS, "greedy": true, "cases": results})
    );
    Ok(())
}
fn cases() -> [(&'static str, &'static str, &'static str, bool); 6] {
    [
        (
            "append",
            "<|turn>user\nHello.<turn|>\n<|turn>model\nHello.",
            "<|turn>user\nHello.<turn|>\n<|turn>model\nHello. How are you?",
            false,
        ),
        (
            "normalized",
            "<|turn>user\nCount.<turn|>\n<|turn>model\n42",
            "<|turn>user\nCount.<turn|>\n<|turn>model\nforty two",
            false,
        ),
        (
            "token_limit",
            "<|turn>user\nCount.<turn|>\n<|turn>model\nTwelve. An unfinished tail",
            "<|turn>user\nCount.<turn|>\n<|turn>model\nTwelve.",
            false,
        ),
        (
            "interrupted",
            "<|turn>user\nHello.<turn|>\n<|turn>model\nHello. Queued unplayed sentence.",
            "<|turn>user\nHello.<turn|>\n<|turn>model\nHello.",
            false,
        ),
        (
            "evicted",
            "<|turn>user\nOld turn.<turn|>\n<|turn>model\nOld reply.<turn|>\n<|turn>user\nCurrent.",
            "<|turn>user\nCurrent.",
            false,
        ),
        (
            "removal_failure",
            "<|turn>user\nOld.<turn|>\n<|turn>model\nOld answer.",
            "<|turn>user\nNew question.",
            true,
        ),
    ]
}
fn probe(
    model: &LlamaModel,
    context: &mut LlamaContext<'_>,
    name: &str,
    before: &str,
    after: &str,
    forced_failure: bool,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let history = "<|turn>user\nPlease remember the quiet river, the green valley, and the distant mountain for a later question.<turn|>\n<|turn>model\nI will remember the river, valley, and mountain.<turn|>\n".repeat(48);
    let retained = if name == "evicted" { "" } else { &history };
    let original =
        model.str_to_token(&format!("{SYSTEM}{history}{before}{NEXT}"), AddBos::Always)?;
    let target = model.str_to_token(&format!("{SYSTEM}{retained}{after}{NEXT}"), AddBos::Always)?;
    assert!(original.len() < policy::CONTEXT_TOKENS as usize);
    assert!(target.len() + policy::OUTPUT_TOKENS as usize <= policy::CONTEXT_TOKENS as usize);
    context.clear_kv_cache();
    evaluate(context, &target, 0)?;
    let expected = continuation(model, context, target.len())?;
    context.clear_kv_cache();
    evaluate(context, &original, 0)?;
    let common = original
        .iter()
        .zip(&target)
        .take_while(|(a, b)| a == b)
        .count()
        .min(target.len() - 1);
    let reused = if forced_failure {
        false
    } else {
        context.clear_kv_cache_seq(Some(0), Some(common.try_into()?), None)?
    };
    let prefix = if reused {
        common
    } else {
        context.clear_kv_cache();
        0
    };
    evaluate(context, &target[prefix..], prefix)?;
    let actual = continuation(model, context, target.len())?;
    let equal = expected == actual;
    let value = serde_json::json!({"case":name,"prefix_tokens":prefix,"evaluated_prompt_tokens":target.len()-prefix,
        "prompt_tokens":target.len(),"partial_remove_succeeded":reused,"forced_failure":forced_failure,
        "greedy_tokens_compared":expected.len(),"continuation_equal":equal});
    assert!(equal, "cache continuation mismatch in {name}");
    if !forced_failure {
        assert!(reused, "profile cannot remove suffix in {name}");
    }
    Ok(value)
}
fn evaluate(
    context: &mut LlamaContext<'_>,
    tokens: &[LlamaToken],
    start: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut batch = LlamaBatch::new(policy::PREFILL_BATCH, 1);
    for (chunk_index, chunk) in tokens.chunks(policy::PREFILL_BATCH).enumerate() {
        batch.clear();
        for (index, token) in chunk.iter().enumerate() {
            let relative = chunk_index * policy::PREFILL_BATCH + index;
            batch.add(
                *token,
                (start + relative).try_into()?,
                &[0],
                relative + 1 == tokens.len(),
            )?;
        }
        context.decode(&mut batch)?;
    }
    Ok(())
}
fn continuation(
    model: &LlamaModel,
    context: &mut LlamaContext<'_>,
    start: usize,
) -> Result<Vec<LlamaToken>, Box<dyn std::error::Error>> {
    let mut output = Vec::with_capacity(8);
    let mut sampler = LlamaSampler::greedy();
    let mut batch = LlamaBatch::new(1, 1);
    for index in 0..8 {
        let token = sampler.sample(context, -1);
        output.push(token);
        if model.is_eog_token(token) {
            break;
        }
        batch.clear();
        batch.add(token, (start + index).try_into()?, &[0], true)?;
        context.decode(&mut batch)?;
    }
    Ok(output)
}
