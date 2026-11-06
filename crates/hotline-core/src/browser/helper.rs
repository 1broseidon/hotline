//! The browser of a desk with no window (`hotline serve`): the Linux desk
//! browser, WebKitGTK under a private X display, run in a helper process of
//! its own (`hotline-browser`, beside `hotline`), which this side starts,
//! talks to over [`pipe`](super::pipe) and starts again when it is gone.
//!
//! The server's binary never links GTK or WebKit, so a server without them
//! runs as it always did: the helper is not found or its probe fails, no
//! browser is installed, and no teammate is offered the tool. With them,
//! [`install`] puts a [`Helper`] in the desk, which starts the helper at a
//! teammate's first page, not before, so a desk nobody browses on pays
//! nothing for it. If the helper dies, every teammate's browser is gone with
//! it: a call in flight is told so, a call that needs a page says the
//! browser stopped, and the next page starts a new helper.
//!
//! The helper's side, and the reaping of what WebKit and its display leave,
//! is `crates/hotline-browser`.

use super::pipe::{Call, Down, PIPE_FD, State, Up, line, picture_in};
use super::{Act, Acted, DeskBrowser, Field, FullPage, Gate, Page, Reading, Showing, Step};
use crate::contract::BrowserInput;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::{mpsc, oneshot};
use url::Url;

/// The helper's name, beside the `hotline` binary.
pub const PROGRAM: &str = "hotline-browser";

const NO_PAGE: &str = "Your browser has no page open yet: navigate to one first.";
const STOPPED: &str = "Your browser stopped, and its page with it. Navigate to open a new one.";
/// How long a new helper has to start its display and GTK and say so.
const READY_WAIT: Duration = Duration::from_secs(30);
/// The longest one call is waited for. The helper bounds each of its own
/// waits well under this; this is for a helper that hangs.
const CALL_WAIT: Duration = Duration::from_secs(180);
/// How long the probe may take: it starts nothing.
const PROBE_WAIT: Duration = Duration::from_secs(15);

type States = Arc<Mutex<HashMap<String, State>>>;
type Pending = Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>;

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Puts the server's browser in `desk` when its helper is beside this
/// binary and answers its probe, which says WebKitGTK and Xvfb are
/// installed: the desk's own ports are `never_ports`, and each teammate's
/// web data goes under `data`. Answers what the probe found, or why there
/// is no browser.
pub async fn install(
    desk: &crate::desk::Desk,
    never_ports: &[u16],
    data: &Path,
) -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let program = exe.with_file_name(PROGRAM);
    if !program.is_file() {
        return Err(format!(
            "{PROGRAM} is not installed beside {}",
            exe.display()
        ));
    }
    let found = ours(&probe(&program).await?)?;
    let helper = Helper::new(program, data.to_path_buf(), desk.browser_gate());
    desk.install_browser(Arc::new(helper), never_ports);
    Ok(found)
}

/// What the probe said of the helper, when it is this desk's own version:
/// the pipe between the two is theirs alone, and changes between releases.
/// Answers the rest of what it said (its WebKitGTK).
fn ours(said: &str) -> Result<String, String> {
    let version = concat!("hotline-browser ", env!("CARGO_PKG_VERSION"), ", ");
    said.strip_prefix(version)
        .map(str::to_string)
        .ok_or_else(|| {
            format!(
                "{PROGRAM} is not this desk's version {}: it says {said:?}",
                env!("CARGO_PKG_VERSION")
            )
        })
}

/// Asks the helper whether it can run here, without starting a browser:
/// its libraries load (the system's dynamic loader refuses it before
/// `main` otherwise) and Xvfb is on `PATH`. Answers what it said of itself.
pub async fn probe(program: &Path) -> Result<String, String> {
    let ran = tokio::time::timeout(
        PROBE_WAIT,
        tokio::process::Command::new(program)
            .arg("--probe")
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| format!("{PROGRAM} --probe did not answer"))?
    .map_err(|error| format!("{PROGRAM} did not run: {error}"))?;
    let said = |bytes: &[u8]| {
        String::from_utf8_lossy(bytes)
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or_default()
            .to_string()
    };
    if ran.status.success() {
        return Ok(said(&ran.stdout));
    }
    let why = said(&ran.stderr);
    // The dynamic loader names the first library it could not find, which
    // is any of WebKitGTK's stack (GLib, GTK, WebKit) on a server without it.
    let missing = why
        .split_once("error while loading shared libraries: ")
        .map(|(_, rest)| rest.split(':').next().unwrap_or(rest).to_string());
    Err(if let Some(library) = missing {
        format!("WebKitGTK is not installed ({library} is missing)")
    } else if why.is_empty() {
        format!("{PROGRAM} --probe failed ({})", ran.status)
    } else {
        why
    })
}

/// One running helper, as the desk talks to it: lines down to it, and the
/// calls waiting on its replies.
struct Conn {
    down: mpsc::UnboundedSender<String>,
    pending: Pending,
    next: AtomicU64,
    alive: AtomicBool,
}

impl Conn {
    /// Talks to a helper on `stream`: a task writes what is sent down, and
    /// another reads what comes up, answers its asks from `gate`, keeps each
    /// teammate's state in `states`, and, when the helper is gone, fails
    /// every call still waiting and forgets every state. The second answers
    /// the helper's `ready`.
    fn open(
        stream: UnixStream,
        gate: Arc<Gate>,
        states: States,
    ) -> (Arc<Self>, oneshot::Receiver<()>) {
        let (read, mut write) = stream.into_split();
        let (down, mut queued) = mpsc::unbounded_channel::<String>();
        let conn = Arc::new(Self {
            down,
            pending: Mutex::default(),
            next: AtomicU64::new(1),
            alive: AtomicBool::new(true),
        });
        // When the last sender goes, so does the write half, which shuts the
        // socket for writing: the helper reads its end and exits.
        tokio::spawn(async move {
            while let Some(text) = queued.recv().await {
                if write.write_all(text.as_bytes()).await.is_err() {
                    break;
                }
            }
        });
        let (ready, readied) = oneshot::channel();
        let weak = Arc::downgrade(&conn);
        tokio::spawn(read_up(read, weak, gate, states, ready));
        (conn, readied)
    }

    fn send(&self, down: &Down) -> Result<(), String> {
        self.down.send(line(down)).map_err(|_| STOPPED.to_string())
    }

    async fn call(&self, persona: &str, call: Call) -> Result<Value, String> {
        if !self.alive.load(Ordering::Acquire) {
            return Err(STOPPED.into());
        }
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let (answer, answered) = oneshot::channel();
        lock(&self.pending).insert(id, answer);
        self.send(&Down::Call {
            id: Some(id),
            persona: persona.to_string(),
            call: Box::new(call),
        })?;
        match tokio::time::timeout(CALL_WAIT, answered).await {
            Ok(Ok(reply)) => reply,
            Ok(Err(_)) => Err(STOPPED.into()),
            Err(_) => {
                lock(&self.pending).remove(&id);
                Err("Your browser did not answer in time.".into())
            }
        }
    }

    /// A call nothing is answered for.
    fn tell(&self, persona: &str, call: Call) {
        let _ = self.send(&Down::Call {
            id: None,
            persona: persona.to_string(),
            call: Box::new(call),
        });
    }

    fn gone(&self, states: &States) {
        self.alive.store(false, Ordering::Release);
        for (_, waiting) in lock(&self.pending).drain() {
            let _ = waiting.send(Err(STOPPED.into()));
        }
        lock(states).clear();
    }
}

/// Reads the helper's lines until it is gone.
async fn read_up(
    read: tokio::net::unix::OwnedReadHalf,
    conn: Weak<Conn>,
    gate: Arc<Gate>,
    states: States,
    ready: oneshot::Sender<()>,
) {
    let mut ready = Some(ready);
    let mut lines = BufReader::new(read).lines();
    while let Ok(Some(text)) = lines.next_line().await {
        let Some(live) = conn.upgrade() else {
            return;
        };
        match serde_json::from_str::<Up>(&text) {
            Ok(Up::Ready { .. }) => {
                if let Some(ready) = ready.take() {
                    let _ = ready.send(());
                }
            }
            Ok(Up::Reply { id, ok, error }) => {
                let reply = match error {
                    Some(said) => Err(said),
                    None => Ok(ok.unwrap_or(Value::Null)),
                };
                if let Some(waiting) = lock(&live.pending).remove(&id) {
                    let _ = waiting.send(reply);
                }
            }
            Ok(Up::Ask { ask, url }) => {
                let admits = Url::parse(&url).is_ok_and(|url| gate.admits_frame(&url));
                let _ = live.send(&Down::Answer { ask, admits });
            }
            Ok(Up::State { persona, state }) => {
                let mut states = lock(&states);
                if state == State::default() {
                    states.remove(&persona);
                } else {
                    states.insert(persona, state);
                }
            }
            Err(_) => eprintln!("[browser] the helper sent a line the desk does not read"),
        }
    }
    if let Some(live) = conn.upgrade() {
        live.gone(&states);
    }
}

/// The server's browser: each teammate's page in the helper, which it
/// starts at the first page and again after it has gone.
pub struct Helper {
    program: PathBuf,
    data: PathBuf,
    gate: Arc<Gate>,
    /// Held while a helper starts, so two first pages start one.
    starting: tokio::sync::Mutex<()>,
    /// The helper last started, alive or not.
    conn: Mutex<Option<Arc<Conn>>>,
    states: States,
    /// Each teammate's name, told again to a helper started anew.
    names: Mutex<HashMap<String, String>>,
}

impl Helper {
    pub fn new(program: PathBuf, data: PathBuf, gate: Arc<Gate>) -> Self {
        Self {
            program,
            data,
            gate,
            starting: tokio::sync::Mutex::default(),
            conn: Mutex::default(),
            states: Arc::default(),
            names: Mutex::default(),
        }
    }

    /// The running helper, if one is.
    fn running(&self) -> Option<Arc<Conn>> {
        lock(&self.conn)
            .clone()
            .filter(|conn| conn.alive.load(Ordering::Acquire))
    }

    /// The running helper, or why there is none to ask: no page was ever
    /// opened, or the helper it was in stopped.
    fn needed(&self) -> Result<Arc<Conn>, String> {
        match lock(&self.conn).clone() {
            Some(conn) if conn.alive.load(Ordering::Acquire) => Ok(conn),
            Some(_) => Err(STOPPED.into()),
            None => Err(NO_PAGE.into()),
        }
    }

    /// The running helper, started now if none is.
    async fn started(&self) -> Result<Arc<Conn>, String> {
        let _starting = self.starting.lock().await;
        if let Some(conn) = self.running() {
            return Ok(conn);
        }
        let conn = self.spawn().await?;
        for (persona, name) in lock(&self.names).iter() {
            conn.tell(persona, Call::Name { name: name.clone() });
        }
        *lock(&self.conn) = Some(conn.clone());
        Ok(conn)
    }

    /// Starts the helper on a socket pair, its end on [`PIPE_FD`], in a
    /// process group of its own, and waits for it to say it is ready. Its
    /// standard error comes out on the desk's, a line at a time, and its
    /// exit is waited for so it never lingers as a zombie of the desk.
    async fn spawn(&self) -> Result<Arc<Conn>, String> {
        let failed = |error: std::io::Error| format!("Your browser could not start: {error}");
        let (ours, theirs) = std::os::unix::net::UnixStream::pair().map_err(failed)?;
        let mut command = tokio::process::Command::new(&self.program);
        command
            .arg("--data")
            .arg(&self.data)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let fd = std::os::fd::AsRawFd::as_raw_fd(&theirs);
        // SAFETY: between fork and exec only async-signal-safe calls are
        // made (dup2, fcntl, setpgid), on a descriptor open in the child.
        unsafe {
            command.pre_exec(move || {
                if fd == PIPE_FD {
                    let flags = libc::fcntl(fd, libc::F_GETFD);
                    if flags < 0 || libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                } else if libc::dup2(fd, PIPE_FD) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::setpgid(0, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command.spawn().map_err(failed)?;
        drop(theirs);
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(text)) = lines.next_line().await {
                    eprintln!("[browser] {text}");
                }
            });
        }
        let pid = child.id();
        ours.set_nonblocking(true).map_err(failed)?;
        let stream = UnixStream::from_std(ours).map_err(failed)?;
        let (conn, readied) = Conn::open(stream, self.gate.clone(), self.states.clone());
        let ready = tokio::time::timeout(READY_WAIT, readied).await;
        if !matches!(ready, Ok(Ok(()))) {
            let _ = child.start_kill();
        }
        tokio::spawn(async move {
            match child.wait().await {
                Ok(status) if status.success() => {}
                Ok(status) => eprintln!("[browser] the helper ({pid:?}) stopped: {status}"),
                Err(error) => eprintln!("[browser] the helper ({pid:?}) was lost: {error}"),
            }
            if let Some(group) = pid.and_then(|pid| libc::pid_t::try_from(pid).ok()) {
                let _ = tokio::task::spawn_blocking(move || clear_group(group)).await;
            }
        });
        match ready {
            Ok(Ok(())) => Ok(conn),
            Ok(Err(_)) => {
                Err("Your browser could not start: its helper stopped as it started.".into())
            }
            Err(_) => {
                Err("Your browser could not start: its helper did not get ready in time.".into())
            }
        }
    }

    async fn ask(&self, persona: &str, call: Call) -> Result<Value, String> {
        self.needed()?.call(persona, call).await
    }

    fn state(&self, persona: &str) -> Option<State> {
        lock(&self.states).get(persona).cloned()
    }
}

/// How long what a dead helper left in its process group has to be reaped.
const CLEAR_WAIT: Duration = Duration::from_secs(5);

/// Kills whatever a helper that is gone left in its process group, and
/// reaps what of it this process was left. The helper's own first process
/// reaps everything below it, so this finds nothing unless that process
/// was killed outright: then its browser, display and WebKit's processes
/// go to process 1, which is this desk in a container with no init, and
/// would stay zombies of it. The group is the helper's alone (it was put
/// in its own at spawn), so no other child's exit is taken here.
fn clear_group(group: libc::pid_t) {
    // SAFETY: a signal to the process group this desk made for its helper.
    unsafe { libc::kill(-group, libc::SIGKILL) };
    let deadline = std::time::Instant::now() + CLEAR_WAIT;
    loop {
        // SAFETY: a wait for any child in that group, without blocking.
        let reaped = unsafe { libc::waitpid(-group, std::ptr::null_mut(), libc::WNOHANG) };
        if reaped > 0 {
            continue;
        }
        // None of the group is this process's child any more.
        if reaped < 0 || std::time::Instant::now() >= deadline {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn read<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value)
        .map_err(|error| format!("Your browser's answer was unreadable: {error}"))
}

#[async_trait]
impl DeskBrowser for Helper {
    async fn open(&self, persona_id: &str, url: &Url) -> Result<Page, String> {
        let conn = self.started().await?;
        read(
            conn.call(
                persona_id,
                Call::Open {
                    url: url.to_string(),
                },
            )
            .await?,
        )
    }

    async fn step(&self, persona_id: &str, step: Step) -> Result<Page, String> {
        read(self.ask(persona_id, Call::Step { step }).await?)
    }

    async fn read(&self, persona_id: &str, reading: Reading) -> Result<String, String> {
        read(self.ask(persona_id, Call::Read { reading }).await?)
    }

    async fn act(&self, persona_id: &str, act: &Act) -> Result<Acted, String> {
        read(self.ask(persona_id, Call::Act { act: act.clone() }).await?)
    }

    async fn screenshot(&self, persona_id: &str) -> Result<Vec<u8>, String> {
        picture_in(&self.ask(persona_id, Call::Screenshot).await?)
    }

    async fn full_page(&self, persona_id: &str, most: usize) -> Result<FullPage, String> {
        let answer = self.ask(persona_id, Call::FullPage { most }).await?;
        let pictures = answer["pictures"]
            .as_array()
            .ok_or("Your browser's answer was unreadable.")?
            .iter()
            .map(picture_in)
            .collect::<Result<_, _>>()?;
        Ok(FullPage {
            pictures,
            more: answer["more"] == true,
        })
    }

    async fn field(&self, persona_id: &str, target: &str) -> Result<Field, String> {
        let target = target.to_string();
        read(self.ask(persona_id, Call::Field { target }).await?)
    }

    async fn showing(&self, persona_id: &str) -> Result<Showing, String> {
        read(self.ask(persona_id, Call::Showing).await?)
    }

    async fn viewport(&self, persona_id: &str, size: Option<(u32, u32)>) -> Result<Page, String> {
        read(self.ask(persona_id, Call::Viewport { size }).await?)
    }

    fn size(&self, persona_id: &str) -> Option<(u32, u32)> {
        self.state(persona_id)?.size
    }

    async fn page_size(&self, persona_id: &str) -> Option<(u32, u32)> {
        let answer = self.ask(persona_id, Call::PageSize).await.ok()?;
        read(answer).ok().flatten()
    }

    async fn person_input(&self, persona_id: &str, input: &BrowserInput) -> Result<(), String> {
        let input = input.clone();
        self.ask(persona_id, Call::PersonInput { input }).await?;
        Ok(())
    }

    async fn visit_anyway(&self, persona_id: &str) -> Result<(), String> {
        self.ask(persona_id, Call::VisitAnyway).await?;
        Ok(())
    }

    fn taken(&self, persona_id: &str, taken: bool) {
        if let Some(conn) = self.running() {
            conn.tell(persona_id, Call::Taken { taken });
        }
    }

    fn current(&self, persona_id: &str) -> Option<String> {
        self.state(persona_id)?.current
    }

    fn needs(&self, persona_id: &str) -> Option<String> {
        self.state(persona_id)?.needs
    }

    fn name_teammate(&self, persona_id: &str, name: &str) {
        let told = lock(&self.names).insert(persona_id.to_string(), name.to_string());
        if told.as_deref() != Some(name)
            && let Some(conn) = self.running()
        {
            conn.tell(persona_id, Call::Name { name: name.into() });
        }
    }

    /// Closes the teammate's page in the helper, if one runs. With `wipe`
    /// and no helper running, nothing of its storage is in memory: its
    /// directory is all there is, and goes.
    async fn close(&self, persona_id: &str, wipe: bool) -> Result<(), String> {
        let closed = match self.running() {
            Some(conn) => conn.call(persona_id, Call::Close { wipe }).await.map(drop),
            None if wipe => {
                let directory = super::store_dir(&self.data, persona_id);
                let removed =
                    tokio::task::spawn_blocking(move || std::fs::remove_dir_all(directory))
                        .await
                        .map_err(|error| error.to_string())?;
                match removed {
                    Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                        Err(format!("Its storage was not removed: {error}"))
                    }
                    _ => Ok(()),
                }
            }
            None => Ok(()),
        };
        lock(&self.states).remove(persona_id);
        closed
    }
}

#[cfg(test)]
mod tests;
