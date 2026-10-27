//! Protocol fixture for client tests. It never loads a model.
#![forbid(unsafe_code)]

use std::{io, thread, time::Duration};

use teto_llm_protocol::{
    Clause, ClauseEvent, ClauseIndex, FinalizedTurn, GenerationStop, JournalIdentity, PlaybackStop,
    Request, Response, Sensitivity, SpeakableText, TurnId, TurnMetrics, UserInput, policy,
    read_ipc, write_ipc,
};

fn main() {
    let slow = std::env::args().any(|arg| arg == "--slow");
    let crash = std::env::args().any(|arg| arg == "--crash");
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    let Ok((seq, request)) = read_ipc::<Request>(&mut input) else {
        return;
    };
    match request {
        Request::Initialize { .. } => {
            let _ = send(&mut output, seq, Response::Ready);
        }
        Request::Inspect(_) => {
            let _ = send(
                &mut output,
                seq,
                Response::Identity(JournalIdentity {
                    journal: [1; 16],
                    key: [2; 16],
                }),
            );
        }
        Request::PrepareDelete { .. } => {
            let _ = send(&mut output, seq, Response::Ready);
            let Ok((next, Request::Delete)) = read_ipc::<Request>(&mut input) else {
                return;
            };
            let _ = send(&mut output, next, Response::Closed);
        }
        Request::Open(_) => {
            let _ = send(&mut output, seq, Response::Ready);
            if crash {
                std::process::exit(1);
            }
            serve(&mut input, &mut output, slow);
        }
        _ => {
            let _ = send(
                &mut output,
                seq,
                Response::Error(teto_llm_protocol::failure(
                    teto_llm_protocol::ErrorKind::Protocol,
                )),
            );
        }
    }
}

fn serve(input: &mut impl io::Read, output: &mut impl io::Write, slow: bool) {
    let turn = TurnId::new(1).expect("valid turn");
    let mut pulled = false;
    while let Ok((seq, request)) = read_ipc::<Request>(input) {
        let response = match request {
            Request::Start(_) => Response::Started(turn),
            Request::Pull(id) if id == turn => {
                if slow {
                    thread::sleep(Duration::from_millis(200));
                }
                if pulled {
                    Response::Event(ClauseEvent::End {
                        turn,
                        reason: GenerationStop::Eos,
                    })
                } else {
                    pulled = true;
                    let clause = Clause::new(
                        turn,
                        ClauseIndex::new(0).expect("valid index"),
                        SpeakableText::try_from("Hello.".to_owned()).expect("valid text"),
                    )
                    .expect("valid clause");
                    Response::Event(ClauseEvent::Clause(clause))
                }
            }
            Request::Played(_) => Response::Acknowledged,
            Request::Cancel(_) => continue,
            Request::Finish {
                turn,
                playback,
                source,
            } => Response::Finished(Box::new(record(turn, playback, source))),
            Request::Reset => Response::Reset,
            Request::Retry => Response::Finished(Box::new(record(
                turn,
                PlaybackStop::Complete,
                teto_llm_protocol::ReceiptSource::Display,
            ))),
            Request::Close => {
                let _ = send(output, seq, Response::Closed);
                return;
            }
            _ => Response::Error(teto_llm_protocol::failure(
                teto_llm_protocol::ErrorKind::Protocol,
            )),
        };
        if send(output, seq, response).is_err() {
            return;
        }
    }
}

fn record(
    turn: TurnId,
    playback: PlaybackStop,
    source: teto_llm_protocol::ReceiptSource,
) -> FinalizedTurn {
    FinalizedTurn {
        schema_version: 2,
        policy_version: policy::VERSION,
        session: [0; 16],
        turn,
        profile: policy::PROFILE_ID.into(),
        user: UserInput::new("hello".into(), false).expect("valid input"),
        assistant: vec!["Hello.".into()],
        generation_stop: GenerationStop::Eos,
        playback_stop: playback,
        partial: playback != PlaybackStop::Complete,
        receipt_source: source,
        metrics: TurnMetrics::default(),
    }
}

fn send(output: &mut impl io::Write, seq: u64, response: Response) -> io::Result<()> {
    write_ipc(output, seq, Sensitivity::Public, &response)
        .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "write"))
}
