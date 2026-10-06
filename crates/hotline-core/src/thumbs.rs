//! Smaller copies of pictures for a phone's link.
//!
//! A picture in a conversation or a teammate's face is drawn on a phone a
//! few hundred points wide, so the phone asks for it at that size and the
//! desk makes the copy once and keeps it. A photo becomes a JPEG; a face
//! keeps its transparency as a PNG. The original stays where it was, for
//! when the picture is opened.

use crate::images::{self, DECODERS};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

/// The smallest and largest edge a copy may be asked for.
const MIN_EDGE: u32 = 48;
const MAX_EDGE: u32 = 1600;
const QUALITY: u8 = 75;
/// Copies kept before the oldest are cleared.
const KEEP: usize = 2000;

/// What kind of picture the copy is of.
#[derive(Clone, Copy)]
pub(crate) enum Kind {
    /// A photo or screenshot: flattened onto white, a JPEG.
    Photo,
    /// A face on a transparent ground: kept transparent, a PNG.
    Face,
}

/// The kept copy of `source` at most `edge` px on its longer side, made now
/// if it was not already. The copy is named by the source's path, length
/// and modification time, so a replaced source gets a fresh copy.
pub(crate) async fn copy_of(
    root: &Path,
    source: &Path,
    edge: u32,
    kind: Kind,
) -> Result<PathBuf, String> {
    let unreadable = || "That picture could not be read.".to_string();
    let edge = edge.clamp(MIN_EDGE, MAX_EDGE);
    let metadata = fs::symlink_metadata(source).map_err(|_| unreadable())?;
    if !metadata.is_file() {
        return Err(unreadable());
    }
    let modified = metadata
        .modified()
        .ok()
        .and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |at| at.as_nanos());
    let mut key = Sha256::new();
    key.update(source.as_os_str().as_encoded_bytes());
    key.update(metadata.len().to_le_bytes());
    key.update(modified.to_le_bytes());
    key.update(edge.to_le_bytes());
    let extension = match kind {
        Kind::Photo => "jpg",
        Kind::Face => "png",
    };
    let dir = root.join("cache").join("thumbs");
    let out = dir.join(format!("{}.{extension}", hex::encode(key.finalize())));
    if fs::symlink_metadata(&out).is_ok_and(|metadata| metadata.is_file()) {
        return Ok(out);
    }

    let source = source.to_path_buf();
    let permit = DECODERS
        .acquire()
        .await
        .map_err(|_| "Pictures are not being made now.".to_string())?;
    let made = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let bytes = fs::read(&source).map_err(|_| unreadable())?;
        match kind {
            Kind::Photo => images::shrink(&bytes, edge, QUALITY)
                .map_err(|_| "That picture could not be made smaller.".to_string()),
            Kind::Face => face(&bytes, edge),
        }
    })
    .await
    .map_err(|_| "That picture could not be made smaller.".to_string())??;

    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let partial = out.with_extension(format!("{extension}.part"));
    fs::write(&partial, made).map_err(|error| error.to_string())?;
    fs::rename(&partial, &out).map_err(|error| error.to_string())?;
    prune(&dir);
    Ok(out)
}

/// A face made smaller with its transparency kept.
fn face(bytes: &[u8], edge: u32) -> Result<Vec<u8>, String> {
    let fail = || "That picture could not be made smaller.".to_string();
    let decoded = image::load_from_memory(bytes).map_err(|_| fail())?;
    let sized = if decoded.width().max(decoded.height()) > edge {
        decoded.resize(edge, edge, image::imageops::FilterType::Lanczos3)
    } else {
        decoded
    };
    let mut out = std::io::Cursor::new(Vec::new());
    sized
        .write_to(&mut out, image::ImageFormat::Png)
        .map_err(|_| fail())?;
    Ok(out.into_inner())
}

/// Clears the oldest copies once there are more than [`KEEP`]; any of them
/// is made again the next time it is asked for.
fn prune(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut copies: Vec<_> = entries
        .flatten()
        .filter_map(|entry| {
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((modified, entry.path()))
        })
        .collect();
    if copies.len() <= KEEP {
        return;
    }
    copies.sort();
    for (_, path) in copies.iter().take(copies.len() - KEEP) {
        let _ = fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let picture = image::RgbaImage::from_pixel(width, height, image::Rgba([10, 200, 120, 128]));
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(picture)
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    #[tokio::test]
    async fn a_copy_is_made_once_at_the_size_asked_and_kept() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("photo.png");
        fs::write(&source, png(1200, 800)).unwrap();

        let photo = copy_of(root.path(), &source, 300, Kind::Photo)
            .await
            .unwrap();
        let made = image::open(&photo).unwrap();
        assert_eq!((made.width(), made.height()), (300, 200));
        assert_eq!(photo.extension().unwrap(), "jpg");
        let again = copy_of(root.path(), &source, 300, Kind::Photo)
            .await
            .unwrap();
        assert_eq!(photo, again);

        let face = copy_of(root.path(), &source, 64, Kind::Face).await.unwrap();
        let made = image::open(&face).unwrap();
        assert_eq!(made.width(), 64);
        assert!(made.color().has_alpha());
    }

    #[tokio::test]
    async fn a_size_out_of_range_is_held_to_the_range() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("small.png");
        fs::write(&source, png(4000, 100)).unwrap();
        let copy = copy_of(root.path(), &source, 1, Kind::Photo).await.unwrap();
        assert_eq!(image::open(&copy).unwrap().width(), MIN_EDGE);
    }
}
