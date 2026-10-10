//! How a teammate's face is drawn beyond its picture: the style, a video
//! clip per mood, and a drawing. Only the person sets a look, through the
//! wire; no teammate tool reaches it, so it grants a teammate nothing.
//!
//! Clips live in the teammate's picture directory as `<sha256>.mp4` or
//! `.webm`, and the drawing as `<sha256>.svg`, written once and never
//! changed, so a window caches one by hash. Replacing or clearing one removes
//! the file the look no longer names.
//!
//! A drawing is inlined by the window so CSS can reach its parts, which makes
//! it markup rather than a picture. The desk turns away what is plainly not a
//! drawing; the window's own cleaning is what keeps it inert.

use super::Room;
use crate::contract::{Clip, Look, LookStyle, Mood};
use crate::paths;
use crate::sent;
use base64::Engine;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::Path;

/// The most one clip may weigh: a few seconds of a small square loop.
const MAX_CLIP_BYTES: usize = 4 * 1024 * 1024;

/// The most a drawing may weigh: room for fur drawn strand by strand.
const MAX_DRAWING_BYTES: usize = 1024 * 1024;

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

impl Room {
    /// `look.set`: the style.
    pub(crate) async fn set_look(&self, persona_id: &str, style: LookStyle) -> Result<(), String> {
        let gate = self.policy_update_lock();
        let _held = gate.lock().await;
        let mut persona = self.persona(persona_id)?;
        let (clips, drawing) = persona
            .look
            .take()
            .map(|look| (look.clips, look.drawing))
            .unwrap_or_default();
        persona.look = Some(Look {
            style,
            clips,
            drawing,
            updated_at: now(),
        });
        persona.updated_at = super::now_ms();
        crate::room::append_persona(&self.log, &persona)
    }

    /// `look.clip_set`: one mood's video, kept under its hash.
    pub(crate) async fn set_clip(
        &self,
        persona_id: &str,
        mood: Mood,
        mime_type: &str,
        data: &str,
    ) -> Result<(), String> {
        if data.len() > MAX_CLIP_BYTES.div_ceil(3) * 4 {
            return Err("That clip is too large; keep a mood under 4 MB.".into());
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|_| "That clip could not be read.".to_string())?;
        if bytes.is_empty() || bytes.len() > MAX_CLIP_BYTES {
            return Err("That clip is too large; keep a mood under 4 MB.".into());
        }
        let root = self.log.root();
        let hash = hex::encode(Sha256::digest(&bytes));
        let path = mime_type
            .starts_with("video/")
            .then(|| paths::clip_path(root, persona_id, &hash, mime_type))
            .flatten()
            .ok_or_else(|| "A clip is an MP4 or a WebM video.".to_string())?;
        let gate = self.policy_update_lock();
        let _held = gate.lock().await;
        let mut persona = self.persona(persona_id)?;
        store(&path, &bytes).map_err(|_| "The clip could not be saved.".to_string())?;
        let mut look = persona.look.take().unwrap_or(Look {
            style: LookStyle::Video,
            clips: Default::default(),
            drawing: None,
            updated_at: now(),
        });
        look.clips.insert(
            mood,
            Clip {
                hash,
                mime_type: mime_type.to_string(),
            },
        );
        look.updated_at = now();
        persona.look = Some(look);
        persona.updated_at = super::now_ms();
        crate::room::append_persona(&self.log, &persona)?;
        remove_unused(root, persona_id, persona.look.as_ref());
        Ok(())
    }

    /// `look.clip_clear`: one mood without a clip, which then falls back.
    pub(crate) async fn clear_clip(&self, persona_id: &str, mood: Mood) -> Result<(), String> {
        let gate = self.policy_update_lock();
        let _held = gate.lock().await;
        let mut persona = self.persona(persona_id)?;
        let Some(look) = persona.look.as_mut() else {
            return Ok(());
        };
        if look.clips.remove(&mood).is_none() {
            return Ok(());
        }
        look.updated_at = now();
        persona.updated_at = super::now_ms();
        crate::room::append_persona(&self.log, &persona)?;
        remove_unused(self.log.root(), persona_id, persona.look.as_ref());
        Ok(())
    }

    /// `look.drawing_set`: the drawn face's SVG, kept under its hash.
    pub(crate) async fn set_drawing(&self, persona_id: &str, svg: &str) -> Result<(), String> {
        screen(svg)?;
        let root = self.log.root();
        let hash = hex::encode(Sha256::digest(svg.as_bytes()));
        let path = paths::clip_path(root, persona_id, &hash, "image/svg+xml")
            .expect("a digest is a genuine hash");
        let gate = self.policy_update_lock();
        let _held = gate.lock().await;
        let mut persona = self.persona(persona_id)?;
        store(&path, svg.as_bytes()).map_err(|_| "The drawing could not be saved.".to_string())?;
        let mut look = persona.look.take().unwrap_or(Look {
            style: LookStyle::Drawn,
            clips: Default::default(),
            drawing: None,
            updated_at: now(),
        });
        look.drawing = Some(Clip {
            hash,
            mime_type: "image/svg+xml".to_string(),
        });
        look.updated_at = now();
        persona.look = Some(look);
        persona.updated_at = super::now_ms();
        crate::room::append_persona(&self.log, &persona)?;
        remove_unused(root, persona_id, persona.look.as_ref());
        Ok(())
    }

    /// `look.drawing_clear`: the look without a drawing, which then draws as before.
    pub(crate) async fn clear_drawing(&self, persona_id: &str) -> Result<(), String> {
        let gate = self.policy_update_lock();
        let _held = gate.lock().await;
        let mut persona = self.persona(persona_id)?;
        let Some(look) = persona.look.as_mut() else {
            return Ok(());
        };
        if look.drawing.take().is_none() {
            return Ok(());
        }
        look.updated_at = now();
        persona.updated_at = super::now_ms();
        crate::room::append_persona(&self.log, &persona)?;
        remove_unused(self.log.root(), persona_id, persona.look.as_ref());
        Ok(())
    }
}

/// Turns away what is plainly not a drawing the window can use: too large, not
/// an SVG, or carrying what no face needs and an attacker would want.
fn screen(svg: &str) -> Result<(), String> {
    if svg.len() > MAX_DRAWING_BYTES {
        return Err("That drawing is too large; keep it under 1 MB.".into());
    }
    let lower = svg.to_ascii_lowercase();
    if !lower.contains("<svg") {
        return Err("A drawing is an SVG.".into());
    }
    if lower.contains("<!doctype") || lower.contains("<!entity") {
        return Err("A drawing may not declare a doctype or entities.".into());
    }
    if lower.contains("<script") || lower.contains("<foreignobject") {
        return Err("A drawing may not carry scripts or pages.".into());
    }
    Ok(())
}

/// Keeps a clip under its hash, unless it is already there.
fn store(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if path.is_file() {
        return Ok(());
    }
    let dir = path.parent().expect("a clip lives in a directory");
    std::fs::create_dir_all(dir)?;
    let mut temporary = tempfile::NamedTempFile::new_in(dir)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

/// Removes the clips and drawing the look no longer names; pictures are not its to touch.
fn remove_unused(root: &Path, persona_id: &str, look: Option<&Look>) {
    let Ok(entries) = std::fs::read_dir(paths::avatar_dir(root, persona_id)) else {
        return;
    };
    let used: Vec<&str> = look
        .map(|look| {
            look.clips
                .values()
                .chain(&look.drawing)
                .map(|clip| clip.hash.as_str())
                .collect()
        })
        .unwrap_or_default();
    for entry in entries.flatten() {
        let path = entry.path();
        let video = path.extension().is_some_and(|extension| {
            extension == "mp4" || extension == "webm" || extension == "svg"
        });
        let named = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .is_some_and(|stem| used.contains(&stem));
        if video && !named {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// One part of a kept clip or drawing, the way `avatar.read` answers a picture.
pub(crate) fn read(
    root: &Path,
    persona_id: &str,
    hash: &str,
    offset: i64,
) -> Result<crate::contract::FileChunk, String> {
    let missing = || "That teammate has no such clip.".to_string();
    let path = ["video/mp4", "video/webm", "image/svg+xml"]
        .into_iter()
        .filter_map(|mime_type| paths::clip_path(root, persona_id, hash, mime_type))
        .find(|path| path.is_file())
        .ok_or_else(missing)?;
    sent::read_path(&path, offset, false).map_err(|_| missing())
}

#[cfg(test)]
mod tests;
