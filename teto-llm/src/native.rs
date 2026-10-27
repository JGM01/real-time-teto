use crate::cache::PrefixCache;
use crate::*;
use crate::{
    context::{ContextBuilder, PreparedPrompt},
    engine::{Active, Command, Reply, Session},
    mailbox::Mailbox,
    text::TextPipeline,
};
use crossbeam_channel::Receiver;
use llama_cpp_2::{
    context::{LlamaContext, params::LlamaContextParams},
    llama_backend::LlamaBackend,
    llama_batch::LlamaBatch,
    model::{LlamaModel, params::LlamaModelParams},
    sampling::LlamaSampler,
};
use sha2::{Digest, Sha256};
use std::{num::NonZeroU32, sync::Arc, time::Instant};

pub(crate) fn run(config: OpenConfig, commands: Receiver<Command>, ready: Reply<()>) -> Result<()> {
    let profile = Profile::lookup(config.profile.as_str())?;
    Profile::validate_assets_for(&config.assets, &profile)?;
    let journal = match config.journal {
        JournalAccess::Disabled => None,
        JournalAccess::Encrypted { directory, key } => {
            Some(journal::Journal::open(directory, key)?)
        }
    };
    let mut backend = LlamaBackend::init().map_err(|_| failure(ErrorKind::Worker))?;
    if std::env::var("TETO_LLM_STARTUP_DIAGNOSTICS").as_deref() != Ok("1") {
        backend.void_logs();
    }
    let parameters = LlamaModelParams::default().with_n_gpu_layers(
        if cfg!(any(target_os = "macos", target_os = "ios")) {
            u32::MAX
        } else {
            0
        },
    );
    let model = LlamaModel::load_from_file(&backend, config.assets.join(profile.file), &parameters)
        .map_err(|_| failure(ErrorKind::Profile))?;
    Profile::validate_model(&model, &profile)?;
    let parameters = LlamaContextParams::default()
        .with_n_ctx(NonZeroU32::new(policy::CONTEXT_TOKENS))
        .with_n_batch(policy::PREFILL_BATCH as u32)
        .with_n_ubatch(policy::PREFILL_BATCH as u32)
        .with_swa_full(true);
    let mut context = model
        .new_context(&backend, parameters)
        .map_err(|_| failure(ErrorKind::Generation))?;
    backend.void_logs();
    let mut cache = PrefixCache::new(Profile::cache_mode(profile.id));
    let mut session = Session {
        context: ContextBuilder::new(config.system_prompt, profile.generation_prompt),
        active: None,
        pending: None,
        journal,
        session: journal::random()?,
        profile: profile.id,
    };
    ready.send(Ok(())).map_err(|_| failure(ErrorKind::Worker))?;
    while let Ok(command) = commands.recv() {
        if handle(
            command,
            &mut session,
            &model,
            &mut context,
            config.seed,
            &mut cache,
        ) {
            break;
        }
    }
    Ok(())
}

fn handle(
    command: Command,
    session: &mut Session,
    model: &LlamaModel,
    context: &mut LlamaContext<'_>,
    seed: u64,
    cache: &mut PrefixCache,
) -> bool {
    match command {
        Command::Start(input, mailbox, reply) => {
            match admit(session, model, context.n_ctx(), input, Arc::clone(&mailbox)) {
                Ok(prompt) => {
                    if reply.send(Ok(())).is_err() {
                        mailbox.cancel();
                    }
                    if !generate(context, model, &mailbox, prompt, seed, cache) {
                        return true;
                    }
                }
                Err(error) => {
                    let _ = reply.send(Err(error));
                }
            }
        }
        Command::Finish(id, playback, source, reply) => {
            let _ = reply.send(session.finalize(id, playback, source, model));
        }
        Command::Reset(reply) => {
            let result = if session.active.is_some() || session.pending.is_some() {
                Err(failure(ErrorKind::PlaybackPending))
            } else {
                session.context.reset();
                context.clear_kv_cache();
                cache.invalidate(CacheReason::Reset);
                Ok(())
            };
            let _ = reply.send(result);
        }
        Command::Retry(reply) => {
            let result = session.persist().and_then(|()| session.commit(model));
            let _ = reply.send(result);
        }
        Command::Close(reply) => {
            let result = if session.active.is_some() || session.pending.is_some() {
                Err(failure(ErrorKind::PlaybackPending))
            } else {
                Ok(())
            };
            let _ = reply.send(result);
            return true;
        }
    }
    false
}

fn admit(
    session: &mut Session,
    model: &LlamaModel,
    limit: u32,
    input: UserInput,
    mailbox: Arc<Mailbox>,
) -> Result<PreparedPrompt> {
    if session.active.is_some() || session.pending.is_some() {
        return Err(failure(ErrorKind::PlaybackPending));
    }
    if let Some(journal) = &mut session.journal {
        journal.reserve()?;
    }
    let prompt = session.context.prepare(model, &input, limit)?;
    session.active = Some(Active { input, mailbox });
    Ok(prompt)
}

struct Generation {
    metrics: TurnMetrics,
    seed: u64,
    started: Instant,
}
fn generate(
    context: &mut LlamaContext<'_>,
    model: &LlamaModel,
    mailbox: &Mailbox,
    prompt: PreparedPrompt,
    seed: u64,
    cache: &mut PrefixCache,
) -> bool {
    let mut run = Generation {
        started: mailbox.started(),
        metrics: prompt.metrics,
        seed,
    };
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        decode(context, model, mailbox, &prompt.tokens, &mut run, cache)
    }));
    let result = match outcome {
        Ok(result) => result,
        Err(_) => {
            mailbox.worker_failed();
            return false;
        }
    };
    let (stop, error) = match result {
        Ok(stop) => (stop, None),
        Err(error) if error.kind() == ErrorKind::Cancelled => (GenerationStop::Cancelled, None),
        Err(error) => {
            cache.invalidate(CacheReason::GenerationFailed);
            (GenerationStop::Failed, Some(error))
        }
    };
    run.metrics.generation_ms = (run.started.elapsed().as_secs_f64() * 1000.0
        - run.metrics.prompt_ms
        - run.metrics.prompt_render_ms)
        .max(0.0);
    mailbox.end(stop, error, run.metrics);
    true
}

fn decode(
    context: &mut LlamaContext<'_>,
    model: &LlamaModel,
    mailbox: &Mailbox,
    tokens: &[llama_cpp_2::token::LlamaToken],
    run: &mut Generation,
    cache: &mut PrefixCache,
) -> Result<GenerationStop> {
    let prompt_started = Instant::now();
    let prefix = cache.prepare(context, tokens, &mut run.metrics);
    let prefilled = prefill(
        context,
        mailbox,
        &tokens[prefix..],
        prefix,
        cache,
        &mut run.metrics,
    );
    run.metrics.prompt_ms = prompt_started.elapsed().as_secs_f64() * 1000.0;
    prefilled?;
    let started = run.started;
    let seed = run.seed;
    let metrics = &mut run.metrics;
    let mut sampler = sampler(seed, mailbox.id);
    let mut text = TextPipeline::new(mailbox.id);
    let mut emit = |clause: Clause| {
        mailbox.publish(clause)?;
        let now = started.elapsed().as_secs_f64() * 1000.0;
        metrics.first_clause_ms.get_or_insert(now);
        metrics.last_clause_ms = Some(now);
        Ok(())
    };
    let mut batch = LlamaBatch::new(1, 1);
    for offset in 0..policy::OUTPUT_TOKENS {
        mailbox.wait_for_capacity()?;
        let token = sampler.sample(context, -1);
        sampler.accept(token);
        metrics.generated_tokens += 1;
        if model.is_eog_token(token) {
            text.finish(true, &mut emit)?;
            return Ok(GenerationStop::Eos);
        }
        let piece = model
            .token_to_piece_bytes(token, 4096, true, None)
            .map_err(|_| failure(ErrorKind::Generation))?;
        if piece == b"<turn|>" {
            text.finish(true, &mut emit)?;
            return Ok(GenerationStop::Eos);
        }
        let answer = text.push(&piece, &mut emit)?;
        if answer {
            metrics
                .first_answer_ms
                .get_or_insert(started.elapsed().as_secs_f64() * 1000.0);
        }
        if mailbox.cancelled() {
            return Err(failure(ErrorKind::Cancelled));
        }
        batch.clear();
        let position = i32::try_from(tokens.len() + offset as usize)
            .map_err(|_| failure(ErrorKind::Generation))?;
        batch
            .add(token, position, &[0], true)
            .map_err(|_| failure(ErrorKind::Generation))?;
        context
            .decode(&mut batch)
            .map_err(|_| failure(ErrorKind::Generation))?;
        cache.remember(&[token]);
    }
    text.finish(false, &mut emit)?;
    Ok(GenerationStop::TokenLimit)
}

fn sampler(seed: u64, turn: TurnId) -> LlamaSampler {
    let mut hash = Sha256::new();
    hash.update(seed.to_le_bytes());
    hash.update(turn.get().to_le_bytes());
    let seed = u32::from_le_bytes(hash.finalize()[..4].try_into().expect("four seed bytes"));
    LlamaSampler::chain_simple([
        LlamaSampler::top_k(policy::SAMPLER_TOP_K),
        LlamaSampler::top_p(policy::SAMPLER_TOP_P, 1),
        LlamaSampler::temp(policy::SAMPLER_TEMP),
        LlamaSampler::dist(seed),
    ])
}

fn prefill(
    context: &mut LlamaContext<'_>,
    mailbox: &Mailbox,
    tokens: &[llama_cpp_2::token::LlamaToken],
    start: usize,
    cache: &mut PrefixCache,
    metrics: &mut TurnMetrics,
) -> Result<()> {
    let mut batch = LlamaBatch::new(policy::PREFILL_BATCH, 1);
    for (chunk, tokens_chunk) in tokens.chunks(policy::PREFILL_BATCH).enumerate() {
        if mailbox.cancelled() {
            return Err(failure(ErrorKind::Cancelled));
        }
        batch.clear();
        for (index, token) in tokens_chunk.iter().enumerate() {
            let relative = chunk * policy::PREFILL_BATCH + index;
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
        cache.remember(tokens_chunk);
        metrics.evaluated_prompt_tokens += tokens_chunk.len() as u32;
    }
    Ok(())
}
