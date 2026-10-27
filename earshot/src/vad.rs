//! Voice activity detection and its owning worker.

use crate::{
    Error, Id, Result,
    capture::{InputSource, pipeline::capture},
    constants::*,
    runtime::{self, Exit, Role},
    trace::{CaptureTrace, TraceArena},
};
use crossbeam_channel::{Receiver, Sender};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

/// One scored, complete VAD window on the normalized source timeline.
#[derive(Debug)]
pub(crate) struct ScoredFrame {
    pub samples: [f32; VAD_FRAME_SAMPLES],
    pub probability: f32,
}

/// A speech detector that owns its model state.
pub(crate) trait VadEngine: Send {
    fn reset(&mut self);
    fn score(&mut self, audio: &[f32; VAD_FRAME_SAMPLES]) -> Result<f32>;
}

/// The bundled Silero detector.
pub(crate) struct Silero {
    session: silero::Session,
    stream: silero::StreamState,
}

impl Silero {
    pub fn new() -> Result<Self> {
        let mut engine = Self {
            session: silero::Session::bundled().map_err(Error::VadInitialization)?,
            stream: silero::StreamState::new(silero::SampleRate::Rate16k),
        };
        for _ in 0..8 {
            engine.score(&[0.0; VAD_FRAME_SAMPLES])?;
        }
        engine.reset();
        Ok(engine)
    }
}

impl VadEngine for Silero {
    fn reset(&mut self) {
        self.stream.reset();
    }

    fn score(&mut self, audio: &[f32; VAD_FRAME_SAMPLES]) -> Result<f32> {
        self.session
            .infer_chunk(&mut self.stream, audio)
            .map_err(Error::VadInference)
    }
}

pub(crate) type VadFactory = Box<dyn FnOnce() -> Result<Box<dyn VadEngine>> + Send>;

/// One capture request for the VAD worker.
pub(crate) struct Listen {
    pub id: Id,
    pub source: InputSource,
    pub stop: Arc<AtomicBool>,
}

/// Messages from the VAD worker to the coordinator.
///
/// The frame is the channel payload; it moves by value on the bounded queue.
#[allow(
    clippy::large_enum_variant,
    reason = "the frame is the channel payload"
)]
pub(crate) enum VadMessage {
    Ready {
        id: Id,
        watermark: crate::capture::Watermark,
        at: Instant,
    },
    Frame {
        id: Id,
        frame: ScoredFrame,
        inference: Duration,
    },
    End {
        id: Id,
        result: Result<()>,
        at: Instant,
        trace: Option<CaptureTrace>,
    },
}

/// Start the VAD worker. It owns one engine for its whole lifetime.
pub(crate) fn start_vad(
    factory: VadFactory,
    requests: Receiver<Listen>,
    events: Sender<VadMessage>,
    stop: Arc<AtomicBool>,
    exits: Sender<Exit>,
    traced: bool,
) -> Result<JoinHandle<()>> {
    let (ready, wait) = crossbeam_channel::bounded(1);
    let handle = runtime::spawn_reported("earshot-vad", Role::Vad, exits, move || {
        let mut engine = match factory() {
            Ok(engine) => engine,
            Err(error) => {
                let _ = ready.send(Err(error));
                return Ok(());
            }
        };
        if ready.send(Ok(())).is_err() {
            return Ok(());
        }
        let mut trace = traced.then(|| TraceArena::new(TRACE_SAMPLES));
        while !stop.load(Ordering::Acquire) {
            let listen = match requests.recv_timeout(Duration::from_millis(QUEUE_WAIT_MS)) {
                Ok(listen) => listen,
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
                Err(_) => break,
            };
            engine.reset();
            let id = listen.id;
            let (result, trace) = capture(&mut *engine, listen, &events, trace.as_mut());
            if !runtime::send(
                &events,
                VadMessage::End {
                    id,
                    result,
                    at: Instant::now(),
                    trace,
                },
                &stop,
            )? {
                break;
            }
        }
        Ok(())
    })?;
    runtime::finish_start(handle, wait)
}

/// Check probabilities at the model boundary.
pub(crate) fn validate_probability(probability: f32) -> Result<()> {
    if probability.is_finite() && (0.0..=1.0).contains(&probability) {
        Ok(())
    } else {
        Err(Error::InvalidProbability(probability))
    }
}
