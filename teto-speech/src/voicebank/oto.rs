//! Teto English OTO import seam.
use std::path::Path;

use crate::{error::Result, types::VoicebankManifest, voicebank::OtoImport};

/// Import the Teto English OTO and PCM files.
pub struct TetoOtoImport;

impl OtoImport for TetoOtoImport {
    fn load(&mut self, _path: &Path) -> Result<VoicebankManifest> {
        todo!("import OTO rows and PCM files")
    }
}
