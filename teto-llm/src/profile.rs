use crate::*;
use llama_cpp_2::{
    model::{AddBos, LlamaModel},
    token::LlamaToken,
};
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read, path::Path};

/// Built-in immutable profile catalog and asset validation.
pub struct Profile;
pub(crate) struct ProfileSpec {
    pub id: &'static str,
    pub file: &'static str,
    pub sha256: &'static str,
    pub template_sha256: &'static str,
    pub generation_prompt: &'static str,
}
impl Profile {
    pub(crate) fn cache_mode(id: &str) -> CacheMode {
        if id != policy::PROFILE_ID {
            return CacheMode::FullRebuild;
        }
        #[cfg(target_os = "macos")]
        {
            let device = std::process::Command::new("/usr/sbin/sysctl")
                .args(["-n", "hw.model"])
                .output();
            let os = std::process::Command::new("/usr/bin/sw_vers")
                .arg("-productVersion")
                .output();
            if let (Ok(device), Ok(os)) = (device, os) {
                if device.status.success()
                    && os.status.success()
                    && device.stdout.strip_suffix(b"\n")
                        == Some(policy::PREFIX_REUSE_DEVICE.as_bytes())
                    && os.stdout.strip_suffix(b"\n") == Some(policy::PREFIX_REUSE_OS.as_bytes())
                {
                    return CacheMode::VerifiedPrefixReuse;
                }
            }
        }
        CacheMode::FullRebuild
    }
    pub(crate) fn lookup(id: &str) -> Result<ProfileSpec> {
        match id {
            policy::PROFILE_ID => Ok(ProfileSpec {
                id: policy::PROFILE_ID,
                file: policy::MODEL_FILE,
                sha256: policy::MODEL_SHA256,
                template_sha256: policy::DESKTOP_TEMPLATE_SHA256,
                generation_prompt: "<turn|>\n<|turn>model\n<|channel>thought\n<channel|>",
            }),
            policy::PHONE_PROFILE => Ok(ProfileSpec {
                id: policy::PHONE_PROFILE,
                file: policy::PHONE_FILE,
                sha256: policy::PHONE_SHA256,
                template_sha256: policy::PHONE_TEMPLATE_SHA256,
                generation_prompt: "<turn|>\n<|turn>model\n",
            }),
            _ => Err(failure(ErrorKind::Profile)),
        }
    }
    pub fn validate_config(config: &OpenConfig) -> Result<()> {
        Self::lookup(config.profile.as_str())?;
        if config.system_prompt.len() > policy::SYSTEM_BYTES || config.system_prompt.contains('\0')
        {
            return Err(failure(ErrorKind::Config));
        }
        Ok(())
    }
    pub fn validate_assets(root: &Path) -> Result<()> {
        Self::validate_assets_for(root, &Self::lookup(policy::PROFILE_ID)?)
    }
    pub(crate) fn validate_assets_for(root: &Path, profile: &ProfileSpec) -> Result<()> {
        let mut file =
            File::open(root.join(profile.file)).map_err(|_| failure(ErrorKind::Profile))?;
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
        if format!("{:x}", hash.finalize()) != profile.sha256 {
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
    pub(crate) fn control(model: &LlamaModel, text: &str, first: bool) -> Result<Vec<LlamaToken>> {
        model
            .str_to_token(text, if first { AddBos::Always } else { AddBos::Never })
            .map_err(|_| failure(ErrorKind::Profile))
    }
    /// The pinned safe binding always parses specials. Split before completing '<...>'
    /// so untrusted content cannot introduce a structural token. Decoded text is unchanged.
    pub(crate) fn literal(model: &LlamaModel, text: &str) -> Result<Vec<LlamaToken>> {
        let mut output = Vec::new();
        for piece in text.split_inclusive('<') {
            output.extend(
                model
                    .str_to_token(piece, AddBos::Never)
                    .map_err(|_| failure(ErrorKind::Profile))?,
            );
        }
        Ok(output)
    }
}

pub(crate) const INSTRUCTION: &str = "Answer directly in concise plain English for speech. Do not think aloud. Do not use markdown, tools, URLs, emoji or code. Write numbers, currency, dates, ordinals and units as English words. Preserve conversational references. Never expose control tokens.";

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> OpenConfig {
        OpenConfig {
            assets: "missing".into(),
            profile: ProfileId::desktop(),
            output_language: OutputLanguage::English,
            system_prompt: String::new(),
            seed: 0,
            journal: JournalAccess::Disabled,
        }
    }
    #[test]
    fn unsupported_language_rejected() {
        let bytes = encode(&"ja").unwrap();
        assert!(ciborium::from_reader::<OutputLanguage, _>(bytes.as_slice()).is_err());
    }
    #[test]
    fn invalid_profile_prevents_load() {
        assert!(ProfileId::parse("arbitrary.gguf").is_err());
        assert!(ProfileId::parse("missing").is_err());
    }
    #[test]
    fn oversize_system_prompt_rejected() {
        let mut config = config();
        config.system_prompt = "a".repeat(policy::SYSTEM_BYTES + 1);
        assert_eq!(
            Profile::validate_config(&config).unwrap_err().kind(),
            ErrorKind::Config
        );
    }
    #[test]
    fn single_profile_lifetime() {
        invalid_profile_prevents_load();
    }
    #[test]
    fn direct_answer_template() {
        assert!(INSTRUCTION.contains("Answer directly"));
        assert!(!INSTRUCTION.contains("<|think|>"));
    }
    #[test]
    fn thinking_disabled_template() {
        direct_answer_template();
        let desktop = Profile::lookup(policy::PROFILE_ID).unwrap();
        let phone = Profile::lookup(policy::PHONE_PROFILE).unwrap();
        assert!(
            desktop
                .generation_prompt
                .ends_with("<|channel>thought\n<channel|>")
        );
        assert_eq!(phone.generation_prompt, "<turn|>\n<|turn>model\n");
        assert_ne!(desktop.template_sha256, phone.template_sha256);
    }
    #[test]
    fn qualification_identity() {
        assert!(policy::RUNTIME.contains("=0.1.154"));
        assert_eq!(policy::MODEL_SHA256.len(), 64);
        assert_eq!(
            Profile::cache_mode(policy::PHONE_PROFILE),
            CacheMode::FullRebuild
        );
    }
    #[test]
    fn profile_template_mismatch() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(policy::MODEL_FILE), b"wrong model").unwrap();
        assert!(Profile::validate_assets(root.path()).is_err());
    }
    #[test]
    fn profile_stop_tokens() {
        assert!(include_str!("native.rs").contains("model.is_eog_token(token)"));
        assert!(include_str!("native.rs").contains("<turn|>"));
    }
}
