//! Bounded local archive and directory reads.
use crate::{
    error::{Result, SpeechError},
    voicebank::limits::*,
};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

/// One normalized relative path.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) struct SafePath(String);

impl SafePath {
    /// Parse one archive path.
    pub fn from_archive(value: &str) -> Result<Self> {
        let value = value.replace('\\', "/");
        Self::parse(&value)
    }

    /// Parse one filesystem path.
    pub fn from_filesystem(value: &str) -> Result<Self> {
        Self::parse(value)
    }

    fn parse(value: &str) -> Result<Self> {
        if value.is_empty()
            || value.len() > MAX_PATH_BYTES
            || value.starts_with('/')
            || value.contains(['\0', ':'])
        {
            return Err(SpeechError::VoicebankInvalid);
        }
        let mut parts = Vec::new();
        for part in value.split('/') {
            if part.is_empty() || part == "." {
                continue;
            }
            if part == ".." {
                return Err(SpeechError::VoicebankInvalid);
            }
            parts.push(part);
        }
        if parts.is_empty() {
            return Err(SpeechError::VoicebankInvalid);
        }
        let path = parts.join("/");
        if path.len() > MAX_PATH_BYTES {
            return Err(SpeechError::VoicebankInvalid);
        }
        Ok(Self(path))
    }

    /// Return the path text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One archive or directory entry.
#[derive(Clone)]
pub(super) struct Entry {
    pub path: SafePath,
    pub index: Option<usize>,
    pub size: u64,
}

pub(super) enum BankReader {
    Directory(PathBuf, Vec<Entry>),
    Archive(zip::ZipArchive<File>, Vec<Entry>, [u8; 32]),
}

impl BankReader {
    pub fn open(path: &Path) -> Result<Self> {
        if fs::symlink_metadata(path)?.file_type().is_symlink() {
            return Err(SpeechError::VoicebankInvalid);
        }
        if path.is_dir() {
            let mut entries = Vec::new();
            walk(path, path, &mut entries, &mut 0)?;
            entries.sort_by(|a, b| a.path.cmp(&b.path));
            return Ok(Self::Directory(path.canonicalize()?, entries));
        }
        let mut file = File::open(path)?;
        if file.metadata()?.len() > MAX_ARCHIVE_BYTES {
            return Err(SpeechError::LimitExceeded);
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
            return Err(SpeechError::LimitExceeded);
        }
        let mut entries = Vec::with_capacity(zip.len());
        for index in 0..zip.len() {
            let entry = zip
                .by_index(index)
                .map_err(|_| SpeechError::VoicebankInvalid)?;
            if entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
                return Err(SpeechError::VoicebankInvalid);
            }
            let name = decode_name(entry.name_raw())?;
            let path = SafePath::from_archive(&name)?;
            if !entry.is_dir() {
                entries.push(Entry {
                    path,
                    index: Some(index),
                    size: entry.size(),
                });
            }
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        if entries.windows(2).any(|pair| pair[0].path == pair[1].path) {
            return Err(SpeechError::VoicebankInvalid);
        }
        Ok(Self::Archive(zip, entries, hash.finalize().into()))
    }

    pub fn entries(&self) -> &[Entry] {
        match self {
            Self::Directory(_, entries) | Self::Archive(_, entries, _) => entries,
        }
    }

    pub fn digest(&self) -> Option<[u8; 32]> {
        match self {
            Self::Archive(_, _, hash) => Some(*hash),
            _ => None,
        }
    }

    pub fn reader(&mut self, name: &SafePath) -> Result<EntryReader<'_>> {
        match self {
            Self::Directory(root, _) => {
                let path = root.join(name.as_str());
                reject_symlink_components(root, &path)?;
                Ok(EntryReader::File(File::open(path)?))
            }
            Self::Archive(zip, entries, _) => {
                let index = entries
                    .binary_search_by(|entry| entry.path.as_str().cmp(name.as_str()))
                    .map_err(|_| SpeechError::VoicebankInvalid)?;
                let index = entries[index].index.ok_or(SpeechError::VoicebankInvalid)?;
                Ok(EntryReader::Zip(
                    zip.by_index(index)
                        .map_err(|_| SpeechError::VoicebankInvalid)?,
                ))
            }
        }
    }

    pub fn bytes(&mut self, name: &SafePath, limit: usize) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        self.reader(name)?
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > limit {
            return Err(SpeechError::LimitExceeded);
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
            Self::File(file) => file.read(bytes),
            Self::Zip(file) => file.read(bytes),
        }
    }
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

fn walk(root: &Path, path: &Path, entries: &mut Vec<Entry>, visited: &mut usize) -> Result<()> {
    let mut stack = vec![path.to_path_buf()];
    while let Some(directory) = stack.pop() {
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            *visited += 1;
            if *visited > MAX_ENTRIES {
                return Err(SpeechError::LimitExceeded);
            }
            let meta = fs::symlink_metadata(entry.path())?;
            if meta.file_type().is_symlink() {
                return Err(SpeechError::VoicebankInvalid);
            }
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .map_err(|_| SpeechError::VoicebankInvalid)?
                .to_str()
                .ok_or(SpeechError::VoicebankInvalid)?;
            let relative = SafePath::from_filesystem(relative)?;
            if relative.as_str().split('/').count() > 16 || entries.len() >= MAX_ENTRIES {
                return Err(SpeechError::LimitExceeded);
            }
            if meta.is_dir() {
                stack.push(entry.path());
            } else {
                entries.push(Entry {
                    path: relative,
                    index: None,
                    size: meta.len(),
                });
            }
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
        .rposition(|bytes| bytes == b"PK\x05\x06")
        .ok_or(SpeechError::VoicebankInvalid)?;
    let record = tail
        .get(start..start + 22)
        .ok_or(SpeechError::VoicebankInvalid)?;
    let field = |index| u16::from_le_bytes([record[index], record[index + 1]]);
    if field(4) != 0 || field(6) != 0 || field(8) != field(10) || field(10) as usize > MAX_ENTRIES {
        return Err(SpeechError::LimitExceeded);
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
            .is_none_or(|end| end > length - tail_length + start as u64)
        || start + 22 + field(20) as usize != tail.len()
        || (start >= 20 && &tail[start - 20..start - 16] == b"PK\x06\x07")
    {
        return Err(SpeechError::VoicebankInvalid);
    }
    file.seek(SeekFrom::Start(0))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::SafePath;

    #[test]
    fn safe_path_normalizes_archive_names_and_rejects_parents() {
        assert_eq!(
            SafePath::from_archive("./a\\b//c").unwrap().as_str(),
            "a/b/c"
        );
        assert!(SafePath::from_archive("../a").is_err());
        assert!(SafePath::from_archive("/a").is_err());
        assert!(SafePath::from_filesystem("a\\b").unwrap().as_str() == "a\\b");
    }
}
