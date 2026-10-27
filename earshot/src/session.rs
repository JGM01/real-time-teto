//! Shared control state and the coordinator mailbox.

use crate::{Event, Result, runtime::Exit, types::ActivationId as Id};
use crossbeam_channel::{Receiver, Sender};
use std::sync::atomic::{AtomicBool, AtomicU64};

/// State shared by the public handles and the coordinator.
pub(crate) struct Shared {
    pub shutdown: AtomicBool,
    pub cancelled: AtomicU64,
    pub received: AtomicU64,
    pub wake: Sender<()>,
}

/// One admission request. The coordinator replies with the new ID or an error.
pub(crate) struct Activate {
    pub reply: Sender<Result<Id>>,
}

/// The coordinator's input and output channels.
pub(crate) struct Mailbox {
    pub commands: Receiver<Activate>,
    pub events: Sender<Event>,
    pub wake: Receiver<()>,
    pub exits: Receiver<Exit>,
}
