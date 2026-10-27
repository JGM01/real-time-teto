//! Private desktop child. Standard input and output carry IPC frames only.
#![forbid(unsafe_code)]
use crossbeam_channel::bounded;
use std::{
    io,
    sync::{Arc, Mutex},
    thread,
};
use teto_llm::*;

type Output = Arc<Mutex<io::Stdout>>;

struct Incoming {
    seq: u64,
    request: Request,
}

fn send(output: &Output, seq: u64, response: Response) -> Result<()> {
    let mut stdout = output.lock().map_err(|_| failure(ErrorKind::Worker))?;
    write_ipc(&mut *stdout, seq, Sensitivity::Public, &response)
}

fn read_request(input: &mut impl io::Read) -> Result<Incoming> {
    let (seq, request) = read_ipc(input)?;
    Ok(Incoming { seq, request })
}

fn main() -> std::process::ExitCode {
    let output = Arc::new(Mutex::new(io::stdout()));
    match run(&output) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(_) => std::process::ExitCode::FAILURE,
    }
}

fn run(output: &Output) -> Result<()> {
    let mut input = io::stdin().lock();
    let first = match read_request(&mut input) {
        Ok(first) => first,
        Err(error) => {
            let _ = send(output, 0, Response::Error(error.clone()));
            return Err(error);
        }
    };
    let seq = first.seq;
    let result = match first.request {
        Request::Open(config) => {
            drop(input);
            return match Llm::open(config) {
                Ok(llm) => serve(llm, seq, output),
                Err(error) => {
                    send(output, seq, Response::Error(error))?;
                    Ok(())
                }
            };
        }
        Request::Initialize {
            directory,
            identity,
            key,
        } => {
            journal::Journal::initialize(&directory, identity, &key)?;
            send(output, seq, Response::Ready)
        }
        Request::Inspect(directory) => send(
            output,
            seq,
            Response::Identity(journal::Journal::inspect(&directory)?),
        ),
        Request::PrepareDelete { directory, key } => {
            let journal = journal::Journal::open_for_delete(directory, key)?;
            send(output, seq, Response::Ready)?;
            let next = read_request(&mut input)?;
            if !matches!(next.request, Request::Delete) {
                return Err(failure(ErrorKind::Protocol));
            }
            journal.delete()?;
            send(output, next.seq, Response::Closed)
        }
        _ => Err(failure(ErrorKind::Protocol)),
    };
    if let Err(error) = &result {
        let _ = send(output, seq, Response::Error(error.clone()));
    }
    result
}

fn serve(mut llm: Llm, seq: u64, output: &Output) -> Result<()> {
    let control: Arc<Mutex<Option<TurnControl>>> = Arc::new(Mutex::new(None));
    let (send_request, receive) = bounded(policy::CONTROL_QUEUE);
    let reader_control = Arc::clone(&control);
    let reader_output = Arc::clone(output);
    let reader = thread::spawn(move || {
        let mut input = io::stdin().lock();
        read_requests(&mut input, send_request, reader_control, reader_output)
    });
    send(output, seq, Response::Ready)?;
    let mut turn: Option<Turn> = None;
    while let Ok(incoming) = receive.recv() {
        if matches!(incoming.request, Request::Close) {
            if let Some(turn) = &turn {
                turn.control().cancel();
            }
            if let Err(error) = llm.close() {
                send(output, incoming.seq, Response::Error(error))?;
                continue;
            }
            send(output, incoming.seq, Response::Closed)?;
            let _ = reader.join().map_err(|_| failure(ErrorKind::Worker))?;
            return Ok(());
        }
        let response = dispatch(&mut llm, &mut turn, incoming.request, &control)
            .unwrap_or_else(Response::Error);
        send(output, incoming.seq, response)?;
    }
    if let Some(turn) = &turn {
        turn.control().cancel();
    }
    reader.join().map_err(|_| failure(ErrorKind::Worker))??;
    Err(failure(ErrorKind::Worker))
}

fn read_requests(
    input: &mut impl io::Read,
    sender: crossbeam_channel::Sender<Incoming>,
    control: Arc<Mutex<Option<TurnControl>>>,
    output: Output,
) -> Result<()> {
    let _cancel_on_exit = CancelOnExit(Arc::clone(&control));
    loop {
        let incoming = match read_request(input) {
            Ok(incoming) => incoming,
            Err(error) => {
                if let Some(active) = &*control.lock().map_err(|_| failure(ErrorKind::Worker))? {
                    active.cancel();
                }
                return Err(error);
            }
        };
        match incoming.request {
            Request::Cancel(id) => {
                if let Some(active) = &*control.lock().map_err(|_| failure(ErrorKind::Worker))?
                    && active.id() == id
                {
                    active.cancel();
                }
            }
            Request::Played(receipt) => {
                let result = control
                    .lock()
                    .map_err(|_| failure(ErrorKind::Worker))?
                    .as_ref()
                    .ok_or_else(|| failure(ErrorKind::Receipt))
                    .and_then(|active| active.acknowledge(receipt));
                send(
                    &output,
                    incoming.seq,
                    result.map_or_else(Response::Error, |()| Response::Acknowledged),
                )?;
            }
            _ => {
                sender
                    .try_send(incoming)
                    .map_err(|_| failure(ErrorKind::Protocol))?;
            }
        }
    }
}

struct CancelOnExit(Arc<Mutex<Option<TurnControl>>>);
impl Drop for CancelOnExit {
    fn drop(&mut self) {
        if let Ok(control) = self.0.lock()
            && let Some(active) = &*control
        {
            active.cancel();
        }
    }
}

fn dispatch(
    llm: &mut Llm,
    turn: &mut Option<Turn>,
    request: Request,
    control: &Mutex<Option<TurnControl>>,
) -> Result<Response> {
    match request {
        Request::Start(input) => {
            let started = llm.start_turn(input)?;
            let id = started.id();
            *control.lock().map_err(|_| failure(ErrorKind::Worker))? = Some(started.control());
            *turn = Some(started);
            Ok(Response::Started(id))
        }
        Request::Pull(id) => {
            let turn = turn
                .as_mut()
                .filter(|turn| turn.id() == id)
                .ok_or_else(|| failure(ErrorKind::Protocol))?;
            Ok(Response::Event(turn.next_clause()?))
        }
        Request::Finish {
            turn: id,
            playback,
            source,
        } => {
            let active = turn
                .as_mut()
                .filter(|turn| turn.id() == id)
                .ok_or_else(|| failure(ErrorKind::Receipt))?;
            let record = llm.finish_turn(active, playback, source)?;
            *turn = None;
            *control.lock().map_err(|_| failure(ErrorKind::Worker))? = None;
            Ok(Response::Finished(Box::new(record)))
        }
        Request::Retry => {
            let record = llm.retry_pending()?;
            *turn = None;
            *control.lock().map_err(|_| failure(ErrorKind::Worker))? = None;
            Ok(Response::Finished(Box::new(record)))
        }
        Request::Reset => {
            llm.reset_conversation()?;
            Ok(Response::Reset)
        }
        _ => Err(failure(ErrorKind::Protocol)),
    }
}
