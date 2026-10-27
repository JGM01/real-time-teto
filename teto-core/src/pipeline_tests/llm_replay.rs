use super::*;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use teto_llm_protocol::*;

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Link {
    input: ChildStdin,
    output: ChildStdout,
    seq: u64,
}
impl Link {
    fn call(&mut self, request: Request) -> Response {
        self.seq += 1;
        write_ipc(&mut self.input, self.seq, Sensitivity::Public, &request).unwrap();
        loop {
            let (seq, response) = read_ipc::<Response>(&mut self.output).unwrap();
            if seq == self.seq {
                return response;
            }
        }
    }
}

#[test]
#[ignore = "real WAV, Earshot, Whisper and exact LLM assets; requires TETO_LLM_WORKER"]
fn replay_speech_to_clauses() {
    let mut core = replay_core();
    let Capture::Speech(transcript) = core.next_utterance().unwrap() else {
        panic!("fixture must contain speech");
    };
    let (mut child, mut link) = spawn_worker();
    assert!(matches!(
        link.call(Request::Open(llm_config())),
        Response::Ready
    ));
    let user = UserInput::new(transcript.text().into(), transcript.is_truncated()).unwrap();
    let Response::Started(turn) = link.call(Request::Start(user)) else {
        panic!("turn admission failed");
    };
    let clauses = drain(&mut link, turn, 0);
    assert!(clauses > 0);
    let request = Request::Finish {
        turn,
        playback: PlaybackStop::Complete,
        source: ReceiptSource::TestSink,
    };
    let Response::Finished(record) = link.call(request) else {
        panic!("finalization failed");
    };
    assert_eq!(record.user.text(), transcript.text());
    assert_eq!(record.assistant.len(), clauses);
    assert!(matches!(link.call(Request::Close), Response::Closed));
    drop(link);
    assert!(child.0.wait().unwrap().success());
    assert!(matches!(core.next_utterance().unwrap(), Capture::NoSpeech));
    core.close().unwrap();
}

fn drain(link: &mut Link, turn: TurnId, mut clauses: usize) -> usize {
    loop {
        match link.call(Request::Pull(turn)) {
            Response::Event(ClauseEvent::Clause(clause)) => {
                assert_eq!(clause.turn(), turn);
                assert_eq!(clause.index().get() as usize, clauses);
                assert!(!clause.text().is_empty());
                let receipt = ClausePlayed {
                    turn,
                    index: clause.index(),
                };
                assert!(matches!(
                    link.call(Request::Played(receipt)),
                    Response::Acknowledged
                ));
                clauses += 1;
            }
            Response::Event(ClauseEvent::End { .. }) => return clauses,
            response => panic!("unexpected event {response:?}"),
        }
    }
}

fn spawn_worker() -> (ChildGuard, Link) {
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
    (
        child,
        Link {
            input,
            output,
            seq: 0,
        },
    )
}

fn llm_config() -> OpenConfig {
    OpenConfig {
        assets: std::env::var_os("TETO_LLM_ASSETS")
            .expect("set TETO_LLM_ASSETS")
            .into(),
        profile: ProfileId::Desktop,
        output_language: OutputLanguage::English,
        system_prompt: String::new(),
        seed: 7,
        journal: JournalAccess::Disabled,
        asset_cache: std::env::var_os("TETO_ASSET_CACHE").map(Into::into),
        host: HostIdentity::new("test", "test"),
        delivery_window: DeliveryWindow::One,
    }
}

#[test]
#[ignore = "real WAV/Whisper and exact LLM assets in separate inferring processes"]
fn concurrent_native_inference() {
    let mut core = replay_core();
    let (mut child, mut link) = spawn_worker();
    assert!(matches!(
        link.call(Request::Open(llm_config())),
        Response::Ready
    ));
    let (started, ready) = std::sync::mpsc::sync_channel(1);
    let consumer = std::thread::spawn(move || concurrent_clauses(link, started));
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
    mut link: Link,
    started: std::sync::mpsc::SyncSender<std::time::Instant>,
) -> (usize, std::time::Instant) {
    let prompt = UserInput::new("Write a very long continuous story about a gardener exploring a forest. Include at least a thousand words of descriptive prose. Start immediately with the story and keep going without an introduction or conclusion.".into(), false).unwrap();
    let Response::Started(turn) = link.call(Request::Start(prompt)) else {
        panic!("admission failed")
    };
    let Response::Event(ClauseEvent::Clause(clause)) = link.call(Request::Pull(turn)) else {
        panic!("first clause required")
    };
    assert!(matches!(
        link.call(Request::Played(ClausePlayed {
            turn,
            index: clause.index()
        })),
        Response::Acknowledged
    ));
    started.send(std::time::Instant::now()).unwrap();
    let clauses = drain(&mut link, turn, 1);
    let finished = std::time::Instant::now();
    assert!(matches!(
        link.call(Request::Finish {
            turn,
            playback: PlaybackStop::Complete,
            source: ReceiptSource::TestSink
        }),
        Response::Finished(_)
    ));
    assert!(matches!(link.call(Request::Close), Response::Closed));
    drop(link);
    (clauses, finished)
}
