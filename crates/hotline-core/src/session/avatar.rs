//! A teammate's own picture (BRO-173): making a square of whatever image it
//! points at, keeping it under its hash, and putting it on the teammate's
//! record.
//!
//! The desk keeps `avatars/<teammate>/<sha256>.png`, written once and never
//! changed, so a window that has a hash has the picture for good. Setting a
//! new one removes the old files, and so does clearing. A picture the person
//! chose is not replaced by the teammate.

use super::{Room, generate::read_workspace_image};
use crate::contract::{Avatar, AvatarBy};
use crate::driver::CapabilityLease;
use crate::imagegen::{self, Aspect, ImageRequest};
use crate::images::DECODERS;
use crate::paths;
use crate::sent;
use crate::tools::Workspace;
use image::{DynamicImage, ImageFormat, RgbaImage};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

/// The edge of the square a picture is kept at.
const EDGE: u32 = 512;
/// The most a picture may weigh before it is decoded at all.
const MAX_BYTES: usize = 20 * 1024 * 1024;
const MAX_SOURCE_EDGE: u32 = 16_000;
/// Pixels fainter than this count as empty when looking for the subject.
const SUBJECT_ALPHA: u8 = 8;

/// What `set_avatar` was asked to do: a path wins over anything else sent
/// with it, because models fill in properties they were told to leave out
/// (a strict route makes them fill every one), and a picture it named is
/// plainly what it meant. Only a call with no usable path and no
/// `clear: true` is refused, and the refusal names what arrived so the
/// next try can differ.
fn requested_path(arguments: &Value) -> Result<Option<String>, String> {
    if let Some(path) = arguments
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        return Ok(Some(path.to_string()));
    }
    if arguments.get("clear").and_then(Value::as_bool) == Some(true) {
        return Ok(None);
    }
    let sent = match arguments.as_object() {
        Some(object) if !object.is_empty() => object
            .keys()
            .map(|key| format!("`{key}`"))
            .collect::<Vec<_>>()
            .join(", "),
        _ => "nothing".to_string(),
    };
    Err(format!(
        "set_avatar needs either a `path` (a string naming an image in your workspace) or `clear: true`; this call sent {sent}."
    ))
}

impl Room {
    /// The `set_avatar` tool: a file in the teammate's workspace becomes its
    /// picture, or `clear` puts the initial back.
    pub(crate) async fn set_avatar(
        &self,
        persona_id: &str,
        arguments: &Value,
        capability: Option<CapabilityLease>,
    ) -> Result<String, String> {
        let path = requested_path(arguments)?;
        self.refuse_over_the_persons_choice(persona_id)?;
        let prepared = match path {
            Some(path) => {
                let persona = self.persona(persona_id)?;
                let workspace = Workspace::open_with_capability(
                    PathBuf::from(&persona.cwd),
                    persona.reach.unwrap_or_default(),
                    self.log.root().join("tool-output").join(persona_id),
                    persona.folders.as_deref().unwrap_or_default(),
                    capability.clone(),
                )
                .map_err(|error| error.to_string())?;
                let permit = DECODERS
                    .acquire()
                    .await
                    .map_err(|_| "The picture could not be prepared.".to_string())?;
                let png = tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    let bytes = read_workspace_image(&workspace, &path, "The picture")?;
                    square(&bytes)
                })
                .await
                .map_err(|_| "The picture could not be prepared.".to_string())??;
                Some(png)
            }
            None => None,
        };
        if let Some(capability) = &capability {
            capability.check()?;
        }
        let gate = self.policy_update_lock();
        let _held = gate.lock().await;
        self.refuse_over_the_persons_choice(persona_id)?;
        let hash = self.write_avatar(persona_id, prepared, AvatarBy::Own)?;
        Ok(match hash {
            Some(hash) => json!({ "hash": hash }).to_string(),
            None => "Your picture is cleared; people see your initial again.".to_string(),
        })
    }

    /// Whether setup's offer can go ahead, answered before the drawing starts
    /// so a refusal reaches the person rather than a log.
    pub(crate) fn can_draw_avatar(&self, persona_id: &str) -> Result<(), String> {
        let persona = self.persona(persona_id)?;
        if persona
            .avatar
            .is_some_and(|avatar| avatar.by == AvatarBy::Person)
        {
            return Err("This teammate already has a picture you chose.".into());
        }
        self.image_generators().map(|_| ())
    }

    /// The setup screen's offer: a picture drawn from the teammate's name and
    /// goal, in the house avatar style. It counts as the teammate's own, so
    /// the teammate may redraw it later; one the person chose is kept.
    pub(crate) async fn generate_avatar(&self, persona_id: &str) -> Result<Avatar, String> {
        let persona = self.persona(persona_id)?;
        let _drawing = self.start_drawing(persona_id);
        if persona
            .avatar
            .as_ref()
            .is_some_and(|avatar| avatar.by == AvatarBy::Person)
        {
            return Err("This teammate already has a picture you chose.".into());
        }
        let styled = imagegen::styled(
            Some("avatar"),
            &avatar_subject(&persona.name, &persona.goal),
            Aspect::Square,
            persona_id,
        )?;
        let request = ImageRequest {
            prompt: styled.prompt,
            aspect: styled.aspect,
            transparent: false,
            references: styled.references,
        };
        let drawn = self.draw(&request, &None).await?;
        let permit = DECODERS
            .acquire()
            .await
            .map_err(|_| "The picture could not be prepared.".to_string())?;
        let png = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            square(&drawn.image.bytes)
        })
        .await
        .map_err(|_| "The picture could not be prepared.".to_string())??;
        let gate = self.policy_update_lock();
        let _held = gate.lock().await;
        if self
            .persona(persona_id)?
            .avatar
            .is_some_and(|avatar| avatar.by == AvatarBy::Person)
        {
            return Err("This teammate already has a picture you chose.".into());
        }
        self.write_avatar(persona_id, Some(png), AvatarBy::Own)?;
        self.persona(persona_id)?
            .avatar
            .ok_or_else(|| "The picture could not be saved.".to_string())
    }

    fn refuse_over_the_persons_choice(&self, persona_id: &str) -> Result<(), String> {
        let chosen = self
            .persona(persona_id)?
            .avatar
            .is_some_and(|avatar| avatar.by == AvatarBy::Person);
        if chosen {
            return Err(
                "The person chose your current picture, so leave it as it is unless they ask you to change it."
                    .into(),
            );
        }
        Ok(())
    }

    /// Keeps `png` as the teammate's picture, or clears it, and writes the
    /// record. The caller holds `policy_update_lock`. Answers the new hash.
    fn write_avatar(
        &self,
        persona_id: &str,
        png: Option<Vec<u8>>,
        by: AvatarBy,
    ) -> Result<Option<String>, String> {
        let root = self.log.root();
        let mut persona = self.persona(persona_id)?;
        let hash = match png {
            Some(png) => {
                let hash = hex::encode(Sha256::digest(&png));
                store(root, persona_id, &hash, &png)
                    .map_err(|_| "The picture could not be saved.".to_string())?;
                persona.avatar = Some(Avatar {
                    hash: hash.clone(),
                    by,
                    updated_at: chrono::Utc::now()
                        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                });
                Some(hash)
            }
            None => {
                persona.avatar = None;
                None
            }
        };
        persona.updated_at = super::now_ms();
        crate::room::append_persona(&self.log, &persona)?;
        remove_except(root, persona_id, hash.as_deref());
        Ok(hash)
    }
}

/// Who the crew member is: its name and job, which pick the taste decisions
/// the house style asks for.
fn avatar_subject(name: &str, goal: &str) -> String {
    let goal: String = goal.trim().chars().take(600).collect();
    let mut subject = format!("a teammate called {}", name.trim());
    if !goal.is_empty() {
        subject.push_str(&format!(", whose job is: {goal}"));
    }
    subject.push_str(
        ". Pick its taste decisions from these, the few a person would remember it by. If the name is a thing, its colour, skin or a detail echoes that thing.",
    );
    subject
}

/// Keeps a picture under its hash, unless it is already there.
fn store(root: &Path, persona_id: &str, hash: &str, png: &[u8]) -> std::io::Result<()> {
    let path = paths::avatar_path(root, persona_id, hash)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "not a hash"))?;
    if path.is_file() {
        return Ok(());
    }
    let dir = path.parent().expect("a picture lives in a directory");
    std::fs::create_dir_all(dir)?;
    let mut temporary = tempfile::NamedTempFile::new_in(dir)?;
    temporary.write_all(png)?;
    temporary.as_file().sync_all()?;
    temporary.persist(&path).map_err(|error| error.error)?;
    Ok(())
}

/// Removes the teammate's kept pictures, all but the one named.
pub(crate) fn remove_except(root: &Path, persona_id: &str, keep: Option<&str>) {
    let Ok(entries) = std::fs::read_dir(paths::avatar_dir(root, persona_id)) else {
        return;
    };
    let kept = keep.map(|hash| format!("{hash}.png"));
    for entry in entries.flatten() {
        if kept.as_deref().is_none_or(|kept| entry.file_name() != kept) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// One part of a kept picture, the way `file.read` answers one.
pub(crate) fn read(
    root: &Path,
    persona_id: &str,
    hash: &str,
    offset: i64,
) -> Result<crate::contract::FileChunk, String> {
    let path = paths::avatar_path(root, persona_id, hash)
        .ok_or_else(|| "A picture is named by 64 lowercase hex digits.".to_string())?;
    sent::read_path(&path, offset, true).map_err(|_| "That teammate has no such picture.".into())
}

/// The picture at most `edge` px wide, still transparent: what a phone
/// draws in a list row or beside a bubble.
pub(crate) async fn read_thumb(
    root: &Path,
    persona_id: &str,
    hash: &str,
    offset: i64,
    edge: u32,
) -> Result<crate::contract::FileChunk, String> {
    let path = paths::avatar_path(root, persona_id, hash)
        .ok_or_else(|| "A picture is named by 64 lowercase hex digits.".to_string())?;
    let copy = crate::thumbs::copy_of(root, &path, edge, crate::thumbs::Kind::Face)
        .await
        .map_err(|_| "That teammate has no such picture.".to_string())?;
    sent::read_path(&copy, offset, true).map_err(|_| "That teammate has no such picture.".into())
}

/// The picture as it is kept: the subject of a transparent image found and
/// given a little room, then centred on a transparent square and made 512
/// pixels wide, as a PNG.
fn square(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let not_an_image = || "The picture must be a PNG, JPEG or WebP image.".to_string();
    if bytes.len() > MAX_BYTES {
        return Err("The picture can be at most 20 MB.".into());
    }
    let format = image::guess_format(bytes).map_err(|_| not_an_image())?;
    if !matches!(
        format,
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP
    ) {
        return Err(not_an_image());
    }
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SOURCE_EDGE);
    limits.max_image_height = Some(MAX_SOURCE_EDGE);
    limits.max_alloc = Some(320 * 1024 * 1024);
    reader.limits(limits);
    use image::ImageDecoder;
    let mut decoder = reader.into_decoder().map_err(|_| not_an_image())?;
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut decoded = DynamicImage::from_decoder(decoder)
        .map_err(|_| "The picture could not be decoded.".to_string())?;
    decoded.apply_orientation(orientation);
    let has_alpha = decoded.color().has_alpha();
    let mut rgba = decoded.to_rgba8();
    if has_alpha {
        let (left, top, right, bottom) = subject(&rgba)
            .ok_or_else(|| "The picture is empty: every pixel of it is transparent.".to_string())?;
        let (width, height) = (right - left + 1, bottom - top + 1);
        let padding = (f64::from(width.max(height)) * 0.06).round() as u32;
        let left = left.saturating_sub(padding);
        let top = top.saturating_sub(padding);
        let right = (right + padding).min(rgba.width() - 1);
        let bottom = (bottom + padding).min(rgba.height() - 1);
        rgba = image::imageops::crop_imm(&rgba, left, top, right - left + 1, bottom - top + 1)
            .to_image();
    }
    let side = rgba.width().max(rgba.height());
    let mut canvas = RgbaImage::new(side, side);
    image::imageops::overlay(
        &mut canvas,
        &rgba,
        i64::from((side - rgba.width()) / 2),
        i64::from((side - rgba.height()) / 2),
    );
    let resized =
        image::imageops::resize(&canvas, EDGE, EDGE, image::imageops::FilterType::Lanczos3);
    let mut png = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(resized)
        .write_to(&mut png, ImageFormat::Png)
        .map_err(|_| "The picture could not be encoded.".to_string())?;
    Ok(png.into_inner())
}

/// The box round everything that is not transparent, as inclusive
/// (left, top, right, bottom).
fn subject(image: &RgbaImage) -> Option<(u32, u32, u32, u32)> {
    let mut found: Option<(u32, u32, u32, u32)> = None;
    for (x, y, pixel) in image.enumerate_pixels() {
        if pixel[3] >= SUBJECT_ALPHA {
            found = Some(match found {
                None => (x, y, x, y),
                Some((left, top, right, bottom)) => {
                    (left.min(x), top.min(y), right.max(x), bottom.max(y))
                }
            });
        }
    }
    found
}

#[cfg(test)]
mod tests;
