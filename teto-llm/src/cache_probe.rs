//! Six-case prefix-cache probe. Native model required.
use crate::profile::{Profile, ProfileSpec};
use crate::*;
use llama_cpp_2::{
    context::{LlamaContext, params::LlamaContextParams},
    llama_backend::LlamaBackend,
    llama_batch::LlamaBatch,
    model::{AddBos, LlamaModel, params::LlamaModelParams},
    sampling::LlamaSampler,
    token::LlamaToken,
};
use serde_json::json;
use std::{io::Write, num::NonZeroU32, path::Path, time::Instant};

const SYSTEM: &str = "<|turn>system\nAnswer briefly in plain English.<turn|>\n";
const CURRENT: &str = "<turn|>\n<|turn>user\nBriefly explain.";

/// Run the six cases and write one JSON report.
///
/// Returns true when every case produces an equal continuation and every
/// non-forced case removes the suffix.
pub fn run(
    assets: &Path,
    profile_id: ProfileId,
    host: &HostIdentity,
    output: &mut impl Write,
) -> Result<bool> {
    let profile = Profile::lookup(profile_id)?;
    Profile::validate_assets_for(assets, &profile, None)?;
    let mut backend = LlamaBackend::init().map_err(|_| failure(ErrorKind::Worker))?;
    backend.void_logs();
    let model = LlamaModel::load_from_file(
        &backend,
        assets.join(profile.file),
        &LlamaModelParams::default().with_n_gpu_layers(
            if cfg!(any(target_os = "macos", target_os = "ios")) {
                u32::MAX
            } else {
                0
            },
        ),
    )
    .map_err(|_| failure(ErrorKind::Profile))?;
    Profile::validate_model(&model, &profile)?;
    let parameters = LlamaContextParams::default()
        .with_n_ctx(NonZeroU32::new(policy::CONTEXT_TOKENS))
        .with_n_batch(policy::PREFILL_CHUNK as u32)
        .with_n_ubatch(policy::PREFILL_CHUNK as u32)
        .with_swa_full(true);
    let mut context = model
        .new_context(&backend, parameters)
        .map_err(|_| failure(ErrorKind::Generation))?;
    let started = Instant::now();
    let mut results = Vec::with_capacity(6);
    let mut qualified = true;
    for (name, before, after, forced_failure) in cases() {
        if started.elapsed().as_secs() >= 4 * 60 * 60 {
            return Err(failure(ErrorKind::Generation));
        }
        let (value, case_ok) = probe(
            &model,
            &mut context,
            &profile,
            name,
            before,
            after,
            forced_failure,
        )?;
        qualified &= case_ok;
        results.push(value);
    }
    let report = json!({
        "profile": profile.id,
        "model_sha256": profile.sha256,
        "template_sha256": profile.template_sha256,
        "runtime": policy::RUNTIME,
        "device": host.device,
        "os": host.os,
        "swa_full": true,
        "context": policy::CONTEXT_TOKENS,
        "greedy": true,
        "qualified": qualified,
        "cases": results,
    });
    writeln!(output, "{report}")
        .and_then(|()| output.flush())
        .map_err(|_| failure(ErrorKind::Worker))?;
    Ok(qualified)
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
    profile: &ProfileSpec,
    name: &str,
    before: &str,
    after: &str,
    forced_failure: bool,
) -> Result<(serde_json::Value, bool)> {
    let history = "<|turn>user\nPlease remember the quiet river, the green valley, and the distant mountain for a later question.<turn|>\n<|turn>model\nI will remember the river, valley, and mountain.<turn|>\n".repeat(48);
    let retained = if name == "evicted" { "" } else { &history };
    let next = format!("{CURRENT}{}", profile.generation_prompt);
    let original = model
        .str_to_token(&format!("{SYSTEM}{history}{before}{next}"), AddBos::Always)
        .map_err(|_| failure(ErrorKind::Profile))?;
    let target = model
        .str_to_token(&format!("{SYSTEM}{retained}{after}{next}"), AddBos::Always)
        .map_err(|_| failure(ErrorKind::Profile))?;
    if original.len() >= policy::CONTEXT_TOKENS as usize
        || target.len() + policy::OUTPUT_TOKENS as usize > policy::CONTEXT_TOKENS as usize
    {
        return Err(failure(ErrorKind::ContextBudget));
    }
    context.clear_kv_cache();
    evaluate(context, &target, 0)?;
    let expected = continuation(model, context, target.len())?;
    context.clear_kv_cache();
    evaluate(context, &original, 0)?;
    let common = original
        .iter()
        .zip(&target)
        .take_while(|(left, right)| left == right)
        .count()
        .min(target.len() - 1);
    let reused = if forced_failure {
        false
    } else {
        context
            .clear_kv_cache_seq(
                Some(0),
                Some(u32::try_from(common).map_err(|_| failure(ErrorKind::Protocol))?),
                None,
            )
            .map_err(|_| failure(ErrorKind::Generation))?
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
    let case_ok = equal && (forced_failure || reused);
    Ok((
        json!({
            "case": name,
            "prefix_tokens": prefix,
            "evaluated_prompt_tokens": target.len() - prefix,
            "prompt_tokens": target.len(),
            "partial_remove_succeeded": reused,
            "forced_failure": forced_failure,
            "greedy_tokens_compared": expected.len(),
            "continuation_equal": equal,
        }),
        case_ok,
    ))
}

fn evaluate(context: &mut LlamaContext<'_>, tokens: &[LlamaToken], start: usize) -> Result<()> {
    let mut batch = LlamaBatch::new(policy::PREFILL_CHUNK, 1);
    for (chunk_index, chunk) in tokens.chunks(policy::PREFILL_CHUNK).enumerate() {
        batch.clear();
        for (index, token) in chunk.iter().enumerate() {
            let relative = chunk_index * policy::PREFILL_CHUNK + index;
            batch
                .add(
                    *token,
                    i32::try_from(start + relative)
                        .map_err(|_| failure(ErrorKind::ContextBudget))?,
                    &[0],
                    relative + 1 == tokens.len(),
                )
                .map_err(|_| failure(ErrorKind::Generation))?;
        }
        context
            .decode(&mut batch)
            .map_err(|_| failure(ErrorKind::Generation))?;
    }
    Ok(())
}

fn continuation(
    model: &LlamaModel,
    context: &mut LlamaContext<'_>,
    start: usize,
) -> Result<Vec<LlamaToken>> {
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
        batch
            .add(
                token,
                i32::try_from(start + index).map_err(|_| failure(ErrorKind::Generation))?,
                &[0],
                true,
            )
            .map_err(|_| failure(ErrorKind::Generation))?;
        context
            .decode(&mut batch)
            .map_err(|_| failure(ErrorKind::Generation))?;
    }
    Ok(output)
}
