//! Private desktop child. Stdin/stdout carry CBOR exclusively; never log them.
#![forbid(unsafe_code)]
use crossbeam_channel::bounded;
use std::{
    io,
    sync::{Arc, Mutex},
    thread,
};
use teto_llm::*;

type Output = Arc<Mutex<io::Stdout>>;
fn send(output: &Output, response: Response) -> Result<()> {
    write_frame(
        &mut *output.lock().map_err(|_| failure(ErrorKind::Worker))?,
        &response,
    )
}
fn main() -> std::process::ExitCode {
    let output = Arc::new(Mutex::new(io::stdout()));
    let result = run(&output);
    if let Err(error) = result {
        let _ = send(&output, Response::Error(error));
        return std::process::ExitCode::FAILURE;
    }
    std::process::ExitCode::SUCCESS
}
fn run(output: &Output) -> Result<()> {
    let first: Request = read_frame(&mut io::stdin().lock())?;
    match first {
        Request::Open(config) => serve(Llm::open(config)?, output),
        Request::Initialize {
            directory,
            identity,
            key,
        } => {
            journal::Journal::initialize(&directory, identity, &key)?;
            send(output, Response::Ready)
        }
        Request::Inspect(directory) => send(
            output,
            Response::Identity(journal::Journal::inspect(&directory)?),
        ),
        Request::PrepareDelete { directory, key } => {
            let journal = journal::Journal::open_for_delete(directory, key)?;
            send(output, Response::Ready)?;
            if !matches!(
                read_frame::<Request>(&mut io::stdin().lock())?,
                Request::Delete
            ) {
                return Err(failure(ErrorKind::Protocol));
            }
            journal.delete()?;
            send(output, Response::Closed)
        }
        _ => Err(failure(ErrorKind::Protocol)),
    }
}
fn serve(mut llm: Llm, output: &Output) -> Result<()> {
    let control: Arc<Mutex<Option<TurnControl>>> = Arc::new(Mutex::new(None));
    let (send_request, receive) = bounded(policy::CONTROL_QUEUE);
    let reader_control = Arc::clone(&control);
    let reader_output = Arc::clone(output);
    let reader = thread::spawn(move || read_requests(send_request, reader_control, reader_output));
    send(output, Response::Ready)?;
    let mut turn: Option<Turn> = None;
    while let Ok(request) = receive.recv() {
        if matches!(request, Request::Close) {
            if let Some(turn) = &turn {
                turn.control().cancel();
            }
            if let Err(error) = llm.close() {
                send(output, Response::Error(error))?;
                continue;
            }
            send(output, Response::Closed)?;
            // Close is terminal. Parent closes stdin, allowing reader shutdown.
            let _ = reader.join().map_err(|_| failure(ErrorKind::Worker))?;
            return Ok(());
        }
        let response =
            dispatch(&mut llm, &mut turn, request, &control).unwrap_or_else(Response::Error);
        send(output, response)?;
    }
    if let Some(turn) = &turn {
        turn.control().cancel();
    }
    reader.join().map_err(|_| failure(ErrorKind::Worker))??;
    Err(failure(ErrorKind::Worker))
}
fn read_requests(
    sender: crossbeam_channel::Sender<Request>,
    control: Arc<Mutex<Option<TurnControl>>>,
    output: Output,
) -> Result<()> {
    let _cancel_on_exit = CancelOnExit(Arc::clone(&control));
    let mut input = io::stdin().lock();
    loop {
        let request = match read_frame::<Request>(&mut input) {
            Ok(request) => request,
            Err(error) => {
                if let Some(active) = &*control.lock().map_err(|_| failure(ErrorKind::Worker))? {
                    active.cancel();
                }
                return Err(error);
            }
        };
        match request {
            Request::Cancel(id) => {
                if let Some(active) = &*control.lock().map_err(|_| failure(ErrorKind::Worker))? {
                    if active.id() == id {
                        active.cancel();
                    }
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
                    result.map_or_else(Response::Error, |()| Response::Acknowledged),
                )?;
            }
            Request::Close => {
                sender
                    .try_send(Request::Close)
                    .map_err(|_| failure(ErrorKind::Protocol))?;
            }
            request => {
                sender
                    .try_send(request)
                    .map_err(|_| failure(ErrorKind::Protocol))?;
            }
        }
    }
}

struct CancelOnExit(Arc<Mutex<Option<TurnControl>>>);
impl Drop for CancelOnExit {
    fn drop(&mut self) {
        if let Ok(control) = self.0.lock() {
            if let Some(active) = &*control {
                active.cancel();
            }
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
