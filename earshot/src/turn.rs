//! Semantic endpoint inference and its owning worker.

use crate::{
    Id, Result,
    constants::QUEUE_WAIT_MS,
    endpoint::Snapshot,
    runtime::{self, Exit, Role},
    smart_turn::SmartTurn,
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

/// Inputs are checked finite audio. Successful predictions satisfy the model contract.
pub(crate) trait TurnEngine: Send {
    fn predict(&mut self, audio: &[f32]) -> Result<f32>;
}

impl TurnEngine for SmartTurn {
    fn predict(&mut self, audio: &[f32]) -> Result<f32> {
        SmartTurn::predict(self, audio)
    }
}

pub(crate) type TurnFactory = Box<dyn FnOnce() -> Result<Box<dyn TurnEngine>> + Send>;

/// One model request. The snapshot buffer returns with the result.
pub(crate) struct TurnRequest {
    pub id: Id,
    pub snapshot: Snapshot,
    pub request: u64,
    pub audio: Vec<f32>,
    pub cancel: Arc<AtomicBool>,
}

/// The model result or a cancellation observed around inference.
pub(crate) enum Inference {
    Cancelled,
    Scored(Result<f32>),
}

/// One model result with the buffers and time spent on the worker.
pub(crate) struct TurnResult {
    pub id: Id,
    pub snapshot: Snapshot,
    pub request: u64,
    pub audio: Vec<f32>,
    pub inference: Inference,
    pub elapsed: Duration,
}

/// Start the endpoint worker. It owns one engine for its whole lifetime.
pub(crate) fn start_turn(
    factory: TurnFactory,
    requests: Receiver<TurnRequest>,
    results: Sender<TurnResult>,
    stop: Arc<AtomicBool>,
    exits: Sender<Exit>,
) -> Result<JoinHandle<()>> {
    let (ready, wait) = crossbeam_channel::bounded(1);
    let handle = runtime::spawn_reported("earshot-endpoint", Role::Turn, exits, move || {
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
        while !stop.load(Ordering::Acquire) {
            let request = match requests.recv_timeout(Duration::from_millis(QUEUE_WAIT_MS)) {
                Ok(request) => request,
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
                Err(_) => break,
            };
            if stop.load(Ordering::Acquire) {
                break;
            }
            let started = Instant::now();
            let inference = if request.cancel.load(Ordering::Acquire) {
                Inference::Cancelled
            } else {
                let result = engine.predict(&request.audio);
                if request.cancel.load(Ordering::Acquire) {
                    Inference::Cancelled
                } else {
                    Inference::Scored(result)
                }
            };
            let result = TurnResult {
                id: request.id,
                snapshot: request.snapshot,
                request: request.request,
                audio: request.audio,
                inference,
                elapsed: started.elapsed(),
            };
            if !runtime::send(&results, result, &stop)? {
                break;
            }
        }
        Ok(())
    })?;
    runtime::finish_start(handle, wait)
}
