//! Earshot activation and terminal event handling.

use crate::Error;
use earshot::{ActivationId, Audio, Earshot, Event, IncompleteReason, Outcome, Timings};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

// The event loop checks the shutdown flag at this interval.
const EVENT_POLL: Duration = Duration::from_millis(50);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Endpoint {
    Complete,
    SilenceLimit,
    DurationLimit,
}

impl Endpoint {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::SilenceLimit => "silence_limit",
            Self::DurationLimit => "duration_limit",
        }
    }

    pub(super) fn truncated(self) -> bool {
        self == Self::DurationLimit
    }
}

pub(super) enum Settled {
    Speech {
        audio: Audio,
        endpoint: Endpoint,
        timings: Timings,
    },
    NoSpeech,
    Cancelled,
}

pub(super) fn await_finished(
    earshot: &mut Earshot,
    id: ActivationId,
    shutdown: &AtomicBool,
) -> Result<Settled, Error> {
    while !shutdown.load(Ordering::Acquire) {
        match earshot.recv_timeout(EVENT_POLL)? {
            None | Some(Event::Listening(_)) => {}
            Some(Event::Finished {
                id: _finished,
                outcome,
            }) => {
                debug_assert_eq!(_finished, id);
                return Ok(match outcome {
                    Outcome::Complete(statement) => {
                        let timings = *statement.timings();
                        Settled::Speech {
                            audio: statement.into_audio(),
                            endpoint: Endpoint::Complete,
                            timings,
                        }
                    }
                    Outcome::Incomplete {
                        audio,
                        reason,
                        timings,
                    } => {
                        let endpoint = match reason {
                            IncompleteReason::SilenceLimit => Endpoint::SilenceLimit,
                            IncompleteReason::DurationLimit => Endpoint::DurationLimit,
                            _ => return Err(Error::UnsupportedOutcome),
                        };
                        Settled::Speech {
                            audio,
                            endpoint,
                            timings,
                        }
                    }
                    Outcome::NoSpeech => Settled::NoSpeech,
                    Outcome::Cancelled => Settled::Cancelled,
                    Outcome::Failed(error) => return Err(error.into()),
                    _ => return Err(Error::UnsupportedOutcome),
                });
            }
            Some(_) => {}
        }
    }

    earshot.control().cancel(id);
    Ok(Settled::Cancelled)
}
