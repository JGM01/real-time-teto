//! Whisper model loading, inference, and text assembly.

use crate::{Config, Error, Language, elapsed_ms};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

pub(super) struct Whisper {
    context: WhisperContext,
    language: Language,
}

pub(super) struct RunResult {
    pub(super) text: Option<String>,
    pub(super) state_init_ms: f64,
    pub(super) decode_ms: f64,
    pub(super) segments: u64,
}

impl Whisper {
    pub(super) fn load(config: &Config) -> Result<Self, Error> {
        let mut parameters = WhisperContextParameters::default();
        parameters.use_gpu(config.use_gpu);

        let context =
            WhisperContext::new_with_params(&config.model, parameters).map_err(|source| {
                Error::Model {
                    path: config.model.clone(),
                    source,
                }
            })?;

        Ok(Self {
            context,
            language: config.language.clone(),
        })
    }

    pub(super) fn run(
        &self,
        samples: &[f32],
        shutdown: &Arc<AtomicBool>,
    ) -> Result<RunResult, Error> {
        if shutdown.load(Ordering::Acquire) {
            return Ok(RunResult {
                text: None,
                state_init_ms: 0.0,
                decode_ms: 0.0,
                segments: 0,
            });
        }

        let state_started = Instant::now();
        let mut state = self.context.create_state()?;
        let state_init_ms = elapsed_ms(state_started);

        let parameters = inference_parameters(&self.language, shutdown);
        let decode_started = Instant::now();
        let result = state.full(parameters, samples);
        let decode_ms = elapsed_ms(decode_started);

        if shutdown.load(Ordering::Acquire) {
            return Ok(RunResult {
                text: None,
                state_init_ms,
                decode_ms,
                segments: 0,
            });
        }
        result?;

        let mut text = String::new();
        let mut segments = 0;
        for segment in state.as_iter() {
            let part = segment.to_str_lossy()?;
            push_words(&mut text, part.as_ref());
            segments += 1;
        }

        Ok(RunResult {
            text: Some(text),
            state_init_ms,
            decode_ms,
            segments,
        })
    }
}

fn inference_parameters<'a>(
    language: &'a Language,
    shutdown: &Arc<AtomicBool>,
) -> FullParams<'a, 'a> {
    let mut parameters = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });

    match language {
        Language::Auto => parameters.set_language(None),
        Language::Code(code) => parameters.set_language(Some(code.as_str())),
    }

    parameters.set_translate(false);
    parameters.set_no_context(true);
    parameters.set_no_timestamps(true);
    parameters.set_suppress_nst(true);
    parameters.set_print_progress(false);
    parameters.set_print_realtime(false);
    parameters.set_print_special(false);
    parameters.set_print_timestamps(false);

    let inference_shutdown = Arc::clone(shutdown);
    let abort_callback: Box<dyn FnMut() -> bool> =
        Box::new(move || inference_shutdown.load(Ordering::Acquire));
    parameters.set_abort_callback_safe(abort_callback);

    parameters
}

fn push_words(out: &mut String, part: &str) {
    for word in part.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn normalize(input: &str) -> String {
        let mut out = String::new();
        push_words(&mut out, input);
        out
    }

    proptest! {
        #[test]
        fn normalize_collapses_whitespace(input in any::<String>()) {
            let once = normalize(&input);
            prop_assert_eq!(once.trim(), once.as_str());
            prop_assert!(!once.contains("  "));
            let twice = normalize(&once);
            prop_assert_eq!(&twice, &once);
            prop_assert_eq!(
                once.split_whitespace().collect::<Vec<_>>(),
                input.split_whitespace().collect::<Vec<_>>()
            );
        }
    }
}
