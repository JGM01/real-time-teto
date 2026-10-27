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
    history_tokens: u32,
}

pub(crate) struct PreparedPrompt {
    pub tokens: Vec<LlamaToken>,
    pub metrics: TurnMetrics,
}

/// Return the context limit and the token headroom.
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

fn prompt_budget(profile: u32, runtime: u32) -> Result<u32> {
    let limit = profile.min(runtime).min(policy::CONTEXT_TOKENS);
    limit
        .checked_sub(policy::OUTPUT_TOKENS)
        .ok_or_else(|| failure(ErrorKind::ContextBudget))
}

impl ContextBuilder {
    pub fn new(system: String, generation_prompt: &'static str) -> Self {
        Self {
            system,
            generation_prompt,
            history: VecDeque::new(),
            bytes: 0,
            history_tokens: 0,
        }
    }

    pub fn reset(&mut self) {
        self.history.clear();
        self.bytes = 0;
        self.history_tokens = 0;
    }

    /// Render one prompt. Evict to the low-water mark when the budget is full.
    pub fn prepare(
        &mut self,
        model: &LlamaModel,
        input: &UserInput,
        runtime_limit: u32,
    ) -> Result<PreparedPrompt> {
        let started = std::time::Instant::now();
        let mut tokens = self.render(model, input)?;
        let mut count =
            u32::try_from(tokens.len()).map_err(|_| failure(ErrorKind::ContextBudget))?;
        let mut evicted_exchanges = 0;
        let mut evicted_tokens = 0;
        if context_budget(count, model.n_ctx_train(), runtime_limit).is_err() {
            if self.history.is_empty() {
                return Err(failure(ErrorKind::ContextBudget));
            }
            let low_water = prompt_budget(model.n_ctx_train(), runtime_limit)?
                .saturating_mul(policy::HISTORY_LOW_WATER_PERCENT)
                / 100;
            let pinned = count.saturating_sub(self.history_tokens);
            let target = low_water.saturating_sub(pinned);
            for _ in 0..2 {
                while !self.history.is_empty() && self.history_tokens > target {
                    let old = self.evict();
                    evicted_exchanges += 1;
                    evicted_tokens += old.tokens;
                }
                tokens = self.render(model, input)?;
                count =
                    u32::try_from(tokens.len()).map_err(|_| failure(ErrorKind::ContextBudget))?;
                if context_budget(count, model.n_ctx_train(), runtime_limit).is_ok() {
                    break;
                }
                if self.history.is_empty() {
                    return Err(failure(ErrorKind::ContextBudget));
                }
            }
            if context_budget(count, model.n_ctx_train(), runtime_limit).is_err() {
                while let Some(old) = self.history.pop_front() {
                    self.bytes = self.bytes.saturating_sub(old.bytes);
                    self.history_tokens = self.history_tokens.saturating_sub(old.tokens);
                    evicted_exchanges += 1;
                    evicted_tokens += old.tokens;
                }
                tokens = self.render(model, input)?;
                count =
                    u32::try_from(tokens.len()).map_err(|_| failure(ErrorKind::ContextBudget))?;
            }
        }
        let (limit, headroom) = context_budget(count, model.n_ctx_train(), runtime_limit)?;
        let render_us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
        Ok(PreparedPrompt {
            tokens,
            metrics: TurnMetrics {
                prompt_render_us: render_us,
                prompt_tokens: count,
                context_tokens: limit,
                headroom_tokens: headroom,
                evicted_exchanges,
                evicted_tokens,
                ..Default::default()
            },
        })
    }

    fn render(&self, model: &LlamaModel, input: &UserInput) -> Result<Vec<LlamaToken>> {
        let mut tokens = Vec::with_capacity(policy::CONTEXT_TOKENS as usize / 2);
        Profile::append_control(model, "<|turn>system\n", true, &mut tokens)?;
        Profile::append_literal(model, INSTRUCTION, &mut tokens)?;
        Profile::append_literal(model, &self.system, &mut tokens)?;
        Profile::append_control(model, "<turn|>\n", false, &mut tokens)?;
        for exchange in &self.history {
            append_exchange(model, exchange, &mut tokens)?;
        }
        Profile::append_control(model, "<|turn>user\n", false, &mut tokens)?;
        if input.truncated() {
            Profile::append_literal(
                model,
                "[Capture reached its duration limit.]\n",
                &mut tokens,
            )?;
        }
        Profile::append_literal(model, input.text(), &mut tokens)?;
        Profile::append_control(model, self.generation_prompt, false, &mut tokens)?;
        Ok(tokens)
    }

    /// Add one finalized turn to history.
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
        exchange.tokens = u32::try_from(render_exchange_len(model, &exchange)?)
            .map_err(|_| failure(ErrorKind::ContextBudget))?;
        self.history_tokens = self.history_tokens.saturating_add(exchange.tokens);
        self.bytes = self.bytes.saturating_add(bytes);
        self.history.push_back(exchange);
        while self.bytes > policy::HISTORY_BYTES && !self.history.is_empty() {
            self.evict();
        }
        Ok(())
    }

    fn evict(&mut self) -> Exchange {
        let old = self.history.pop_front().expect("eviction needs history");
        self.bytes = self.bytes.saturating_sub(old.bytes);
        self.history_tokens = self.history_tokens.saturating_sub(old.tokens);
        old
    }
}

fn append_exchange(
    model: &LlamaModel,
    exchange: &Exchange,
    tokens: &mut Vec<LlamaToken>,
) -> Result<()> {
    Profile::append_control(model, "<|turn>user\n", false, tokens)?;
    Profile::append_literal(model, &exchange.user, tokens)?;
    Profile::append_control(model, "<turn|>\n", false, tokens)?;
    if !exchange.assistant.is_empty() {
        Profile::append_control(model, "<|turn>model\n", false, tokens)?;
        Profile::append_literal(model, &exchange.assistant, tokens)?;
        Profile::append_control(model, "<turn|>\n", false, tokens)?;
    }
    if !exchange.note.is_empty() {
        Profile::append_control(model, "<|turn>user\n", false, tokens)?;
        Profile::append_literal(model, &exchange.note, tokens)?;
        Profile::append_control(model, "<turn|>\n", false, tokens)?;
    }
    Ok(())
}

fn render_exchange_len(model: &LlamaModel, exchange: &Exchange) -> Result<usize> {
    let mut tokens = Vec::new();
    append_exchange(model, exchange, &mut tokens)?;
    Ok(tokens.len())
}

#[cfg(test)]
mod tests;
