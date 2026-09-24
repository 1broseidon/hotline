//! The desk's hands on a file a teammate sent (BRO-98): open a PDF in the
//! system's viewer, or save a copy where the person chooses.
//!
//! Both take the path the conversation names, and both refuse anything that
//! is not one of the desk's own kept copies, under `files/` in the data
//! directory. The page asks; the shell decides what it will touch. Only a
//! PDF or a picture is ever opened, and only when its name and its first
//! bytes agree: the system picks the program by the name, so the bytes
//! alone would let a script that starts like a PDF run.

use hotline_core::paths::{data_root, sent_files_dir};
use std::io::Read;
use std::path::{Path, PathBuf};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

/// The kept copy at `path`, or why the shell will not touch it.
fn kept(path: &str) -> Result<PathBuf, String> {
    kept_under(&sent_files_dir(&data_root()), path)
}

/// `path` as a file under `files`, once every link in it is followed.
fn kept_under(files: &Path, path: &str) -> Result<PathBuf, String> {
    let files = files
        .canonicalize()
        .map_err(|_| "That is not a file a teammate sent.".to_string())?;
    let path = Path::new(path)
        .canonicalize()
        .map_err(|_| "That file is no longer on this computer.".to_string())?;
    if !path.starts_with(&files) || !path.is_file() {
        return Err("That is not a file a teammate sent.".to_string());
    }
    Ok(path)
}

/// Whether the system may be asked to open this: a PDF named `.pdf`, or a
/// JPEG named `.jpg` or `.jpeg`, which is every picture the desk keeps.
fn openable(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let mut head = [0u8; 5];
    if std::fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut head))
        .is_err()
    {
        return false;
    }
    (name.ends_with(".pdf") && &head == b"%PDF-")
        || ((name.ends_with(".jpg") || name.ends_with(".jpeg")) && head[..3] == [0xFF, 0xD8, 0xFF])
}

/// Opens a PDF or a picture a teammate sent in the system's own viewer.
#[tauri::command]
pub fn open_sent_file(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let path = kept(&path)?;
    if !openable(&path) {
        return Err(
            "Only a PDF or a picture opens from the conversation; save anything else.".to_string(),
        );
    }
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|error| format!("The file could not be opened: {error}"))
}

/// Saves a copy of a file a teammate sent where the person chooses, in the
/// system's own save dialog. Answers where the copy went, or nothing when
/// the dialog was dismissed.
#[tauri::command]
pub async fn save_sent_file(app: tauri::AppHandle, path: String) -> Result<Option<String>, String> {
    let path = kept(&path)?;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    tauri::async_runtime::spawn_blocking(move || {
        let Some(chosen) = app.dialog().file().set_file_name(name).blocking_save_file() else {
            return Ok(None);
        };
        let target = chosen
            .into_path()
            .map_err(|error| format!("That place cannot be saved to: {error}"))?;
        std::fs::copy(&path, &target)
            .map_err(|error| format!("The copy could not be written: {error}"))?;
        Ok(Some(target.display().to_string()))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("hotline-app-files-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("files/ada/e1")).unwrap();
        root
    }

    #[test]
    fn only_a_kept_copy_is_touched() {
        let root = scratch("kept");
        let files = root.join("files");
        let report = files.join("ada/e1/report.pdf");
        std::fs::write(&report, b"%PDF-1.4\n").unwrap();
        std::fs::write(root.join("secret.txt"), b"no").unwrap();

        assert_eq!(
            kept_under(&files, &report.to_string_lossy()).unwrap(),
            report.canonicalize().unwrap()
        );
        let refused = "That is not a file a teammate sent.";
        let escape = files.join("ada/e1/../../../secret.txt");
        assert_eq!(
            kept_under(&files, &escape.to_string_lossy()).unwrap_err(),
            refused
        );
        assert_eq!(
            kept_under(&files, &files.join("ada/e1").to_string_lossy()).unwrap_err(),
            refused
        );
        assert_eq!(
            kept_under(&files, &files.join("ada/e1/gone.pdf").to_string_lossy()).unwrap_err(),
            "That file is no longer on this computer."
        );
        #[cfg(unix)]
        {
            let link = files.join("ada/e1/link.txt");
            std::os::unix::fs::symlink(root.join("secret.txt"), &link).unwrap();
            assert_eq!(
                kept_under(&files, &link.to_string_lossy()).unwrap_err(),
                refused
            );
        }
    }

    #[test]
    fn only_a_pdf_or_a_picture_opens_and_only_when_name_and_bytes_agree() {
        let root = scratch("open");
        let at = |name: &str, bytes: &[u8]| {
            let path = root.join("files/ada/e1").join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        };
        assert!(openable(&at("report.pdf", b"%PDF-1.7\n%%EOF\n")));
        assert!(openable(&at("Q3.PDF", b"%PDF-1.7\n")));
        assert!(openable(&at("chart.jpg", b"\xFF\xD8\xFF\xE0\x00\x10JFIF")));
        // A script named a PDF, and a PDF named a script.
        assert!(!openable(&at("run.pdf", b"#!/bin/sh\necho hi\n")));
        assert!(!openable(&at("run.bat", b"%PDF-1.7\r\ncalc.exe\r\n")));
        assert!(!openable(&at(
            "run.command",
            b"%PDF-1.7\nopen -a Calculator\n"
        )));
        assert!(!openable(&at("chart.png.jpg", b"\x89PNG\r\n\x1a\n")));
        assert!(!openable(&at("tiny.pdf", b"%PD")));
        assert!(!openable(&root.join("files/ada/e1/absent.pdf")));
    }
}
