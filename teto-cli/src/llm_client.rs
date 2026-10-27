//! Parent-side client. This module has no inference/runtime dependency.
use std::{
    env, io,
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use teto_llm_protocol::*;

pub(super) struct Client {
    child: Child,
    input: Option<ChildStdin>,
    responses: Receiver<Result<Response>>,
    reader: Option<JoinHandle<()>>,
    active: Option<TurnId>,
    shutdown: Arc<AtomicBool>,
}
impl Client {
    pub fn spawn(shutdown: Arc<AtomicBool>) -> Result<Self> {
        let executable = match env::var_os("TETO_LLM_WORKER") {
            Some(path) => PathBuf::from(path),
            None => env::current_exe()
                .map_err(|_| failure(ErrorKind::Worker))?
                .with_file_name("teto-llm-worker"),
        };
        Self::spawn_program(executable, shutdown)
    }
    fn spawn_program(executable: PathBuf, shutdown: Arc<AtomicBool>) -> Result<Self> {
        let mut child = Command::new(executable)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|_| failure(ErrorKind::Worker))?;
        let input = child
            .stdin
            .take()
            .ok_or_else(|| failure(ErrorKind::Worker))?;
        let mut output = child
            .stdout
            .take()
            .ok_or_else(|| failure(ErrorKind::Worker))?;
        let (send, responses) = mpsc::sync_channel(1);
        let reader = thread::spawn(move || {
            loop {
                let response = read_frame::<Response>(&mut output);
                let failed = response.is_err();
                if send.send(response).is_err() || failed {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            input: Some(input),
            responses,
            reader: Some(reader),
            active: None,
            shutdown,
        })
    }
    pub fn request(&mut self, request: &Request) -> Result<Response> {
        self.request_with_timeout(request, Duration::from_secs(policy::STARTUP_SECONDS))
    }
    pub fn request_with_timeout(
        &mut self,
        request: &Request,
        timeout: Duration,
    ) -> Result<Response> {
        write_frame(
            self.input
                .as_mut()
                .ok_or_else(|| failure(ErrorKind::Worker))?,
            request,
        )?;
        let started = Instant::now();
        let mut cancelled = false;
        loop {
            if started.elapsed() >= timeout {
                return Err(failure(ErrorKind::Worker));
            }
            if self.shutdown.load(Ordering::Acquire) && !cancelled {
                if let Some(id) = self.active {
                    self.cancel(id)?;
                    cancelled = true;
                } else if !matches!(request, Request::Start(_)) {
                    return Err(failure(ErrorKind::Cancelled));
                }
            }
            match self.responses.recv_timeout(Duration::from_millis(25)) {
                Ok(Ok(response)) if is_stale(&response, self.active) => continue,
                Ok(Ok(Response::Error(error))) => return Err(error),
                Ok(result) => return result,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => return Err(failure(ErrorKind::Worker)),
            }
        }
    }
    pub fn cancel(&mut self, id: TurnId) -> Result<()> {
        write_frame(
            self.input
                .as_mut()
                .ok_or_else(|| failure(ErrorKind::Worker))?,
            &Request::Cancel(id),
        )
    }
    pub fn start(&mut self, input: UserInput) -> Result<TurnId> {
        match self.request(&Request::Start(input))? {
            Response::Started(id) => {
                self.active = Some(id);
                Ok(id)
            }
            _ => Err(failure(ErrorKind::Protocol)),
        }
    }
    pub fn finish(&mut self, turn: TurnId, playback: PlaybackStop) -> Result<FinalizedTurn> {
        let request = Request::Finish {
            turn,
            playback,
            source: ReceiptSource::Display,
        };
        match self.request_with_timeout(&request, Duration::from_secs(policy::SHUTDOWN_SECONDS))? {
            Response::Finished(record) if record.turn == turn => {
                self.active = None;
                Ok(*record)
            }
            _ => Err(failure(ErrorKind::Protocol)),
        }
    }
    pub fn close(mut self) -> Result<()> {
        let started = Instant::now();
        // Shutdown is already authorized; do not treat the flag as a new startup cancellation.
        let closing = Request::Close;
        write_frame(
            self.input
                .as_mut()
                .ok_or_else(|| failure(ErrorKind::Worker))?,
            &closing,
        )?;
        self.input.take();
        let response = self
            .responses
            .recv_timeout(
                Duration::from_secs(policy::SHUTDOWN_SECONDS).saturating_sub(started.elapsed()),
            )
            .map_err(|_| failure(ErrorKind::Worker))??;
        match response {
            Response::Closed => (),
            Response::Error(error) => return Err(error),
            _ => return Err(failure(ErrorKind::Protocol)),
        }
        self.wait_exit(started)
    }
    fn wait_exit(&mut self, started: Instant) -> Result<()> {
        loop {
            if let Some(status) = self
                .child
                .try_wait()
                .map_err(|_| failure(ErrorKind::Worker))?
            {
                if !status.success() {
                    return Err(failure(ErrorKind::Worker));
                }
                return Ok(());
            }
            if started.elapsed().as_secs() >= policy::SHUTDOWN_SECONDS {
                return Err(failure(ErrorKind::Worker));
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}

fn is_stale(response: &Response, current: Option<TurnId>) -> bool {
    let Some(current) = current else {
        return false;
    };
    let old = match response {
        Response::Event(ClauseEvent::Clause(clause)) => clause.turn(),
        Response::Event(ClauseEvent::End { turn, .. } | ClauseEvent::Cancelled { turn }) => *turn,
        _ => return false,
    };
    old.get() < current.get()
}

impl Drop for Client {
    fn drop(&mut self) {
        self.input.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
        // Drop the receiver before joining: it may hold the final error response.
        let (_, replacement) = mpsc::sync_channel(1);
        drop(std::mem::replace(&mut self.responses, replacement));
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

pub(super) fn require(response: Response, expected: fn(&Response) -> bool) -> Result<()> {
    if expected(&response) {
        Ok(())
    } else {
        Err(failure(ErrorKind::Protocol))
    }
}

pub(super) fn io_error(_: io::Error) -> Error {
    failure(ErrorKind::Worker)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "macos")]
    #[test]
    fn shutdown_uses_original_deadline() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let program = root.path().join("waiting-child");
        std::fs::write(
            &program,
            "#!/usr/bin/python3\nimport signal\nsignal.pause()\n",
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut client = Client::spawn_program(program, Arc::new(AtomicBool::new(false))).unwrap();
        let started = Instant::now()
            .checked_sub(Duration::from_secs(policy::SHUTDOWN_SECONDS))
            .unwrap();
        assert_eq!(
            client.wait_exit(started).unwrap_err().kind(),
            ErrorKind::Worker
        );
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn shutdown_during_admission_preserves_turn_id() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let response = root.path().join("started.cbor");
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &Response::Started(TurnId::new(1).unwrap())).unwrap();
        std::fs::write(&response, bytes).unwrap();
        let program = root.path().join("protocol-fixture");
        let source = format!(
            "#!/usr/bin/python3\nimport sys\nn=int.from_bytes(sys.stdin.buffer.read(4),'big')\nsys.stdin.buffer.read(n)\nsys.stdout.buffer.write(open({response:?},'rb').read())\nsys.stdout.buffer.flush()\nsys.stdin.buffer.read()\n"
        );
        std::fs::write(&program, source).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut client = Client::spawn_program(program, Arc::new(AtomicBool::new(true))).unwrap();
        let id = client
            .start(UserInput::new("Hello.".into(), false).unwrap())
            .unwrap();
        assert_eq!(id.get(), 1);
        client.cancel(id).unwrap();
    }
    #[test]
    fn stale_clause_dropped() {
        let old = TurnId::new(1).unwrap();
        let current = TurnId::new(2).unwrap();
        let event = Response::Event(ClauseEvent::Cancelled { turn: old });
        assert!(is_stale(&event, Some(current)));
        assert!(!is_stale(&event, Some(old)));
        assert!(!is_stale(&event, None));
    }
}
