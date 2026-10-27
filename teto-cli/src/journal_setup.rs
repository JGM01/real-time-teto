//! Apple key access stays in the parent. Native-free child messages perform disk work.
use crate::llm_client::{Client, require};
use sha2::{Digest, Sha256};
use std::{
    env,
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
};
use teto_llm_protocol::*;
use zeroize::{Zeroize, Zeroizing};

const SERVICE: &str = "org.teto.encrypted-journal.v1";
fn directory() -> Result<PathBuf> {
    let path = PathBuf::from(
        env::var_os("TETO_JOURNAL_DIR")
            .filter(|v| !v.is_empty())
            .ok_or_else(|| failure(ErrorKind::Config))?,
    );
    if !path.is_absolute() {
        return Err(failure(ErrorKind::Config));
    }
    if path.exists() {
        return path
            .canonicalize()
            .map_err(|_| failure(ErrorKind::Persistence));
    }
    let parent = path
        .parent()
        .ok_or_else(|| failure(ErrorKind::Config))?
        .canonicalize()
        .map_err(|_| failure(ErrorKind::Persistence))?;
    Ok(parent.join(path.file_name().ok_or_else(|| failure(ErrorKind::Config))?))
}
fn account(path: &std::path::Path) -> Result<String> {
    let path = path.to_str().ok_or_else(|| failure(ErrorKind::Config))?;
    Ok(format!("{:x}", Sha256::digest(path.as_bytes())))
}
pub(super) fn access() -> Result<JournalAccess> {
    match env::var("TETO_JOURNAL").as_deref() {
        Ok("off") => Ok(JournalAccess::Disabled),
        Ok("on") | Err(_) => {
            let directory = directory()?;
            let key =
                lookup(&account(&directory)?)?.ok_or_else(|| failure(ErrorKind::Persistence))?;
            if !directory.is_dir() {
                return Err(failure(ErrorKind::Persistence));
            }
            Ok(JournalAccess::Encrypted { directory, key })
        }
        _ => Err(failure(ErrorKind::Config)),
    }
}
fn client() -> Result<Client> {
    Client::spawn(Arc::new(AtomicBool::new(false)))
}
fn random<const N: usize>() -> Result<[u8; N]> {
    let mut output = [0; N];
    getrandom::fill(&mut output).map_err(|_| failure(ErrorKind::Persistence))?;
    Ok(output)
}
pub(super) fn initialize() -> Result<()> {
    let directory = directory()?;
    let account = account(&directory)?;
    if directory.exists() || lookup(&account)?.is_some() {
        return Err(failure(ErrorKind::Persistence));
    }
    let key = SecretKey::new(random()?);
    let identity = JournalIdentity {
        journal: random()?,
        key: random()?,
    };
    // SecItemAdd fails on duplicates. Never use the password setter, which overwrites.
    create_key(&account, &key)?;
    let result = client().and_then(|mut client| {
        require(
            client.request(&Request::Initialize {
                directory,
                identity,
                key,
            })?,
            |r| matches!(r, Response::Ready),
        )
    });
    if result.is_err() {
        delete_key(&account)?;
    }
    result
}
pub(super) fn delete() -> Result<()> {
    let directory = directory()?;
    let account = account(&directory)?;
    let key = lookup(&account)?.ok_or_else(|| failure(ErrorKind::Persistence))?;
    let mut client = client()?;
    require(
        client.request(&Request::PrepareDelete { directory, key })?,
        |r| matches!(r, Response::Ready),
    )?;
    // The child holds the authenticated journal's writer lock before shredding.
    delete_key(&account)?;
    require(client.request(&Request::Delete)?, |r| {
        matches!(r, Response::Closed)
    })
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn lookup(account: &str) -> Result<Option<SecretKey>> {
    match security_framework::passwords::get_generic_password(SERVICE, account) {
        Ok(bytes) => {
            let bytes = Zeroizing::new(bytes);
            let mut key: [u8; 32] = bytes
                .as_slice()
                .try_into()
                .map_err(|_| failure(ErrorKind::Persistence))?;
            let secret = SecretKey::new(key);
            key.zeroize();
            Ok(Some(secret))
        }
        Err(error) if error.code() == -25300 => Ok(None),
        Err(_) => Err(failure(ErrorKind::Persistence)),
    }
}
#[cfg(any(target_os = "macos", target_os = "ios"))]
fn create_key(account: &str, key: &SecretKey) -> Result<()> {
    use core_foundation::data::CFData;
    use security_framework::item::{ItemAddOptions, ItemAddValue, ItemClass};
    let mut options = ItemAddOptions::new(ItemAddValue::Data {
        class: ItemClass::generic_password(),
        data: CFData::from_buffer(key.bytes()),
    });
    options.set_service(SERVICE).set_account_name(account);
    options.add().map_err(|_| failure(ErrorKind::Persistence))
}
#[cfg(any(target_os = "macos", target_os = "ios"))]
fn delete_key(account: &str) -> Result<()> {
    security_framework::passwords::delete_generic_password(SERVICE, account)
        .map_err(|_| failure(ErrorKind::Persistence))
}
#[cfg(not(any(target_os = "macos", target_os = "ios")))]
fn lookup(_: &str) -> Result<Option<SecretKey>> {
    Err(failure(ErrorKind::Persistence))
}
#[cfg(not(any(target_os = "macos", target_os = "ios")))]
fn create_key(_: &str, _: &SecretKey) -> Result<()> {
    Err(failure(ErrorKind::Persistence))
}
#[cfg(not(any(target_os = "macos", target_os = "ios")))]
fn delete_key(_: &str) -> Result<()> {
    Err(failure(ErrorKind::Persistence))
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    struct Cleanup(String);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = delete_key(&self.0);
        }
    }
    #[test]
    #[ignore = "explicit test creates and removes one isolated Keychain entry"]
    fn keychain_adapter() {
        let account = format!("test-{:x}", Sha256::digest(random::<32>().unwrap()));
        let _cleanup = Cleanup(account.clone());
        assert!(lookup(&account).unwrap().is_none());
        let key = SecretKey::new(random().unwrap());
        create_key(&account, &key).unwrap();
        assert_eq!(lookup(&account).unwrap().unwrap().bytes(), key.bytes());
        assert!(create_key(&account, &SecretKey::new([0; 32])).is_err());
        assert_eq!(lookup(&account).unwrap().unwrap().bytes(), key.bytes());
        delete_key(&account).unwrap();
        assert!(lookup(&account).unwrap().is_none());
    }
}
