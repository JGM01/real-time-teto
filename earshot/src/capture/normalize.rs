//! Convert native audio to 16 kHz mono `f32` and expose contiguous chunks.

use crate::constants::*;
use rubato::{Fft, FixedSync, Resampler, audioadapter_buffers::direct::SequentialSlice};
use std::ops::Range;

/// A borrowed normalized chunk on the 16 kHz timeline.
pub(crate) struct Chunk<'a> {
    pub samples: &'a [f32],
    pub start_sample: u64,
}

/// Resampler state, reusable buffers, and the normalized output position.
pub(crate) struct Normalizer {
    rate_hz: u32,
    engine: Option<Fft<f32>>,
    input: Box<[f32]>,
    filled: usize,
    output: Box<[f32]>,
    skip: usize,
    available: Range<usize>,
    position: u64,
}

impl Normalizer {
    /// Create reusable buffers and an optional resampler for a validated rate.
    ///
    /// The 16 kHz path copies samples without resampling. Other rates use a
    /// persistent FFT resampler and discard its initial output delay.
    ///
    /// # Panics
    ///
    /// Panics if the resampler rejects the validated rate and fixed configuration.
    pub(crate) fn new(rate_hz: u32) -> Self {
        let engine = if rate_hz == SAMPLE_RATE_HZ {
            None
        } else {
            Some(
                Fft::new(
                    rate_hz as usize,
                    SAMPLE_RATE_HZ as usize,
                    NORMALIZATION_BLOCK_SAMPLES,
                    1,
                    FixedSync::Output,
                )
                .unwrap_or_else(|_| {
                    unreachable!(
                        "validated input rate and fixed output rate configure the resampler"
                    )
                }),
            )
        };
        let input_samples = engine
            .as_ref()
            .map_or(VAD_FRAME_SAMPLES, |engine| engine.input_frames_max());
        let output_samples = engine
            .as_ref()
            .map_or(VAD_FRAME_SAMPLES, |engine| engine.output_frames_max());
        let skip = engine.as_ref().map_or(0, |engine| engine.output_delay());
        Self {
            rate_hz,
            engine,
            input: vec![0.0; input_samples].into_boxed_slice(),
            filled: 0,
            output: vec![0.0; output_samples].into_boxed_slice(),
            skip,
            available: 0..0,
            position: 0,
        }
    }

    #[must_use]
    pub(crate) fn rate_hz(&self) -> u32 {
        self.rate_hz
    }

    #[must_use]
    pub(crate) fn position(&self) -> u64 {
        self.position
    }

    /// Return the number of native mono samples needed for the next block.
    #[must_use]
    pub(crate) fn required(&self) -> usize {
        self.engine
            .as_ref()
            .map_or(VAD_FRAME_SAMPLES, |engine| engine.input_frames_next())
    }

    /// Return whether the input block is full.
    #[must_use]
    pub(crate) fn is_full(&self) -> bool {
        self.filled == self.required()
    }

    /// Return whether normalized output is ready.
    #[must_use]
    pub(crate) fn has_output(&self) -> bool {
        !self.available.is_empty()
    }

    /// Append one native mono sample.
    #[inline]
    pub(crate) fn write(&mut self, sample: f32) {
        assert!(self.filled < self.input.len(), "normalizer input is full");
        self.input[self.filled] = sample;
        self.filled += 1;
    }

    /// Process one input block and expose its valid normalized output.
    ///
    /// At EOF, pad the input as needed and limit output to the target sample count.
    /// Discard the remaining initial filter delay and reuse both buffers.
    ///
    /// # Panics
    ///
    /// Panics if output is undrained, input exceeds the requested block, input is
    /// incomplete before EOF, the EOF target precedes the output position, or the
    /// resampler buffers do not match its configuration.
    pub(crate) fn process(&mut self, eof_target: Option<u64>) {
        assert!(
            self.available.is_empty(),
            "normalizer output is drained before processing"
        );
        let needed = self.required();
        let real = self.filled;
        assert!(real <= needed, "normalizer input fits the requested block");
        assert!(
            eof_target.is_some() || real == needed,
            "normalizer partial input occurs only at EOF"
        );
        assert!(
            eof_target.is_none_or(|target| target >= self.position),
            "normalizer EOF target does not precede output position"
        );
        if real < needed {
            self.input[real..needed].fill(0.0);
        }
        let count = if let Some(engine) = &mut self.engine {
            let input = SequentialSlice::new(&self.input, 1, needed).unwrap_or_else(|_| {
                unreachable!("resized mono input matches its declared dimensions")
            });
            let output_len = self.output.len();
            let mut output = SequentialSlice::new_mut(&mut self.output, 1, output_len)
                .unwrap_or_else(|_| {
                    unreachable!("mono output length matches its declared dimensions")
                });
            engine
                .process_into_buffer(&input, &mut output, None)
                .unwrap_or_else(|_| {
                    unreachable!("normalizer resampler buffers match its configured mono block")
                })
                .1
        } else {
            self.output[..real].copy_from_slice(&self.input[..real]);
            real
        };
        self.filled = 0;
        let skipped = self.skip.min(count);
        self.skip -= skipped;
        let valid = eof_target.map_or(count - skipped, |target| {
            (target - self.position).min((count - skipped) as u64) as usize
        });
        self.available = skipped..skipped + valid;
    }

    /// Copy up to one VAD frame of available output into an owned chunk.
    ///
    /// Advance the output position and return `None` when no output is available.
    ///
    /// # Panics
    ///
    /// Panics if the available output range exceeds the output buffer.
    pub(crate) fn take(&mut self) -> Option<Chunk<'_>> {
        assert!(
            self.available.end <= self.output.len(),
            "normalizer available output fits its buffer"
        );
        if self.available.is_empty() {
            return None;
        }
        let start = self.available.start;
        let end = self.available.end.min(start + VAD_FRAME_SAMPLES);
        let start_sample = self.position;
        self.position += (end - start) as u64;
        self.available.start = end;
        Some(Chunk {
            samples: &self.output[start..end],
            start_sample,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(not(debug_assertions))]
    use std::time::Duration;

    #[test]
    fn sixteen_kilohertz_passthrough_keeps_bits() {
        let mut normalizer = Normalizer::new(SAMPLE_RATE_HZ);
        let samples = [0.0_f32, -0.0, 0.125, f32::MIN_POSITIVE];
        for sample in samples {
            normalizer.write(sample);
        }
        for _ in samples.len()..VAD_FRAME_SAMPLES {
            normalizer.write(0.25);
        }
        normalizer.process(None);
        let chunk = normalizer.take().unwrap();
        assert_eq!(chunk.start_sample, 0);
        assert_eq!(chunk.samples[..samples.len()], samples);
    }

    #[test]
    fn take_returns_one_vad_frame() {
        let mut normalizer = Normalizer::new(SAMPLE_RATE_HZ);
        for _ in 0..VAD_FRAME_SAMPLES {
            normalizer.write(0.5);
        }
        normalizer.process(None);
        let chunk = normalizer.take().unwrap();
        assert_eq!(chunk.samples.len(), VAD_FRAME_SAMPLES);
        assert_eq!(chunk.start_sample, 0);
        assert!(normalizer.take().is_none());
    }

    /// Measure warmed normalization blocks against processing and buffering budgets.
    #[cfg(not(debug_assertions))]
    #[test]
    #[ignore = "release-only normalization budget gate"]
    fn release_normalization_budgets() {
        for rate_hz in SUPPORTED_INPUT_RATES_HZ {
            let mut normalizer = Normalizer::new(rate_hz);
            let delay = normalizer.skip;
            let native = vec![[8192_i16, -4096_i16]; normalizer.input.len()];
            let mut maximum_input = 0;
            let mut maximum = Duration::ZERO;
            for block in 0..10_100 {
                let needed = normalizer.required();
                maximum_input = maximum_input.max(needed);
                let start = std::time::Instant::now();
                for pair in std::hint::black_box(&native[..needed]) {
                    normalizer.write(
                        0.5 * (f32::from(pair[0]) / 32768.0) + 0.5 * (f32::from(pair[1]) / 32768.0),
                    );
                }
                normalizer.process(None);
                let elapsed = start.elapsed();
                if block >= 100 {
                    maximum = maximum.max(elapsed);
                }
                normalizer.available = 0..0;
            }
            let buffering = maximum_input as f64 * 1000.0 / f64::from(rate_hz)
                + delay as f64 * 1000.0 / f64::from(SAMPLE_RATE_HZ);
            assert!(
                buffering <= NORMALIZATION_BUFFERING_BUDGET_MS as f64,
                "normalizer buffering stays within its budget at {rate_hz} Hz: {buffering}ms"
            );
            println!("{rate_hz}Hz: 10,000 blocks, max {maximum:?}, buffering {buffering:.3}ms");
            assert!(maximum <= Duration::from_millis(NORMALIZATION_BLOCK_BUDGET_MS));
        }
    }
}
