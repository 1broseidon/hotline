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
use std::path::{Component, Path, PathBuf};
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

/// Unpacks by the archive's extension. Every entry is admitted before it is
/// written, so an archive with one entry it should not have installs nothing:
/// the scratch folder it was going into is dropped with the error. A URL that
/// is not an archive at all is the program itself, and is put where the
/// command says it is.
fn unpack(url: &str, from: &Path, into: &Path, command: &Path) -> Result<(), String> {
    let name = Path::new(url.split(['?', '#']).next().unwrap_or(url));
    let extension = |path: &Path| {
        path.extension()
            .and_then(std::ffi::OsStr::to_str)
            .map(str::to_ascii_lowercase)
    };
    let outer = extension(name);
    let inner = name.file_stem().map(Path::new).and_then(extension);
    let open = || std::fs::File::open(from).map_err(|error| error.to_string());
    std::fs::create_dir_all(into).map_err(|error| error.to_string())?;
    match (inner.as_deref(), outer.as_deref()) {
        (_, Some("zip")) => unzip(open()?, into),
        (Some("tar"), Some("gz")) | (_, Some("tgz")) => {
            untar(flate2::read::GzDecoder::new(open()?), into)
        }
        (Some("tar"), Some("bz2")) | (_, Some("tbz2")) => {
            untar(bzip2::read::BzDecoder::new(open()?), into)
        }
        (_, Some("tar")) => untar(open()?, into),
        _ => {
            let target = into.join(command);
            if let Some(directory) = target.parent() {
                std::fs::create_dir_all(directory).map_err(|error| error.to_string())?;
            }
            std::fs::copy(from, target)
                .map(|_| ())
                .map_err(|error| error.to_string())
        }
    }
}

/// A zip's directory is at its end, so every entry is admitted from it before
/// anything is extracted.
fn unzip(file: std::fs::File, into: &Path) -> Result<(), String> {
    let broken =
        |error: zip::result::ZipError| format!("the archive could not be unpacked: {error}");
    let mut archive = zip::ZipArchive::new(file).map_err(broken)?;
    for index in 0..archive.len() {
        let entry = archive.by_index_raw(index).map_err(broken)?;
        admit(
            &entry.name().map_err(broken)?,
            entry.enclosed_name().is_some(),
            entry.is_symlink(),
            entry.unix_mode(),
        )?;
    }
    archive.extract(into).map_err(broken)
}

/// A tarball is a stream with no directory, so each entry is admitted as it
/// arrives, before it is written.
fn untar(reader: impl std::io::Read, into: &Path) -> Result<(), String> {
    let broken = |error: std::io::Error| format!("the archive could not be unpacked: {error}");
    let mut archive = tar::Archive::new(reader);
    for entry in archive.entries().map_err(broken)? {
        let mut entry = entry.map_err(broken)?;
        let path = entry.path().map_err(broken)?.into_owned();
        let kind = entry.header().entry_type();
        admit(
            &path.to_string_lossy(),
            enclosed(&path),
            kind.is_symlink() || kind.is_hard_link(),
            Some(entry.header().mode().map_err(broken)?),
        )?;
        entry.unpack_in(into).map_err(broken)?;
    }
    Ok(())
}

/// Whether `path` stays inside the folder it is unpacked into: relative, and
/// never stepping up out of it.
fn enclosed(path: &Path) -> bool {
    path.components()
        .all(|part| matches!(part, Component::Normal(_) | Component::CurDir))
}

/// The rules every entry of a registry archive is held to, whatever its
/// format. A link is refused outright, because where it points is only known
/// once it is on disk, and a later entry can be written through it. A mode
/// that is set-id or writable by anyone would hand the host's other users a
/// way into what the agent runs as.
fn admit(name: &str, enclosed: bool, link: bool, mode: Option<u32>) -> Result<(), String> {
    if !enclosed {
        return Err(format!(
            "the archive's entry {name} reaches outside its folder"
        ));
    }
    if link {
        return Err(format!("the archive's entry {name} is a link"));
    }
    if let Some(mode) = mode {
        if mode & 0o6000 != 0 {
            return Err(format!("the archive's entry {name} is set-id"));
        }
        if mode & 0o002 != 0 {
            return Err(format!("the archive's entry {name} is writable by anyone"));
        }
    }
    Ok(())
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

    /// One tar entry of any kind. The name is written into the header raw,
    /// because the builder would refuse the `../` an attacker's archive has.
    fn tar_entry(
        builder: &mut tar::Builder<impl std::io::Write>,
        path: &str,
        kind: tar::EntryType,
        link: Option<&Path>,
        mode: u32,
        contents: &[u8],
    ) {
        let mut header = tar::Header::new_gnu();
        header.as_old_mut().name[..path.len()].copy_from_slice(path.as_bytes());
        header.set_entry_type(kind);
        if let Some(link) = link {
            header.set_link_name(link).unwrap();
        }
        header.set_size(contents.len() as u64);
        header.set_mode(mode);
        header.set_cksum();
        builder.append(&header, contents).unwrap();
    }

    /// Fetches an archive that must be refused, and checks that it was refused
    /// for the reason given and left neither an install nor its scratch.
    async fn refused(body: Vec<u8>, name: &str, reason: &str) {
        let root = tempfile::tempdir().unwrap();
        let url = format!("{}/{name}", serve(body).await);
        let archive = archive(root.path(), url, None);
        let error = fetch(&archive, &archive.dir.join("agent"))
            .await
            .unwrap_err();
        assert!(error.contains(reason), "{error}");
        assert!(!archive.dir.exists());
        assert_eq!(
            std::fs::read_dir(root.path().join("agent"))
                .unwrap()
                .count(),
            0
        );
    }

    #[tokio::test]
    async fn a_symlink_out_of_the_scratch_folder_is_refused() {
        let outside = tempfile::tempdir().unwrap();
        let mut builder = tar::Builder::new(Vec::new());
        tar_entry(
            &mut builder,
            "agent",
            tar::EntryType::Regular,
            None,
            0o755,
            b"x",
        );
        tar_entry(
            &mut builder,
            "escape",
            tar::EntryType::Symlink,
            Some(outside.path()),
            0o777,
            b"",
        );
        refused(builder.into_inner().unwrap(), "agent.tar", "is a link").await;

        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("agent", options).unwrap();
        writer.write_all(b"x").unwrap();
        writer
            .add_symlink("escape", outside.path().to_string_lossy(), options)
            .unwrap();
        refused(
            writer.finish().unwrap().into_inner(),
            "agent.zip",
            "is a link",
        )
        .await;
    }

    /// The write through a link is what does the damage, so the test checks
    /// that the folder outside is still empty afterwards, not only the error.
    #[tokio::test]
    async fn a_write_through_a_symlink_lands_nowhere() {
        let outside = tempfile::tempdir().unwrap();
        let mut builder = tar::Builder::new(Vec::new());
        tar_entry(
            &mut builder,
            "escape",
            tar::EntryType::Symlink,
            Some(outside.path()),
            0o777,
            b"",
        );
        tar_entry(
            &mut builder,
            "escape/agent",
            tar::EntryType::Regular,
            None,
            0o755,
            b"x",
        );
        refused(builder.into_inner().unwrap(), "agent.tar", "is a link").await;

        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        writer
            .add_symlink("escape", outside.path().to_string_lossy(), options)
            .unwrap();
        writer.start_file("escape/agent", options).unwrap();
        writer.write_all(b"x").unwrap();
        refused(
            writer.finish().unwrap().into_inner(),
            "agent.zip",
            "is a link",
        )
        .await;

        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn an_entry_that_climbs_out_of_its_folder_is_refused() {
        let mut builder = tar::Builder::new(Vec::new());
        tar_entry(
            &mut builder,
            "agent",
            tar::EntryType::Regular,
            None,
            0o755,
            b"x",
        );
        tar_entry(
            &mut builder,
            "../planted",
            tar::EntryType::Regular,
            None,
            0o644,
            b"x",
        );
        refused(
            builder.into_inner().unwrap(),
            "agent.tar",
            "outside its folder",
        )
        .await;

        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("agent", options).unwrap();
        writer.write_all(b"x").unwrap();
        writer.start_file("../planted", options).unwrap();
        writer.write_all(b"x").unwrap();
        refused(
            writer.finish().unwrap().into_inner(),
            "agent.zip",
            "outside its folder",
        )
        .await;
    }

    #[tokio::test]
    async fn a_set_id_or_world_writable_mode_is_refused() {
        let mut builder = tar::Builder::new(Vec::new());
        tar_entry(
            &mut builder,
            "agent",
            tar::EntryType::Regular,
            None,
            0o4755,
            b"x",
        );
        refused(builder.into_inner().unwrap(), "agent.tar", "set-id").await;

        let mut builder = tar::Builder::new(Vec::new());
        tar_entry(
            &mut builder,
            "agent",
            tar::EntryType::Regular,
            None,
            0o757,
            b"x",
        );
        refused(
            builder.into_inner().unwrap(),
            "agent.tar",
            "writable by anyone",
        )
        .await;

        // A zip keeps a Unix mode in the top half of its external attributes,
        // read only from an archive made on Unix, as registry archives are; a
        // writer on Windows would mark it made on DOS unless told otherwise.
        let zip_with_mode = |mode: u32| {
            let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
            let options = zip::write::SimpleFileOptions::default()
                .system(zip::System::Unix)
                .external_attributes((0o100_000 | mode) << 16);
            writer.start_file("agent", options).unwrap();
            writer.write_all(b"x").unwrap();
            writer.finish().unwrap().into_inner()
        };
        refused(zip_with_mode(0o757), "agent.zip", "writable by anyone").await;

        // The zip reader clears set-id bits before anyone sees the mode, which
        // is the fix the version pin is for, so such a zip installs without them.
        let root = tempfile::tempdir().unwrap();
        let url = format!("{}/agent.zip", serve(zip_with_mode(0o6755)).await);
        let archive = archive(root.path(), url, None);
        let command = archive.dir.join("agent");
        fetch(&archive, &command).await.unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&command).unwrap().permissions().mode();
            assert_eq!(mode & 0o6000, 0);
        }
    }

    /// The registry's names are not always lower case, and a `.ZIP` is a zip.
    #[tokio::test]
    async fn an_archive_is_known_by_its_extension_in_any_case() {
        let root = tempfile::tempdir().unwrap();
        let url = format!("{}/Agent.ZIP", serve(zipped("agent", b"binary")).await);
        let archive = archive(root.path(), url, None);
        let command = archive.dir.join("agent");
        fetch(&archive, &command).await.unwrap();
        assert_eq!(std::fs::read(&command).unwrap(), b"binary".to_vec());
    }
}
