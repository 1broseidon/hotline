//! Each teammate's browser as a live screen, the way the computer streams
//! its own (hotline-computer, `src/screen.rs`, George's, MIT/Apache): X
//! DAMAGE says which rectangles of the page changed, so an idle page costs
//! nothing and a busy one only the pixels that moved; each viewer is sent
//! those rectangles as PNGs, read through MIT-SHM, in the computer viewer's
//! protocol (`hotline_browser_core::screen`).
//!
//! Every teammate's page is a window of its own on the helper's one display
//! (`display.rs`), at its origin and the size of the page, with a popup the
//! page opened on top of it. Composite keeps each window's pixels in a
//! pixmap of its own (`redirect`), so a page under another is still whole:
//! a teammate's screen is its front window (its popup while one is open,
//! its page otherwise), and its damage and pixels are that window's alone.
//! Watching one teammate never shows or wakes for another's page.
//!
//! A display of its own a teammate would have isolated their pointers and
//! keyboards too, at about 10 MB a display, but WebKitGTK makes its cursors
//! on GTK's default display, and GTK aborts when one is set on a window of
//! another. So the one display's pointer and keyboard are shared: the
//! person's hands raise and focus the window of the page they drive before
//! each gesture (`Hands`), so their keys go to that page and no other, and
//! the teammates' own acts never use them (they are the page's own events,
//! `linux::input`). Two people driving two teammates' pages at the same
//! moment would take the pointer from each other's hover; only the owner
//! drives.
//!
//! Each viewer keeps the changes it has not yet been sent as one rectangle
//! ([`Watcher`]): when its socket is ready it is sent their bounding box,
//! read and encoded then, at most every [`INTERVAL`]. A viewer whose link
//! is slow is never queued pictures it will be shown too late; it is sent
//! what changed since its last one, in one. Damage arrives on one X
//! connection and pixels are read on another (each viewer's own), as the
//! computer learned to: reading on the connection that receives damage can
//! land an event inside a reply.
//!
//! The person's input goes to the display's pointer and keyboard with
//! XTEST (`xtest.rs`), from one thread, in order, and only while the person
//! has that teammate's browser (`taken`): the desk passes input on only
//! from a seat that drives, and this is checked again here.

use crate::xtest::Hands;
use hotline_browser_core::screen::{
    BINARY, Frame, FromViewer, Hello, RELEASE, Rect, TEXT, read_message, write_message,
};
use std::collections::HashMap;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{Duration, Instant};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Notify, broadcast, mpsc, watch};
use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::Event;
use x11rb::protocol::composite::{ConnectionExt as _, Redirect};
use x11rb::protocol::damage::{ConnectionExt as _, ReportLevel};
use x11rb::protocol::shm::ConnectionExt as _;
use x11rb::protocol::xproto::{ConnectionExt as _, ImageFormat, Window};
use x11rb::rust_connection::RustConnection;

/// The shortest time between two pictures to one viewer: about sixty a
/// second, if they can be read, encoded and sent that fast.
const INTERVAL: Duration = Duration::from_millis(16);
/// How long a window's damage is waited on at once; a closed one is found
/// by then.
const DAMAGE_WAIT: Duration = Duration::from_secs(1);
/// After a window's damage connection failed, before it is watched again.
const REOPEN_DELAY: Duration = Duration::from_secs(1);
/// How long a viewer's socket has to name its teammate.
const HELLO_WAIT: Duration = Duration::from_secs(5);
/// The largest page a viewer's shared memory holds: the widest viewport.
const LARGEST: (u16, u16) = (3840, 2400);

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What sets the display's clipboard, which is GTK's to own: the text.
pub type Clipboard = fn(String) -> Result<(), String>;

/// One teammate's screen: the window in front of its page, its size, the
/// viewers watching it, and the display's hands.
pub struct Screen {
    name: String,
    /// The X window shown: the page's, or a popup's over it.
    window: Window,
    /// The window's size: the rectangle streamed.
    size: Mutex<(u16, u16)>,
    watchers: Mutex<Vec<Weak<Watcher>>>,
    hands: Arc<HandsThread>,
    /// Where the person's pointer went, as `pointer` messages, for every
    /// viewer: the frames never show a cursor.
    pointer: broadcast::Sender<String>,
}

/// One viewer's changes not yet sent, and its wake-up.
pub struct Watcher {
    dirty: Mutex<Option<Rect>>,
    wake: Notify,
}

impl Watcher {
    fn add(&self, rect: Rect) {
        let mut dirty = lock(&self.dirty);
        *dirty = Some(dirty.map_or(rect, |was| was.union(rect)));
        drop(dirty);
        self.wake.notify_one();
    }

    /// What changed since this viewer's last picture, inside the page.
    fn take(&self, size: (u16, u16)) -> Option<Rect> {
        lock(&self.dirty).take()?.clip(size.0, size.1)
    }
}

impl Screen {
    pub fn size(&self) -> (u16, u16) {
        *lock(&self.size)
    }

    /// The window is `width` by `height` now: every viewer is sent all of it.
    pub fn resize(&self, width: u16, height: u16) {
        *lock(&self.size) = (width, height);
        self.all_changed();
    }

    fn all_changed(&self) {
        let (width, height) = self.size();
        self.changed(Rect::new(0, 0, width, height));
    }

    fn changed(&self, rect: Rect) {
        let mut watchers = lock(&self.watchers);
        watchers.retain(|watcher| match watcher.upgrade() {
            Some(watcher) => {
                watcher.add(rect);
                true
            }
            None => false,
        });
    }

    /// A new viewer, sent the whole page first.
    fn watch(&self) -> Arc<Watcher> {
        let (width, height) = self.size();
        let watcher = Arc::new(Watcher {
            dirty: Mutex::new(None),
            wake: Notify::new(),
        });
        watcher.add(Rect::new(0, 0, width, height));
        lock(&self.watchers).push(Arc::downgrade(&watcher));
        watcher
    }

    fn input(self: &Arc<Self>, message: FromViewer, reply: mpsc::Sender<String>) {
        self.hands.send(Gesture::Input {
            screen: Arc::downgrade(self),
            message,
            reply,
        });
    }
}

/// Watches a window's damage for as long as its screen is wanted, opening
/// the connection again after it fails, with every viewer sent all of the
/// page after.
fn damage(screen: &Weak<Screen>) {
    loop {
        let Some((name, window)) = screen
            .upgrade()
            .map(|screen| (screen.name.clone(), screen.window))
        else {
            return;
        };
        if let Err(error) = watch_damage(&name, window, screen) {
            eprintln!("[browser] screen {window:#x}: {error}; watching it again");
        }
        std::thread::sleep(REOPEN_DELAY);
        match screen.upgrade() {
            Some(screen) => screen.all_changed(),
            None => return,
        }
    }
}

fn watch_damage(name: &str, window: Window, screen: &Weak<Screen>) -> Result<(), String> {
    let (connection, _) =
        x11rb::connect(Some(name)).map_err(|error| format!("x11 connect: {error}"))?;
    connection
        .damage_query_version(1, 1)
        .map_err(|error| error.to_string())?
        .reply()
        .map_err(|error| format!("DAMAGE: {error}"))?;
    let damage = connection
        .generate_id()
        .map_err(|error| error.to_string())?;
    connection
        .damage_create(damage, window, ReportLevel::BOUNDING_BOX)
        .map_err(|error| error.to_string())?
        .check()
        .map_err(|error| format!("the page's window is gone: {error}"))?;
    let fd = connection.stream().as_raw_fd();
    loop {
        let mut area: Option<Rect> = None;
        while let Some(event) = connection
            .poll_for_event()
            .map_err(|error| format!("display connection lost: {error}"))?
        {
            if let Event::DamageNotify(notify) = event {
                let rect = rect_of(notify.area);
                area = Some(area.map_or(rect, |was| was.union(rect)));
            }
        }
        if let Some(area) = area {
            // Cleared before anything is read: what is drawn from here on is
            // reported again.
            connection
                .damage_subtract(damage, x11rb::NONE, x11rb::NONE)
                .map_err(|error| error.to_string())?;
            connection.flush().map_err(|error| error.to_string())?;
            let Some(screen) = screen.upgrade() else {
                return Ok(());
            };
            screen.changed(area);
        }
        let mut ready = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one descriptor, the connection's, open while it is.
        unsafe { libc::poll(&raw mut ready, 1, DAMAGE_WAIT.as_millis() as libc::c_int) };
        if screen.strong_count() == 0 {
            return Ok(());
        }
    }
}

fn rect_of(area: x11rb::protocol::xproto::Rectangle) -> Rect {
    let x = area.x.max(0) as u16;
    let y = area.y.max(0) as u16;
    let width = (i32::from(area.x) + i32::from(area.width) - i32::from(x)).max(0) as u16;
    let height = (i32::from(area.y) + i32::from(area.height) - i32::from(y)).max(0) as u16;
    Rect::new(x, y, width, height)
}

/// What the display's hands are asked: a viewer's input on a teammate's
/// screen, with where to answer it, or to let go of everything.
enum Gesture {
    Input {
        screen: Weak<Screen>,
        message: FromViewer,
        reply: mpsc::Sender<String>,
    },
    Release,
}

/// The display's one pointer and keyboard, on a thread of their own that
/// starts at the first gesture: each gesture in order.
pub struct HandsThread {
    name: String,
    clipboard: Clipboard,
    sender: Mutex<Option<std::sync::mpsc::Sender<Gesture>>>,
}

impl HandsThread {
    fn send(&self, gesture: Gesture) {
        let mut sender = lock(&self.sender);
        if sender.is_none() {
            let (made, gestures) = std::sync::mpsc::channel();
            let (name, clipboard) = (self.name.clone(), self.clipboard);
            let started = std::thread::Builder::new()
                .name("hands".into())
                .spawn(move || drive(&name, clipboard, &gestures));
            if started.is_err() {
                return;
            }
            *sender = Some(made);
        }
        if let Some(sender) = sender.as_ref() {
            let _ = sender.send(gesture);
        }
    }
}

fn drive(name: &str, clipboard: Clipboard, gestures: &std::sync::mpsc::Receiver<Gesture>) {
    let mut hands: Option<Hands> = None;
    while let Ok(gesture) = gestures.recv() {
        if hands.is_none() {
            match Hands::new(name) {
                Ok(made) => hands = Some(made),
                Err(error) => {
                    if let Gesture::Input { reply, .. } = gesture {
                        let _ = reply.try_send(error_message(&format!(
                            "The browser's hands could not start: {error}"
                        )));
                    }
                    continue;
                }
            }
        }
        let Some(held) = hands.as_mut() else {
            continue;
        };
        let (screen, message, reply) = match gesture {
            Gesture::Release => {
                let _ = held.release_all();
                continue;
            }
            Gesture::Input {
                screen,
                message,
                reply,
            } => (screen, message, reply),
        };
        let Some(screen) = screen.upgrade() else {
            continue;
        };
        let done = held.front(screen.window).and_then(|()| match message {
            FromViewer::Move { x, y } => {
                let (width, height) = screen.size();
                let most = |side: u16, of: u16| (side.min(of) as i16).saturating_sub(1).max(0);
                let x = x.clamp(0, most(width, crate::display::SIZE.0));
                let y = y.clamp(0, most(height, crate::display::SIZE.1));
                let _ = screen
                    .pointer
                    .send(serde_json::json!({"t": "pointer", "x": x, "y": y}).to_string());
                held.move_to(x, y)
            }
            FromViewer::Button { b, down } => held.button(b, down),
            FromViewer::Wheel { dx, dy } => held.wheel(dx, dy),
            FromViewer::Key { key, down } => held.key(&key, down),
            FromViewer::Paste { text } => clipboard(text).and_then(|()| {
                for key in ["Control", "Meta", "Shift", "Alt"] {
                    held.key(key, false)?;
                }
                held.combo("ctrl+v")?;
                let _ = reply.try_send(r#"{"t":"paste","ok":true}"#.to_string());
                Ok(())
            }),
            FromViewer::Ping { .. } | FromViewer::Control { .. } => Ok(()),
        });
        if let Err(error) = done {
            let _ = reply.try_send(error_message(&error));
        }
    }
}

fn error_message(error: &str) -> String {
    serde_json::json!({"t": "error", "error": error, "driving": true}).to_string()
}

/// One teammate's place in the streams: its screen, if its page is open,
/// which changes when a popup opens or closes over it, and whether the
/// person has its browser.
pub struct Entry {
    screen: watch::Sender<Option<Arc<Screen>>>,
    taken: AtomicBool,
}

/// Every teammate's screen on the display, and what they share: the
/// display's hands, and the connection that keeps each window's pixels its
/// own.
pub struct Screens {
    name: String,
    folder: PathBuf,
    hands: Arc<HandsThread>,
    entries: Mutex<HashMap<String, Arc<Entry>>>,
    /// Holds every page's window redirected to a pixmap of its own for as
    /// long as the helper runs: Composite lets go when this closes.
    _redirect: RustConnection,
}

impl Screens {
    /// The screens of display `number`, whose private folder is `folder`.
    pub fn new(number: u32, folder: &Path, clipboard: Clipboard) -> Result<Self, String> {
        let name = format!(":{number}");
        let (redirect, screen_number) =
            x11rb::connect(Some(&name)).map_err(|error| format!("x11 connect: {error}"))?;
        let root = redirect.setup().roots[screen_number].root;
        redirect
            .composite_query_version(0, 4)
            .map_err(|error| error.to_string())?
            .reply()
            .map_err(|error| format!("Composite: {error}"))?;
        redirect
            .composite_redirect_subwindows(root, Redirect::AUTOMATIC)
            .map_err(|error| error.to_string())?
            .check()
            .map_err(|error| format!("Composite: {error}"))?;
        Ok(Self {
            hands: Arc::new(HandsThread {
                name: name.clone(),
                clipboard,
                sender: Mutex::default(),
            }),
            name,
            folder: folder.to_path_buf(),
            entries: Mutex::default(),
            _redirect: redirect,
        })
    }

    /// Where viewers' sockets are opened.
    pub fn socket(&self) -> PathBuf {
        self.folder.join("screen")
    }

    /// A screen of X window `window`, `size` big.
    pub fn screen(&self, window: Window, size: (u16, u16)) -> Arc<Screen> {
        let screen = Arc::new(Screen {
            name: self.name.clone(),
            window,
            size: Mutex::new(size),
            watchers: Mutex::default(),
            hands: self.hands.clone(),
            pointer: broadcast::channel(256).0,
        });
        let watched = Arc::downgrade(&screen);
        let _ = std::thread::Builder::new()
            .name(format!("damage {window:#x}"))
            .spawn(move || damage(&watched));
        screen
    }

    fn entry(&self, persona: &str) -> Arc<Entry> {
        lock(&self.entries)
            .entry(persona.to_string())
            .or_insert_with(|| {
                Arc::new(Entry {
                    screen: watch::channel(None).0,
                    taken: AtomicBool::new(false),
                })
            })
            .clone()
    }

    /// The teammate's front window is `screen` now, or its page has closed
    /// (`None`): its viewers follow.
    pub fn show(&self, persona: &str, screen: Option<Arc<Screen>>) {
        let entry = self.entry(persona);
        if screen.is_none() {
            entry.taken.store(false, Ordering::SeqCst);
        }
        entry.screen.send_replace(screen);
    }

    /// The person took the teammate's browser (`true`) or handed it back:
    /// their input reaches its page only in between, and handing it back
    /// lets go of whatever their hands held.
    pub fn taken(&self, persona: &str, taken: bool) {
        let entry = self.entry(persona);
        let was = entry.taken.swap(taken, Ordering::SeqCst);
        if was && !taken {
            self.hands.send(Gesture::Release);
        }
    }
}

/// Answers viewers' sockets on `listener` until the helper stops.
pub async fn serve(listener: UnixListener, screens: Arc<Screens>) {
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        tokio::spawn(viewer(stream, screens.clone()));
    }
}

/// One viewer: its teammate's screen as it changes, and its input to the
/// screen's hands, until either side hangs up.
async fn viewer(stream: UnixStream, screens: Arc<Screens>) {
    let (mut read, mut write) = stream.into_split();
    let Ok(Ok(Some((TEXT, hello)))) =
        tokio::time::timeout(HELLO_WAIT, read_message(&mut read)).await
    else {
        return;
    };
    let Ok(Hello { persona }) = serde_json::from_slice(&hello) else {
        return;
    };
    let entry = screens.entry(&persona);
    let mut shown = entry.screen.subscribe();
    let (answer, mut answers) = mpsc::channel::<String>(16);
    let mut input = tokio::spawn(take_input(read, entry.clone(), answer));
    'screens: loop {
        let Some(screen) = shown.borrow_and_update().clone() else {
            tokio::select! {
                changed = shown.changed() => if changed.is_err() { break; },
                _ = &mut input => break,
                Some(text) = answers.recv() => {
                    if write_message(&mut write, TEXT, text.as_bytes()).await.is_err() { break; }
                }
            }
            continue;
        };
        let watcher = screen.watch();
        let mut pointer = screen.pointer.subscribe();
        let mut capture: Option<Capture> = None;
        let mut last = Instant::now()
            .checked_sub(INTERVAL)
            .unwrap_or_else(Instant::now);
        loop {
            tokio::select! {
                changed = shown.changed() => {
                    if changed.is_err() { break 'screens; }
                    continue 'screens;
                }
                _ = &mut input => break 'screens,
                () = watcher.wake.notified() => {
                    let wait = INTERVAL.saturating_sub(last.elapsed());
                    if !wait.is_zero() {
                        tokio::time::sleep(wait).await;
                    }
                    let size = screen.size();
                    let Some(rect) = watcher.take(size) else { continue; };
                    last = Instant::now();
                    let (name, window, kept) = (screen.name.clone(), screen.window, capture.take());
                    let read = tokio::task::spawn_blocking(move || {
                        let mut capture = match kept {
                            Some(capture) => capture,
                            None => Capture::open(&name, window)?,
                        };
                        let frame = capture.frame(rect, size)?;
                        Ok::<_, String>((capture, frame))
                    }).await;
                    match read {
                        Ok(Ok((kept, frame))) => {
                            capture = Some(kept);
                            if write_message(&mut write, BINARY, &frame.encode()).await.is_err() {
                                            break 'screens;
                            }
                        }
                        Ok(Err(error)) => {
                            eprintln!("[browser] screen {:#x}: {error}", screen.window);
                            tokio::time::sleep(REOPEN_DELAY).await;
                            watcher.add(Rect::new(0, 0, size.0, size.1));
                        }
                        Err(_) => break 'screens,
                    }
                }
                said = pointer.recv() => match said {
                    Ok(text) => if write_message(&mut write, TEXT, text.as_bytes()).await.is_err() { break 'screens; },
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break 'screens,
                },
                Some(text) = answers.recv() => {
                    if write_message(&mut write, TEXT, text.as_bytes()).await.is_err() { break 'screens; }
                }
            }
        }
    }
    input.abort();
}

/// The viewer's input, as the desk passes it on: to the hands of the
/// screen the teammate's page is on now, only while the person has it.
async fn take_input(
    mut read: tokio::net::unix::OwnedReadHalf,
    entry: Arc<Entry>,
    answer: mpsc::Sender<String>,
) {
    while let Ok(Some((kind, bytes))) = read_message(&mut read).await {
        if kind != TEXT {
            return;
        }
        let Ok(text) = String::from_utf8(bytes) else {
            return;
        };
        let Some(screen) = entry.screen.borrow().clone() else {
            continue;
        };
        if text == RELEASE {
            screen.hands.send(Gesture::Release);
            continue;
        }
        let message = match FromViewer::parse(&text) {
            Ok(message) if message.is_input() => message,
            Ok(_) => continue,
            Err(error) => {
                let _ = answer.try_send(error_message(&error));
                continue;
            }
        };
        if !entry.taken.load(Ordering::SeqCst) {
            if matches!(message, FromViewer::Paste { .. }) {
                let _ = answer.try_send(
                    serde_json::json!({"t": "error", "error": "Take control before pasting", "driving": false})
                        .to_string(),
                );
            }
            continue;
        }
        screen.input(message, answer.clone());
    }
}

/// A viewer's own connection to the display, pixels only, of the one
/// window it watches: it selects no events, so nothing lands inside a reply.
struct Capture {
    connection: RustConnection,
    window: Window,
    shared: Option<Shared>,
}

impl Capture {
    fn open(name: &str, window: Window) -> Result<Self, String> {
        let (connection, _) =
            x11rb::connect(Some(name)).map_err(|error| format!("x11 connect: {error}"))?;
        // Only what is touched of it is ever memory.
        let bytes = usize::from(LARGEST.0) * usize::from(LARGEST.1) * 4;
        let shared = match Shared::attach(&connection, bytes) {
            Ok(shared) => Some(shared),
            Err(error) => {
                eprintln!(
                    "[browser] screen {name}: MIT-SHM unavailable ({error}); reading it over the socket"
                );
                None
            }
        };
        Ok(Self {
            connection,
            window,
            shared,
        })
    }

    /// `rect` of the page `size` big, as a frame.
    fn frame(&mut self, rect: Rect, size: (u16, u16)) -> Result<Frame, String> {
        let (x, y) = (rect.x as i16, rect.y as i16);
        let (depth, data): (u8, &[u8]);
        let owned;
        if let Some(shared) = &self.shared {
            let reply = self
                .connection
                .shm_get_image(
                    self.window,
                    x,
                    y,
                    rect.width,
                    rect.height,
                    u32::MAX,
                    u8::from(ImageFormat::Z_PIXMAP),
                    shared.segment,
                    0,
                )
                .map_err(|error| format!("ShmGetImage: {error}"))?
                .reply()
                .map_err(|error| format!("ShmGetImage: {error}"))?;
            let size = reply.size as usize;
            if size > shared.len {
                return Err(format!(
                    "ShmGetImage wrote {size} bytes into a {} byte segment",
                    shared.len
                ));
            }
            depth = reply.depth;
            // SAFETY: the server has finished writing when its reply
            // arrives; the mapping is this viewer's alone until its next read.
            data = unsafe { std::slice::from_raw_parts(shared.map, size) };
        } else {
            let reply = self
                .connection
                .get_image(
                    ImageFormat::Z_PIXMAP,
                    self.window,
                    x,
                    y,
                    rect.width,
                    rect.height,
                    u32::MAX,
                )
                .map_err(|error| format!("GetImage: {error}"))?
                .reply()
                .map_err(|error| format!("GetImage: {error}"))?;
            depth = reply.depth;
            owned = reply.data;
            data = &owned;
        }
        let rgb = unpack(self.connection.setup(), depth, rect, data)?;
        Ok(Frame {
            rect,
            screen: size,
            png: encode_png(rect, &rgb)?,
        })
    }
}

/// ZPixmap bytes at the server's depth, as RGB.
fn unpack(
    setup: &x11rb::protocol::xproto::Setup,
    depth: u8,
    rect: Rect,
    data: &[u8],
) -> Result<Vec<u8>, String> {
    let format = setup
        .pixmap_formats
        .iter()
        .find(|format| format.depth == depth)
        .ok_or_else(|| format!("x11: no pixel format for depth {depth}"))?;
    let bytes_per_pixel = usize::from(format.bits_per_pixel / 8);
    if bytes_per_pixel < 3 {
        return Err(format!(
            "x11: unsupported {} bits per pixel",
            format.bits_per_pixel
        ));
    }
    let width = usize::from(rect.width);
    let height = usize::from(rect.height);
    let stride = (width * usize::from(format.bits_per_pixel)).div_ceil(32) * 4;
    if data.len() < stride * height {
        return Err("x11: image reply was shorter than the rectangle".to_owned());
    }
    let mut rgb = Vec::with_capacity(width * height * 3);
    for row in data.chunks_exact(stride).take(height) {
        for pixel in row[..width * bytes_per_pixel].chunks_exact(bytes_per_pixel) {
            rgb.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
        }
    }
    Ok(rgb)
}

fn encode_png(rect: Rect, rgb: &[u8]) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::with_capacity(rgb.len() / 4);
    let mut encoder = png::Encoder::new(
        std::io::Cursor::new(&mut bytes),
        u32::from(rect.width),
        u32::from(rect.height),
    );
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder
        .write_header()
        .map_err(|error| format!("encode PNG: {error}"))?;
    writer
        .write_image_data(rgb)
        .map_err(|error| format!("encode PNG: {error}"))?;
    drop(writer);
    Ok(bytes)
}

/// A memory file both sides have mapped: the server writes pixels into it
/// on ShmGetImage and only a 32-byte reply crosses the socket.
struct Shared {
    segment: u32,
    map: *mut u8,
    len: usize,
    file: OwnedFd,
}

impl Shared {
    fn attach(connection: &RustConnection, len: usize) -> Result<Self, String> {
        if connection
            .extension_information(x11rb::protocol::shm::X11_EXTENSION_NAME)
            .map_err(|error| error.to_string())?
            .is_none()
        {
            return Err("the display has no MIT-SHM".into());
        }
        // SAFETY: a new anonymous memory file, owned below.
        let raw =
            unsafe { libc::memfd_create(c"hotline-browser-screen".as_ptr(), libc::MFD_CLOEXEC) };
        if raw < 0 {
            return Err(format!("memfd_create: {}", std::io::Error::last_os_error()));
        }
        // SAFETY: the descriptor just made, owned here alone.
        let file = unsafe { OwnedFd::from_raw_fd(raw) };
        // SAFETY: sizing the file this function owns.
        if unsafe { libc::ftruncate(file.as_raw_fd(), len as libc::off_t) } < 0 {
            return Err(format!("ftruncate: {}", std::io::Error::last_os_error()));
        }
        // SAFETY: a fresh shared mapping of the whole file.
        let map = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                file.as_raw_fd(),
                0,
            )
        };
        if map == libc::MAP_FAILED {
            return Err(format!("mmap: {}", std::io::Error::last_os_error()));
        }
        let shared = Self {
            segment: connection
                .generate_id()
                .map_err(|error| error.to_string())?,
            map: map.cast::<u8>(),
            len,
            file,
        };
        let for_server = shared
            .file
            .try_clone()
            .map_err(|error| format!("dup: {error}"))?;
        connection
            .shm_attach_fd(shared.segment, for_server, false)
            .map_err(|error| error.to_string())?
            .check()
            .map_err(|error| format!("ShmAttachFd: {error}"))?;
        Ok(shared)
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        // SAFETY: the mapping made in `attach`, unmapped once.
        unsafe {
            libc::munmap(self.map.cast(), self.len);
        }
    }
}

// SAFETY: the mapping is only ever read by the one thread holding the
// capture at a time (a viewer hands it to each read and takes it back).
unsafe impl Send for Shared {}

/// Binds the viewers' socket in the displays' private folder.
pub fn listen(path: &Path) -> Result<std::os::unix::net::UnixListener, String> {
    let _ = std::fs::remove_file(path);
    let listener = std::os::unix::net::UnixListener::bind(path)
        .map_err(|error| format!("the browser's screens could not listen: {error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("the browser's screens could not listen: {error}"))?;
    Ok(listener)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_viewers_changes_wait_as_one_rectangle_inside_the_page() {
        let watcher = Watcher {
            dirty: Mutex::new(None),
            wake: Notify::new(),
        };
        watcher.add(Rect::new(10, 10, 10, 10));
        watcher.add(Rect::new(1200, 700, 200, 200));
        assert_eq!(
            watcher.take((1280, 800)),
            Some(Rect::new(10, 10, 1270, 790))
        );
        assert_eq!(watcher.take((1280, 800)), None, "sent, nothing waits");
        watcher.add(Rect::new(1300, 0, 10, 10));
        assert_eq!(watcher.take((1280, 800)), None, "off the page");
    }

    #[test]
    fn damage_off_the_top_left_is_cut_at_the_edge() {
        use x11rb::protocol::xproto::Rectangle;
        assert_eq!(
            rect_of(Rectangle {
                x: -5,
                y: 3,
                width: 10,
                height: 4
            }),
            Rect::new(0, 3, 5, 4)
        );
    }

    #[test]
    fn pixels_are_read_as_rgb_rows_past_their_padding() {
        let setup = x11rb::protocol::xproto::Setup {
            pixmap_formats: vec![x11rb::protocol::xproto::Format {
                depth: 24,
                bits_per_pixel: 32,
                scanline_pad: 32,
            }],
            ..Default::default()
        };
        // Two pixels a row, BGRX, two rows.
        let data = [1, 2, 3, 0, 4, 5, 6, 0, 7, 8, 9, 0, 10, 11, 12, 0];
        assert_eq!(
            unpack(&setup, 24, Rect::new(0, 0, 2, 2), &data).unwrap(),
            vec![3, 2, 1, 6, 5, 4, 9, 8, 7, 12, 11, 10]
        );
        assert!(unpack(&setup, 24, Rect::new(0, 0, 3, 2), &data).is_err());
        let png = encode_png(Rect::new(0, 0, 2, 2), &[0; 12]).unwrap();
        assert!(png.starts_with(b"\x89PNG"));
    }
}
