//! Public handles and the admission protocol.
//!
//! Only the coordinator changes activation state. `Earshot` owns the event
//! receiver. `Control` is a cloneable intent handle.

use crate::{
    Error, Event, Id, Result,
    constants::{COMMAND_CAPACITY, EVENT_CAPACITY, EXIT_CAPACITY, QUEUE_WAIT_MS},
    coordinator::{Coordinator, Setup},
    runtime,
    session::{Activate, Mailbox, Shared},
};
use crossbeam_channel::{Receiver, Sender, TrySendError};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};

/// Cloneable intent handle. It owns neither capture nor inference workers.
///
/// `Control` is [`Send`] and [`Sync`], so any thread may hold a clone. All
/// methods share one admission state with the [`Earshot`] owner.
#[derive(Clone)]
pub struct Control {
    commands: Sender<Activate>,
    shared: Arc<Shared>,
}

impl Control {
    /// Accept one activation.
    ///
    /// Blocks until the coordinator replies. Return `Busy` while capture or an
    /// unread outcome exists. Admission waits only for the coordinator, never
    /// for microphone readiness.
    ///
    /// # Errors
    ///
    /// Returns `Closed` after the owner closes, or `Busy` when an activation is
    /// already in progress.
    pub fn activate(&self) -> Result<Id> {
        if self.shared.shutdown.load(Ordering::Acquire) {
            return Err(Error::Closed);
        }
        let (reply, result) = crossbeam_channel::bounded(1);
        match self.commands.try_send(Activate { reply }) {
            Ok(()) => loop {
                if self.shared.shutdown.load(Ordering::Acquire) {
                    return Err(Error::Closed);
                }
                match result.recv_timeout(Duration::from_millis(QUEUE_WAIT_MS)) {
                    Ok(reply) => return reply,
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => (),
                    Err(_) => return Err(Error::Closed),
                }
            },
            Err(TrySendError::Full(_)) => Err(Error::Busy),
            Err(TrySendError::Disconnected(_)) => Err(Error::Closed),
        }
    }

    /// Cancel one activation.
    ///
    /// IDs belong to the Earshot that issued this handle. A stale request cannot
    /// cancel a newer activation. Queue saturation cannot lose cancellation.
    pub fn cancel(&self, id: Id) {
        self.shared.cancelled.fetch_max(id.0, Ordering::AcqRel);
        runtime::wake(&self.shared);
    }
}

/// Single event owner. Receiving `Finished` releases the next activation.
///
/// `Earshot::open` loads and warms both engines. It does not open the
/// microphone. One accepted attempt emits at most one `Listening` event and
/// exactly one `Finished` outcome. [`Earshot::recv`] releases the next
/// activation when it receives `Finished`; there is no separate consumer
/// acknowledgment.
///
/// `close` and `Drop` stop capture and join every worker. `close` can wait for
/// an in-flight model call. Closing never waits for an event consumer.
///
/// `Earshot` is [`Send`] but is the single event owner: keep the event loop on
/// one thread and use [`Control`] from other threads. The crate does not
/// promise [`Sync`].
pub struct Earshot {
    control: Control,
    events: Receiver<Event>,
    worker: Option<JoinHandle<Result<()>>>,
}

impl Earshot {
    /// Load and warm both engines on their owning threads. Leave the microphone closed.
    pub fn open() -> Result<Self> {
        Self::start(Setup::microphone())
    }

    pub(crate) fn start(setup: Setup) -> Result<Self> {
        let (commands, command_rx) = crossbeam_channel::bounded(COMMAND_CAPACITY);
        let (event_tx, events) = crossbeam_channel::bounded(EVENT_CAPACITY);
        let (ready_tx, ready_rx) = crossbeam_channel::bounded(1);
        let (wake, wake_rx) = crossbeam_channel::bounded(1);
        let (exit_tx, exit_rx) = crossbeam_channel::bounded(EXIT_CAPACITY);
        let shared = Arc::new(Shared {
            shutdown: AtomicBool::new(false),
            cancelled: AtomicU64::new(0),
            received: AtomicU64::new(0),
            wake,
        });
        let worker_shared = shared.clone();
        let mailbox = Mailbox {
            commands: command_rx,
            events: event_tx,
            wake: wake_rx,
            exits: exit_rx,
        };
        let worker = runtime::spawn("earshot-coordinator", move || {
            let mut core = match Coordinator::new(setup, mailbox, exit_tx, worker_shared) {
                Ok(core) => core,
                Err(error) => {
                    let _ = ready_tx.send(Err(error));
                    return Ok(());
                }
            };
            if ready_tx.send(Ok(())).is_err() {
                return Ok(());
            }
            core.run()
        })?;
        let mut owner = Self {
            control: Control { commands, shared },
            events,
            worker: Some(worker),
        };
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(owner),
            Ok(Err(error)) => {
                owner.stop()?;
                Err(error)
            }
            Err(_) => {
                owner.stop()?;
                Err(Error::Closed)
            }
        }
    }

    /// Clone an intent handle. Keep the event receiver on this owner.
    #[must_use]
    pub fn control(&self) -> Control {
        self.control.clone()
    }

    /// Block for the next event. Receiving `Finished` releases activation admission.
    ///
    /// # Errors
    ///
    /// Returns `Error::Closed` after the owner closes.
    pub fn recv(&mut self) -> Result<Event> {
        let event = self.events.recv().map_err(|_| Error::Closed)?;
        self.receipt(&event);
        Ok(event)
    }

    /// Receive an event, or None when the bounded wait expires.
    ///
    /// # Errors
    ///
    /// Returns `Error::Closed` after the owner closes.
    pub fn recv_timeout(&mut self, timeout: Duration) -> Result<Option<Event>> {
        match self.events.recv_timeout(timeout) {
            Ok(event) => {
                self.receipt(&event);
                Ok(Some(event))
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => Ok(None),
            Err(_) => Err(Error::Closed),
        }
    }

    fn receipt(&self, event: &Event) {
        if let Event::Finished { id, .. } = event {
            self.control.shared.received.store(id.0, Ordering::Release);
        }
    }

    /// Stop capture and join all workers. Pending outcomes need not be read.
    ///
    /// # Errors
    ///
    /// Returns the coordinator failure, if any.
    pub fn close(mut self) -> Result<()> {
        self.stop()
    }

    fn stop(&mut self) -> Result<()> {
        self.control.shared.shutdown.store(true, Ordering::Release);
        runtime::wake(&self.control.shared);
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .unwrap_or(Err(Error::WorkerPanicked("earshot-coordinator")))?;
        }
        Ok(())
    }
}

impl Drop for Earshot {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
