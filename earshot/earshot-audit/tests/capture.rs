//! End-to-end capture through the public replay API and real inference engines.
use earshot::{Audio, Event, IncompleteReason, Outcome, Timings};
use earshot_audit::{Candidate, ReplayPacing, open_samples};
use std::{
    path::Path,
    time::{Duration, Instant},
};

fn capture_to_limit(pacing: ReplayPacing) -> (Audio, Timings, f64) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/speech-silence.wav");
    let mut reader = hound::WavReader::open(path).unwrap();
    let clip: Vec<f32> = reader
        .samples::<i16>()
        .map(|sample| f32::from(sample.unwrap()) / 32768.0)
        .collect();
    // Repeated real speech exercises transport limits, not semantic qualification.
    let samples = clip[16000..64000]
        .iter()
        .copied()
        .cycle()
        .take(308 * 16000)
        .collect();
    let (mut owner, mut traces) = open_samples(
        samples,
        pacing,
        Candidate {
            pause_ms: 1024,
            threshold: 0.01,
        },
    )
    .unwrap();
    let start = Instant::now();
    let id = owner.control().activate().unwrap();
    let (audio, timings) = loop {
        match owner
            .recv_timeout(Duration::from_secs(320))
            .unwrap()
            .unwrap()
        {
            Event::Listening(actual) => assert_eq!(actual, id),
            Event::Finished {
                id: actual,
                outcome,
            } => {
                assert_eq!(actual, id);
                let Outcome::Incomplete {
                    audio,
                    reason,
                    timings,
                } = outcome
                else {
                    panic!("expected capture duration limit: {outcome:?}")
                };
                assert_eq!(reason, IncompleteReason::DurationLimit);
                break (audio, timings);
            }
            event => panic!("unexpected event: {event:?}"),
        }
    };
    let elapsed = start.elapsed().as_secs_f64();
    let trace = traces.try_recv().unwrap().unwrap();
    assert!(trace.complete);
    assert_eq!(trace.activation_id, id.get());
    assert_eq!(audio.len(), 300 * 16000);
    owner.close().unwrap();
    (audio, timings, elapsed)
}

#[test]
fn accelerated_capture_reaches_the_owned_audio_limit() {
    capture_to_limit(ReplayPacing::Accelerated);
}

#[cfg(not(debug_assertions))]
#[test]
#[ignore = "five-minute release replay on the development Mac"]
fn release_five_minute_capture_has_no_growing_backlog() {
    let (_, timings, elapsed) = capture_to_limit(ReplayPacing::Realtime);
    let backlog_ms = elapsed * 1000.0 - timings.decision_sample as f64 / 16.0;
    println!(
        "five-minute replay: elapsed {elapsed:.6}s, backlog {backlog_ms:.3}ms, VAD max {:.3}ms",
        timings.vad_max_ms
    );
    assert!(backlog_ms < 250.0, "backlog must stay bounded");
    assert!(timings.vad_max_ms < 10.0);
}
