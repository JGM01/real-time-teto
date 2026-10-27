//! Pure sample-timeline endpoint policy.
//!
//! Inference never changes the retained waveform. The endpoint owns the
//! pre-roll ring, the retained audio arena, and the pause timer.

use crate::{Audio, constants::*, policy::Policy, vad::ScoredFrame};

/// The endpoint state after one observed frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Flow {
    Continue,
    Stop(StopReason),
}

/// Why the endpoint stopped the capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StopReason {
    SilenceLimit,
    DurationLimit,
    NoSpeech,
}

/// Which part of the retained audio to deliver.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Keep {
    SpeechTail,
    All,
}

/// The public speech position values.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SpeechInfo {
    pub onset: u64,
    pub last: u64,
    pub revision: u64,
}

/// One speech-anchored model snapshot.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Snapshot {
    pub revision: u64,
    pub end: u64,
    pub scheduled_at: u64,
}

/// The endpoint state for one activation.
pub(crate) struct Endpoint {
    pub policy: Policy,
    pub end: u64,
    phase: Phase,
}

#[allow(
    clippy::large_enum_variant,
    reason = "the pre-roll is fixed storage inside the boxed capture"
)]
enum Phase {
    Waiting {
        preroll: Preroll,
        consecutive: usize,
        audio: Vec<f32>,
    },
    Speech(Speech),
}

struct Speech {
    audio: Vec<f32>,
    start: u64,
    onset: u64,
    last: u64,
    revision: u64,
    pause: Pause,
}

enum Pause {
    Talking,
    Open { anchor: u64, scheduled: bool },
}

/// The last ten frames before the endpoint. Ten frames is the pre-roll plus
/// the two frames that confirm onset.
struct Preroll {
    frames: [[f32; VAD_FRAME_SAMPLES]; PREROLL_FRAMES],
    next: usize,
    len: usize,
}

impl Default for Preroll {
    fn default() -> Self {
        Self {
            frames: [[0.0; VAD_FRAME_SAMPLES]; PREROLL_FRAMES],
            next: 0,
            len: 0,
        }
    }
}

impl Preroll {
    fn push(&mut self, samples: &[f32; VAD_FRAME_SAMPLES]) {
        self.frames[self.next] = *samples;
        self.next = (self.next + 1) % PREROLL_FRAMES;
        self.len = (self.len + 1).min(PREROLL_FRAMES);
    }

    fn append_to(&self, audio: &mut Vec<f32>) {
        let first = (self.next + PREROLL_FRAMES - self.len) % PREROLL_FRAMES;
        for index in 0..self.len {
            audio.extend_from_slice(&self.frames[(first + index) % PREROLL_FRAMES]);
        }
    }
}

impl Endpoint {
    /// Create an endpoint with a reusable audio arena.
    ///
    /// # Panics
    ///
    /// Panics if the arena is not empty or is smaller than the policy maximum.
    pub(crate) fn new(policy: Policy, audio: Vec<f32>) -> Self {
        assert!(audio.is_empty(), "the audio arena starts empty");
        assert!(
            audio.capacity() >= policy.maximum_samples,
            "the audio arena covers the policy maximum"
        );
        Self {
            policy,
            end: 0,
            phase: Phase::Waiting {
                preroll: Preroll::default(),
                consecutive: 0,
                audio,
            },
        }
    }

    /// Observe one scored frame and return the next action.
    pub(crate) fn observe(&mut self, frame: &ScoredFrame) -> Flow {
        self.end += VAD_FRAME_SAMPLES as u64;
        let mut onset = None;
        match &mut self.phase {
            Phase::Waiting {
                preroll,
                consecutive,
                audio,
            } => {
                preroll.push(&frame.samples);
                *consecutive = if frame.probability >= 0.5 {
                    *consecutive + 1
                } else {
                    0
                };
                if *consecutive == 2 {
                    let onset_sample = self.end - 2 * VAD_FRAME_SAMPLES as u64;
                    let start = onset_sample.saturating_sub(PRE_SPEECH_SAMPLES as u64);
                    let mut retained = std::mem::take(audio);
                    retained.clear();
                    preroll.append_to(&mut retained);
                    onset = Some((start, onset_sample, retained));
                }
            }
            Phase::Speech(speech) => {
                let room = self
                    .policy
                    .maximum_samples
                    .saturating_sub(speech.audio.len());
                let count = room.min(VAD_FRAME_SAMPLES);
                speech.audio.extend_from_slice(&frame.samples[..count]);
                if frame.probability >= 0.35 {
                    speech.last = self.end;
                    speech.revision += 1;
                    speech.pause = Pause::Talking;
                } else if matches!(speech.pause, Pause::Talking) {
                    speech.pause = Pause::Open {
                        anchor: speech.last,
                        scheduled: false,
                    };
                }
            }
        }
        if let Some((start, onset_sample, audio)) = onset {
            self.phase = Phase::Speech(Speech {
                audio,
                start,
                onset: onset_sample,
                last: self.end,
                revision: 1,
                pause: Pause::Talking,
            });
        }
        self.flow()
    }

    fn flow(&self) -> Flow {
        match &self.phase {
            Phase::Waiting { .. } if self.end >= self.policy.no_speech_samples => {
                Flow::Stop(StopReason::NoSpeech)
            }
            Phase::Speech(speech) if speech.audio.len() >= self.policy.maximum_samples => {
                Flow::Stop(StopReason::DurationLimit)
            }
            Phase::Speech(speech)
                if self.end - speech.last >= self.policy.silence_limit_samples =>
            {
                Flow::Stop(StopReason::SilenceLimit)
            }
            _ => Flow::Continue,
        }
    }

    /// Return the current speech position.
    #[must_use]
    pub(crate) fn speech(&self) -> Option<SpeechInfo> {
        match &self.phase {
            Phase::Waiting { .. } => None,
            Phase::Speech(speech) => Some(SpeechInfo {
                onset: speech.onset,
                last: speech.last,
                revision: speech.revision,
            }),
        }
    }

    /// Copy one speech-anchored snapshot into `out`.
    ///
    /// The snapshot ends at the pause boundary. More silence does not create a
    /// second snapshot for the same pause.
    pub(crate) fn snapshot(&mut self, out: &mut Vec<f32>) -> Option<Snapshot> {
        let Phase::Speech(speech) = &mut self.phase else {
            return None;
        };
        let Pause::Open { anchor, scheduled } = &mut speech.pause else {
            return None;
        };
        let end = *anchor + self.policy.pause_samples;
        if *scheduled || self.end < end {
            return None;
        }
        let high = (end - speech.start) as usize;
        let low = high.saturating_sub(MODEL_SAMPLES);
        assert!(
            high <= speech.audio.len(),
            "snapshot stays inside the audio"
        );
        assert!(
            out.capacity() >= MODEL_SAMPLES,
            "the snapshot buffer is reused"
        );
        out.clear();
        out.extend_from_slice(&speech.audio[low..high]);
        *scheduled = true;
        Some(Snapshot {
            revision: speech.revision,
            end,
            scheduled_at: self.end,
        })
    }

    /// Deliver retained audio and return the arena.
    ///
    /// # Panics
    ///
    /// Panics if the endpoint has no confirmed speech.
    pub(crate) fn finish(self, keep: Keep) -> (Audio, Vec<f32>) {
        let Phase::Speech(speech) = self.phase else {
            panic!("only confirmed speech produces audio");
        };
        let Speech {
            mut audio,
            start,
            last,
            ..
        } = speech;
        let count = match keep {
            Keep::SpeechTail => ((last - start) as usize + TRAILING_SAMPLES).min(audio.len()),
            Keep::All => audio.len(),
        };
        let exact = audio[..count].to_vec();
        audio.clear();
        (
            Audio {
                samples: exact,
                start_sample: start,
            },
            audio,
        )
    }

    /// Return the arena without audio.
    pub(crate) fn abort(self) -> Vec<f32> {
        match self.phase {
            Phase::Waiting { mut audio, .. } => {
                audio.clear();
                audio
            }
            Phase::Speech(speech) => {
                let mut audio = speech.audio;
                audio.clear();
                audio
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn frame(probability: f32, value: f32) -> ScoredFrame {
        ScoredFrame {
            samples: [value; VAD_FRAME_SAMPLES],
            probability,
        }
    }

    #[test]
    fn onset_hysteresis_retains_preroll_internal_pauses_and_tail() {
        let policy = Policy {
            maximum_samples: 128 * VAD_FRAME_SAMPLES,
            ..Policy::default()
        };
        let mut endpoint = Endpoint::new(policy, Vec::with_capacity(policy.maximum_samples));
        let mut original = Vec::new();
        let probabilities = [
            vec![0.0; 12],
            vec![0.8; 4],
            vec![0.0; 12],
            vec![0.4; 4],
            vec![0.0; 36],
        ]
        .concat();
        for (index, probability) in probabilities.into_iter().enumerate() {
            let value = (index as f32 + 1.0) / 100.0;
            original.extend_from_slice(&[value; VAD_FRAME_SAMPLES]);
            endpoint.observe(&frame(probability, value));
        }
        let speech = endpoint.speech().unwrap();
        assert_eq!(speech.onset, 12 * 512);
        assert_eq!(speech.last, 32 * 512);
        let (audio, _) = endpoint.finish(Keep::SpeechTail);
        assert_eq!(audio.start_sample(), 4 * 512);
        assert_eq!(audio.end_sample(), 40 * 512);
        assert_eq!(audio.samples(), &original[4 * 512..40 * 512]);
    }

    #[test]
    fn snapshot_is_anchored_once_per_pause_and_reuses_storage() {
        let policy = Policy {
            pause_samples: 4_096,
            maximum_samples: 128 * VAD_FRAME_SAMPLES,
            ..Policy::default()
        };
        let mut endpoint = Endpoint::new(policy, Vec::with_capacity(policy.maximum_samples));
        for index in 0..16 {
            endpoint.observe(&frame(if index < 4 { 0.9 } else { 0.0 }, index as f32));
        }
        let mut buffer = Vec::with_capacity(MODEL_SAMPLES);
        let pointer = buffer.as_ptr();
        let snapshot = endpoint.snapshot(&mut buffer).unwrap();
        assert_eq!(snapshot.end, 12 * 512);
        assert_eq!(buffer.len(), 12 * 512);
        assert_eq!(buffer.as_ptr(), pointer);
        assert!(endpoint.snapshot(&mut buffer).is_none());
        endpoint.observe(&frame(0.4, 0.2));
        assert_ne!(endpoint.speech().unwrap().revision, snapshot.revision);
    }

    #[test]
    fn arena_is_returned_for_delivery_and_abort() {
        let policy = Policy {
            maximum_samples: 20 * VAD_FRAME_SAMPLES,
            ..Policy::default()
        };
        let arena = Vec::with_capacity(policy.maximum_samples);
        let capacity = arena.capacity();
        let mut endpoint = Endpoint::new(policy, arena);
        for _ in 0..8 {
            endpoint.observe(&frame(0.9, 0.2));
        }
        let (audio, arena) = endpoint.finish(Keep::SpeechTail);
        assert_eq!(arena.capacity(), capacity);
        assert!(audio.samples().len() <= 8 * VAD_FRAME_SAMPLES + TRAILING_SAMPLES);
        let mut endpoint = Endpoint::new(policy, arena);
        endpoint.observe(&frame(0.9, 0.2));
        let arena = endpoint.abort();
        assert_eq!(arena.capacity(), capacity);
    }

    fn probability_sequence() -> impl Strategy<Value = Vec<f32>> {
        prop::collection::vec(0.0f32..=1.0, 0..300)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]
        #[test]
        fn retained_audio_is_one_contiguous_window(probabilities in probability_sequence()) {
            let policy = Policy {
                pause_samples: 1_024,
                threshold: 0.01,
                no_speech_samples: 2_048,
                silence_limit_samples: 4_096,
                maximum_samples: 20 * VAD_FRAME_SAMPLES,
            };
            let mut endpoint = Endpoint::new(policy, Vec::with_capacity(policy.maximum_samples));
            let mut observed = Vec::new();
            let mut buffer = Vec::with_capacity(MODEL_SAMPLES);
            for (index, probability) in probabilities.into_iter().enumerate() {
                let value = (index as f32 + 1.0) / 1_000.0;
                observed.extend_from_slice(&[value; VAD_FRAME_SAMPLES]);
                let flow = endpoint.observe(&frame(probability, value));
                if let Some(snapshot) = endpoint.snapshot(&mut buffer) {
                    prop_assert!(snapshot.end <= endpoint.end);
                    prop_assert!(!buffer.is_empty());
                    prop_assert!(buffer.len() <= MODEL_SAMPLES);
                }
                if matches!(flow, Flow::Stop(_)) {
                    break;
                }
            }
            let Some(speech) = endpoint.speech() else {
                return Ok(());
            };
            prop_assert!(speech.onset <= speech.last);
            prop_assert!(speech.last <= endpoint.end);
            let (audio, arena) = endpoint.finish(Keep::SpeechTail);
            let start = audio.start_sample() as usize;
            let end = audio.end_sample() as usize;
            prop_assert!(end <= observed.len());
            prop_assert_eq!(audio.samples(), &observed[start..end]);
            prop_assert!(arena.capacity() >= policy.maximum_samples);
        }
    }

    #[test]
    fn duration_limit_keeps_all_retained_audio() {
        let policy = Policy {
            maximum_samples: 4 * VAD_FRAME_SAMPLES,
            ..Policy::default()
        };
        let mut endpoint = Endpoint::new(policy, Vec::with_capacity(policy.maximum_samples));
        let mut flow = Flow::Continue;
        for _ in 0..8 {
            flow = endpoint.observe(&frame(0.9, 0.2));
        }
        assert_eq!(flow, Flow::Stop(StopReason::DurationLimit));
        let (audio, _) = endpoint.finish(Keep::All);
        assert_eq!(audio.len(), policy.maximum_samples);
    }

    #[test]
    fn no_speech_limit_stops_a_waiting_endpoint() {
        let policy = Policy {
            no_speech_samples: 2 * VAD_FRAME_SAMPLES as u64,
            ..Policy::default()
        };
        let mut endpoint = Endpoint::new(policy, Vec::with_capacity(policy.maximum_samples));
        endpoint.observe(&frame(0.0, 0.0));
        let flow = endpoint.observe(&frame(0.0, 0.0));
        assert_eq!(flow, Flow::Stop(StopReason::NoSpeech));
        assert!(endpoint.speech().is_none());
        let arena = endpoint.abort();
        assert!(arena.is_empty());
    }
}
