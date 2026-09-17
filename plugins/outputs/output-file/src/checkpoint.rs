use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub const SCHEMA_VERSION: u32 = 1;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Cursor {
    pub epoch: String,
    pub next: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Gap {
    pub stream: String,
    pub epoch: String,
    pub from: u64,
    pub to: u64,
    pub reason: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FileIdentity {
    pub volume: u64,
    pub index: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileCheckpoint {
    pub schema_version: u32,
    pub archive_id: String,
    pub config_digest: String,
    pub stream: String,
    pub initial: Cursor,
    pub cursor: Cursor,
    pub file_identity: FileIdentity,
    pub index_identity: FileIdentity,
    pub confirmed_len: u64,
    pub confirmed_sha256: String,
    pub index_len: u64,
    pub index_sha256: String,
    pub gap_count: u64,
    pub incomplete: bool,
}
pub fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}
pub fn hex_hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn digest(hasher: &Sha256) -> String {
    format!("{:x}", hasher.clone().finalize())
}
pub fn prefix_hash(file: &mut File, length: u64) -> Result<Sha256> {
    use std::io::{Seek, SeekFrom};
    if file.metadata()?.len() < length {
        bail!("archive shorter than its confirmed checkpoint");
    }
    file.seek(SeekFrom::Start(0))?;
    let mut remaining = length;
    let mut buffer = [0u8; 65536];
    let mut hasher = Sha256::new();
    while remaining > 0 {
        let n = file.read(&mut buffer[..remaining.min(65536) as usize])?;
        if n == 0 {
            bail!("unexpected EOF while checking archive");
        }
        hasher.update(&buffer[..n]);
        remaining -= n as u64;
    }
    Ok(hasher)
}
#[cfg(unix)]
pub fn identity(file: &File) -> Result<FileIdentity> {
    use std::os::unix::fs::MetadataExt;
    let m = file.metadata()?;
    Ok(FileIdentity {
        volume: m.dev(),
        index: m.ino(),
    })
}
#[cfg(windows)]
pub fn identity(file: &File) -> Result<FileIdentity> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(FileIdentity {
        volume: info.dwVolumeSerialNumber as u64,
        index: ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
    })
}
#[cfg(unix)]
pub fn sync_parent(path: &Path) -> Result<()> {
    File::open(path.parent().context("archive path has no parent")?)?.sync_all()?;
    Ok(())
}
#[cfg(windows)]
pub fn sync_parent(_path: &Path) -> Result<()> {
    Ok(())
}
#[cfg(unix)]
fn replace(from: &Path, to: &Path) -> Result<()> {
    std::fs::rename(from, to)?;
    sync_parent(to)
}
#[cfg(windows)]
fn replace(from: &Path, to: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    if unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}
pub fn save_checkpoint(path: &Path, checkpoint: &FileCheckpoint) -> Result<()> {
    let temp = sibling(path, ".tmp");
    // A stale temporary file is expendable only after its owning checkpoint was authenticated.
    if temp.exists() {
        std::fs::remove_file(&temp).context("remove prior checkpoint temporary file")?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    file.write_all(&serde_json::to_vec(checkpoint)?)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    drop(file);
    replace(&temp, path)
}
