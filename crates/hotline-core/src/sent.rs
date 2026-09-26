//! Files teammates send the person (BRO-98): what each one is taken to be,
//! where the desk keeps its copy, and reading it back a part at a time.
//!
//! The desk keeps its own copy of every file sent, because the conversation
//! outlives where the file came from: a workspace file is edited, a computer
//! is replaced, a screen moves on. The copy is written once, before the
//! message that names it, under `files/<teammate>/<message id>/<name>` in the
//! data directory, and nothing changes it afterwards.
//!
//! An image goes through the same policy as one the person attaches
//! ([`crate::images`]) and is kept as the JPEG that makes. Anything else is
//! kept byte for byte, up to [`MAX_BYTES`]. No type is refused: the person
//! opens a file by choice, and the desk never opens one on its own.
//!
//! User images have separate, byte-for-byte readback copies under each
//! message's `user/<attachment index>`. Their original paths remain on the
//! tape, but readback never reopens them: later edits are not earlier pictures.

use crate::contract::{Attachment, AttachmentKind, FileChunk};
use crate::images::{self, Unfit};
use crate::log::{Log, StreamId};
use crate::paths;
use base64::{Engine, prelude::BASE64_STANDARD};
use serde_json::Value;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// The most a file may weigh, whatever it is.
pub(crate) const MAX_BYTES: u64 = 25 * 1024 * 1024;

/// The most one `file.read` answers. Its base64 is well inside the largest
/// frame a phone's socket is sent.
pub(crate) const CHUNK_BYTES: u64 = 512 * 1024;

/// The longest name a sent file keeps.
const MAX_NAME_CHARS: usize = 120;

/// A file as it will be kept and shown.
pub(crate) struct Prepared {
    pub(crate) name: String,
    pub(crate) mime_type: String,
    pub(crate) kind: AttachmentKind,
    pub(crate) bytes: Vec<u8>,
    pub(crate) dimensions: Option<(u32, u32)>,
    /// Why an image is being sent as a plain file instead of a picture.
    pub(crate) unfit: Option<Unfit>,
}

/// Takes a file for what its bytes say it is. `name` is what it was called
/// where it came from. Decoding an image is slow, so this is for a blocking
/// thread.
pub(crate) fn prepare(name: &str, bytes: Vec<u8>) -> Result<Prepared, String> {
    let name = clean_name(name);
    let unfit = if images::readable(&bytes) {
        match images::normalize(&bytes) {
            Ok(jpeg) => {
                return Ok(Prepared {
                    name: as_jpeg(&name),
                    mime_type: "image/jpeg".to_string(),
                    kind: AttachmentKind::Image,
                    dimensions: images::dimensions(&jpeg),
                    bytes: jpeg,
                    unfit: None,
                });
            }
            // An image the policy cannot make into a picture still reaches
            // the person, as the file it is.
            Err(unfit) => Some(unfit),
        }
    } else {
        None
    };
    if bytes.len() as u64 > MAX_BYTES {
        return Err(too_large(&name, Some(bytes.len() as u64)));
    }
    let mime_type = mime_type(&name, &bytes);
    let name = match mime_type.as_str() {
        "application/pdf" => as_pdf(&name),
        _ => name,
    };
    Ok(Prepared {
        mime_type,
        name,
        kind: AttachmentKind::File,
        bytes,
        dimensions: None,
        unfit,
    })
}

/// The refusal for a file over the cap, which says what the cap is. `size`
/// is `None` when the file was cut off before its end was known.
pub(crate) fn too_large(name: &str, size: Option<u64>) -> String {
    let cap = size_text(MAX_BYTES);
    match size {
        // Just over the cap reads as the cap itself, which would say
        // nothing about why.
        Some(size) if size >= MAX_BYTES + 1024 * 1024 / 10 => format!(
            "{name} is {}; a file can be at most {cap}.",
            size_text(size)
        ),
        _ => format!("{name} is larger than the {cap} a file can be."),
    }
}

/// What a file is, from its first bytes where they say, and otherwise from
/// its name. A PDF is one only by its bytes, since the desk offers to open it.
pub(crate) fn mime_type(name: &str, bytes: &[u8]) -> String {
    if bytes.starts_with(b"%PDF-") {
        return "application/pdf".to_string();
    }
    if let Ok(format) = image::guess_format(bytes)
        && images::readable(bytes)
    {
        return format.to_mime_type().to_string();
    }
    match mime_guess::from_path(name).first_raw() {
        Some("application/pdf") | None => "application/octet-stream".to_string(),
        Some(guessed) => guessed.to_string(),
    }
}

/// A size in the words a person reads it in.
pub(crate) fn size_text(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * 1024;
    if bytes < KB {
        format!("{bytes} bytes")
    } else if bytes < MB {
        format!("{} KB", bytes.div_ceil(KB))
    } else if bytes.is_multiple_of(MB) {
        format!("{} MB", bytes / MB)
    } else {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    }
}

/// A name the desk can keep and a person can read: the last part of a path,
/// without the characters a filesystem refuses, never empty and never long.
fn clean_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = base
        .chars()
        .map(|character| {
            if character.is_control()
                || matches!(character, '<' | '>' | ':' | '"' | '|' | '?' | '*')
            {
                '_'
            } else {
                character
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_end_matches(['.', ' ']);
    let cleaned = if cleaned.is_empty() { "file" } else { cleaned };
    let stem = cleaned.split('.').next().unwrap_or_default();
    let reserved = matches!(
        stem.to_ascii_lowercase().as_str(),
        "con" | "prn" | "aux" | "nul"
    ) || (stem.len() == 4
        && stem
            .get(..3)
            .is_some_and(|prefix| ["com", "lpt"].contains(&prefix.to_ascii_lowercase().as_str()))
        && matches!(stem.as_bytes()[3], b'1'..=b'9'));
    let cleaned = if reserved {
        format!("_{cleaned}")
    } else {
        cleaned.to_string()
    };
    shortened(&cleaned)
}

/// A long name cut in its stem, so the extension that says what it is stays.
fn shortened(name: &str) -> String {
    if name.chars().count() <= MAX_NAME_CHARS {
        return name.to_string();
    }
    let (stem, extension) = match name.rfind('.') {
        Some(dot) if dot > 0 && name[dot..].chars().count() <= 16 => name.split_at(dot),
        _ => (name, ""),
    };
    let keep = MAX_NAME_CHARS - extension.chars().count();
    format!("{}{extension}", stem.chars().take(keep).collect::<String>())
}

/// The name an image is kept under once it is a JPEG.
/// A PDF's name ends in `.pdf`, whatever it was called. The system picks
/// the program that opens a file by its name, and the desk offers to open a
/// PDF, so a PDF called anything else would be opened as that.
fn as_pdf(name: &str) -> String {
    if name.to_ascii_lowercase().ends_with(".pdf") {
        return name.to_string();
    }
    shortened(&format!("{name}.pdf"))
}

fn as_jpeg(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        return name.to_string();
    }
    let stem = match name.rfind('.') {
        Some(0) => "image",
        Some(dot) => &name[..dot],
        None => name,
    };
    shortened(&format!("{stem}.jpg"))
}

/// Keeps the desk's copy of the file one message carries, and says where.
pub(crate) fn store(
    root: &Path,
    persona_id: &str,
    event_id: &str,
    file: &Prepared,
) -> io::Result<PathBuf> {
    let dir = paths::sent_file_dir(root, persona_id, event_id)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "not a message id"))?;
    fs::create_dir_all(&dir)?;
    let path = dir.join(&file.name);
    let written = File::create_new(&path).and_then(|mut out| {
        out.write_all(&file.bytes)?;
        out.sync_all()
    });
    if let Err(error) = written {
        let _ = fs::remove_dir_all(&dir);
        return Err(error);
    }
    Ok(path)
}

/// Forgets the copy of a file whose message never reached the tape.
pub(crate) fn discard(root: &Path, persona_id: &str, event_id: &str) {
    if let Some(dir) = paths::sent_file_dir(root, persona_id, event_id) {
        let _ = fs::remove_dir_all(dir);
    }
}

/// The one file a message's directory holds.
fn kept(root: &Path, persona_id: &str, event_id: &str) -> Option<PathBuf> {
    let dir = paths::sent_file_dir(root, persona_id, event_id)?;
    let mut files = fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()));
    let only = files.next()?;
    files.next().is_none().then(|| only.path())
}

/// The readback copy of a user image. Its original array position is part
/// of the address; neither the attachment name nor its source path is one.
fn user_image_path(root: &Path, persona_id: &str, event_id: &str, index: u32) -> Option<PathBuf> {
    Some(
        paths::sent_file_dir(root, persona_id, event_id)?
            .join("user")
            .join(index.to_string()),
    )
}

/// Best-effort readback copies do not change whether a prompt is delivered.
/// An unavailable, oversized or unrecognized source remains an attachment for
/// the agent, but is never later reopened on behalf of a phone.
pub(crate) fn retain_user_images(
    root: &Path,
    persona_id: &str,
    event_id: &str,
    attachments: &[Attachment],
) {
    for (index, attachment) in attachments.iter().enumerate() {
        if attachment.kind != AttachmentKind::Image {
            continue;
        }
        let Ok(index) = u32::try_from(index) else {
            continue;
        };
        if let Err(error) = retain_user_image(root, persona_id, event_id, index, attachment) {
            eprintln!("Could not retain user image {event_id}/{index}: {error}");
        }
    }
}

fn retain_user_image(
    root: &Path,
    persona_id: &str,
    event_id: &str,
    index: u32,
    attachment: &Attachment,
) -> io::Result<()> {
    let invalid =
        || io::Error::other("The image is unavailable, changed, unsupported or over 25 MiB.");
    // Check before opening so a named pipe is not an unbounded wait. The
    // authenticated desk chooses this source; the phone supplies upload IDs.
    let metadata = fs::metadata(&attachment.path)?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES {
        return Err(invalid());
    }
    let mut source = File::open(&attachment.path)?;
    let before = source.metadata()?;
    if !before.is_file()
        || before.len() > MAX_BYTES
        || before.len() != metadata.len()
        || before.modified()? != metadata.modified()?
    {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    (&mut source).take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    let after = source.metadata()?;
    if bytes.len() as u64 != before.len()
        || after.len() != before.len()
        || after.modified()? != before.modified()?
        || !images::readable(&bytes)
    {
        return Err(invalid());
    }
    let path = user_image_path(root, persona_id, event_id, index).ok_or_else(invalid)?;
    let dir = path.parent().unwrap();
    fs::create_dir_all(dir)?;
    // Publish only complete copies, and never replace one already published.
    let mut copy = tempfile::NamedTempFile::new_in(dir)?;
    copy.write_all(&bytes)?;
    copy.as_file().sync_all()?;
    copy.persist_noclobber(path).map_err(|error| error.error)?;
    Ok(())
}

/// Read authority is a message and an attachment position, never a source
/// path. Old user events lack a send-time copy and fail closed: even matching
/// size or MIME cannot prove their source has not since been replaced.
pub(crate) fn read_message(
    log: &Log,
    persona_id: &str,
    event_id: &str,
    index: u32,
    offset: i64,
) -> Result<FileChunk, String> {
    let missing = || "That message has no file.".to_string();
    paths::sent_file_dir(log.root(), persona_id, event_id).ok_or_else(missing)?;
    let tape = log
        .try_load(&StreamId::Tape(persona_id.to_string()))
        .map_err(|_| "That conversation could not be read.".to_string())?;
    let event = tape.iter().find(|event| event["id"] == event_id);
    if let Some(event) = event.filter(|event| event["kind"] == "user") {
        let attachment = event["attachments"]
            .as_array()
            .and_then(|attachments| attachments.get(index as usize))
            .ok_or_else(|| "That attachment index is outside the message.".to_string())?;
        if attachment["kind"] != "image" {
            return Err("That attachment is not an image.".to_string());
        }
        let name = attachment["name"].as_str().unwrap_or("image");
        if attachment["size"]
            .as_u64()
            .is_some_and(|size| size > MAX_BYTES)
        {
            return Err(too_large(name, attachment["size"].as_u64()));
        }
        let path = user_image_path(log.root(), persona_id, event_id, index).ok_or_else(missing)?;
        let mut chunk = read_path(&path, offset, true)?;
        chunk.name = name.to_string();
        return Ok(chunk);
    }
    if index != 0 {
        return Err("A teammate's file has only attachment index zero.".to_string());
    }
    read(log.root(), persona_id, event_id, offset)
}

/// The existing teammate readback remains independent of the tape's age.
pub(crate) fn read(
    root: &Path,
    persona_id: &str,
    event_id: &str,
    offset: i64,
) -> Result<FileChunk, String> {
    let path =
        kept(root, persona_id, event_id).ok_or_else(|| "That message has no file.".to_string())?;
    read_path(&path, offset, false)
}

fn read_path(path: &Path, offset: i64, image_only: bool) -> Result<FileChunk, String> {
    let missing = || {
        if image_only {
            "That image has no retained copy. Send it again to make it available.".to_string()
        } else {
            "That message has no file.".to_string()
        }
    };
    // A copy is a regular file, not a link out of its message directory.
    if !fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_file()) {
        return Err(missing());
    }
    let mut file = File::open(path).map_err(|_| missing())?;
    let size = file.metadata().map_err(|_| missing())?.len();
    if size > MAX_BYTES {
        return Err(too_large("The file", Some(size)));
    }
    let start = u64::try_from(offset)
        .ok()
        .filter(|start| *start <= size)
        .ok_or_else(|| format!("The file is {size} bytes; {offset} is not a place in it."))?;
    let mut head = Vec::with_capacity(32);
    (&mut file)
        .take(32)
        .read_to_end(&mut head)
        .map_err(|error| error.to_string())?;
    if image_only && !images::readable(&head) {
        return Err("The retained attachment is not a supported image.".to_string());
    }
    file.seek(SeekFrom::Start(start))
        .map_err(|error| error.to_string())?;
    let mut data = Vec::new();
    file.take(CHUNK_BYTES.min(size - start))
        .read_to_end(&mut data)
        .map_err(|error| error.to_string())?;
    if data.len() as u64 != CHUNK_BYTES.min(size - start) {
        return Err("The kept file changed while it was being read.".to_string());
    }
    let end = start + data.len() as u64;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(FileChunk {
        mime_type: mime_type(&name, &head),
        name,
        size: size as i64,
        offset: start as i64,
        data: BASE64_STANDARD.encode(&data),
        next: (end < size).then_some(end as i64),
    })
}

/// A message as text, with the name of each file it carries, for whatever
/// reads a conversation as words: the model's memory of it, a chapter's
/// note, the search index. A file sent without a caption is a message with
/// no words, and its name is all there is to go on.
pub(crate) fn message_text(event: &Value) -> String {
    let text = event
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let files: Vec<String> = file_names(event)
        .map(|name| format!("[file: {name}]"))
        .collect();
    match (text.trim().is_empty(), files.is_empty()) {
        (_, true) => text.to_string(),
        (true, false) => files.join("\n"),
        (false, false) => format!("{text}\n{}", files.join("\n")),
    }
}

/// The names of the files a message carries.
pub(crate) fn file_names(event: &Value) -> impl Iterator<Item = &str> {
    event
        .get("attachments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|attachment| attachment.get("name").and_then(Value::as_str))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::RgbaImage::from_pixel(width, height, image::Rgba([10, 20, 30, 255]))
            .write_to(&mut io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    #[test]
    fn a_picture_is_made_into_the_policys_jpeg_and_named_for_it() {
        let file = prepare("shots/after.png", png(3000, 1500)).unwrap();
        assert_eq!(file.kind, AttachmentKind::Image);
        assert_eq!(file.name, "after.jpg");
        assert_eq!(file.mime_type, "image/jpeg");
        assert_eq!(file.dimensions, Some((2000, 1000)));
        assert!(file.bytes.starts_with(&[0xFF, 0xD8, 0xFF]));
        assert!(file.unfit.is_none());
    }

    #[test]
    fn anything_else_is_kept_as_it_is_and_told_by_its_bytes_before_its_name() {
        let pdf = prepare("report", b"%PDF-1.7\n...".to_vec()).unwrap();
        assert_eq!(
            (pdf.kind, pdf.mime_type.as_str(), pdf.name.as_str()),
            (AttachmentKind::File, "application/pdf", "report.pdf")
        );
        assert_eq!(pdf.bytes, b"%PDF-1.7\n...");

        // The system opens a file by its name, so a PDF is always named one:
        // a script that starts like a PDF is never opened as a script.
        let script = prepare("run.bat", b"%PDF-1.7\r\ncalc.exe\r\n".to_vec()).unwrap();
        assert_eq!(script.name, "run.bat.pdf");
        assert_eq!(
            prepare("Q3.PDF", b"%PDF-1.7".to_vec()).unwrap().name,
            "Q3.PDF"
        );

        let csv = prepare("/tmp/out/data.csv", b"a,b\n1,2\n".to_vec()).unwrap();
        assert_eq!(
            (csv.name.as_str(), csv.mime_type.as_str()),
            ("data.csv", "text/csv")
        );

        // Named a PDF, but not one: the desk would offer to open it as one.
        let fake = prepare("invoice.pdf", b"MZ\x90\x00".to_vec()).unwrap();
        assert_eq!(fake.mime_type, "application/octet-stream");

        let unknown = prepare("blob", vec![0, 1, 2]).unwrap();
        assert_eq!(unknown.mime_type, "application/octet-stream");
    }

    #[test]
    fn a_picture_the_policy_cannot_make_is_sent_as_the_file_it_is() {
        let mut broken = png(4, 4);
        broken.truncate(40);
        let file = prepare("broken.png", broken.clone()).unwrap();
        assert_eq!(file.kind, AttachmentKind::File);
        assert_eq!(file.name, "broken.png");
        assert_eq!(file.mime_type, "image/png");
        assert_eq!(file.bytes, broken);
        assert!(file.unfit.is_some());
    }

    #[test]
    fn a_file_over_the_cap_is_refused_in_a_sentence_that_says_the_cap() {
        let refused = prepare("dump.bin", vec![7; MAX_BYTES as usize + 1])
            .err()
            .unwrap();
        assert_eq!(refused, "dump.bin is larger than the 25 MB a file can be.");
        assert!(prepare("edge.bin", vec![7; MAX_BYTES as usize]).is_ok());
        assert_eq!(
            too_large("film.mov", Some(40 * 1024 * 1024 + 300 * 1024)),
            "film.mov is 40.3 MB; a file can be at most 25 MB."
        );
        assert_eq!(
            too_large("big.iso", None),
            "big.iso is larger than the 25 MB a file can be."
        );
    }

    #[test]
    fn names_are_what_a_filesystem_keeps_and_a_person_reads() {
        assert_eq!(clean_name("a/b\\c:d?.txt"), "c_d_.txt");
        assert_eq!(clean_name(""), "file");
        assert_eq!(clean_name(".."), "file");
        assert_eq!(clean_name("notes. "), "notes");
        assert_eq!(clean_name(".env"), ".env");
        assert_eq!(clean_name("CON.txt"), "_CON.txt");
        assert_eq!(clean_name("com1"), "_com1");
        assert_eq!(clean_name("compose.yaml"), "compose.yaml");
        let long = format!("{}.pdf", "x".repeat(300));
        let kept = clean_name(&long);
        assert_eq!(kept.chars().count(), MAX_NAME_CHARS);
        assert!(kept.ends_with("x.pdf"));
        assert_eq!(as_jpeg("photo.PNG"), "photo.jpg");
        assert_eq!(as_jpeg("photo.jpeg"), "photo.jpeg");
        assert_eq!(as_jpeg("screenshot"), "screenshot.jpg");
        assert_eq!(as_jpeg(".png"), "image.jpg");
    }

    #[test]
    fn sizes_read_as_a_person_says_them() {
        assert_eq!(size_text(12), "12 bytes");
        assert_eq!(size_text(1536), "2 KB");
        assert_eq!(size_text(25 * 1024 * 1024), "25 MB");
        assert_eq!(size_text(3 * 1024 * 1024 / 2), "1.5 MB");
    }

    #[test]
    fn a_kept_file_reads_back_a_part_at_a_time_and_only_by_its_message() {
        let root = tempfile::tempdir().unwrap();
        let event = "9f2b7c1e-0d4a-4e8b-9c55-2f1d3a6b7e80";
        let bytes: Vec<u8> = (0..CHUNK_BYTES + 100).map(|at| (at % 251) as u8).collect();
        let file = prepare("log.txt", bytes.clone()).unwrap();
        let path = store(root.path(), "ada", event, &file).unwrap();
        assert!(path.ends_with(format!("files/ada/{event}/log.txt")));

        let first = read(root.path(), "ada", event, 0).unwrap();
        assert_eq!(
            (first.name.as_str(), first.mime_type.as_str()),
            ("log.txt", "text/plain")
        );
        assert_eq!((first.size, first.offset), (bytes.len() as i64, 0));
        assert_eq!(first.next, Some(CHUNK_BYTES as i64));
        let second = read(root.path(), "ada", event, first.next.unwrap()).unwrap();
        assert_eq!(second.next, None);
        let mut whole = BASE64_STANDARD.decode(&first.data).unwrap();
        whole.extend(BASE64_STANDARD.decode(&second.data).unwrap());
        assert_eq!(whole, bytes);

        // The end of the file is a place in it, and past the end is not.
        let end = read(root.path(), "ada", event, bytes.len() as i64).unwrap();
        assert_eq!((end.data.as_str(), end.next), ("", None));
        assert!(read(root.path(), "ada", event, bytes.len() as i64 + 1).is_err());
        assert!(read(root.path(), "ada", event, -1).is_err());

        // Another teammate's message, a made-up one, and a path for an id.
        assert!(read(root.path(), "bob", event, 0).is_err());
        assert!(
            read(
                root.path(),
                "ada",
                "9f2b7c1e-0000-4e8b-9c55-2f1d3a6b7e80",
                0
            )
            .is_err()
        );
        assert!(read(root.path(), "ada", "../ada", 0).is_err());
        assert!(read(root.path(), "ada", "", 0).is_err());

        // A second file beside it makes the directory no one message's.
        fs::write(path.with_file_name("other.txt"), "x").unwrap();
        assert!(read(root.path(), "ada", event, 0).is_err());
    }

    #[test]
    fn user_image_copies_keep_their_bytes_and_cannot_be_replaced() {
        let root = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        let path = source.path().join("picture");
        for (index, format) in [
            image::ImageFormat::Png,
            image::ImageFormat::Jpeg,
            image::ImageFormat::Gif,
            image::ImageFormat::WebP,
        ]
        .into_iter()
        .enumerate()
        {
            let mut encoded = io::Cursor::new(Vec::new());
            image::DynamicImage::new_rgb8(2, 2)
                .write_to(&mut encoded, format)
                .unwrap();
            let bytes = encoded.into_inner();
            fs::write(&path, &bytes).unwrap();
            let attachment: Attachment = serde_json::from_value(json!({
                "kind":"image", "name":"picture", "path":path,
            }))
            .unwrap();
            let event = format!("image-{index}");
            retain_user_image(root.path(), "ada", &event, 0, &attachment).unwrap();
            // Even a second capture of the same address cannot overwrite it.
            fs::write(&path, png(3, 3)).unwrap();
            assert!(retain_user_image(root.path(), "ada", &event, 0, &attachment).is_err());
            let kept = user_image_path(root.path(), "ada", &event, 0).unwrap();
            let chunk = read_path(&kept, 0, true).unwrap();
            assert_eq!(chunk.mime_type, format.to_mime_type());
            assert_eq!(BASE64_STANDARD.decode(chunk.data).unwrap(), bytes);
        }
        // The cap is inclusive, and is measured from the opened file, not
        // an optional or stale size supplied with the attachment.
        fs::write(&path, png(2, 2)).unwrap();
        let file = fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.set_len(MAX_BYTES).unwrap();
        let attachment: Attachment = serde_json::from_value(json!({
            "kind":"image", "name":"picture", "path":path, "size":1,
        }))
        .unwrap();
        retain_user_image(root.path(), "ada", "at-cap", 0, &attachment).unwrap();
        let chunk = read_path(
            &user_image_path(root.path(), "ada", "at-cap", 0).unwrap(),
            0,
            true,
        )
        .unwrap();
        assert_eq!(chunk.size, MAX_BYTES as i64);
        assert_eq!(chunk.next, Some(CHUNK_BYTES as i64));
        file.set_len(MAX_BYTES + 1).unwrap();
        assert!(retain_user_image(root.path(), "ada", "over-cap", 0, &attachment).is_err());
        assert!(
            !user_image_path(root.path(), "ada", "over-cap", 0)
                .unwrap()
                .exists()
        );
    }

    #[test]
    fn a_message_reads_as_its_words_and_the_names_of_its_files() {
        let caption =
            json!({"kind": "agent", "text": "Here it is", "attachments": [{"name": "a.pdf"}]});
        assert_eq!(message_text(&caption), "Here it is\n[file: a.pdf]");
        let bare = json!({"kind": "agent", "text": "", "attachments": [{"name": "a.pdf"}]});
        assert_eq!(message_text(&bare), "[file: a.pdf]");
        let words = json!({"kind": "user", "text": "hello"});
        assert_eq!(message_text(&words), "hello");
    }
}
