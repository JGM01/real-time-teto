use super::*;
use std::{
    io::{Read, Write},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};
use teto_llm_protocol::*;

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn call(input: &mut impl Write, output: &mut impl Read, request: Request) -> Response {
    write_frame(input, &request).unwrap();
    read_frame(output).unwrap()
}
#[test]
#[ignore = "real WAV, Earshot, Whisper and exact LLM assets; requires TETO_LLM_WORKER"]
fn replay_speech_to_clauses() {
    let mut core = replay_core();
    let Capture::Speech(transcript) = core.next_utterance().unwrap() else {
        panic!("fixture must contain speech");
    };
    let (mut child, mut input, mut output) = spawn_worker();
    let config = llm_config();
    assert!(matches!(
        call(&mut input, &mut output, Request::Open(config)),
        Response::Ready
    ));
    let user = UserInput::new(transcript.text().into(), transcript.is_truncated()).unwrap();
    let Response::Started(turn) = call(&mut input, &mut output, Request::Start(user)) else {
        panic!("turn admission failed");
    };
    let clauses = drain(&mut input, &mut output, turn, 0);
    assert!(clauses > 0);
    let request = Request::Finish {
        turn,
        playback: PlaybackStop::Complete,
        source: ReceiptSource::TestSink,
    };
    let Response::Finished(record) = call(&mut input, &mut output, request) else {
        panic!("finalization failed");
    };
    assert_eq!(record.user.text(), transcript.text());
    assert_eq!(record.assistant.len(), clauses);
    assert!(matches!(
        call(&mut input, &mut output, Request::Close),
        Response::Closed
    ));
    drop(input);
    assert!(child.0.wait().unwrap().success());
    // Silence is not submitted to the child at all.
    assert!(matches!(core.next_utterance().unwrap(), Capture::NoSpeech));
    core.close().unwrap();
}
fn drain(
    input: &mut ChildStdin,
    output: &mut ChildStdout,
    turn: TurnId,
    mut clauses: usize,
) -> usize {
    loop {
        match call(input, output, Request::Pull(turn)) {
            Response::Event(ClauseEvent::Clause(clause)) => {
                assert_eq!(clause.turn(), turn);
                assert_eq!(clause.index().get() as usize, clauses);
                assert!(!clause.text().is_empty());
                let receipt = ClausePlayed {
                    turn,
                    index: clause.index(),
                };
                assert!(matches!(
                    call(input, output, Request::Played(receipt)),
                    Response::Acknowledged
                ));
                clauses += 1;
            }
            Response::Event(ClauseEvent::End { .. }) => return clauses,
            response => panic!("unexpected event {response:?}"),
        }
    }
}

fn spawn_worker() -> (ChildGuard, ChildStdin, ChildStdout) {
    let worker = std::env::var_os("TETO_LLM_WORKER").expect("set TETO_LLM_WORKER");
    let mut child = ChildGuard(
        Command::new(worker)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let input = child.0.stdin.take().unwrap();
    let output = child.0.stdout.take().unwrap();
    (child, input, output)
}
fn llm_config() -> OpenConfig {
    OpenConfig {
        assets: std::env::var_os("TETO_LLM_ASSETS")
            .expect("set TETO_LLM_ASSETS")
            .into(),
        profile: ProfileId::desktop(),
        output_language: OutputLanguage::English,
        system_prompt: String::new(),
        seed: 7,
        journal: JournalAccess::Disabled,
    }
}
#[test]
#[ignore = "real WAV/Whisper and exact LLM assets in separate simultaneously inferring processes"]
fn concurrent_native_inference() {
    let mut core = replay_core();
    let (mut child, mut input, mut output) = spawn_worker();
    assert!(matches!(
        call(&mut input, &mut output, Request::Open(llm_config())),
        Response::Ready
    ));
    let (started, ready) = std::sync::mpsc::sync_channel(1);
    let consumer = std::thread::spawn(move || concurrent_clauses(input, output, started));
    let llm_started = ready
        .recv_timeout(std::time::Duration::from_secs(policy::STARTUP_SECONDS))
        .unwrap();
    let asr_started = std::time::Instant::now();
    let Capture::Speech(transcript) = core.next_utterance().unwrap() else {
        panic!("speech fixture required")
    };
    assert!(!transcript.text().is_empty());
    let asr_finished = std::time::Instant::now();
    let (clauses, llm_finished) = consumer.join().unwrap();
    let overlap = llm_finished
        .min(asr_finished)
        .duration_since(asr_started.max(llm_started));
    assert!(overlap > std::time::Duration::ZERO);
    assert!(clauses > 1);
    assert!(child.0.wait().unwrap().success());
    core.close().unwrap();
    println!(
        "native_inference_overlap_seconds={} clauses={clauses}",
        overlap.as_secs_f64()
    );
}
fn concurrent_clauses(
    mut input: ChildStdin,
    mut output: ChildStdout,
    started: std::sync::mpsc::SyncSender<std::time::Instant>,
) -> (usize, std::time::Instant) {
    let prompt = UserInput::new("Write a very long continuous story about a gardener exploring a forest. Include at least a thousand words of descriptive prose. Start immediately with the story and keep going without an introduction or conclusion.".into(), false).unwrap();
    let Response::Started(turn) = call(&mut input, &mut output, Request::Start(prompt)) else {
        panic!("admission failed")
    };
    let Response::Event(ClauseEvent::Clause(clause)) =
        call(&mut input, &mut output, Request::Pull(turn))
    else {
        panic!("first clause required")
    };
    assert!(matches!(
        call(
            &mut input,
            &mut output,
            Request::Played(ClausePlayed {
                turn,
                index: clause.index()
            })
        ),
        Response::Acknowledged
    ));
    started.send(std::time::Instant::now()).unwrap();
    let clauses = drain(&mut input, &mut output, turn, 1);
    let finished = std::time::Instant::now();
    assert!(matches!(
        call(
            &mut input,
            &mut output,
            Request::Finish {
                turn,
                playback: PlaybackStop::Complete,
                source: ReceiptSource::TestSink
            }
        ),
        Response::Finished(_)
    ));
    assert!(matches!(
        call(&mut input, &mut output, Request::Close),
        Response::Closed
    ));
    drop(input);
    (clauses, finished)
}
