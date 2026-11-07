//! A teammate's browser as a live screen, in the computer viewer's own
//! protocol, so a viewer that draws a teammate's computer draws its browser
//! too (task-390). The desk serves it at `/v2/browser/{persona}/ws` beside
//! `/v2/computer/{persona}/ws`, and the desktop's bridge at
//! `/browser/{persona}/ws`.
//!
//! What a viewer is sent: binary frames, each a 12-byte header of
//! little-endian u16s (x, y, width, height, screen width, screen height) and
//! a PNG of that rectangle of the screen ([`Frame`]), only for what changed;
//! and JSON text (`state`, `pointer`, `paste`, `error`, `pong`). What it
//! sends: JSON ([`FromViewer`]): `ping`, `control`, `move`, `button`,
//! `wheel`, `key` and `paste`, none of them answered by a picture, so input
//! never waits behind one.
//!
//! The desk answers `ping` and `control` itself, `control` as the browser's
//! Take control and Give it back, and passes the rest to the browser's
//! hands only while that seat drives. Between the desk and the server's
//! helper each viewer has a socket of its own (`connect`, Unix only), whose
//! messages are a kind byte ([`TEXT`] or [`BINARY`]), a little-endian u32
//! length and the bytes; its first message names the teammate ([`Hello`]).
//! A [`Link`] is one viewer's end, as the desk holds it: what the screen
//! shows, a message at a time, and where input goes. Its queue of what is
//! shown holds one message, so a viewer whose link is slow holds up the
//! browser's next picture rather than queueing old ones, and the browser
//! sends the changes since in one picture when the viewer is ready.

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

/// The bytes before a frame's PNG.
pub const HEADER: usize = 12;
/// The most a paste carries, as the computer takes it.
pub const PASTE_LIMIT: usize = 1_048_576;
/// The longest key name a viewer sends (`ArrowRight`, `AudioVolumeMute`).
pub const KEY_LIMIT: usize = 32;
/// The most one message between the desk and the helper may be: a whole
/// screen's PNG at the widest viewport, and a paste.
pub const MESSAGE_LIMIT: usize = 64 * 1024 * 1024;
/// A JSON message, on the socket between the desk and the helper.
pub const TEXT: u8 = 0;
/// A frame, on that socket.
pub const BINARY: u8 = 1;
/// What the desk tells the browser's hands when a seat stops driving, by
/// choice or by vanishing: let go of every key and button still down.
pub const RELEASE: &str = r#"{"t":"release"}"#;

/// A rectangle of the screen, in its pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl Rect {
    pub fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    fn right(self) -> u32 {
        u32::from(self.x) + u32::from(self.width)
    }

    fn bottom(self) -> u32 {
        u32::from(self.y) + u32::from(self.height)
    }

    /// The smallest rectangle holding both: what two changes cost as one
    /// picture.
    pub fn union(self, other: Self) -> Self {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Self {
            x,
            y,
            width: (self.right().max(other.right()) - u32::from(x)) as u16,
            height: (self.bottom().max(other.bottom()) - u32::from(y)) as u16,
        }
    }

    /// The part of this inside a screen `width` by `height`, if any.
    pub fn clip(self, width: u16, height: u16) -> Option<Self> {
        let right = self.right().min(u32::from(width));
        let bottom = self.bottom().min(u32::from(height));
        (right > u32::from(self.x) && bottom > u32::from(self.y)).then(|| Self {
            x: self.x,
            y: self.y,
            width: (right - u32::from(self.x)) as u16,
            height: (bottom - u32::from(self.y)) as u16,
        })
    }

    pub fn intersects(self, other: Self) -> bool {
        u32::from(self.x) < other.right()
            && u32::from(other.x) < self.right()
            && u32::from(self.y) < other.bottom()
            && u32::from(other.y) < self.bottom()
    }
}

/// One changed rectangle of a screen `screen` big, as a PNG.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub rect: Rect,
    pub screen: (u16, u16),
    pub png: Vec<u8>,
}

impl Frame {
    /// The wire form: six little-endian u16 (x, y, width, height, screen
    /// width, screen height), then the PNG.
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(HEADER + self.png.len());
        for value in [
            self.rect.x,
            self.rect.y,
            self.rect.width,
            self.rect.height,
            self.screen.0,
            self.screen.1,
        ] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&self.png);
        bytes
    }

    /// Where an encoded frame goes and how big its screen is, when it is
    /// one: the header, and a rectangle inside the screen.
    pub fn header(bytes: &[u8]) -> Option<(Rect, (u16, u16))> {
        let field = |at: usize| Some(u16::from_le_bytes([*bytes.get(at)?, *bytes.get(at + 1)?]));
        let rect = Rect::new(field(0)?, field(2)?, field(4)?, field(6)?);
        let screen = (field(8)?, field(10)?);
        (!rect.is_empty() && rect.clip(screen.0, screen.1) == Some(rect)).then_some((rect, screen))
    }
}

/// What a viewer says, as the computer's viewer says it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "lowercase", deny_unknown_fields)]
pub enum FromViewer {
    /// Liveness, answered `{"t":"pong","id":…}`; it never takes or keeps
    /// control.
    Ping { id: u64 },
    /// Take control (`take`), or give it back.
    Control { take: bool },
    /// The pointer to a point on the screen.
    Move { x: i16, y: i16 },
    /// A pointer button (1 left, 2 middle, 3 right) down or up where the
    /// pointer is.
    Button { b: u8, down: bool },
    /// One notch: `dy` −1 is up and 1 is down; `dx` −1 is left and 1 right.
    Wheel {
        #[serde(default)]
        dx: i8,
        #[serde(default)]
        dy: i8,
    },
    /// Text from the viewer's own clipboard, pasted where the focus is.
    Paste { text: String },
    /// A key as a browser names it (`KeyboardEvent.key`), down or up.
    Key { key: String, down: bool },
}

impl FromViewer {
    /// Reads one message, refusing one out of the protocol's bounds.
    pub fn parse(text: &str) -> Result<Self, String> {
        let message: Self =
            serde_json::from_str(text).map_err(|_| "invalid viewer input".to_string())?;
        match &message {
            Self::Paste { text } if text.len() > PASTE_LIMIT => Err("Paste exceeds 1 MiB".into()),
            Self::Paste { text } if text.contains('\0') => {
                Err("Clipboard text contains a NUL byte".into())
            }
            Self::Key { key, .. } if key.is_empty() || key.len() > KEY_LIMIT => {
                Err("invalid viewer input".into())
            }
            Self::Button { b, .. } if !(1..=3).contains(b) => Err("invalid viewer input".into()),
            Self::Wheel { dx, dy } if dx.abs() > 1 || dy.abs() > 1 => {
                Err("invalid viewer input".into())
            }
            _ => Ok(message),
        }
    }

    /// Whether this moves the page: what only a seat that drives sends on.
    pub fn is_input(&self) -> bool {
        !matches!(self, Self::Ping { .. } | Self::Control { .. })
    }
}

/// The first message a viewer's socket to the helper carries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub persona: String,
}

/// What a teammate's screen shows a viewer: a frame, encoded, or a JSON
/// message (the person's pointer, a paste done, an error).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shown {
    Frame(Vec<u8>),
    Text(String),
}

/// One viewer's end of a teammate's screen. Dropping it ends the viewer.
pub struct Link {
    /// What the screen shows, one message waiting at most.
    pub shown: mpsc::Receiver<Shown>,
    /// The viewer's input for the browser's hands, as JSON, and
    /// [`RELEASE`].
    pub input: mpsc::Sender<String>,
}

#[cfg(unix)]
pub use socket::{connect, read_message, write_message};

#[cfg(unix)]
mod socket {
    use super::{BINARY, Hello, Link, MESSAGE_LIMIT, Shown, TEXT};
    use std::path::Path;
    use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
    use tokio::net::UnixStream;
    use tokio::sync::mpsc;

    /// Writes one message: its kind, its length and its bytes.
    pub async fn write_message<W: AsyncWrite + Unpin>(
        write: &mut W,
        kind: u8,
        bytes: &[u8],
    ) -> std::io::Result<()> {
        let length = u32::try_from(bytes.len())
            .ok()
            .filter(|length| *length as usize <= MESSAGE_LIMIT)
            .ok_or_else(|| std::io::Error::other("a screen message too long to send"))?;
        let mut head = [0u8; 5];
        head[0] = kind;
        head[1..].copy_from_slice(&length.to_le_bytes());
        write.write_all(&head).await?;
        write.write_all(bytes).await?;
        write.flush().await
    }

    /// Reads one message, or none at the end of the stream. A message of
    /// another kind or past the limit ends it as an error.
    pub async fn read_message<R: AsyncRead + Unpin>(
        read: &mut R,
    ) -> std::io::Result<Option<(u8, Vec<u8>)>> {
        let mut head = [0u8; 5];
        match read.read_exact(&mut head).await {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(error) => return Err(error),
        }
        let kind = head[0];
        let length = u32::from_le_bytes([head[1], head[2], head[3], head[4]]) as usize;
        if !matches!(kind, TEXT | BINARY) || length > MESSAGE_LIMIT {
            return Err(std::io::Error::other("not a screen message"));
        }
        let mut bytes = vec![0u8; length];
        read.read_exact(&mut bytes).await?;
        Ok(Some((kind, bytes)))
    }

    /// Opens a viewer's socket to the helper's screens at `path`, for
    /// `persona`'s browser: a task reads what it shows into the link, at
    /// most one message ahead of the viewer, and another writes the
    /// viewer's input. Either ends when the link is dropped or the helper
    /// closes the socket.
    pub async fn connect(path: &Path, persona: &str) -> std::io::Result<Link> {
        let stream = UnixStream::connect(path).await?;
        let (mut read, mut write) = stream.into_split();
        let hello = serde_json::to_vec(&Hello {
            persona: persona.to_string(),
        })
        .map_err(std::io::Error::other)?;
        write_message(&mut write, TEXT, &hello).await?;
        let (shown_to, shown) = mpsc::channel(1);
        let (input, mut inputs) = mpsc::channel::<String>(64);
        tokio::spawn(async move {
            while let Ok(Some((kind, bytes))) = read_message(&mut read).await {
                let message = if kind == BINARY {
                    Shown::Frame(bytes)
                } else {
                    match String::from_utf8(bytes) {
                        Ok(text) => Shown::Text(text),
                        Err(_) => break,
                    }
                };
                if shown_to.send(message).await.is_err() {
                    break;
                }
            }
        });
        tokio::spawn(async move {
            while let Some(text) = inputs.recv().await {
                if write_message(&mut write, TEXT, text.as_bytes())
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        Ok(Link { shown, input })
    }
}

#[cfg(test)]
#[path = "screen_tests.rs"]
mod tests;
