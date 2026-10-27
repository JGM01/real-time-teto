//! Golden replay of the bundled speech fixture through the production pipeline.

use earshot::{Event, Outcome};
use earshot_audit::{Candidate, ReplayPacing, open_samples};
use std::time::Duration;

#[test]
fn golden_fixture_replay_has_stable_boundaries() {
    let path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/speech-silence.wav");
    let mut reader = hound::WavReader::open(path).unwrap();
    let mut samples: Vec<f32> = reader
        .samples::<i16>()
        .map(|sample| f32::from(sample.unwrap()) / 32768.0)
        .collect();
    assert_eq!(samples.len(), 80_000);
    samples.extend(vec![0.0; 5 * 16_000]);
    let (mut owner, mut traces) = open_samples(
        samples,
        ReplayPacing::Accelerated,
        Candidate {
            pause_ms: 1_024,
            threshold: 0.01,
        },
    )
    .unwrap();
    let id = owner.control().activate().unwrap();
    let outcome = loop {
        match owner
            .recv_timeout(Duration::from_secs(30))
            .unwrap()
            .unwrap()
        {
            Event::Listening(actual) => assert_eq!(actual, id),
            Event::Finished {
                id: actual,
                outcome,
            } => {
                assert_eq!(actual, id);
                break outcome;
            }
            event => panic!("unexpected event: {event:?}"),
        }
    };
    let Outcome::Complete(statement) = outcome else {
        panic!("the fixture has a complete final pause");
    };
    assert_eq!(statement.audio().start_sample(), 0);
    assert_eq!(statement.audio().end_sample(), 68_096);
    let timings = statement.timings();
    assert_eq!(timings.onset_sample, Some(512));
    assert_eq!(timings.last_speech_sample, Some(64_000));
    let pause_end = 64_000 + 16_384;
    assert!(timings.decision_sample >= pause_end);
    assert!(timings.decision_sample <= pause_end + 16_384);
    assert_eq!(timings.endpoint_calls, 1);

    let trace = traces.try_recv().unwrap().unwrap();
    assert!(trace.complete);
    assert_eq!(trace.outcome, "complete");
    assert!(trace.captured_samples >= trace.timings.decision_sample);
    assert!(trace.samples.len() as u64 >= trace.timings.decision_sample);
    assert_eq!(trace.evaluations.len(), 1);
    owner.close().unwrap();
}
