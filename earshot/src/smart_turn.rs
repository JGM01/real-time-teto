//! Smart Turn v3.2 CPU inference with Whisper log-mel preprocessing.
//!
//! Weights: pipecat-ai/smart-turn-v3, revision f766f81. Feature math follows
//! Pipecat's _whisper_features.py (BSD-2-Clause, with Apache-2.0 portions).
//! The `turn` worker owns one session for its whole lifetime.

#[cfg(all(test, not(debug_assertions)))]
use crate::constants::MODEL_SAMPLES;
use crate::{Error, Result};
use ort::{
    session::{Session, builder::GraphOptimizationLevel},
    value::{TensorElementType, TensorRef},
};

pub(crate) const MODEL_BYTES: &[u8] = include_bytes!("../assets/smart-turn-v3.2-cpu.onnx");
/// The bundled model name.
#[cfg(feature = "tooling")]
pub const MODEL_NAME: &str = "smart-turn-v3.2-cpu";
/// The SHA-256 of the bundled model.
#[cfg(any(test, feature = "tooling"))]
pub const MODEL_SHA256: &str = "2bb026316b14a660486a75b1733cd3fbab8c2fd0314dc9af7be49f8cca967e4f";
/// The upstream revision of the bundled model.
#[cfg(feature = "tooling")]
pub const MODEL_REVISION: &str = "f766f81d3cfdf7737ac64aad813d91bbfd56bf93";
/// The license of the bundled model.
#[cfg(feature = "tooling")]
pub const MODEL_LICENSE: &str = "BSD-2-Clause";
pub(crate) mod features;
use crate::vad::validate_probability;
use features::{FRAMES, Features, MELS};

/// One CPU ONNX session and its private feature storage.
pub(crate) struct SmartTurn {
    session: Session,
    features: Features,
}

impl SmartTurn {
    /// Validate and warm the embedded model before the program becomes ready.
    pub(crate) fn new() -> Result<Self> {
        let session = Session::builder()
            .map_err(Error::TurnInitialization)?
            .with_intra_threads(1)
            .map_err(|error| Error::TurnInitialization(error.into()))?
            .with_inter_threads(1)
            .map_err(|error| Error::TurnInitialization(error.into()))?
            .with_parallel_execution(false)
            .map_err(|error| Error::TurnInitialization(error.into()))?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|error| Error::TurnInitialization(error.into()))?
            .commit_from_memory(MODEL_BYTES)
            .map_err(Error::TurnInitialization)?;
        if session.inputs().len() != 1 || session.outputs().len() != 1 {
            return Err(Error::ModelContract);
        }
        let input = &session.inputs()[0];
        let shape = input.dtype().tensor_shape().ok_or(Error::ModelContract)?;
        if input.name() != "input_features"
            || input.dtype().tensor_type() != Some(TensorElementType::Float32)
            || shape.len() != 3
            || !matches!(shape[0], -1 | 1)
            || shape[1] != MELS as i64
            || shape[2] != FRAMES as i64
            || session.outputs()[0].dtype().tensor_type() != Some(TensorElementType::Float32)
        {
            return Err(Error::ModelContract);
        }
        let mut model = Self {
            session,
            features: Features::new(),
        };
        for _ in 0..3 {
            model.predict(&[0.0; 512])?;
        }
        Ok(model)
    }

    /// Evaluate audio on the owning worker. Padding is private to preprocessing.
    pub(crate) fn predict(&mut self, audio: &[f32]) -> Result<f32> {
        let features = self.features.compute_validated(audio);
        let tensor = TensorRef::from_array_view(([1_usize, MELS, FRAMES], features))
            .map_err(Error::TurnInference)?;
        let outputs = self
            .session
            .run(ort::inputs!["input_features" => tensor])
            .map_err(Error::TurnInference)?;
        let (_, values) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(Error::TurnInference)?;
        if values.len() != 1 {
            return Err(Error::ModelContract);
        }
        let probability = values[0];
        validate_probability(probability)?;
        Ok(probability)
    }
}

#[cfg(test)]
mod tests {
    //! Compare native features and model probabilities with independent fixtures.
    use super::*;
    use sha2::{Digest, Sha256};

    fn signal(name: &str, count: usize) -> Vec<f32> {
        (0..count)
            .map(|index| {
                if name == "silence" {
                    0.0
                } else {
                    let x = index as f64;
                    (0.17 * (x * 0.071).sin() + 0.05 * (x * x * 0.000001).cos()) as f32
                }
            })
            .collect()
    }

    #[test]
    fn model_checksum_and_reference_parity() {
        assert_eq!(format!("{:x}", Sha256::digest(MODEL_BYTES)), MODEL_SHA256);
        let metadata: serde_json::Value =
            serde_json::from_str(include_str!("../fixtures/smart-turn/reference.json")).unwrap();
        let mut model = SmartTurn::new().unwrap();
        for case in metadata["cases"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let audio = signal(name, case["samples"].as_u64().unwrap() as usize);
            let expected = std::fs::read(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join(format!("fixtures/smart-turn/{name}.features.f32le")),
            )
            .unwrap();
            let features = model.features.compute(&audio).unwrap();
            let maximum = features
                .iter()
                .zip(expected.chunks_exact(4))
                .map(|(actual, bytes)| {
                    (actual - f32::from_le_bytes(bytes.try_into().unwrap())).abs()
                })
                .fold(0.0_f32, f32::max);
            assert!(maximum <= 1e-4, "{name}: feature max error {maximum}");
            let probability = model.predict(&audio).unwrap();
            let reference = case["probability"].as_f64().unwrap() as f32;
            assert!(
                (probability - reference).abs() <= 1e-3,
                "{name}: score {probability}, expected {reference}"
            );
        }
    }

    #[test]
    fn nonfinite_input_and_invalid_probabilities_are_rejected() {
        let mut features = Features::new();
        assert!(matches!(
            features.compute(&[f32::NAN]),
            Err(Error::InvalidAudio)
        ));
        for probability in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
            assert!(validate_probability(probability).is_err());
        }
        for probability in [0.0, 0.8, 1.0] {
            validate_probability(probability).unwrap();
        }
    }

    /// The performance gate is measured only in release mode on the development Mac.
    #[cfg(not(debug_assertions))]
    #[test]
    #[ignore = "release-only ARM64 model budget gate"]
    fn release_model_budgets() {
        use crate::vad::VadEngine;
        use std::time::Instant;
        let mut model = SmartTurn::new().unwrap();
        let audio = signal("short", MODEL_SAMPLES);
        for _ in 0..20 {
            model.predict(&audio).unwrap();
        }
        let mut times = Vec::with_capacity(200);
        for _ in 0..200 {
            let start = Instant::now();
            model.predict(std::hint::black_box(&audio)).unwrap();
            times.push(start.elapsed());
        }
        times.sort();
        println!(
            "Smart Turn preprocessing+inference p50 {:?}, p95 {:?}, max {:?}",
            times[99], times[189], times[199]
        );
        assert!(times[189] <= std::time::Duration::from_millis(100));
        let mut vad = crate::vad::Silero::new().unwrap();
        let mut maximum = std::time::Duration::ZERO;
        for _ in 0..200 {
            let start = Instant::now();
            vad.score(&[0.1; 512]).unwrap();
            maximum = maximum.max(start.elapsed());
        }
        println!("Silero maximum frame processing: {maximum:?}");
        assert!(maximum <= std::time::Duration::from_millis(10));
    }
}
