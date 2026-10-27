//! Fixed Whisper feature extraction shared with endpoint qualification.

use crate::constants::{MODEL_SAMPLES, SAMPLE_RATE_HZ};
#[cfg(test)]
use crate::{Error, Result};
use realfft::{RealFftPlanner, RealToComplex, num_complex::Complex};
use std::sync::Arc;

const FFT: usize = 400;
const BINS: usize = FFT / 2 + 1;
const HOP: usize = 160;
pub(crate) const MELS: usize = 80;
pub(crate) const FRAMES: usize = MODEL_SAMPLES / HOP;
const MEL_FLOOR: f64 = 1e-10;

/// Reusable feature-extraction storage. FFT and mel accumulation use f64.
pub struct Features {
    waveform: Vec<f32>,
    window: Vec<f64>,
    filterbank: Filterbank,
    fft: Arc<dyn RealToComplex<f64>>,
    input: Vec<f64>,
    spectrum: Vec<Complex<f64>>,
    scratch: Vec<Complex<f64>>,
    power: [f64; BINS],
    mels: [f64; MELS],
    output: Vec<f32>,
}

/// A sparse triangular filterbank. Each FFT bin contributes to at most two mels.
struct Filterbank {
    start: [u32; BINS + 1],
    mel: Vec<u8>,
    weight: Vec<f64>,
}

impl Filterbank {
    fn new() -> Self {
        let max_mel = hz_to_mel(f64::from(SAMPLE_RATE_HZ) / 2.0);
        let edges: Vec<f64> = (0..MELS + 2)
            .map(|index| mel_to_hz(max_mel * index as f64 / (MELS + 1) as f64))
            .collect();
        let bin_hz = f64::from(SAMPLE_RATE_HZ) / FFT as f64;
        let mut per_bin: Vec<Vec<(u8, f64)>> = vec![Vec::new(); BINS];
        for (mel, window) in edges.windows(3).enumerate() {
            let (low, center, high) = (window[0], window[1], window[2]);
            for (bin, entries) in per_bin.iter_mut().enumerate() {
                let hz = bin as f64 * bin_hz;
                let down = (hz - low) / (center - low);
                let up = (high - hz) / (high - center);
                let weight = down.min(up).max(0.0) * 2.0 / (high - low);
                if weight != 0.0 {
                    entries.push((mel as u8, weight));
                }
            }
        }
        let mut start = [0_u32; BINS + 1];
        let mut mel = Vec::new();
        let mut weight = Vec::new();
        for (bin, entries) in per_bin.iter().enumerate() {
            start[bin] = mel.len() as u32;
            for (index, value) in entries {
                mel.push(*index);
                weight.push(*value);
            }
        }
        start[BINS] = mel.len() as u32;
        Self { start, mel, weight }
    }
}

impl Features {
    /// Allocate plans and filters once, outside capture callbacks.
    pub fn new() -> Self {
        let fft = RealFftPlanner::new().plan_fft_forward(FFT);
        Self {
            waveform: vec![0.0; MODEL_SAMPLES],
            window: (0..FFT)
                .map(|index| 0.5 - 0.5 * (std::f64::consts::TAU * index as f64 / FFT as f64).cos())
                .collect(),
            filterbank: Filterbank::new(),
            input: fft.make_input_vec(),
            spectrum: fft.make_output_vec(),
            scratch: fft.make_scratch_vec(),
            fft,
            power: [0.0; BINS],
            mels: [0.0; MELS],
            output: vec![0.0; MELS * FRAMES],
        }
    }

    /// Left-pad or keep the last eight seconds, then compute [80, 800] features.
    #[cfg(test)]
    pub fn compute(&mut self, audio: &[f32]) -> Result<&[f32]> {
        if audio.iter().any(|value| !value.is_finite()) {
            return Err(Error::InvalidAudio);
        }
        Ok(self.compute_validated(audio))
    }

    pub(crate) fn compute_validated(&mut self, audio: &[f32]) -> &[f32] {
        let count = audio.len().min(MODEL_SAMPLES);
        self.waveform.fill(0.0);
        self.waveform[MODEL_SAMPLES - count..].copy_from_slice(&audio[audio.len() - count..]);
        let mean = (self
            .waveform
            .iter()
            .map(|value| f64::from(*value))
            .sum::<f64>()
            / MODEL_SAMPLES as f64) as f32;
        let variance = (self
            .waveform
            .iter()
            .map(|value| f64::from((*value - mean).powi(2)))
            .sum::<f64>()
            / MODEL_SAMPLES as f64) as f32;
        let scale = (variance + 1e-7_f32).sqrt();
        for sample in &mut self.waveform {
            *sample = (*sample - mean) / scale;
        }
        let mut maximum = f64::NEG_INFINITY;
        for frame in 0..FRAMES {
            for (index, input) in self.input.iter_mut().enumerate() {
                let position = frame as isize * HOP as isize + index as isize - (FFT / 2) as isize;
                let reflected = if position < 0 {
                    -position
                } else if position >= MODEL_SAMPLES as isize {
                    2 * MODEL_SAMPLES as isize - 2 - position
                } else {
                    position
                } as usize;
                *input = f64::from(self.waveform[reflected]) * self.window[index];
            }
            self.fft
                .process_with_scratch(&mut self.input, &mut self.spectrum, &mut self.scratch)
                .expect("fixed FFT scratch dimensions");
            for (bin, value) in self.spectrum.iter().enumerate() {
                self.power[bin] = value.norm_sqr();
            }
            self.mels.fill(0.0);
            for bin in 0..BINS {
                let power = self.power[bin];
                let start = self.filterbank.start[bin] as usize;
                let end = self.filterbank.start[bin + 1] as usize;
                for index in start..end {
                    let mel = self.filterbank.mel[index] as usize;
                    self.mels[mel] += power * self.filterbank.weight[index];
                }
            }
            for (mel, value) in self.mels.iter().enumerate() {
                let value = value.max(MEL_FLOOR).log10();
                maximum = maximum.max(value);
                self.output[mel * FRAMES + frame] = value as f32;
            }
        }
        let floor = (maximum - 8.0) as f32;
        for value in &mut self.output {
            *value = (value.max(floor) + 4.0) / 4.0;
        }
        &self.output
    }
}

impl Default for Features {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert hertz to the piecewise Slaney mel scale.
fn hz_to_mel(hz: f64) -> f64 {
    if hz < 1000.0 {
        3.0 * hz / 200.0
    } else {
        15.0 + (hz / 1000.0).ln() / (6.4_f64.ln() / 27.0)
    }
}

/// Convert the piecewise Slaney mel scale to hertz.
fn mel_to_hz(mel: f64) -> f64 {
    if mel < 15.0 {
        200.0 * mel / 3.0
    } else {
        1000.0 * ((mel - 15.0) * (6.4_f64.ln() / 27.0)).exp()
    }
}
