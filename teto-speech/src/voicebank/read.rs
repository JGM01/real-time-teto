//! Bounded local archive and directory reads.
use crate::{
    error::{Result, SpeechError},
    limits::*,
};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

pub(super) enum BankReader {
    Directory(PathBuf, Vec<(String, u64)>),
    Archive(zip::ZipArchive<File>, Vec<(String, usize, u64)>, [u8; 32]),
}
impl BankReader {
    pub fn open(path: &Path) -> Result<Self> {
        if fs::symlink_metadata(path)?.file_type().is_symlink() {
            return Err(SpeechError::VoicebankInvalid);
        }
        if path.is_dir() {
            let mut entries = Vec::new();
            walk(path, path, &mut entries, &mut 0)?;
            entries.sort();
            return Ok(Self::Directory(path.canonicalize()?, entries));
        }
        let mut file = File::open(path)?;
        if file.metadata()?.len() > MAX_ARCHIVE_BYTES {
            return Err(SpeechError::Bounds);
        }
        let mut hash = Sha256::new();
        let mut buffer = [0; 65536];
        loop {
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hash.update(&buffer[..n]);
        }
        file.seek(SeekFrom::Start(0))?;
        preflight(&mut file)?;
        let mut zip = zip::ZipArchive::new(file).map_err(|_| SpeechError::VoicebankInvalid)?;
        if zip.len() > MAX_ENTRIES {
            return Err(SpeechError::Bounds);
        }
        let mut entries = Vec::with_capacity(zip.len());
        for i in 0..zip.len() {
            let entry = zip.by_index(i).map_err(|_| SpeechError::VoicebankInvalid)?;
            if entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
                return Err(SpeechError::VoicebankInvalid);
            }
            let name = decode_name(entry.name_raw())?;
            let name = safe_path(&name)?;
            if !entry.is_dir() {
                entries.push((name, i, entry.size()));
            }
        }
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        if entries.windows(2).any(|p| p[0].0 == p[1].0) {
            return Err(SpeechError::VoicebankInvalid);
        }
        Ok(Self::Archive(zip, entries, hash.finalize().into()))
    }
    pub fn entries(&self) -> Vec<(String, u64)> {
        match self {
            Self::Directory(_, entries) => entries.clone(),
            Self::Archive(_, entries, _) => {
                entries.iter().map(|(n, _, s)| (n.clone(), *s)).collect()
            }
        }
    }
    pub fn digest(&self) -> Option<[u8; 32]> {
        match self {
            Self::Archive(_, _, hash) => Some(*hash),
            _ => None,
        }
    }
    pub fn reader(&mut self, name: &str) -> Result<EntryReader<'_>> {
        match self {
            Self::Directory(root, _) => {
                let path = root.join(safe_path(name)?);
                reject_symlink_components(root, &path)?;
                Ok(EntryReader::File(File::open(path)?))
            }
            Self::Archive(zip, entries, _) => {
                let index = entries
                    .binary_search_by(|e| e.0.as_str().cmp(name))
                    .map_err(|_| SpeechError::VoicebankInvalid)?;
                Ok(EntryReader::Zip(
                    zip.by_index(entries[index].1)
                        .map_err(|_| SpeechError::VoicebankInvalid)?,
                ))
            }
        }
    }
    pub fn bytes(&mut self, name: &str, limit: usize) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        self.reader(name)?
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > limit {
            return Err(SpeechError::Bounds);
        }
        Ok(bytes)
    }
}
#[allow(clippy::large_enum_variant)]
pub(super) enum EntryReader<'a> {
    File(File),
    Zip(zip::read::ZipFile<'a, File>),
}
impl Read for EntryReader<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::File(f) => f.read(bytes),
            Self::Zip(z) => z.read(bytes),
        }
    }
}
pub(super) fn safe_path(value: &str) -> Result<String> {
    let path = value.replace('\\', "/");
    if path.len() > MAX_PATH_BYTES
        || path.starts_with('/')
        || path.contains(['\0', ':'])
        || path.split('/').any(|p| p == "..")
    {
        return Err(SpeechError::VoicebankInvalid);
    }
    let path = path
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect::<Vec<_>>()
        .join("/");
    if path.is_empty() {
        return Err(SpeechError::VoicebankInvalid);
    }
    Ok(path)
}
fn reject_symlink_components(root: &Path, path: &Path) -> Result<()> {
    let mut cursor = root.to_path_buf();
    for component in path
        .strip_prefix(root)
        .map_err(|_| SpeechError::VoicebankInvalid)?
        .components()
    {
        cursor.push(component);
        if fs::symlink_metadata(&cursor)?.file_type().is_symlink() {
            return Err(SpeechError::VoicebankInvalid);
        }
    }
    Ok(())
}
fn walk(
    root: &Path,
    path: &Path,
    entries: &mut Vec<(String, u64)>,
    visited: &mut usize,
) -> Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        *visited += 1;
        if *visited > MAX_ENTRIES {
            return Err(SpeechError::Bounds);
        }
        let meta = fs::symlink_metadata(entry.path())?;
        if meta.file_type().is_symlink() {
            return Err(SpeechError::VoicebankInvalid);
        }
        let relative = entry
            .path()
            .strip_prefix(root)
            .map_err(|_| SpeechError::VoicebankInvalid)?
            .to_string_lossy()
            .into_owned();
        safe_path(&relative)?;
        if relative.split('/').count() > 16 || entries.len() >= MAX_ENTRIES {
            return Err(SpeechError::Bounds);
        }
        if meta.is_dir() {
            walk(root, &entry.path(), entries, visited)?;
        } else {
            entries.push((relative, meta.len()));
        }
    }
    Ok(())
}
fn decode_name(bytes: &[u8]) -> Result<String> {
    if let Ok(value) = std::str::from_utf8(bytes) {
        return Ok(value.to_owned());
    }
    let (text, _, bad) = encoding_rs::SHIFT_JIS.decode(bytes);
    if bad {
        return Err(SpeechError::VoicebankInvalid);
    }
    Ok(text.into_owned())
}

fn preflight(file: &mut File) -> Result<()> {
    let length = file.metadata()?.len();
    let tail_length = length.min(65557);
    file.seek(SeekFrom::End(-(tail_length as i64)))?;
    let mut tail = vec![0u8; tail_length as usize];
    file.read_exact(&mut tail)?;
    let start = tail
        .windows(4)
        .rposition(|b| b == b"PK\x05\x06")
        .ok_or(SpeechError::VoicebankInvalid)?;
    let record = tail
        .get(start..start + 22)
        .ok_or(SpeechError::VoicebankInvalid)?;
    let field = |i| u16::from_le_bytes([record[i], record[i + 1]]);
    if field(4) != 0 || field(6) != 0 || field(8) != field(10) || field(10) as usize > MAX_ENTRIES {
        return Err(SpeechError::Bounds);
    }
    let size = u32::from_le_bytes(
        record[12..16]
            .try_into()
            .map_err(|_| SpeechError::VoicebankInvalid)?,
    ) as u64;
    let offset = u32::from_le_bytes(
        record[16..20]
            .try_into()
            .map_err(|_| SpeechError::VoicebankInvalid)?,
    ) as u64;
    if size > MAX_OTO_BYTES as u64
        || offset
            .checked_add(size)
            .is_none_or(|n| n > length - tail_length + start as u64)
        || start + 22 + field(20) as usize != tail.len()
        || (start >= 20 && &tail[start - 20..start - 16] == b"PK\x06\x07")
    {
        return Err(SpeechError::VoicebankInvalid);
    }
    file.seek(SeekFrom::Start(0))?;
    Ok(())
}
