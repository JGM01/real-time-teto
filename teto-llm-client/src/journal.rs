use std::time::Duration;

use teto_llm_protocol::{JournalIdentity, Request, Response, SecretKey, Sensitivity, policy};

use crate::{ClientError, WorkerLaunch, transport::Transport};

fn timeout() -> Duration {
    Duration::from_secs(policy::STARTUP_SECONDS)
}

/// One-shot journal administration over a private worker.
pub struct JournalAdmin;

impl JournalAdmin {
    /// Create a new journal.
    pub fn initialize(
        launch: WorkerLaunch,
        directory: impl Into<std::path::PathBuf>,
        identity: JournalIdentity,
        key: SecretKey,
    ) -> Result<(), ClientError> {
        let mut transport = Transport::spawn(launch)?;
        let response = transport.request(
            Request::Initialize {
                directory: directory.into(),
                identity,
                key,
            },
            Sensitivity::Secret,
            timeout(),
        )?;
        expect(response, |response| matches!(response, Response::Ready))
    }

    /// Read the authenticated journal identity.
    pub fn inspect(
        launch: WorkerLaunch,
        directory: impl Into<std::path::PathBuf>,
    ) -> Result<JournalIdentity, ClientError> {
        let mut transport = Transport::spawn(launch)?;
        match transport.request(
            Request::Inspect(directory.into()),
            Sensitivity::Public,
            timeout(),
        )? {
            Response::Identity(identity) => Ok(identity),
            _ => Err(ClientError::Protocol),
        }
    }

    /// Authenticate the journal and hold its writer lock for deletion.
    pub fn prepare_delete(
        launch: WorkerLaunch,
        directory: impl Into<std::path::PathBuf>,
        key: SecretKey,
    ) -> Result<PreparedDelete, ClientError> {
        let mut transport = Transport::spawn(launch)?;
        let response = transport.request(
            Request::PrepareDelete {
                directory: directory.into(),
                key,
            },
            Sensitivity::Secret,
            timeout(),
        )?;
        expect(response, |response| matches!(response, Response::Ready))?;
        Ok(PreparedDelete { transport })
    }
}

/// A journal that is authenticated and writer-locked for deletion.
pub struct PreparedDelete {
    transport: Transport,
}

impl PreparedDelete {
    /// Delete the prepared journal.
    pub fn delete(mut self) -> Result<(), ClientError> {
        let response = self
            .transport
            .request(Request::Delete, Sensitivity::Public, timeout())?;
        expect(response, |response| matches!(response, Response::Closed))
    }
}

fn expect(
    response: Response,
    predicate: impl FnOnce(&Response) -> bool,
) -> Result<(), ClientError> {
    if predicate(&response) {
        Ok(())
    } else {
        Err(ClientError::Protocol)
    }
}
