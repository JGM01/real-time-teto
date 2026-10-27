use crate::*;
use llama_cpp_2::{
    model::{AddBos, LlamaModel},
    token::LlamaToken,
};
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read, path::Path};

/// The built-in, immutable profile catalog.
pub struct Profile;

pub(crate) struct ProfileSpec {
    pub id: &'static str,
    pub file: &'static str,
    pub sha256: &'static str,
    pub template_sha256: &'static str,
    pub generation_prompt: &'static str,
}

impl Profile {
    /// Return the cache mode for one profile and host.
    pub(crate) fn cache_policy(profile: ProfileId, host: &HostIdentity) -> CacheMode {
        match (profile, host.device.as_str(), host.os.as_str()) {
            (ProfileId::Desktop, policy::PREFIX_REUSE_DEVICE, policy::PREFIX_REUSE_OS) => {
                CacheMode::VerifiedPrefixReuse
            }
            (
                ProfileId::PhoneSpike,
                policy::PHONE_PREFIX_REUSE_DEVICE,
                policy::PHONE_PREFIX_REUSE_OS,
            ) => CacheMode::VerifiedPrefixReuse,
            _ => CacheMode::FullRebuild,
        }
    }

    pub(crate) fn lookup(id: ProfileId) -> Result<ProfileSpec> {
        match id {
            ProfileId::Desktop => Ok(ProfileSpec {
                id: policy::PROFILE_ID,
                file: policy::MODEL_FILE,
                sha256: policy::MODEL_SHA256,
                template_sha256: policy::DESKTOP_TEMPLATE_SHA256,
                generation_prompt: "<turn|>\n<|turn>model\n<|channel>thought\n<channel|>",
            }),
            ProfileId::PhoneSpike => Ok(ProfileSpec {
                id: policy::PHONE_PROFILE,
                file: policy::PHONE_FILE,
                sha256: policy::PHONE_SHA256,
                template_sha256: policy::PHONE_TEMPLATE_SHA256,
                generation_prompt: "<turn|>\n<|turn>model\n",
            }),
        }
    }

    pub fn validate_config(config: &OpenConfig) -> Result<()> {
        Self::lookup(config.profile)?;
        if config.system_prompt.len() > policy::SYSTEM_BYTES || config.system_prompt.contains('\0')
        {
            return Err(failure(ErrorKind::Config));
        }
        Ok(())
    }

    pub fn validate_assets(root: &Path) -> Result<()> {
        Self::validate_assets_for(root, &Self::lookup(ProfileId::Desktop)?, None)
    }

    /// Verify the model file. A metadata cache avoids a full hash on each start.
    pub(crate) fn validate_assets_for(
        root: &Path,
        profile: &ProfileSpec,
        cache: Option<&Path>,
    ) -> Result<()> {
        let path = root.join(profile.file);
        if asset_digest(&path, cache)? != profile.sha256 {
            return Err(failure(ErrorKind::Profile));
        }
        Ok(())
    }

    pub(crate) fn validate_model(model: &LlamaModel, profile: &ProfileSpec) -> Result<()> {
        let template = model
            .meta_val_str("tokenizer.chat_template")
            .map_err(|_| failure(ErrorKind::Profile))?;
        if format!("{:x}", Sha256::digest(template.as_bytes())) != profile.template_sha256 {
            return Err(failure(ErrorKind::Profile));
        }
        if model
            .meta_val_str("general.architecture")
            .map_err(|_| failure(ErrorKind::Profile))?
            != "gemma4"
        {
            return Err(failure(ErrorKind::Profile));
        }
        for control in ["<|turn>", "<turn|>", "<|channel>", "<channel|>"] {
            let tokens = model
                .str_to_token(control, AddBos::Never)
                .map_err(|_| failure(ErrorKind::Profile))?;
            if tokens.len() != 1 {
                return Err(failure(ErrorKind::Profile));
            }
        }
        Ok(())
    }

    /// Append a control sequence. `first` adds the start token.
    pub(crate) fn append_control(
        model: &LlamaModel,
        text: &str,
        first: bool,
        out: &mut Vec<LlamaToken>,
    ) -> Result<()> {
        let tokens = model
            .str_to_token(text, if first { AddBos::Always } else { AddBos::Never })
            .map_err(|_| failure(ErrorKind::Profile))?;
        out.extend(tokens);
        Ok(())
    }

    /// Append literal text. The text is split before each `<`.
    ///
    /// A complete special token cannot form inside one piece.
    pub(crate) fn append_literal(
        model: &LlamaModel,
        text: &str,
        out: &mut Vec<LlamaToken>,
    ) -> Result<()> {
        for piece in text.split_inclusive('<') {
            let tokens = model
                .str_to_token(piece, AddBos::Never)
                .map_err(|_| failure(ErrorKind::Profile))?;
            out.extend(tokens);
        }
        Ok(())
    }
}

/// Return the SHA-256 hex digest of one file.
fn asset_digest(path: &Path, cache: Option<&Path>) -> Result<String> {
    let metadata = std::fs::metadata(path).map_err(|_| failure(ErrorKind::Profile))?;
    let Some(cache_directory) = cache else {
        return hash_file(path);
    };
    let identity = format!(
        "{} {} {}",
        metadata.len(),
        modified_ns(&metadata),
        file_key(&metadata)
    );
    let name = format!(
        "{:x}.sha256",
        Sha256::digest(path.as_os_str().as_encoded_bytes())
    );
    let cache_path = cache_directory.join(name);
    if let Ok(saved) = std::fs::read_to_string(&cache_path) {
        let mut lines = saved.lines();
        if lines.next() == Some(identity.as_str())
            && let Some(digest) = lines.next()
        {
            return Ok(digest.to_owned());
        }
    }
    let digest = hash_file(path)?;
    let _ = std::fs::write(&cache_path, format!("{identity}\n{digest}\n"));
    Ok(digest)
}

fn hash_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).map_err(|_| failure(ErrorKind::Profile))?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|_| failure(ErrorKind::Profile))?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

#[cfg(unix)]
fn modified_ns(metadata: &std::fs::Metadata) -> i128 {
    use std::os::unix::fs::MetadataExt;
    i128::from(metadata.mtime()) * 1_000_000_000 + i128::from(metadata.mtime_nsec())
}

#[cfg(not(unix))]
fn modified_ns(metadata: &std::fs::Metadata) -> i128 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |duration| {
            i128::try_from(duration.as_nanos()).unwrap_or(0)
        })
}

#[cfg(unix)]
fn file_key(metadata: &std::fs::Metadata) -> String {
    use std::os::unix::fs::MetadataExt;
    format!("{}:{}", metadata.dev(), metadata.ino())
}

#[cfg(not(unix))]
fn file_key(_: &std::fs::Metadata) -> String {
    String::from("0:0")
}

pub(crate) const INSTRUCTION: &str = "Answer directly in concise plain English for speech. Do not think aloud. Do not use markdown, tools, URLs, emoji or code. Write numbers, currency, dates, ordinals and units as English words. Preserve conversational references. Never expose control tokens.";

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> OpenConfig {
        OpenConfig {
            assets: "missing".into(),
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

    #[test]
    fn invalid_profile_prevents_load() {
        assert!(ProfileId::parse("arbitrary.gguf").is_err());
        assert!(Profile::lookup(ProfileId::Desktop).is_ok());
    }

    #[test]
    fn oversize_system_prompt_rejected() {
        let mut value = config();
        value.system_prompt = "a".repeat(policy::SYSTEM_BYTES + 1);
        assert_eq!(
            Profile::validate_config(&value).unwrap_err().kind(),
            ErrorKind::Config
        );
    }

    #[test]
    fn phone_profile_reuses_prefix_on_recorded_host() {
        let host = HostIdentity::new(
            policy::PHONE_PREFIX_REUSE_DEVICE,
            policy::PHONE_PREFIX_REUSE_OS,
        );
        assert_eq!(
            Profile::cache_policy(ProfileId::PhoneSpike, &host),
            CacheMode::VerifiedPrefixReuse
        );
        let other = HostIdentity::new("Mac15,7", policy::PREFIX_REUSE_OS);
        assert_eq!(
            Profile::cache_policy(ProfileId::PhoneSpike, &other),
            CacheMode::FullRebuild
        );
    }
}
