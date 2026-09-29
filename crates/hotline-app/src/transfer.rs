//! Moving files between this computer and a desk on a server (BRO-145).
//!
//! The page does the talking to the desk, over its bridge, in chunks; the
//! shell only touches this computer's disk, and only where the person said:
//!
//! - Up: the page may read a local file only if the person picked it in the
//!   shell's own dialog or dropped it on the window. Anything else is
//!   refused, so a page that is talked into asking cannot read the disk.
//! - Down: bytes land in a file the person chose in the save dialog, or, to
//!   open a PDF or a picture, in a fresh private temporary directory, and the
//!   system is asked to open it only under the same rules as a kept copy.

use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

/// What the desk takes in one upload chunk.
const CHUNK: u64 = 512 * 1024;

#[derive(Default)]
pub struct Transfers {
    /// Local files the person handed over, by picker or by drop.
    handed: Mutex<HashSet<PathBuf>>,
    /// Downloads being written, by id.
    writing: Mutex<HashMap<String, Download>>,
}

struct Download {
    file: File,
    path: PathBuf,
    /// Opened in the system viewer when finished, rather than kept.
    open: bool,
    /// The private directory an opened download lives in.
    scratch: Option<tempfile::TempDir>,
}

impl Transfers {
    /// Paths the person dropped on the window.
    pub fn hand(&self, paths: &[PathBuf]) {
        let mut handed = self.handed.lock().unwrap();
        for path in paths {
            if let Ok(path) = path.canonicalize() {
                handed.insert(path);
            }
        }
    }

    fn handed(&self, path: &str) -> Result<PathBuf, String> {
        let path = Path::new(path)
            .canonicalize()
            .map_err(|_| "That file is no longer on this computer.".to_string())?;
        if !self.handed.lock().unwrap().contains(&path) || !path.is_file() {
            return Err("Pick or drop a file to send it.".to_string());
        }
        Ok(path)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Picked {
    path: String,
    size: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalChunk {
    /// Base64, at most one upload chunk.
    data: String,
    size: u64,
    /// Where the next chunk starts, or none at the end.
    next: Option<u64>,
}

/// The shell's own file picker, so what it returns is what may be read.
#[tauri::command]
pub async fn transfer_pick(
    app: tauri::AppHandle,
    transfers: tauri::State<'_, std::sync::Arc<Transfers>>,
) -> Result<Vec<Picked>, String> {
    let chosen =
        tauri::async_runtime::spawn_blocking(move || app.dialog().file().blocking_pick_files())
            .await
            .map_err(|error| error.to_string())?
            .unwrap_or_default();
    let paths: Vec<PathBuf> = chosen
        .into_iter()
        .filter_map(|path| path.into_path().ok())
        .collect();
    transfers.hand(&paths);
    Ok(paths
        .iter()
        .filter_map(|path| {
            let size = std::fs::metadata(path).ok()?.len();
            Some(Picked {
                path: path.display().to_string(),
                size,
            })
        })
        .collect())
}

/// One chunk of a file the person handed over.
#[tauri::command]
pub fn transfer_read(
    transfers: tauri::State<'_, std::sync::Arc<Transfers>>,
    path: String,
    offset: u64,
) -> Result<LocalChunk, String> {
    read_chunk(&transfers.handed(&path)?, offset)
}

fn read_chunk(path: &Path, offset: u64) -> Result<LocalChunk, String> {
    let unreadable = |error: std::io::Error| format!("That file could not be read: {error}");
    let mut file = File::open(path).map_err(unreadable)?;
    let size = file.metadata().map_err(unreadable)?.len();
    file.seek(SeekFrom::Start(offset.min(size)))
        .map_err(unreadable)?;
    let mut bytes = Vec::with_capacity(CHUNK.min(size.saturating_sub(offset)) as usize);
    file.take(CHUNK)
        .read_to_end(&mut bytes)
        .map_err(unreadable)?;
    let end = offset + bytes.len() as u64;
    Ok(LocalChunk {
        data: STANDARD.encode(&bytes),
        size,
        next: (end < size && !bytes.is_empty()).then_some(end),
    })
}

/// Starts a download: asks where to save `name`, or makes a private place to
/// open it from. Answers the download's id, or none when the dialog was dismissed.
#[tauri::command]
pub async fn transfer_begin(
    app: tauri::AppHandle,
    transfers: tauri::State<'_, std::sync::Arc<Transfers>>,
    name: String,
    open: bool,
) -> Result<Option<String>, String> {
    let name = plain_name(&name);
    let (path, scratch) = if open {
        let scratch = tempfile::Builder::new()
            .prefix("hotline-open-")
            .tempdir()
            .map_err(|error| format!("There is no room to open it: {error}"))?;
        (scratch.path().join(&name), Some(scratch))
    } else {
        let chosen = tauri::async_runtime::spawn_blocking(move || {
            app.dialog().file().set_file_name(name).blocking_save_file()
        })
        .await
        .map_err(|error| error.to_string())?;
        let Some(chosen) = chosen else {
            return Ok(None);
        };
        let path = chosen
            .into_path()
            .map_err(|error| format!("That place cannot be saved to: {error}"))?;
        (path, None)
    };
    let file =
        File::create(&path).map_err(|error| format!("The copy could not be written: {error}"))?;
    let id = format!("{:032x}", rand::random::<u128>());
    transfers.writing.lock().unwrap().insert(
        id.clone(),
        Download {
            file,
            path,
            open,
            scratch,
        },
    );
    Ok(Some(id))
}

#[tauri::command]
pub fn transfer_write(
    transfers: tauri::State<'_, std::sync::Arc<Transfers>>,
    id: String,
    data: String,
) -> Result<(), String> {
    let bytes = STANDARD
        .decode(data)
        .map_err(|_| "The desk sent a damaged chunk.".to_string())?;
    let mut writing = transfers.writing.lock().unwrap();
    let download = writing.get_mut(&id).ok_or("That download has ended.")?;
    download
        .file
        .write_all(&bytes)
        .map_err(|error| format!("The copy could not be written: {error}"))
}

/// Ends a download. Finished, it answers where the copy is, and opens it if
/// that was the point; abandoned, the partial copy is removed.
#[tauri::command]
pub fn transfer_end(
    app: tauri::AppHandle,
    transfers: tauri::State<'_, std::sync::Arc<Transfers>>,
    id: String,
    finished: bool,
) -> Result<Option<String>, String> {
    let Some(mut download) = transfers.writing.lock().unwrap().remove(&id) else {
        return Ok(None);
    };
    let flushed = download.file.flush();
    drop(download.file);
    if !finished || flushed.is_err() {
        let _ = std::fs::remove_file(&download.path);
        flushed.map_err(|error| format!("The copy could not be written: {error}"))?;
        return Ok(None);
    }
    if download.open {
        if !crate::files::openable(&download.path) {
            return Err(
                "Only a PDF or a picture opens from the conversation; save anything else."
                    .to_string(),
            );
        }
        app.opener()
            .open_path(download.path.to_string_lossy(), None::<&str>)
            .map_err(|error| format!("The file could not be opened: {error}"))?;
        // The viewer reads it after this returns; the directory stays for
        // the session and the system's temp cleaning takes it after.
        if let Some(scratch) = download.scratch.take() {
            let _ = scratch.keep();
        }
    }
    Ok(Some(download.path.display().to_string()))
}

/// A name from the desk as a single path component, never a path.
fn plain_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or_default().trim();
    if base.is_empty() || base == "." || base == ".." {
        "download".to_string()
    } else {
        base.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_handed_over_file_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let picked = dir.path().join("picked.txt");
        let other = dir.path().join("other.txt");
        std::fs::write(&picked, b"hello").unwrap();
        std::fs::write(&other, b"secret").unwrap();
        let transfers = Transfers::default();
        transfers.hand(std::slice::from_ref(&picked));
        assert!(transfers.handed(&picked.to_string_lossy()).is_ok());
        assert_eq!(
            transfers.handed(&other.to_string_lossy()).unwrap_err(),
            "Pick or drop a file to send it."
        );
        let sideways = dir.path().join("x/../other.txt");
        assert!(transfers.handed(&sideways.to_string_lossy()).is_err());
        assert!(transfers.handed(&dir.path().to_string_lossy()).is_err());
    }

    #[test]
    fn a_file_reads_in_chunks_to_its_end() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.bin");
        let bytes: Vec<u8> = (0..(CHUNK + 10)).map(|at| at as u8).collect();
        std::fs::write(&path, &bytes).unwrap();
        let first = read_chunk(&path, 0).unwrap();
        assert_eq!(first.size, CHUNK + 10);
        assert_eq!(first.next, Some(CHUNK));
        let second = read_chunk(&path, CHUNK).unwrap();
        assert_eq!(second.next, None);
        let mut joined = STANDARD.decode(first.data).unwrap();
        joined.extend(STANDARD.decode(second.data).unwrap());
        assert_eq!(joined, bytes);

        let empty = dir.path().join("empty");
        std::fs::write(&empty, b"").unwrap();
        let only = read_chunk(&empty, 0).unwrap();
        assert_eq!((only.data.as_str(), only.next), ("", None));
    }

    #[test]
    fn a_name_from_the_desk_is_never_a_path() {
        assert_eq!(plain_name("report.pdf"), "report.pdf");
        assert_eq!(plain_name("../../.ssh/authorized_keys"), "authorized_keys");
        assert_eq!(plain_name("C:\\Windows\\evil.exe"), "evil.exe");
        assert_eq!(plain_name(".."), "download");
        assert_eq!(plain_name("dir/"), "download");
    }
}
