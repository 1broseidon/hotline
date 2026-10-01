//! Fetching an agent the ACP registry ships as a prebuilt archive.
//!
//! The archive is downloaded and unpacked into a scratch folder beside its
//! final one, then renamed into place, so a folder that exists is a finished
//! install: an interrupted download leaves only scratch, and two sessions
//! starting the same agent at once both end up with the one that won the
//! rename. Each release has its own folder, and older ones are removed once
//! the new one is in place.

use super::registry::Archive;
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long the connection may take to open, and how long it may then go
/// without a byte. There is no limit on the whole download: archives run to a
/// few hundred megabytes, and a slow line is not a broken one.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const READ_TIMEOUT: Duration = Duration::from_secs(60);

const SCRATCH: &str = ".fetch-";

/// Downloads `archive`, checks it against its published digest, and unpacks it
/// into `archive.dir`, where `command` must then be.
pub(super) async fn fetch(archive: &Archive, command: &Path) -> Result<(), String> {
    let parent = archive
        .dir
        .parent()
        .ok_or("the install folder has no parent")?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let scratch = tempfile::Builder::new()
        .prefix(SCRATCH)
        .tempdir_in(parent)
        .map_err(|error| error.to_string())?;
    let download = scratch.path().join("download");
    let digest = download_to(&archive.url, &download).await?;
    if let Some(expected) = &archive.sha256
        && !expected.eq_ignore_ascii_case(&digest)
    {
        return Err(format!(
            "the download does not match its published checksum (expected {expected}, got {digest})"
        ));
    }

    let unpacked = scratch.path().join("unpacked");
    let relative = command
        .strip_prefix(&archive.dir)
        .map_err(|_| "the command is not inside its install folder")?
        .to_path_buf();
    let url = archive.url.clone();
    let (from, into, wanted) = (download.clone(), unpacked.clone(), relative.clone());
    tokio::task::spawn_blocking(move || unpack(&url, &from, &into, &wanted))
        .await
        .map_err(|_| "unpacking stopped unexpectedly".to_string())??;

    let found = unpacked.join(&relative);
    if !found.is_file() {
        return Err(format!(
            "the archive has no {} in it",
            relative.to_string_lossy()
        ));
    }
    executable(&found)?;

    if let Err(error) = std::fs::rename(&unpacked, &archive.dir)
        && !command.is_file()
    {
        return Err(error.to_string());
    }
    prune(parent, &archive.dir);
    Ok(())
}

/// Streams `url` into `path`, returning the hex SHA-256 of what arrived.
async fn download_to(url: &str, path: &Path) -> Result<String, String> {
    let response = reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_TIMEOUT)
        .build()
        .map_err(|error| error.to_string())?
        .get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| format!("the download failed: {error}"))?;
    let mut file = std::fs::File::create(path).map_err(|error| error.to_string())?;
    let mut hasher = Sha256::new();
    let mut body = response.bytes_stream();
    while let Some(chunk) = body.next().await {
        let chunk = chunk.map_err(|error| format!("the download failed: {error}"))?;
        hasher.update(&chunk);
        file.write_all(&chunk).map_err(|error| error.to_string())?;
    }
    file.flush().map_err(|error| error.to_string())?;
    Ok(hex::encode(hasher.finalize()))
}

/// Unpacks by the archive's extension. Both unpackers refuse entries that
/// would land outside `into`. A URL that is not an archive at all is the
/// program itself, and is put where the command says it is.
fn unpack(url: &str, from: &Path, into: &Path, command: &Path) -> Result<(), String> {
    let name = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase();
    let open = || std::fs::File::open(from).map_err(|error| error.to_string());
    let broken = |error: std::io::Error| format!("the archive could not be unpacked: {error}");
    std::fs::create_dir_all(into).map_err(|error| error.to_string())?;
    if name.ends_with(".zip") {
        zip::ZipArchive::new(open()?)
            .and_then(|mut archive| archive.extract(into))
            .map_err(|error| format!("the archive could not be unpacked: {error}"))
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        tar::Archive::new(flate2::read::GzDecoder::new(open()?))
            .unpack(into)
            .map_err(broken)
    } else if name.ends_with(".tar.bz2") || name.ends_with(".tbz2") {
        tar::Archive::new(bzip2::read::BzDecoder::new(open()?))
            .unpack(into)
            .map_err(broken)
    } else if name.ends_with(".tar") {
        tar::Archive::new(open()?).unpack(into).map_err(broken)
    } else {
        let target = into.join(command);
        if let Some(directory) = target.parent() {
            std::fs::create_dir_all(directory).map_err(|error| error.to_string())?;
        }
        std::fs::copy(from, target)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

/// A zip carries no Unix mode unless its maker put one in, so the command is
/// marked runnable whatever the archive said.
#[cfg(unix)]
fn executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(path)
        .map_err(|error| error.to_string())?
        .permissions();
    permissions.set_mode(permissions.mode() | 0o755);
    std::fs::set_permissions(path, permissions).map_err(|error| error.to_string())
}

#[cfg(not(unix))]
fn executable(_path: &Path) -> Result<(), String> {
    Ok(())
}

/// Removes the agent's other releases. Best effort: a release a running
/// session still holds open on Windows stays until the next install.
fn prune(parent: &Path, keep: &Path) {
    let Ok(entries) = std::fs::read_dir(parent) else {
        return;
    };
    let stale: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path != keep)
        .filter(|path| {
            !path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(SCRATCH))
        })
        .collect();
    for path in stale {
        let _ = std::fs::remove_dir_all(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Serves `body` once per connection at any path, as a CDN would.
    async fn serve(body: Vec<u8>) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let body = body.clone();
                tokio::spawn(async move {
                    let mut request = [0u8; 4096];
                    let _ = socket.read(&mut request).await;
                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(head.as_bytes()).await;
                    let _ = socket.write_all(&body).await;
                });
            }
        });
        format!("http://{address}")
    }

    fn tar_gz(path: &str, contents: &[u8]) -> Vec<u8> {
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::fast(),
        ));
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append_data(&mut header, path, contents).unwrap();
        builder.into_inner().unwrap().finish().unwrap()
    }

    fn zipped(path: &str, contents: &[u8]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        writer
            .start_file(path, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(contents).unwrap();
        writer.finish().unwrap().into_inner()
    }

    fn archive(root: &Path, url: String, sha256: Option<String>) -> Archive {
        Archive {
            url,
            sha256,
            dir: root.join("agent").join("release"),
        }
    }

    #[tokio::test]
    async fn a_tarball_is_unpacked_into_place_and_its_command_made_runnable() {
        let root = tempfile::tempdir().unwrap();
        let body = tar_gz("bin/agent", b"#!/bin/sh\necho hi\n");
        let digest = hex::encode(Sha256::digest(&body));
        let url = format!("{}/agent-linux.tar.gz", serve(body).await);
        let archive = archive(root.path(), url, Some(digest.to_uppercase()));
        let command = archive.dir.join("bin").join("agent");

        // An older release beside it goes once this one is in place.
        let old = root.path().join("agent").join("older");
        std::fs::create_dir_all(&old).unwrap();

        fetch(&archive, &command).await.unwrap();
        assert_eq!(
            std::fs::read(&command).unwrap(),
            b"#!/bin/sh\necho hi\n".to_vec()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&command).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111);
        }
        assert!(!old.exists());
        let leftovers: Vec<_> = std::fs::read_dir(root.path().join("agent"))
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name())
            .collect();
        assert_eq!(leftovers, vec![std::ffi::OsString::from("release")]);
    }

    #[tokio::test]
    async fn a_zip_without_modes_still_starts() {
        let root = tempfile::tempdir().unwrap();
        let body = zipped("agy_acp_server.par", b"binary");
        let url = format!("{}/agy-acp-server-linux-x86_64.zip", serve(body).await);
        let archive = archive(root.path(), url, None);
        let command = archive.dir.join("agy_acp_server.par");
        fetch(&archive, &command).await.unwrap();
        assert_eq!(std::fs::read(&command).unwrap(), b"binary".to_vec());
    }

    #[tokio::test]
    async fn a_bare_binary_is_put_where_the_command_says() {
        let root = tempfile::tempdir().unwrap();
        let url = format!("{}/sigit-linux-amd64", serve(b"elf".to_vec()).await);
        let archive = archive(root.path(), url, None);
        let command = archive.dir.join("sigit-linux-amd64");
        fetch(&archive, &command).await.unwrap();
        assert_eq!(std::fs::read(&command).unwrap(), b"elf".to_vec());
    }

    /// A download that is not what the catalogue published is never unpacked,
    /// and leaves nothing behind that looks like an install.
    #[tokio::test]
    async fn a_checksum_mismatch_installs_nothing() {
        let root = tempfile::tempdir().unwrap();
        let url = format!("{}/agent.tar.gz", serve(tar_gz("agent", b"x")).await);
        let archive = archive(root.path(), url, Some("00".repeat(32)));
        let command = archive.dir.join("agent");
        let error = fetch(&archive, &command).await.unwrap_err();
        assert!(error.contains("checksum"), "{error}");
        assert!(!archive.dir.exists());
        assert_eq!(
            std::fs::read_dir(root.path().join("agent"))
                .unwrap()
                .count(),
            0
        );
    }

    #[tokio::test]
    async fn an_archive_without_its_command_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let url = format!("{}/agent.tar.gz", serve(tar_gz("other", b"x")).await);
        let archive = archive(root.path(), url, None);
        let error = fetch(&archive, &archive.dir.join("agent"))
            .await
            .unwrap_err();
        assert!(error.contains("no agent"), "{error}");
        assert!(!archive.dir.exists());
    }
}
