//! Worker startup, panic capture, and bounded sends.

use crate::{Error, Result, constants::QUEUE_WAIT_MS, session::Shared};
use crossbeam_channel::{Receiver, SendTimeoutError, Sender};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::atomic::{AtomicBool, Ordering},
    thread::{self, JoinHandle},
    time::Duration,
};

/// The two inference workers.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Role {
    Vad,
    Turn,
}

impl Role {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Vad => "earshot-vad",
            Self::Turn => "earshot-endpoint",
        }
    }
}

/// The terminal result of one worker.
pub(crate) struct Exit {
    pub role: Role,
    pub result: Result<()>,
}

/// Start a named thread. A panic becomes `Error::WorkerPanicked`.
pub(crate) fn spawn(
    name: &'static str,
    body: impl FnOnce() -> Result<()> + Send + 'static,
) -> Result<JoinHandle<Result<()>>> {
    thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            catch_unwind(AssertUnwindSafe(body))
                .unwrap_or_else(|_| Err(Error::WorkerPanicked(name)))
        })
        .map_err(Error::ThreadSpawn)
}

/// Start a named worker and report its terminal result on `exits`.
pub(crate) fn spawn_reported(
    name: &'static str,
    role: Role,
    exits: Sender<Exit>,
    body: impl FnOnce() -> Result<()> + Send + 'static,
) -> Result<JoinHandle<()>> {
    thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            let result = catch_unwind(AssertUnwindSafe(body))
                .unwrap_or_else(|_| Err(Error::WorkerPanicked(name)));
            let _ = exits.try_send(Exit { role, result });
        })
        .map_err(Error::ThreadSpawn)
}

/// Send one value and stop when cancellation is requested.
///
/// Return `Ok(false)` when the stop flag is set.
pub(crate) fn send<T>(sender: &Sender<T>, mut value: T, stop: &AtomicBool) -> Result<bool> {
    loop {
        if stop.load(Ordering::Acquire) {
            return Ok(false);
        }
        match sender.send_timeout(value, Duration::from_millis(QUEUE_WAIT_MS)) {
            Ok(()) => return Ok(true),
            Err(SendTimeoutError::Timeout(returned)) => value = returned,
            Err(SendTimeoutError::Disconnected(_)) => return Err(Error::Closed),
        }
    }
}

/// Wait for a worker to report readiness, then return its join handle.
pub(crate) fn finish_start(
    handle: JoinHandle<()>,
    ready: Receiver<Result<()>>,
) -> Result<JoinHandle<()>> {
    match ready.recv() {
        Ok(Ok(())) => Ok(handle),
        Ok(Err(error)) => {
            let _ = handle.join();
            Err(error)
        }
        Err(_) => {
            let _ = handle.join();
            Err(Error::WorkerPanicked("engine startup"))
        }
    }
}

/// Stop workers and join them.
pub(crate) struct Engines {
    pub stop: std::sync::Arc<AtomicBool>,
    pub handles: Vec<JoinHandle<()>>,
}

impl Drop for Engines {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        for handle in self.handles.drain(..) {
            let _ = handle.join();
        }
    }
}

/// Wake the coordinator after a signal changes shared state.
pub(crate) fn wake(shared: &Shared) {
    let _ = shared.wake.try_send(());
}
