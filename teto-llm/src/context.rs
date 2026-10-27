use crate::profile::INSTRUCTION;
use crate::*;
use llama_cpp_2::{model::LlamaModel, token::LlamaToken};
use std::collections::VecDeque;

struct Exchange {
    user: String,
    assistant: String,
    note: String,
    bytes: usize,
    tokens: u32,
}
pub(crate) struct ContextBuilder {
    system: String,
    generation_prompt: &'static str,
    history: VecDeque<Exchange>,
    bytes: usize,
    evicted_turns: u32,
    evicted_tokens: u32,
}
pub(crate) struct PreparedPrompt {
    pub tokens: Vec<LlamaToken>,
    pub metrics: TurnMetrics,
}

pub(crate) fn context_budget(prompt: u32, profile: u32, runtime: u32) -> Result<(u32, u32)> {
    let limit = profile.min(runtime).min(policy::CONTEXT_TOKENS);
    let total = prompt
        .checked_add(policy::OUTPUT_TOKENS)
        .ok_or_else(|| failure(ErrorKind::ContextBudget))?;
    if total > limit {
        return Err(failure(ErrorKind::ContextBudget));
    }
    Ok((limit, limit - total))
}

impl ContextBuilder {
    pub fn new(system: String, generation_prompt: &'static str) -> Self {
        Self {
            system,
            generation_prompt,
            history: VecDeque::new(),
            bytes: 0,
            evicted_turns: 0,
            evicted_tokens: 0,
        }
    }
    pub fn reset(&mut self) {
        self.history.clear();
        self.bytes = 0;
        self.evicted_turns = 0;
        self.evicted_tokens = 0;
    }
    pub fn prepare(
        &mut self,
        model: &LlamaModel,
        input: &UserInput,
        runtime_limit: u32,
    ) -> Result<PreparedPrompt> {
        let started = std::time::Instant::now();
        loop {
            let tokens = self.render(model, input)?;
            let count =
                u32::try_from(tokens.len()).map_err(|_| failure(ErrorKind::ContextBudget))?;
            if let Ok((limit, headroom)) = context_budget(count, model.n_ctx_train(), runtime_limit)
            {
                let metrics = TurnMetrics {
                    prompt_render_ms: started.elapsed().as_secs_f64() * 1000.0,
                    prompt_tokens: count,
                    uncached_prompt_tokens: count,
                    context_tokens: limit,
                    headroom_tokens: headroom,
                    evicted_turns: std::mem::take(&mut self.evicted_turns),
                    evicted_tokens: std::mem::take(&mut self.evicted_tokens),
                    ..Default::default()
                };
                return Ok(PreparedPrompt { tokens, metrics });
            }
            if self.history.is_empty() {
                return Err(failure(ErrorKind::ContextBudget));
            }
            self.evict();
        }
    }
    fn render(&self, model: &LlamaModel, input: &UserInput) -> Result<Vec<LlamaToken>> {
        let mut tokens = Profile::control(model, "<|turn>system\n", true)?;
        tokens.extend(Profile::literal(model, INSTRUCTION)?);
        tokens.extend(Profile::literal(model, &self.system)?);
        tokens.extend(Profile::control(model, "<turn|>\n", false)?);
        for turn in &self.history {
            tokens.extend(render_exchange(model, turn)?);
        }
        tokens.extend(Profile::control(model, "<|turn>user\n", false)?);
        if input.truncated() {
            tokens.extend(Profile::literal(
                model,
                "[Capture reached its duration limit.]\n",
            )?);
        }
        tokens.extend(Profile::literal(model, input.text())?);
        tokens.extend(Profile::control(model, self.generation_prompt, false)?);
        Ok(tokens)
    }
    pub fn commit(&mut self, model: &LlamaModel, record: &FinalizedTurn) -> Result<()> {
        let assistant = record.assistant.join(" ");
        let note = if record.partial || assistant.is_empty() || record.user.truncated() {
            format!(
                "Previous response: generation {:?}; playback {:?}; partial {}; capture truncated {}.",
                record.generation_stop,
                record.playback_stop,
                record.partial,
                record.user.truncated()
            )
        } else {
            String::new()
        };
        let bytes = record.user.text().len() + assistant.len() + note.len();
        let mut exchange = Exchange {
            user: record.user.text().into(),
            assistant,
            note,
            bytes,
            tokens: 0,
        };
        exchange.tokens = u32::try_from(render_exchange(model, &exchange)?.len())
            .map_err(|_| failure(ErrorKind::ContextBudget))?;
        self.history.push_back(exchange);
        self.bytes += bytes;
        while self.history.len() > policy::HISTORY_TURNS || self.bytes > policy::HISTORY_BYTES {
            self.evict();
        }
        Ok(())
    }
    fn evict(&mut self) {
        let old = self
            .history
            .pop_front()
            .expect("eviction requires a retained exchange");
        self.bytes -= old.bytes;
        self.evicted_turns += 1;
        self.evicted_tokens += old.tokens;
    }
}

#[cfg(test)]
mod tests;

fn render_exchange(model: &LlamaModel, exchange: &Exchange) -> Result<Vec<LlamaToken>> {
    let mut tokens = Profile::control(model, "<|turn>user\n", false)?;
    tokens.extend(Profile::literal(model, &exchange.user)?);
    tokens.extend(Profile::control(model, "<turn|>\n", false)?);
    if !exchange.assistant.is_empty() {
        tokens.extend(Profile::control(model, "<|turn>model\n", false)?);
        tokens.extend(Profile::literal(model, &exchange.assistant)?);
        tokens.extend(Profile::control(model, "<turn|>\n", false)?);
    }
    // Metadata is ordinary quoted user context, never fabricated assistant speech.
    if !exchange.note.is_empty() {
        tokens.extend(Profile::control(model, "<|turn>user\n", false)?);
        tokens.extend(Profile::literal(model, &exchange.note)?);
        tokens.extend(Profile::control(model, "<turn|>\n", false)?);
    }
    Ok(tokens)
}
