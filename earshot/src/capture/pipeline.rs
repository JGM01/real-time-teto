//! Capture pipeline: prepare a source, frame the audio, and score it once.
//!
//! The VAD worker owns this loop. It joins the capture producer before it sends
//! `End` to the coordinator.

use crate::{
    Error, Id, Result,
    capture::{ReplayPacing, SourceContext, normalize::Chunk, source::Source},
    constants::*,
    runtime,
    trace::{CaptureTrace, TraceArena},
    vad::{Listen, ScoredFrame, VadEngine, VadMessage, validate_probability},
};
use crossbeam_channel::Sender;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Instant,
};

struct CaptureGuard {
    worker: Option<JoinHandle<Result<()>>>,
    stop: Arc<AtomicBool>,
}

impl CaptureGuard {
    fn finish(&mut self, cancel: bool) -> Result<()> {
        let Some(worker) = self.worker.take() else {
            return Ok(());
        };
        if cancel {
            self.stop.store(true, Ordering::Release);
        }
        worker
            .join()
            .unwrap_or(Err(Error::WorkerPanicked("earshot-capture")))
    }
}

impl Drop for CaptureGuard {
    fn drop(&mut self) {
        let _ = self.finish(true);
    }
}

pub(crate) fn capture(
    engine: &mut dyn VadEngine,
    listen: Listen,
    events: &Sender<VadMessage>,
    mut trace: Option<&mut TraceArena>,
) -> (Result<()>, Option<CaptureTrace>) {
    let processing = (|| {
        if listen.stop.load(Ordering::Acquire) {
            return Ok(());
        }
        let pacing = if listen.source.is_realtime() {
            ReplayPacing::Realtime
        } else {
            ReplayPacing::Accelerated
        };
        let mut source = Source::prepare(listen.source)?;
        let worker = match source.start(SourceContext::new(listen.stop.clone(), pacing)) {
            Err(Error::Closed) if listen.stop.load(Ordering::Acquire) => return Ok(()),
            result => result?,
        };
        let mut guard = CaptureGuard {
            worker: Some(worker),
            stop: listen.stop.clone(),
        };
        let result = consume(
            engine,
            &mut source,
            listen.id,
            &listen.stop,
            events,
            trace.as_deref_mut(),
        );
        let joined = guard.finish(result.is_err());
        if let Some(trace) = trace.as_deref_mut() {
            let drain = drain_trace(&mut source, trace);
            result.and(joined).and(drain)
        } else {
            result.and(joined)
        }
    })();
    if processing.is_err() {
        listen.stop.store(true, Ordering::Release);
        if let Some(trace) = trace.as_deref_mut() {
            trace.fail();
        }
    }
    let captured = trace.map(TraceArena::finish);
    (processing, captured)
}

struct Frames {
    buffer: [f32; VAD_FRAME_SAMPLES],
    used: usize,
    position: u64,
}

impl Default for Frames {
    fn default() -> Self {
        Self {
            buffer: [0.0; VAD_FRAME_SAMPLES],
            used: 0,
            position: 0,
        }
    }
}

fn consume(
    engine: &mut dyn VadEngine,
    source: &mut Source,
    id: Id,
    stop: &AtomicBool,
    events: &Sender<VadMessage>,
    mut trace: Option<&mut TraceArena>,
) -> Result<()> {
    if !runtime::send(
        events,
        VadMessage::Ready {
            id,
            watermark: source.watermark(),
            at: Instant::now(),
        },
        stop,
    )? {
        return Ok(());
    }
    let mut frames = Frames::default();
    while !stop.load(Ordering::Acquire) {
        let Some(chunk) = source.next_chunk(stop)? else {
            break;
        };
        if !chunk.samples.iter().all(|sample| sample.is_finite()) {
            return Err(Error::InvalidAudio);
        }
        if let Some(trace) = trace.as_deref_mut() {
            trace.append(chunk.samples);
        }
        if !frames.feed(&chunk, engine, id, stop, events)? {
            break;
        }
    }
    Ok(())
}

impl Frames {
    fn feed(
        &mut self,
        chunk: &Chunk<'_>,
        engine: &mut dyn VadEngine,
        id: Id,
        stop: &AtomicBool,
        events: &Sender<VadMessage>,
    ) -> Result<bool> {
        let expected = self.position + self.used as u64;
        if chunk.start_sample != expected {
            return Err(Error::CaptureOverrun {
                expected,
                actual: chunk.start_sample,
            });
        }
        let mut rest = chunk.samples;
        while !rest.is_empty() {
            let take = (VAD_FRAME_SAMPLES - self.used).min(rest.len());
            self.buffer[self.used..self.used + take].copy_from_slice(&rest[..take]);
            self.used += take;
            rest = &rest[take..];
            if self.used != VAD_FRAME_SAMPLES {
                continue;
            }
            if stop.load(Ordering::Acquire) {
                return Ok(false);
            }
            let started = Instant::now();
            let probability = engine.score(&self.buffer)?;
            let inference = started.elapsed();
            validate_probability(probability)?;
            let frame = ScoredFrame {
                samples: self.buffer,
                probability,
            };
            self.position += VAD_FRAME_SAMPLES as u64;
            self.used = 0;
            if !runtime::send(
                events,
                VadMessage::Frame {
                    id,
                    frame,
                    inference,
                },
                stop,
            )? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

fn drain_trace(source: &mut Source, trace: &mut TraceArena) -> Result<()> {
    let done = AtomicBool::new(false);
    while let Some(chunk) = source.next_chunk(&done)? {
        if !chunk.samples.iter().all(|sample| sample.is_finite()) {
            return Err(Error::InvalidAudio);
        }
        trace.append(chunk.samples);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Engine;

    impl VadEngine for Engine {
        fn reset(&mut self) {}

        fn score(&mut self, audio: &[f32; VAD_FRAME_SAMPLES]) -> Result<f32> {
            Ok(if audio[0] > 0.1 { 0.9 } else { 0.0 })
        }
    }

    fn chunk(samples: &[f32], start_sample: u64) -> Chunk<'_> {
        Chunk {
            samples,
            start_sample,
        }
    }

    #[test]
    fn framing_copies_across_chunk_boundaries() {
        let (events, received) = crossbeam_channel::bounded(SCORED_CAPACITY);
        let stop = AtomicBool::new(false);
        let mut frames = Frames::default();
        let first = [0.2_f32; 300];
        let second = [0.2_f32; 300];
        assert!(
            frames
                .feed(&chunk(&first, 0), &mut Engine, Id(1), &stop, &events)
                .unwrap()
        );
        assert!(
            frames
                .feed(&chunk(&second, 300), &mut Engine, Id(1), &stop, &events)
                .unwrap()
        );
        assert_eq!(received.len(), 1);
        let VadMessage::Frame { frame, .. } = received.try_recv().unwrap() else {
            panic!("expected one scored frame")
        };
        assert_eq!(frame.samples[0], 0.2);
        assert!((frame.probability - 0.9).abs() < f32::EPSILON);
    }

    #[test]
    fn gaps_are_reported_before_scoring() {
        let (events, _received) = crossbeam_channel::bounded(SCORED_CAPACITY);
        let stop = AtomicBool::new(false);
        let mut frames = Frames::default();
        let samples = [0.0_f32; 512];
        assert!(matches!(
            frames.feed(&chunk(&samples, 1), &mut Engine, Id(1), &stop, &events),
            Err(Error::CaptureOverrun {
                expected: 0,
                actual: 1
            })
        ));
    }
}
