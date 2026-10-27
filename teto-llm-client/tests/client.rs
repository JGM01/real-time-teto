use std::{ffi::OsString, path::PathBuf, time::Duration};

use teto_llm_client::{Client, ClientError, ClientOptions, JournalAdmin, WorkerLaunch};
use teto_llm_protocol::*;

fn launch(args: &[&str]) -> WorkerLaunch {
    WorkerLaunch::new(PathBuf::from(env!("CARGO_BIN_EXE_teto-llm-client-fixture")))
        .with_args(args.iter().map(OsString::from))
}

fn config() -> OpenConfig {
    OpenConfig {
        assets: PathBuf::from("unused"),
        profile: ProfileId::Desktop,
        output_language: OutputLanguage::English,
        system_prompt: String::new(),
        seed: 0,
        journal: JournalAccess::Disabled,
        asset_cache: None,
        host: HostIdentity::new("test", "test"),
        delivery_window: DeliveryWindow::One,
    }
}

fn options() -> ClientOptions {
    ClientOptions {
        open_us: 2_000_000,
        start_us: 2_000_000,
        pull_us: 2_000_000,
        finish_us: 2_000_000,
        close_us: 2_000_000,
        shutdown_poll_us: 25_000,
        shutdown: None,
    }
}

fn user() -> UserInput {
    UserInput::new("hello".into(), false).expect("valid input")
}

#[test]
fn start_pull_ack_finish_and_close() {
    let mut client = Client::open(launch(&[]), config(), options()).expect("open");
    let turn = client.start(user()).expect("start");
    let ClauseEvent::Clause(clause) = client.pull(turn).expect("pull") else {
        panic!("expected clause");
    };
    assert_eq!(clause.text(), "Hello.");
    client
        .acknowledge(ClausePlayed {
            turn,
            index: clause.index(),
        })
        .expect("acknowledge");
    assert!(matches!(
        client.pull(turn).expect("end"),
        ClauseEvent::End {
            reason: GenerationStop::Eos,
            ..
        }
    ));
    let record = client
        .finish(turn, PlaybackStop::Complete, ReceiptSource::Display)
        .expect("finish");
    assert_eq!(record.assistant, ["Hello."]);
    assert_eq!(record.receipt_source, ReceiptSource::Display);
    client.close().expect("close");
}

#[test]
fn finish_preserves_playback_source() {
    let mut client = Client::open(launch(&[]), config(), options()).expect("open");
    let turn = client.start(user()).expect("start");
    let record = client
        .finish(turn, PlaybackStop::Complete, ReceiptSource::Playback)
        .expect("finish");
    assert_eq!(record.receipt_source, ReceiptSource::Playback);
    client.close().expect("close");
}

#[test]
fn slow_pull_times_out() {
    let mut slow = options();
    slow.pull_us = 50_000;
    let mut client = Client::open(launch(&["--slow"]), config(), slow).expect("open");
    let turn = client.start(user()).expect("start");
    let error = client.pull(turn).expect_err("pull must time out");
    assert!(matches!(error, ClientError::Timeout { operation: "pull" }));
}

#[test]
fn crash_is_a_transport_failure() {
    let mut client = Client::open(launch(&["--crash"]), config(), options()).expect("open");
    std::thread::sleep(Duration::from_millis(50));
    assert!(client.start(user()).is_err());
}

#[test]
fn journal_admin_round_trip() {
    let directory = PathBuf::from("/unused");
    let identity = JournalIdentity {
        journal: [1; 16],
        key: [2; 16],
    };
    JournalAdmin::initialize(
        launch(&[]),
        directory.clone(),
        identity,
        SecretKey::new([7; 32]),
    )
    .expect("initialize");
    assert_eq!(
        JournalAdmin::inspect(launch(&[]), directory.clone()).expect("inspect"),
        identity
    );
    let prepared = JournalAdmin::prepare_delete(launch(&[]), directory, SecretKey::new([7; 32]))
        .expect("prepare delete");
    prepared.delete().expect("delete");
}
