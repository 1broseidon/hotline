//! Operator file access uses server paths. Companions never enter this module.
//! Upload handles belong to one socket; dropping it removes unfinished files.
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fs::OpenOptions,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

const CHUNK: usize = 512 * 1024;
#[derive(Default)]
pub(super) struct Uploads(HashMap<String, Upload>);
struct Upload {
    file: tempfile::NamedTempFile,
    path: PathBuf,
    offset: u64,
}
impl Uploads {
    pub(super) fn start(&mut self, path: &str) -> Result<Value, String> {
        if self.0.len() >= 8 {
            return Err("Finish or cancel an upload before starting another.".into());
        }
        let path = absolute(path)?;
        if path.file_name().is_none() || path.try_exists().map_err(error)? {
            return Err(
                "Choose a new destination file; uploads never replace existing files.".into(),
            );
        }
        let parent = path
            .parent()
            .ok_or("The destination needs a parent folder.")?;
        let file = tempfile::NamedTempFile::new_in(parent).map_err(error)?;
        let id = uuid::Uuid::new_v4().to_string();
        self.0.insert(
            id.clone(),
            Upload {
                file,
                path,
                offset: 0,
            },
        );
        Ok(json!({"uploadId":id,"offset":0}))
    }
    pub(super) fn write(&mut self, id: &str, offset: u64, data: &str) -> Result<Value, String> {
        let upload = self
            .0
            .get_mut(id)
            .ok_or("That upload belongs to another connection or has ended.")?;
        if offset != upload.offset {
            return Err("The upload offset does not match. Restart the upload.".into());
        }
        if data.len() > CHUNK.div_ceil(3) * 4 {
            return Err("Send at most 512 KiB per upload chunk.".into());
        }
        let bytes = STANDARD.decode(data).map_err(|_| "Invalid upload data.")?;
        if bytes.len() > CHUNK || bytes.is_empty() {
            return Err("Send 1–512 KiB per upload chunk.".into());
        }
        if let Err(failure) = upload.file.write_all(&bytes) {
            self.0.remove(id);
            return Err(error(failure));
        }
        upload.offset += bytes.len() as u64;
        Ok(json!({"offset":upload.offset}))
    }
    pub(super) fn finish(&mut self, id: &str) -> Result<Value, String> {
        let upload = self
            .0
            .remove(id)
            .ok_or("That upload belongs to another connection or has ended.")?;
        upload.file.as_file().sync_all().map_err(error)?;
        upload
            .file
            .persist_noclobber(&upload.path)
            .map_err(|e| error(e.error))?;
        Ok(json!({"path":upload.path,"size":upload.offset}))
    }
    pub(super) fn cancel(&mut self, id: &str) -> Result<Value, String> {
        self.0
            .remove(id)
            .ok_or("That upload belongs to another connection or has ended.")?;
        Ok(Value::Null)
    }
}
pub(super) async fn upload(
    command: crate::contract::Command,
    uploads: std::sync::Arc<std::sync::Mutex<Uploads>>,
    cancel: tokio_util::sync::CancellationToken,
) -> Result<Value, String> {
    // File writes and fsync must not block a wire executor thread.
    tokio::task::spawn_blocking(move || {
        let mut uploads = uploads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if cancel.is_cancelled() {
            return Err("This connection has closed.".into());
        }
        use crate::contract::Command;
        match command {
            Command::FilesUploadStart { path } => uploads.start(&path),
            Command::FilesUploadChunk {
                upload_id,
                offset,
                data,
            } => uploads.write(&upload_id, offset, &data),
            Command::FilesUploadFinish { upload_id } => uploads.finish(&upload_id),
            Command::FilesUploadCancel { upload_id } => uploads.cancel(&upload_id),
            _ => Err("Not an upload command.".into()),
        }
    })
    .await
    .map_err(|_| "The upload stopped.".to_string())?
}

fn absolute(path: &str) -> Result<PathBuf, String> {
    let path = Path::new(path);
    if !path.is_absolute() {
        return Err("Choose an absolute path on the server.".into());
    }
    Ok(path.to_owned())
}
fn error(error: impl std::fmt::Display) -> String {
    format!("The server could not access that file: {error}")
}

pub(super) fn browse(path: &str) -> Result<Value, String> {
    let path = absolute(path)?.canonicalize().map_err(error)?;
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(&path).map_err(error)?.take(10001) {
        if entries.len() == 10000 {
            return Err("This folder has too many entries to browse. Choose a subfolder.".into());
        }
        let entry = entry.map_err(error)?;
        let entry_path = entry.path();
        let entry_path = entry_path
            .to_str()
            .ok_or("This folder contains a filename that cannot be represented as text.")?;
        let metadata = std::fs::metadata(entry_path)
            .or_else(|_| entry.metadata())
            .map_err(error)?;
        entries.push(json!({"name":entry.file_name().to_string_lossy(),"path":entry_path,"directory":metadata.is_dir(),"size":metadata.len()}));
    }
    entries.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    let parent = path
        .parent()
        .map(|parent| {
            parent
                .to_str()
                .ok_or("The parent path cannot be represented as text.")
        })
        .transpose()?;
    let path = path
        .to_str()
        .ok_or("This path cannot be represented as text.")?;
    Ok(json!({"path":path,"parent":parent,"entries":entries}))
}
pub(super) fn mkdir(path: &str) -> Result<Value, String> {
    let path = absolute(path)?;
    std::fs::create_dir(&path).map_err(error)?;
    Ok(json!({"path":path}))
}

pub(super) fn download(path: &str, offset: u64) -> Result<Value, String> {
    let path = absolute(path)?;
    if !std::fs::metadata(&path).map_err(error)?.is_file() {
        return Err("Choose a regular file.".into());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let mut file = options.open(&path).map_err(error)?;
    let metadata = file.metadata().map_err(error)?;
    if !metadata.is_file() || offset > metadata.len() {
        return Err("Choose a regular file and a valid byte offset.".into());
    }
    file.seek(SeekFrom::Start(offset)).map_err(error)?;
    let expected = (metadata.len() - offset).min(CHUNK as u64);
    let mut bytes = Vec::new();
    file.take(expected).read_to_end(&mut bytes).map_err(error)?;
    if bytes.len() as u64 != expected {
        return Err(
            "The file changed while it was being downloaded. Start the download again.".into(),
        );
    }
    let next = offset + bytes.len() as u64;
    Ok(
        json!({"name":path.file_name().unwrap_or_default().to_string_lossy(),"mimeType":mime_guess::from_path(&path).first_or_octet_stream().to_string(),"size":metadata.len(),"offset":offset,"data":STANDARD.encode(bytes),"next":if next < metadata.len() {Some(next)} else {None}}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downloads_cross_chunk_boundaries_and_uploads_refuse_oversized_chunks() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let bytes = vec![37; CHUNK + 17];
        std::fs::write(&source, &bytes).unwrap();
        let first = download(source.to_str().unwrap(), 0).unwrap();
        assert_eq!(first["next"], CHUNK);
        let second = download(source.to_str().unwrap(), CHUNK as u64).unwrap();
        assert!(second["next"].is_null());
        let joined = [
            STANDARD.decode(first["data"].as_str().unwrap()).unwrap(),
            STANDARD.decode(second["data"].as_str().unwrap()).unwrap(),
        ]
        .concat();
        assert_eq!(joined, bytes);
        assert!(download(root.path().to_str().unwrap(), 0).is_err());
        assert!(download(source.to_str().unwrap(), (bytes.len() + 1) as u64).is_err());
        let destination = root.path().join("destination");
        let mut uploads = Uploads::default();
        let started = uploads.start(destination.to_str().unwrap()).unwrap();
        let id = started["uploadId"].as_str().unwrap();
        assert!(uploads.write(id, 0, &STANDARD.encode(&bytes)).is_err());
        uploads
            .write(id, 0, first["data"].as_str().unwrap())
            .unwrap();
        uploads
            .write(id, CHUNK as u64, second["data"].as_str().unwrap())
            .unwrap();
        uploads.finish(id).unwrap();
        assert_eq!(std::fs::read(destination).unwrap(), bytes);
        let child = root.path().join("child");
        mkdir(child.to_str().unwrap()).unwrap();
        assert!(child.is_dir());
        assert!(mkdir(child.to_str().unwrap()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_non_utf8_filename_is_an_error_instead_of_a_wire_panic() {
        use std::os::unix::ffi::OsStringExt;
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join(std::ffi::OsString::from_vec(vec![255])),
            b"test",
        )
        .unwrap();
        assert!(
            browse(root.path().to_str().unwrap())
                .unwrap_err()
                .contains("represented as text")
        );
    }
}
