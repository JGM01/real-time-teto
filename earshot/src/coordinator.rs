//! Central lifecycle owner.
//!
//! The event queue reserves capacity for one ready notification and one outcome.
//! Receipt, not downstream work, releases admission. The coordinator sleeps on a
//! select and wakes for commands, frames, results, exits, cancellation, or a
//! no-speech deadline.

use crate::{
    Error, Event, Id, IncompleteReason, Outcome, Result, Statement, Timings,
    capture::InputSource,
    capture::{ReplayPacing, Watermark},
    constants::*,
    endpoint::{Endpoint, Flow, Keep, StopReason},
    policy::Policy,
    runtime::{Engines, Exit, Role},
    session::{Activate, Mailbox, Shared},
    smart_turn::SmartTurn,
    trace::{CaptureTrace, Log, mark_accepted, mark_discarded, mark_stale},
    turn::{Inference, TurnFactory, TurnRequest, TurnResult},
    vad::{Listen, Silero, VadFactory, VadMessage},
};
#[cfg(feature = "tooling")]
use crate::{
    smart_turn::MODEL_SHA256,
    trace::{Sink, Trace},
};
use crossbeam_channel::{Receiver, Select, Sender};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

pub(crate) type SourceFactory = Box<dyn FnMut() -> Result<InputSource> + Send>;

/// Runtime setup chosen by the application or the audit tools.
pub(crate) struct Setup {
    pub policy: Policy,
    pub source: SourceFactory,
    pub vad: VadFactory,
    pub turn: TurnFactory,
    #[cfg(feature = "tooling")]
    pub traces: Option<Sink>,
}

impl Setup {
    pub fn microphone() -> Self {
        Self {
            policy: Policy::default(),
            source: Box::new(|| Ok(InputSource::Microphone)),
            vad: Box::new(|| Ok(Box::new(Silero::new()?))),
            turn: Box::new(|| Ok(Box::new(SmartTurn::new()?))),
            #[cfg(feature = "tooling")]
            traces: None,
        }
    }
}

/// The per-capture stop flag.
struct Stop(Arc<AtomicBool>);

impl Drop for Stop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

/// The clock kind for one capture. The clock owns all replay-time behavior.
enum Clock {
    Opening {
        requested: Instant,
        pacing: ReplayPacing,
    },
    Realtime {
        at: Instant,
        watermark: Watermark,
    },
    Simulated,
}

impl Clock {
    fn target(&self, scheduled_at: u64, elapsed: Duration) -> u64 {
        match self {
            Self::Realtime { watermark, .. } => watermark.complete_sample(),
            Self::Simulated => {
                let samples = (elapsed.as_secs_f64() * f64::from(SAMPLE_RATE_HZ)).ceil() as u64;
                scheduled_at + samples
            }
            Self::Opening { .. } => unreachable!("a decision needs a ready capture"),
        }
    }

    fn after_ready_ms(&self, end_sample: u64) -> f64 {
        match self {
            Self::Realtime { at, .. } => at.elapsed().as_secs_f64() * 1000.0,
            Self::Simulated | Self::Opening { .. } => {
                end_sample as f64 * 1000.0 / f64::from(SAMPLE_RATE_HZ)
            }
        }
    }

    fn freezes_inference(&self) -> bool {
        matches!(self, Self::Simulated)
    }

    fn no_speech_deadline(&self, no_speech_samples: u64) -> Option<Instant> {
        match self {
            Self::Realtime { at, .. } => Some(
                *at + Duration::from_secs_f64(no_speech_samples as f64 / f64::from(SAMPLE_RATE_HZ)),
            ),
            Self::Simulated | Self::Opening { .. } => None,
        }
    }
}

/// The current semantic decision for one pause.
enum Decision {
    Accept,
    Fail(Error),
}

/// The evidence for one pause, if any.
enum Evidence {
    Clear,
    Pending {
        decision: Decision,
        revision: u64,
        target: u64,
        log: Option<usize>,
    },
}

/// The mutable state for one accepted activation.
struct Capture {
    id: Id,
    endpoint: Endpoint,
    stop: Stop,
    clock: Clock,
    evidence: Evidence,
    timings: Timings,
    log: Option<Log>,
}

/// The reason that the current capture is finalizing.
enum Finish {
    Complete,
    Incomplete(IncompleteReason),
    NoSpeech,
    Cancelled,
    Failed(Error),
}

struct Finalizing {
    capture: Capture,
    reason: Finish,
    started: Instant,
}

enum State {
    Idle,
    Capturing(Box<Capture>),
    Finalizing(Box<Finalizing>),
    OutcomePending(Id),
}

#[allow(
    clippy::large_enum_variant,
    reason = "the frame moves once from the queue"
)]
enum Woke {
    Command(Activate),
    Wake,
    Turn(TurnResult),
    Exit(Exit),
    Vad(VadMessage),
    NoSpeech,
}

pub(crate) struct Coordinator {
    state: State,
    policy: Policy,
    source: SourceFactory,
    shared: Arc<Shared>,
    mailbox: Mailbox,
    #[cfg(feature = "tooling")]
    traces: Option<Sink>,
    audio_arena: Vec<f32>,
    vad_tx: Sender<Listen>,
    vad_rx: Receiver<VadMessage>,
    turn_tx: Sender<TurnRequest>,
    turn_rx: Receiver<TurnResult>,
    _engines: Engines,
    next_id: u64,
    next_request: u64,
    flight: Option<(Id, u64)>,
    snapshot: Vec<f32>,
}

impl Coordinator {
    pub fn new(
        setup: Setup,
        mailbox: Mailbox,
        exits: Sender<Exit>,
        shared: Arc<Shared>,
    ) -> Result<Self> {
        #[cfg(feature = "tooling")]
        let traces = setup.traces;
        #[cfg(feature = "tooling")]
        let traced = traces.is_some();
        #[cfg(not(feature = "tooling"))]
        let traced = false;
        let Setup {
            policy,
            source,
            vad,
            turn,
            ..
        } = setup;
        let (vad_tx, requests) = crossbeam_channel::bounded(1);
        let (scored, vad_rx) = crossbeam_channel::bounded(SCORED_CAPACITY);
        let (turn_tx, turns) = crossbeam_channel::bounded(MODEL_CAPACITY);
        let (predictions, turn_rx) = crossbeam_channel::bounded(MODEL_CAPACITY);
        let stop = Arc::new(AtomicBool::new(false));
        let mut engines = Engines {
            stop: stop.clone(),
            handles: Vec::with_capacity(2),
        };
        engines.handles.push(crate::vad::start_vad(
            vad,
            requests,
            scored,
            stop.clone(),
            exits.clone(),
            traced,
        )?);
        engines.handles.push(crate::turn::start_turn(
            turn,
            turns,
            predictions,
            stop,
            exits,
        )?);
        Ok(Self {
            state: State::Idle,
            policy,
            source,
            shared,
            mailbox,
            #[cfg(feature = "tooling")]
            traces,
            audio_arena: Vec::with_capacity(policy.maximum_samples),
            vad_tx,
            vad_rx,
            turn_tx,
            turn_rx,
            _engines: engines,
            next_id: 1,
            next_request: 1,
            flight: None,
            snapshot: Vec::with_capacity(MODEL_SAMPLES),
        })
    }

    pub fn run(&mut self) -> Result<()> {
        loop {
            if self.shared.shutdown.load(Ordering::Acquire) {
                return Ok(());
            }
            self.receipt();
            match self.wait()? {
                Woke::Command(command) => self.activate(command)?,
                Woke::Wake => self.on_wake()?,
                Woke::Turn(result) => {
                    self.result(result)?;
                    self.accept()?;
                }
                Woke::Exit(exit) => self.engine_lost(exit)?,
                Woke::Vad(message) => {
                    self.message(message)?;
                    self.cancellation();
                    self.accept()?;
                    self.schedule()?;
                }
                Woke::NoSpeech => self.finish(Finish::NoSpeech),
            }
        }
    }

    fn wait(&mut self) -> Result<Woke> {
        let deadline = self.no_speech_deadline();
        let accepts = self.accepts_frames();
        let mut select = Select::new();
        select.recv(&self.mailbox.commands);
        select.recv(&self.mailbox.wake);
        select.recv(&self.turn_rx);
        select.recv(&self.mailbox.exits);
        if accepts {
            select.recv(&self.vad_rx);
        }
        let selected = match deadline {
            Some(deadline) => select
                .select_timeout(deadline.saturating_duration_since(Instant::now()))
                .ok(),
            None => Some(select.select()),
        };
        let Some(operation) = selected else {
            return Ok(Woke::NoSpeech);
        };
        let index = operation.index();
        match index {
            0 => Ok(Woke::Command(
                operation
                    .recv(&self.mailbox.commands)
                    .map_err(|_| Error::Closed)?,
            )),
            1 => {
                let _ = operation.recv(&self.mailbox.wake);
                Ok(Woke::Wake)
            }
            2 => match operation.recv(&self.turn_rx) {
                Ok(result) => Ok(Woke::Turn(result)),
                Err(_) => Ok(Woke::Exit(self.take_exit(Role::Turn))),
            },
            3 => Ok(Woke::Exit(
                operation
                    .recv(&self.mailbox.exits)
                    .map_err(|_| Error::Closed)?,
            )),
            4 if accepts => match operation.recv(&self.vad_rx) {
                Ok(message) => Ok(Woke::Vad(message)),
                Err(_) => Ok(Woke::Exit(self.take_exit(Role::Vad))),
            },
            _ => unreachable!("select returns a registered operation"),
        }
    }

    fn take_exit(&self, role: Role) -> Exit {
        // A worker drops its output sender before it reports its exit.
        // Wait for the report so a panic keeps its name.
        self.mailbox
            .exits
            .recv_timeout(Duration::from_millis(QUEUE_WAIT_MS))
            .unwrap_or(Exit {
                role,
                result: Err(Error::WorkerExited(role.name())),
            })
    }

    fn receipt(&mut self) {
        if let State::OutcomePending(id) = self.state
            && self.shared.received.load(Ordering::Acquire) >= id.0
        {
            self.state = State::Idle;
        }
    }

    fn on_wake(&mut self) -> Result<()> {
        let _ = self.mailbox.wake.try_recv();
        if self.shared.shutdown.load(Ordering::Acquire) {
            return Ok(());
        }
        self.cancellation();
        Ok(())
    }

    fn accepts_frames(&self) -> bool {
        match &self.state {
            State::Capturing(capture) => {
                !(capture.clock.freezes_inference() && self.flight.is_some())
            }
            State::Finalizing(_) => true,
            State::Idle | State::OutcomePending(_) => false,
        }
    }

    fn no_speech_deadline(&self) -> Option<Instant> {
        let State::Capturing(capture) = &self.state else {
            return None;
        };
        if capture.endpoint.speech().is_some() {
            return None;
        }
        capture
            .clock
            .no_speech_deadline(self.policy.no_speech_samples)
    }

    fn activate(&mut self, command: Activate) -> Result<()> {
        self.receipt();
        if self.shared.shutdown.load(Ordering::Acquire) {
            let _ = command.reply.try_send(Err(Error::Closed));
            return Ok(());
        }
        if !matches!(self.state, State::Idle) {
            let _ = command.reply.try_send(Err(Error::Busy));
            return Ok(());
        }
        let id = Id(self.next_id);
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("activation IDs fit in u64");
        let source = (self.source)();
        match source {
            Ok(source) => {
                let stop = Arc::new(AtomicBool::new(false));
                let pacing = source.pacing();
                let arena = std::mem::take(&mut self.audio_arena);
                let capture = Capture {
                    id,
                    endpoint: Endpoint::new(self.policy, arena),
                    stop: Stop(stop.clone()),
                    clock: Clock::Opening {
                        requested: Instant::now(),
                        pacing,
                    },
                    evidence: Evidence::Clear,
                    timings: Timings::default(),
                    log: self.new_log(),
                };
                self.state = State::Capturing(Box::new(capture));
                match self.vad_tx.try_send(Listen { id, source, stop }) {
                    Ok(()) => (),
                    Err(crossbeam_channel::TrySendError::Disconnected(_)) => {
                        self.finish(Finish::Failed(Error::Closed));
                        self.ended(id, Err(Error::Closed), Instant::now(), None)?;
                    }
                    Err(crossbeam_channel::TrySendError::Full(_)) => {
                        unreachable!("one admitted activation owns the capture request slot")
                    }
                }
            }
            Err(error) => self.publish(id, Outcome::Failed(error))?,
        }
        let _ = command.reply.try_send(Ok(id));
        Ok(())
    }

    fn publish(&mut self, id: Id, outcome: Outcome) -> Result<()> {
        self.state = State::OutcomePending(id);
        self.emit(Event::Finished { id, outcome })
    }

    fn emit(&self, event: Event) -> Result<()> {
        match self.mailbox.events.try_send(event) {
            Ok(()) => Ok(()),
            Err(crossbeam_channel::TrySendError::Disconnected(_)) => Err(Error::Closed),
            Err(crossbeam_channel::TrySendError::Full(_)) => {
                unreachable!("one activation reserves Listening and Finished event slots")
            }
        }
    }

    fn cancellation(&mut self) {
        let id = match &self.state {
            State::Capturing(capture) => capture.id,
            _ => return,
        };
        if self.shared.cancelled.load(Ordering::Acquire) >= id.0 {
            self.finish(Finish::Cancelled);
        }
    }

    fn message(&mut self, message: VadMessage) -> Result<()> {
        match message {
            VadMessage::Ready { id, watermark, at } => self.ready(id, watermark, at)?,
            VadMessage::Frame {
                id,
                frame,
                inference,
            } => self.frame(id, &frame, inference),
            VadMessage::End {
                id,
                result,
                at,
                trace,
            } => self.ended(id, result, at, trace)?,
        }
        Ok(())
    }

    fn ready(&mut self, id: Id, watermark: Watermark, at: Instant) -> Result<()> {
        let State::Capturing(capture) = &mut self.state else {
            return Ok(());
        };
        if capture.id != id {
            return Ok(());
        }
        let (requested, pacing) = match &capture.clock {
            Clock::Opening { requested, pacing } => (*requested, *pacing),
            _ => unreachable!("capture readiness is sent once"),
        };
        capture.timings.activation_to_ready_ms =
            at.saturating_duration_since(requested).as_secs_f64() * 1000.0;
        capture.clock = match pacing {
            ReplayPacing::Realtime => Clock::Realtime { at, watermark },
            ReplayPacing::Accelerated => Clock::Simulated,
        };
        self.emit(Event::Listening(id))
    }

    fn frame(&mut self, id: Id, frame: &crate::vad::ScoredFrame, inference: Duration) {
        let flow = {
            let State::Capturing(capture) = &mut self.state else {
                return;
            };
            if capture.id != id {
                return;
            }
            capture.timings.vad_frames += 1;
            capture.timings.vad_max_ms = capture
                .timings
                .vad_max_ms
                .max(inference.as_secs_f64() * 1000.0);
            capture.endpoint.observe(frame)
        };
        if let Flow::Stop(reason) = flow {
            self.finish(stop_finish(reason));
        }
    }

    fn schedule(&mut self) -> Result<()> {
        if self.flight.is_some() {
            return Ok(());
        }
        let State::Capturing(capture) = &mut self.state else {
            return Ok(());
        };
        if !matches!(capture.evidence, Evidence::Clear) {
            return Ok(());
        }
        let Some(snapshot) = capture.endpoint.snapshot(&mut self.snapshot) else {
            return Ok(());
        };
        let request = self.next_request;
        self.next_request += 1;
        capture.timings.endpoint_calls += 1;
        self.flight = Some((capture.id, request));
        let turn = TurnRequest {
            id: capture.id,
            snapshot,
            request,
            audio: std::mem::take(&mut self.snapshot),
            cancel: capture.stop.0.clone(),
        };
        match self.turn_tx.try_send(turn) {
            Ok(()) => Ok(()),
            Err(crossbeam_channel::TrySendError::Disconnected(_)) => self.engine_lost(Exit {
                role: Role::Turn,
                result: Err(Error::WorkerExited("earshot-endpoint")),
            }),
            Err(crossbeam_channel::TrySendError::Full(_)) => {
                unreachable!("one in-flight model request owns the endpoint slot")
            }
        }
    }

    fn result(&mut self, result: TurnResult) -> Result<()> {
        assert_eq!(self.flight.take(), Some((result.id, result.request)));
        let TurnResult {
            id,
            snapshot,
            request,
            audio,
            inference,
            elapsed,
        } = result;
        self.snapshot = audio;
        let current = match &self.state {
            State::Capturing(capture) => {
                capture.id == id
                    && capture
                        .endpoint
                        .speech()
                        .is_some_and(|speech| speech.revision == snapshot.revision)
            }
            _ => false,
        };
        if !current {
            return Ok(());
        }
        if self.is_cancelled(id) {
            self.finish(Finish::Cancelled);
            return Ok(());
        }
        let State::Capturing(capture) = &mut self.state else {
            return Ok(());
        };
        let Inference::Scored(scored) = inference else {
            return Ok(());
        };
        let (decision, log) = match scored {
            Ok(probability) => {
                let processing_ms = elapsed.as_secs_f64() * 1000.0;
                capture.timings.endpoint_max_ms =
                    capture.timings.endpoint_max_ms.max(processing_ms);
                let positive = probability >= capture.endpoint.policy.threshold;
                let log = capture.log.as_mut().and_then(|log| {
                    log.record(
                        request,
                        snapshot.revision,
                        snapshot.end,
                        probability,
                        processing_ms,
                        positive,
                    )
                });
                if !positive {
                    return Ok(());
                }
                (Decision::Accept, log)
            }
            Err(error) => (Decision::Fail(error), None),
        };
        let target = capture.clock.target(snapshot.scheduled_at, elapsed);
        capture.evidence = Evidence::Pending {
            decision,
            revision: snapshot.revision,
            target,
            log,
        };
        Ok(())
    }

    fn is_cancelled(&self, id: Id) -> bool {
        self.shared.cancelled.load(Ordering::Acquire) >= id.0
    }

    fn accept(&mut self) -> Result<()> {
        let mut finish = None;
        {
            let State::Capturing(capture) = &mut self.state else {
                return Ok(());
            };
            let pending = match &capture.evidence {
                Evidence::Clear => None,
                Evidence::Pending {
                    revision,
                    target,
                    log,
                    ..
                } => Some((*revision, *target, *log)),
            };
            let Some((revision, target, log)) = pending else {
                return Ok(());
            };
            let speech = capture
                .endpoint
                .speech()
                .expect("semantic results follow confirmed speech");
            if speech.revision != revision {
                if let Some(entry) = &mut capture.log {
                    mark_stale(entry, log);
                }
                capture.evidence = Evidence::Clear;
            } else if capture.endpoint.end >= target {
                if let Some(entry) = &mut capture.log {
                    mark_accepted(entry, log);
                }
                let Evidence::Pending { decision, .. } =
                    std::mem::replace(&mut capture.evidence, Evidence::Clear)
                else {
                    unreachable!("current evidence")
                };
                finish = Some(match decision {
                    Decision::Accept => Finish::Complete,
                    Decision::Fail(error) => Finish::Failed(error),
                });
            }
        }
        if let Some(reason) = finish {
            self.finish(reason);
        }
        Ok(())
    }

    fn finish(&mut self, reason: Finish) {
        let State::Capturing(capture) = std::mem::replace(&mut self.state, State::Idle) else {
            unreachable!("only an active capture begins finalization")
        };
        let mut capture = *capture;
        if let Evidence::Pending { log, .. } = &capture.evidence
            && !matches!(reason, Finish::Complete)
            && let Some(entry) = &mut capture.log
        {
            mark_discarded(entry, *log);
        }
        capture.timings.decision_sample = capture.endpoint.end;
        capture.timings.decision_after_ready_ms =
            capture.clock.after_ready_ms(capture.endpoint.end);
        if let Some(speech) = capture.endpoint.speech() {
            capture.timings.onset_sample = Some(speech.onset);
            capture.timings.last_speech_sample = Some(speech.last);
        }
        capture.stop.0.store(true, Ordering::Release);
        self.state = State::Finalizing(Box::new(Finalizing {
            capture,
            reason,
            started: Instant::now(),
        }));
    }

    fn ended(
        &mut self,
        id: Id,
        result: Result<()>,
        at: Instant,
        trace: Option<CaptureTrace>,
    ) -> Result<()> {
        let matched = match &self.state {
            State::Capturing(capture) if capture.id == id => 1,
            State::Finalizing(finalizing) if finalizing.capture.id == id => 2,
            _ => 0,
        };
        match matched {
            1 => self.finish(Finish::Failed(Error::SourceEnded)),
            2 => (),
            _ => return Ok(()),
        }
        let State::Finalizing(finalizing) = std::mem::replace(&mut self.state, State::Idle) else {
            unreachable!("matching capture is finalizing")
        };
        let mut finalizing = *finalizing;
        if let Err(error) = result {
            finalizing.reason = Finish::Failed(error);
        }
        finalizing.capture.timings.capture_stop_ms = at
            .saturating_duration_since(finalizing.started)
            .as_secs_f64()
            * 1000.0;
        self.trace(&mut finalizing, trace);
        let Finalizing {
            capture, reason, ..
        } = finalizing;
        let Capture {
            id,
            endpoint,
            timings,
            ..
        } = capture;
        let (audio, arena) = match reason {
            Finish::Complete => {
                let (audio, arena) = endpoint.finish(Keep::SpeechTail);
                (Some(audio), arena)
            }
            Finish::Incomplete(reason) => {
                let keep = if reason == IncompleteReason::DurationLimit {
                    Keep::All
                } else {
                    Keep::SpeechTail
                };
                let (audio, arena) = endpoint.finish(keep);
                (Some(audio), arena)
            }
            Finish::NoSpeech | Finish::Cancelled | Finish::Failed(_) => (None, endpoint.abort()),
        };
        self.audio_arena = arena;
        let outcome = match reason {
            Finish::Complete => Outcome::Complete(Statement::detected(
                id,
                audio.expect("complete owns audio"),
                timings,
            )),
            Finish::Incomplete(reason) => Outcome::Incomplete {
                audio: audio.expect("incomplete speech owns audio"),
                reason,
                timings,
            },
            Finish::NoSpeech => Outcome::NoSpeech,
            Finish::Cancelled => Outcome::Cancelled,
            Finish::Failed(error) => Outcome::Failed(error),
        };
        self.publish(id, outcome)
    }

    #[cfg(feature = "tooling")]
    fn new_log(&self) -> Option<Log> {
        self.traces.as_ref().map(|_| Log::new())
    }

    #[cfg(not(feature = "tooling"))]
    fn new_log(&self) -> Option<Log> {
        None
    }

    #[cfg(feature = "tooling")]
    fn trace(&self, finalizing: &mut Finalizing, captured: Option<CaptureTrace>) {
        let (Some(sink), Some(log), Some(captured)) =
            (&self.traces, finalizing.capture.log.take(), captured)
        else {
            return;
        };
        let outcome = match &finalizing.reason {
            Finish::Complete => "complete",
            Finish::Incomplete(_) => "incomplete",
            Finish::NoSpeech => "no-speech",
            Finish::Cancelled => "cancelled",
            Finish::Failed(_) => "failed",
        };
        let trace = Trace {
            model_sha256: MODEL_SHA256,
            activation_id: finalizing.capture.id.get(),
            samples: captured.samples,
            captured_samples: captured.captured_samples,
            complete: captured.complete && log.omitted == 0,
            timings: finalizing.capture.timings,
            outcome,
            evaluations: log.evaluations,
            omitted_evaluations: log.omitted,
            minimum_pause_samples: self.policy.pause_samples,
            completion_threshold: self.policy.threshold,
            omitted_traces: 0,
        };
        sink.send(trace);
    }

    #[cfg(not(feature = "tooling"))]
    fn trace(&self, _finalizing: &mut Finalizing, _captured: Option<CaptureTrace>) {}

    fn engine_lost(&mut self, exit: Exit) -> Result<()> {
        let error = match exit.result {
            Ok(()) => Error::WorkerExited(exit.role.name()),
            Err(error) => error,
        };
        self.shared.shutdown.store(true, Ordering::Release);
        let id = match std::mem::replace(&mut self.state, State::Idle) {
            State::Capturing(capture) => {
                capture.stop.0.store(true, Ordering::Release);
                Some(capture.id)
            }
            State::Finalizing(finalizing) => {
                finalizing.capture.stop.0.store(true, Ordering::Release);
                Some(finalizing.capture.id)
            }
            State::Idle | State::OutcomePending(_) => None,
        };
        if let Some(id) = id {
            self.publish(id, Outcome::Failed(error))?;
        }
        Ok(())
    }
}

fn stop_finish(reason: StopReason) -> Finish {
    match reason {
        StopReason::NoSpeech => Finish::NoSpeech,
        StopReason::SilenceLimit => Finish::Incomplete(IncompleteReason::SilenceLimit),
        StopReason::DurationLimit => Finish::Incomplete(IncompleteReason::DurationLimit),
    }
}

impl Drop for Coordinator {
    fn drop(&mut self) {
        match &self.state {
            State::Capturing(capture) => capture.stop.0.store(true, Ordering::Release),
            State::Finalizing(finalizing) => {
                finalizing.capture.stop.0.store(true, Ordering::Release)
            }
            State::Idle | State::OutcomePending(_) => (),
        }
        self.shared.shutdown.store(true, Ordering::Release);
        while let Ok(command) = self.mailbox.commands.try_recv() {
            let _ = command.reply.try_send(Err(Error::Closed));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Earshot, capture::ReplayPacing, turn::TurnEngine, vad::VadEngine};
    use std::sync::atomic::AtomicUsize;
    use std::thread;

    struct Vad;

    impl VadEngine for Vad {
        fn reset(&mut self) {}

        fn score(&mut self, audio: &[f32; VAD_FRAME_SAMPLES]) -> Result<f32> {
            Ok(if audio[0] > 0.1 { 0.9 } else { 0.0 })
        }
    }

    struct Turn {
        probability: f32,
        delay: Duration,
        calls: Arc<AtomicUsize>,
        first_error: bool,
    }

    impl TurnEngine for Turn {
        fn predict(&mut self, _: &[f32]) -> Result<f32> {
            let call = self.calls.fetch_add(1, Ordering::AcqRel);
            thread::sleep(self.delay);
            if self.first_error && call == 0 {
                return Err(Error::ModelContract);
            }
            Ok(self.probability)
        }
    }

    fn setup(
        audio: Vec<f32>,
        policy: Policy,
        probability: f32,
        delay: Duration,
        pacing: ReplayPacing,
        first_error: bool,
    ) -> (Setup, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let worker_calls = calls.clone();
        (
            Setup {
                policy,
                source: Box::new(move || {
                    Ok(InputSource::Samples {
                        audio: Arc::from(audio.clone()),
                        chunk_size: 513,
                        pacing,
                    })
                }),
                vad: Box::new(|| Ok(Box::new(Vad))),
                turn: Box::new(move || {
                    Ok(Box::new(Turn {
                        probability,
                        delay,
                        calls: worker_calls,
                        first_error,
                    }))
                }),
                #[cfg(feature = "tooling")]
                traces: None,
            },
            calls,
        )
    }

    fn utterance() -> Vec<f32> {
        [vec![0.0; 3 * 512], vec![0.2; 4 * 512], vec![0.0; 48 * 512]].concat()
    }

    fn policy() -> Policy {
        Policy {
            pause_samples: 4_096,
            silence_limit_samples: 32 * 512,
            no_speech_samples: 16 * 512,
            maximum_samples: 1_024 * 512,
            ..Policy::default()
        }
    }

    fn outcome(owner: &mut Earshot) -> (Id, Outcome) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            assert!(Instant::now() < deadline, "outcome deadline");
            if let Some(Event::Finished { id, outcome }) =
                owner.recv_timeout(Duration::from_millis(10)).unwrap()
            {
                return (id, outcome);
            }
        }
    }

    #[test]
    fn unread_outcome_is_busy_but_owned_audio_does_not_block_activation() {
        let (setup, _) = setup(
            utterance(),
            policy(),
            0.9,
            Duration::ZERO,
            ReplayPacing::Accelerated,
            false,
        );
        let mut owner = Earshot::start(setup).unwrap();
        let control = owner.control();
        let first = control.activate().unwrap();
        std::thread::sleep(Duration::from_millis(80));
        assert!(matches!(control.activate(), Err(Error::Busy)));
        let (id, result) = outcome(&mut owner);
        assert_eq!(id, first);
        let Outcome::Complete(statement) = result else {
            panic!("expected semantic completion")
        };
        assert_eq!(statement.audio().start_sample(), 0);
        assert_eq!(statement.audio().end_sample(), 15 * 512);
        let second = control.activate().unwrap();
        control.cancel(first);
        assert_ne!(first, second);
        assert!(matches!(outcome(&mut owner).1, Outcome::Complete(_)));
        assert!(!statement.audio().is_empty());
        owner.close().unwrap();
        assert!(matches!(control.activate(), Err(Error::Closed)));
    }

    #[test]
    fn timers_produce_incomplete_or_no_speech_and_never_a_statement() {
        for (audio, expected) in [(vec![0.0; 20 * 512], "empty"), (utterance(), "silence")] {
            let (setup, calls) = setup(
                audio,
                policy(),
                0.001,
                Duration::ZERO,
                ReplayPacing::Accelerated,
                false,
            );
            let mut owner = Earshot::start(setup).unwrap();
            owner.control().activate().unwrap();
            match (expected, outcome(&mut owner).1) {
                ("empty", Outcome::NoSpeech) => assert_eq!(calls.load(Ordering::Acquire), 0),
                ("silence", Outcome::Incomplete { reason, audio, .. }) => {
                    assert_eq!(reason, IncompleteReason::SilenceLimit);
                    assert_eq!(audio.end_sample(), 15 * 512);
                    assert_eq!(calls.load(Ordering::Acquire), 1);
                }
                _ => panic!("timers cannot manufacture completion"),
            }
        }
    }

    #[test]
    fn silence_fallback_delivers_owned_audio_and_releases_the_next_activation() {
        let samples = [vec![0.0; 3 * 512], vec![0.2; 4 * 512], vec![0.0; 160 * 512]].concat();
        let (setup, calls) = setup(
            samples.clone(),
            Policy::default(),
            0.007_809_102_5,
            Duration::ZERO,
            ReplayPacing::Accelerated,
            false,
        );
        let mut owner = Earshot::start(setup).unwrap();
        let control = owner.control();
        let mut delivered = Vec::new();
        for attempt in 1..=2 {
            let id = control.activate().unwrap();
            let (actual, result) = outcome(&mut owner);
            assert_eq!(actual, id);
            let Outcome::Incomplete {
                audio,
                reason,
                timings,
            } = result
            else {
                panic!("low semantic confidence must deliver unconfirmed audio")
            };
            assert_eq!(reason, IncompleteReason::SilenceLimit);
            assert_eq!(audio.start_sample(), 0);
            assert_eq!(audio.samples(), &samples[..15 * 512]);
            let quiet = timings.decision_sample - timings.last_speech_sample.unwrap();
            assert!((80_000..80_512).contains(&quiet));
            assert_eq!(timings.endpoint_calls, 1);
            assert_eq!(calls.load(Ordering::Acquire), attempt);
            delivered.push(audio);
        }
        assert_eq!(delivered[0].samples(), delivered[1].samples());
        owner.close().unwrap();
    }

    #[test]
    fn eof_preserves_queued_inference_but_never_supplies_missing_context() {
        for (tail_frames, complete) in [(12, true), (8, false)] {
            let audio = [vec![0.2; 4 * 512], vec![0.0; tail_frames * 512]].concat();
            let (setup, calls) = setup(
                audio.clone(),
                policy(),
                0.9,
                Duration::from_millis(64),
                ReplayPacing::Accelerated,
                false,
            );
            let mut owner = Earshot::start(setup).unwrap();
            owner.control().activate().unwrap();
            match (complete, outcome(&mut owner).1) {
                (true, Outcome::Complete(statement)) => {
                    assert_eq!(statement.audio().samples(), &audio[..12 * 512]);
                    assert!(statement.timings().decision_sample <= audio.len() as u64);
                }
                (false, Outcome::Failed(Error::SourceEnded)) => (),
                (_, outcome) => panic!("unexpected finite-source outcome: {outcome:?}"),
            }
            assert_eq!(calls.load(Ordering::Acquire), 1);
            owner.close().unwrap();
        }
    }

    #[test]
    fn duration_limit_returns_retained_audio_as_incomplete() {
        let (setup, _) = setup(
            vec![0.2; 64 * 512],
            Policy {
                maximum_samples: 4_096,
                ..policy()
            },
            0.9,
            Duration::ZERO,
            ReplayPacing::Accelerated,
            false,
        );
        let mut owner = Earshot::start(setup).unwrap();
        owner.control().activate().unwrap();
        match outcome(&mut owner).1 {
            Outcome::Incomplete { audio, reason, .. } => {
                assert_eq!(reason, IncompleteReason::DurationLimit);
                assert_eq!(audio.len(), 4_096);
            }
            _ => panic!("duration limit is incomplete"),
        }
    }

    #[test]
    fn cancellation_does_not_wait_for_inference_and_stale_errors_do_not_fail_new_attempts() {
        let (setup, calls) = setup(
            utterance(),
            policy(),
            0.9,
            Duration::from_millis(250),
            ReplayPacing::Accelerated,
            true,
        );
        let mut owner = Earshot::start(setup).unwrap();
        let control = owner.control();
        let first = control.activate().unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while calls.load(Ordering::Acquire) == 0 {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        let start = Instant::now();
        control.cancel(first);
        assert!(matches!(outcome(&mut owner).1, Outcome::Cancelled));
        assert!(start.elapsed() < Duration::from_millis(150));
        control.activate().unwrap();
        assert!(matches!(outcome(&mut owner).1, Outcome::Complete(_)));
    }

    #[test]
    fn resumed_speech_invalidates_old_activation_local_inference_errors() {
        let audio = [
            vec![0.2; 4 * 512],
            vec![0.0; 9 * 512],
            vec![0.2; 4 * 512],
            vec![0.0; 64 * 512],
        ]
        .concat();
        let (setup, calls) = setup(
            audio,
            policy(),
            0.9,
            Duration::from_millis(160),
            ReplayPacing::Realtime,
            true,
        );
        let mut owner = Earshot::start(setup).unwrap();
        owner.control().activate().unwrap();
        let Outcome::Complete(statement) = outcome(&mut owner).1 else {
            panic!("stale error")
        };
        assert!(statement.audio().end_sample() >= 25 * 512);
        assert_eq!(calls.load(Ordering::Acquire), 2);
    }

    #[test]
    fn close_with_occupied_event_and_command_queues_joins_workers() {
        let (setup, _) = setup(
            utterance(),
            policy(),
            0.9,
            Duration::from_millis(20),
            ReplayPacing::Accelerated,
            false,
        );
        let owner = Earshot::start(setup).unwrap();
        let control = owner.control();
        control.activate().unwrap();
        std::thread::sleep(Duration::from_millis(80));
        let handles: Vec<_> = (0..16)
            .map(|_| {
                let control = control.clone();
                std::thread::Builder::new()
                    .name("test-activate".into())
                    .spawn(move || {
                        assert!(matches!(
                            control.activate(),
                            Err(Error::Busy | Error::Closed)
                        ))
                    })
                    .unwrap()
            })
            .collect();
        owner.close().unwrap();
        for handle in handles {
            handle.join().unwrap();
        }
    }

    #[test]
    fn close_while_idle_returns_without_an_activation() {
        let (setup, _) = setup(
            utterance(),
            policy(),
            0.9,
            Duration::ZERO,
            ReplayPacing::Accelerated,
            false,
        );
        let owner = Earshot::start(setup).unwrap();
        let start = Instant::now();
        owner.close().unwrap();
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn source_start_failure_still_produces_exactly_one_terminal_outcome() {
        let (mut setup, _) = setup(
            utterance(),
            policy(),
            0.9,
            Duration::ZERO,
            ReplayPacing::Accelerated,
            false,
        );
        setup.source = Box::new(|| Err(Error::NoInputDevice));
        let mut owner = Earshot::start(setup).unwrap();
        let id = owner.control().activate().unwrap();
        let (actual, result) = outcome(&mut owner);
        assert_eq!(id, actual);
        assert!(matches!(result, Outcome::Failed(Error::NoInputDevice)));
        assert!(
            owner
                .recv_timeout(Duration::from_millis(20))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn paced_and_accelerated_delayed_inference_preserve_the_same_owned_boundaries() {
        let audio = [
            vec![0.2; 4 * 512],
            vec![0.0; 9 * 512],
            vec![0.2; 4 * 512],
            vec![0.0; 64 * 512],
        ]
        .concat();
        for pacing in [ReplayPacing::Accelerated, ReplayPacing::Realtime] {
            let (setup, calls) = setup(
                audio.clone(),
                policy(),
                0.9,
                Duration::from_millis(160),
                pacing,
                false,
            );
            let mut owner = Earshot::start(setup).unwrap();
            owner.control().activate().unwrap();
            let Outcome::Complete(statement) = outcome(&mut owner).1 else {
                panic!("current second pause should complete")
            };
            assert_eq!(statement.audio().start_sample(), 0);
            assert_eq!(statement.audio().end_sample(), 25 * 512);
            assert_eq!(statement.audio().samples(), &audio[..25 * 512]);
            assert_eq!(calls.load(Ordering::Acquire), 2);
        }
    }

    #[test]
    fn current_inference_errors_and_nonfinite_capture_cannot_create_a_statement() {
        for (audio, first_error) in [(utterance(), true), (vec![f32::NAN; 20 * 512], false)] {
            let (setup, _) = setup(
                audio,
                policy(),
                0.9,
                Duration::ZERO,
                ReplayPacing::Accelerated,
                first_error,
            );
            let mut owner = Earshot::start(setup).unwrap();
            owner.control().activate().unwrap();
            assert!(matches!(
                outcome(&mut owner).1,
                Outcome::Failed(Error::ModelContract | Error::InvalidAudio)
            ));
        }
    }

    #[test]
    fn immediate_cancellation_and_drop_release_all_workers_without_reading_events() {
        let (setup, _) = setup(
            utterance(),
            policy(),
            0.9,
            Duration::ZERO,
            ReplayPacing::Realtime,
            false,
        );
        let mut owner = Earshot::start(setup).unwrap();
        let control = owner.control();
        let id = control.activate().unwrap();
        control.cancel(id);
        assert!(matches!(outcome(&mut owner).1, Outcome::Cancelled));
        control.activate().unwrap();
        drop(owner);
        assert!(matches!(control.activate(), Err(Error::Closed)));
    }

    #[test]
    fn capture_start_io_failure_is_one_failed_outcome_without_listening() {
        let (mut setup, _) = setup(
            utterance(),
            policy(),
            0.9,
            Duration::ZERO,
            ReplayPacing::Accelerated,
            false,
        );
        setup.source = Box::new(|| {
            Ok(InputSource::file(
                "/earshot-missing-fixture.wav",
                ReplayPacing::Accelerated,
            ))
        });
        let mut owner = Earshot::start(setup).unwrap();
        let id = owner.control().activate().unwrap();
        match owner.recv().unwrap() {
            Event::Finished {
                id: actual,
                outcome: Outcome::Failed(Error::SourceIo(_)),
            } => assert_eq!(id, actual),
            event => panic!("unexpected startup outcome: {event:?}"),
        }
    }

    #[test]
    fn endpoint_worker_panic_finishes_the_accepted_attempt_and_closes_the_service() {
        struct Panic;

        impl TurnEngine for Panic {
            fn predict(&mut self, _: &[f32]) -> Result<f32> {
                panic!("injected worker panic")
            }
        }
        let (mut setup, _) = setup(
            utterance(),
            policy(),
            0.9,
            Duration::ZERO,
            ReplayPacing::Accelerated,
            false,
        );
        setup.turn = Box::new(|| Ok(Box::new(Panic)));
        let mut owner = Earshot::start(setup).unwrap();
        let control = owner.control();
        control.activate().unwrap();
        let result = outcome(&mut owner).1;
        assert!(
            matches!(
                result,
                Outcome::Failed(Error::WorkerPanicked("earshot-endpoint"))
            ),
            "{result:?}"
        );
        owner.close().unwrap();
        assert!(matches!(control.activate(), Err(Error::Closed)));
    }
}
