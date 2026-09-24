//! A teammate hands the person a file (BRO-98).
//!
//! `send_file` takes the file from where the teammate has it — its
//! workspace, its computer, or its computer's screen — keeps the desk's own
//! copy of it ([`crate::sent`]), and puts it in the conversation as the
//! teammate's message, with the caption as the message's words. A phone
//! hears it the way it hears any reply.
//!
//! A quiet scheduled run is refused rather than demoted. Its words become
//! thinking, and a file in thinking would reach nobody while the teammate
//! believed it had been sent.

use super::{Room, lock, new_id, now_ms, quiet, stamped};
use crate::computer::{self, Ready};
use crate::contract::{Attachment, AttachmentKind, Persona, TranscriptEvent};
use crate::driver::CapabilityLease;
use crate::images::DECODERS;
use crate::sent::{self, Prepared};
use crate::tools::Workspace;
use std::io::Read;
use std::path::{Path, PathBuf};

/// The most a caption may say. More than this is a message of its own.
pub(crate) const MAX_CAPTION_CHARS: usize = 2000;

const QUIET: &str = "This is a quiet scheduled run, and nothing from it reaches the person, so the file was not sent. If they need it, send it when you are next talking with them.";

/// Where the file to send is.
pub(crate) enum Source {
    /// A path in the teammate's workspace, or anywhere under machine reach.
    Workspace(String),
    /// A path on its computer.
    Computer(String),
    /// Its computer's screen now, or one window of it, or one region.
    Screen {
        window: Option<String>,
        region: Option<[i64; 4]>,
    },
}

/// A file as it came off its source.
struct Taken {
    name: String,
    bytes: Vec<u8>,
    origin: String,
}

impl Room {
    /// Sends the person a file from `source`, and answers what the teammate
    /// is told: what was sent, or why nothing was.
    pub(crate) async fn send_file(
        &self,
        persona_id: &str,
        source: Source,
        caption: &str,
        capability: Option<CapabilityLease>,
    ) -> Result<String, String> {
        let persona = self.persona(persona_id)?;
        let caption = caption.trim();
        if caption.chars().count() > MAX_CAPTION_CHARS {
            return Err(format!(
                "A caption is at most {MAX_CAPTION_CHARS} characters; say the rest in your reply."
            ));
        }
        if self.is_quiet(persona_id) {
            return Err(QUIET.to_string());
        }
        let taken = match source {
            Source::Workspace(path) => {
                from_workspace(&persona, self.log.root(), &path, capability).await?
            }
            Source::Computer(path) => {
                let ready = self.computer_to_send_from(persona_id).await?;
                let path = computer::files::on_computer(&path);
                let name = file_name(&path);
                let bytes = computer::files::download(&ready, &path, sent::MAX_BYTES, |size| {
                    sent::too_large(&name, size)
                })
                .await?;
                Taken {
                    name,
                    bytes,
                    origin: format!("{path} on the computer"),
                }
            }
            Source::Screen { window, region } => {
                let ready = self.computer_to_send_from(persona_id).await?;
                let bytes = computer::files::screenshot(&ready, window.as_deref(), region).await?;
                Taken {
                    name: "screenshot.png".to_string(),
                    bytes,
                    origin: screen_origin(window.as_deref(), region),
                }
            }
        };
        let file = prepare(taken.name, taken.bytes).await?;
        let id = new_id();
        let root = self.log.root();
        let path = sent::store(root, persona_id, &id, &file)
            .map_err(|error| format!("The desk could not keep the file: {error}"))?;
        let event = TranscriptEvent::Agent {
            id: id.clone(),
            ts: now_ms(),
            text: caption.to_string(),
            attachments: Some(vec![Attachment {
                kind: file.kind,
                name: file.name.clone(),
                path: path.display().to_string(),
                mime_type: Some(file.mime_type.clone()),
                size: Some(file.bytes.len() as i64),
                width: file.dimensions.map(|(width, _)| width),
                height: file.dimensions.map(|(_, height)| height),
                origin: Some(taken.origin),
            }]),
            reactions: None,
            ring: None,
            receipt: None,
        };
        if let Err(refused) = self.post_file(persona_id, event) {
            sent::discard(root, persona_id, &id);
            return Err(refused);
        }
        let body = if caption.is_empty() {
            format!("Sent {}", file.name)
        } else {
            caption.to_string()
        };
        self.push.notify(&persona.name, &body, persona_id);
        Ok(sent_sentence(&file))
    }

    /// Whether a quiet scheduled run holds this teammate's voice right now.
    fn is_quiet(&self, persona_id: &str) -> bool {
        let session = lock(&self.sessions).get(persona_id).cloned();
        session.is_some_and(|session| quiet::mutes_deltas(lock(&session.quiet).as_ref(), now_ms()))
    }

    /// Writes the file's message through the same stamps and quiet window as
    /// every line the room writes down, and says whether it landed: the
    /// teammate is about to be told the person has the file.
    fn post_file(&self, persona_id: &str, event: TranscriptEvent) -> Result<(), String> {
        let session = lock(&self.sessions).get(persona_id).cloned();
        let event = match &session {
            Some(session) => stamped(session, event, now_ms()),
            None => event,
        };
        // A quiet window that opened while the file was fetched makes the
        // message a thought, which would carry no file.
        if !matches!(event, TranscriptEvent::Agent { .. }) {
            return Err(QUIET.to_string());
        }
        let event = serde_json::to_value(&event).map_err(|error| error.to_string())?;
        self.try_write_value(persona_id, &event)
            .map_err(|error| format!("The conversation could not be written to: {error}"))
    }

    async fn computer_to_send_from(&self, persona_id: &str) -> Result<Ready, String> {
        self.computers.running(persona_id).await.ok_or_else(|| {
            "Your computer is not running, so there is nothing on it to send. `computer_status` says where it is.".to_string()
        })
    }
}

/// A file from the teammate's workspace, found as its read tools find one.
async fn from_workspace(
    persona: &Persona,
    root: &Path,
    requested: &str,
    capability: Option<CapabilityLease>,
) -> Result<Taken, String> {
    let workspace = Workspace::open_with_capability(
        PathBuf::from(&persona.cwd),
        persona.reach.unwrap_or_default(),
        root.join("tool-output").join(&persona.id),
        capability,
    )
    .map_err(|error| error.to_string())?;
    let requested = requested.to_string();
    tokio::task::spawn_blocking(move || {
        let (file, size, path) = workspace
            .open_to_send(&requested)
            .map_err(|error| error.to_string())?;
        let name = file_name(&requested);
        if size > sent::MAX_BYTES {
            return Err(sent::too_large(&name, Some(size)));
        }
        let mut bytes = Vec::with_capacity(size as usize);
        file.take(sent::MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("{requested} could not be read: {error}"))?;
        if bytes.len() as u64 > sent::MAX_BYTES {
            return Err(sent::too_large(&name, None));
        }
        let origin = match path.strip_prefix(workspace.display_root()) {
            Ok(inside) => format!("{} in the workspace", inside.display()),
            Err(_) => path.display().to_string(),
        };
        Ok(Taken {
            name,
            bytes,
            origin,
        })
    })
    .await
    .map_err(|_| "The file could not be read.".to_string())?
}

/// Makes the file what it will be kept as, off the async workers: an image
/// is decoded and encoded again.
async fn prepare(name: String, bytes: Vec<u8>) -> Result<Prepared, String> {
    let permit = DECODERS
        .acquire()
        .await
        .map_err(|_| "The file could not be prepared.".to_string())?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        sent::prepare(&name, bytes)
    })
    .await
    .map_err(|_| "The file could not be prepared.".to_string())?
}

fn file_name(path: &str) -> String {
    Path::new(path.trim())
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Which part of the screen a screenshot shows. A region is what `capture`
/// takes when it is given both.
fn screen_origin(window: Option<&str>, region: Option<[i64; 4]>) -> String {
    match (window, region) {
        (_, Some([x, y, width, height])) => {
            format!("{width} × {height} of the computer's screen at {x}, {y}")
        }
        (Some(window), None) => format!("the window {window:?} on the computer's screen"),
        (None, None) => "the computer's screen".to_string(),
    }
}

/// What the teammate is told once the person has the file.
fn sent_sentence(file: &Prepared) -> String {
    let size = sent::size_text(file.bytes.len() as u64);
    match (file.kind, file.dimensions, file.unfit) {
        (AttachmentKind::Image, Some((width, height)), _) => {
            format!("Sent {} ({width} × {height}, {size}).", file.name)
        }
        (_, _, Some(unfit)) => format!(
            "Sent {} ({size}) as a file rather than a picture: {}.",
            file.name,
            unfit.plainly()
        ),
        _ => format!("Sent {} ({size}).", file.name),
    }
}
