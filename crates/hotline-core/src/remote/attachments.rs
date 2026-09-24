//! Phone uploads are device-scoped files, never paths chosen by the caller.
use super::{atomic_write, message};
use crate::contract::{Attachment, AttachmentKind, MobileAttachmentChunk};
use base64::{Engine, prelude::BASE64_STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
use uuid::Uuid;

const MAX_FILE: u32 = 10 * 1024 * 1024;
const MAX_CHUNK: usize = 32 * 1024;
// Reserve declared sizes, including partial uploads, so disconnecting cannot
// bypass the limit. Retain accepted files because tapes refer to their paths.
const DEVICE_BUDGET: u64 = 256 * 1024 * 1024;

#[derive(Serialize, Deserialize, PartialEq)]
struct Metadata {
    name: String,
    mime_type: Option<String>,
    size: u32,
}

fn directory(root: &Path, device: &str, id: &str) -> Result<PathBuf, String> {
    let id = Uuid::parse_str(id).map_err(|_| "Invalid attachment id.".to_string())?;
    Ok(root
        .join("remote-attachments")
        .join(device)
        .join(id.to_string()))
}

pub(super) fn upload(
    root: &Path,
    device: &str,
    chunk: &MobileAttachmentChunk,
) -> Result<Value, String> {
    if chunk.name.is_empty()
        || chunk.name.len() > 200
        || chunk.name == "."
        || chunk.name == ".."
        || chunk
            .name
            .chars()
            .any(|c| c.is_control() || c == '/' || c == '\\')
        || chunk
            .mime_type
            .as_ref()
            .is_some_and(|m| m.len() > 128 || m.chars().any(char::is_control))
        || chunk.size > MAX_FILE
        || chunk.data.len() > MAX_CHUNK.div_ceil(3) * 4
    {
        return Err("Choose a file with a valid name, up to 10 MB.".into());
    }
    let bytes = BASE64_STANDARD
        .decode(&chunk.data)
        .map_err(|_| "Invalid attachment data.")?;
    let end = u64::from(chunk.offset) + bytes.len() as u64;
    if bytes.len() > MAX_CHUNK || end > u64::from(chunk.size) {
        return Err("Invalid attachment chunk size.".into());
    }
    let dir = directory(root, device, &chunk.id)?;
    let metadata = Metadata {
        name: chunk.name.clone(),
        mime_type: chunk.mime_type.clone(),
        size: chunk.size,
    };
    let record = dir.join("metadata.json");
    if record.exists() {
        let saved: Metadata =
            serde_json::from_slice(&fs::read(&record).map_err(message)?).map_err(message)?;
        if saved != metadata {
            return Err("This attachment id already belongs to another file.".into());
        }
    } else {
        if chunk.offset != 0 {
            return Err("Start an attachment at offset zero.".into());
        }
        let parent = dir.parent().unwrap();
        fs::create_dir_all(parent).map_err(message)?;
        let mut reserved = 0_u64;
        let mut count = 0;
        for entry in fs::read_dir(parent).map_err(message)? {
            let entry = entry.map_err(message)?;
            if !entry.file_type().map_err(message)?.is_dir() {
                continue;
            }
            count += 1;
            let path = entry.path().join("metadata.json");
            if path.exists() {
                let saved: Metadata =
                    serde_json::from_slice(&fs::read(path).map_err(message)?).map_err(message)?;
                reserved += u64::from(saved.size);
            }
        }
        if reserved + u64::from(chunk.size) > DEVICE_BUDGET || count >= 1024 {
            return Err("This phone's attachment storage is full on the desktop (256 MB).".into());
        }
        fs::create_dir_all(dir.join("payload")).map_err(message)?;
        atomic_write(&record, &serde_json::to_vec(&metadata).map_err(message)?).map_err(message)?;
    }
    let path = dir.join("payload").join(&metadata.name);
    let mut file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(message)?;
    let length = file.metadata().map_err(message)?.len();
    if u64::from(chunk.offset) > length || (u64::from(chunk.offset) < length && end > length) {
        return Err("Attachment chunks must arrive in order.".into());
    }
    file.seek(SeekFrom::Start(u64::from(chunk.offset)))
        .map_err(message)?;
    if end <= length {
        let mut existing = vec![0; bytes.len()];
        file.read_exact(&mut existing).map_err(message)?;
        if existing != bytes {
            return Err("An uploaded attachment cannot change.".into());
        }
    } else {
        file.write_all(&bytes).map_err(message)?;
        file.sync_data().map_err(message)?;
    }
    let offset = length.max(end);
    Ok(json!({"offset":offset, "complete":offset == u64::from(metadata.size)}))
}

pub(super) fn resolve(
    root: &Path,
    device: &str,
    ids: &[String],
) -> Result<Vec<Attachment>, String> {
    if ids.len() > 4 {
        return Err("Attach up to four files per message.".into());
    }
    let mut seen = std::collections::HashSet::new();
    ids.iter()
        .map(|id| {
            let dir = directory(root, device, id)?;
            if !seen.insert(dir.clone()) {
                return Err("An attachment was included twice.".into());
            }
            let metadata: Metadata = serde_json::from_slice(
                &fs::read(dir.join("metadata.json"))
                    .map_err(|_| "Attachment not found for this phone.")?,
            )
            .map_err(message)?;
            let path = dir.join("payload").join(&metadata.name);
            if fs::metadata(&path)
                .map_err(|_| "Attachment upload is incomplete.")?
                .len()
                != u64::from(metadata.size)
            {
                return Err("Attachment upload is incomplete.".into());
            }
            Ok(Attachment {
                kind: if metadata
                    .mime_type
                    .as_ref()
                    .is_some_and(|m| m.starts_with("image/"))
                {
                    AttachmentKind::Image
                } else {
                    AttachmentKind::File
                },
                name: metadata.name,
                path: path.to_string_lossy().into_owned(),
                mime_type: metadata.mime_type,
                size: Some(i64::from(metadata.size)),
                width: None,
                height: None,
                origin: None,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_resume_without_overwriting_and_handles_belong_to_one_phone() {
        let root = tempfile::tempdir().unwrap();
        let mut chunk = MobileAttachmentChunk {
            id: Uuid::new_v4().to_string(),
            name: "notes.txt".into(),
            mime_type: Some("text/plain".into()),
            size: 6,
            offset: 0,
            data: BASE64_STANDARD.encode(b"abc"),
        };
        assert_eq!(
            upload(root.path(), "one", &chunk).unwrap(),
            json!({"offset":3,"complete":false})
        );
        assert!(resolve(root.path(), "one", &[chunk.id.clone()]).is_err());
        assert_eq!(upload(root.path(), "one", &chunk).unwrap()["offset"], 3);
        chunk.offset = 4;
        chunk.data = BASE64_STANDARD.encode(b"ef");
        assert!(upload(root.path(), "one", &chunk).is_err());
        chunk.offset = 3;
        chunk.data = BASE64_STANDARD.encode(b"def");
        assert_eq!(
            upload(root.path(), "one", &chunk).unwrap()["complete"],
            true
        );
        chunk.offset = 0;
        chunk.data = BASE64_STANDARD.encode(b"xyz");
        assert!(upload(root.path(), "one", &chunk).is_err());
        chunk.name = "other.txt".into();
        assert!(upload(root.path(), "one", &chunk).is_err());
        assert!(resolve(root.path(), "two", &[chunk.id.clone()]).is_err());
        let files = resolve(root.path(), "one", &[chunk.id.clone()]).unwrap();
        assert_eq!(fs::read(&files[0].path).unwrap(), b"abcdef");
        assert!(resolve(root.path(), "one", &[chunk.id.clone(), chunk.id]).is_err());
    }

    #[test]
    fn invalid_paths_and_sizes_are_rejected_before_creating_files() {
        let root = tempfile::tempdir().unwrap();
        let mut chunk = MobileAttachmentChunk {
            id: Uuid::new_v4().to_string(),
            name: "../escape.txt".into(),
            mime_type: None,
            size: 0,
            offset: 0,
            data: String::new(),
        };
        assert!(upload(root.path(), "one", &chunk).is_err());
        chunk.name = "empty.txt".into();
        chunk.size = MAX_FILE + 1;
        assert!(upload(root.path(), "one", &chunk).is_err());
        chunk.size = 0;
        chunk.id = "../../outside".into();
        assert!(upload(root.path(), "one", &chunk).is_err());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
        chunk.id = Uuid::new_v4().to_string();
        assert_eq!(
            upload(root.path(), "one", &chunk).unwrap()["complete"],
            true
        );
        assert_eq!(
            resolve(root.path(), "one", &[chunk.id]).unwrap()[0].size,
            Some(0)
        );
    }
}
