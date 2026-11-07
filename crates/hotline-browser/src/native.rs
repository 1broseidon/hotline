//! Each teammate's browser in the helper, as the desk drives it through
//! [`DeskBrowser`]: the app's Linux desk browser (`hotline-app`'s
//! `browser.rs`) with its window and pane taken away. The files that
//! touch WebKitGTK are the app's own, compiled here from where they are, so
//! the two never drift: the platform seam (`linux.rs`), the teammate's
//! pointer (`linux/hands.rs`), what a failed load and a refused certificate
//! do (`trust.rs`), what a page may ask of the machine (`asks.rs`), how a
//! page starts (`guard.rs`), the whole-page look (`full_page.rs`) and every
//! page script. What Tauri does for the app is done here by WebKit's own
//! signals ([`views`]): the gate's word on every load, a page's popups,
//! loads counted, downloads refused, and a web context of its own for each
//! teammate, on a data directory under the desk's.
//!
//! What it may load is still the desk's [`Gate`](hotline_browser_core::Gate),
//! asked over the pipe before every load of every frame ([`Link`]): the
//! desk's own ports are known only to the desk.
//!
//! A server has no pane: nobody watches the page here, so there is no
//! shield to raise or bar to answer. The person drives a page they have
//! taken over from the phone or a linked desktop, by the desk's pictures of
//! it, and chooses Visit anyway there (`browser.visit_anyway`), which lets a
//! refused certificate through by the same trust code as the bar's. A
//! page's file picker is refused even then: it would open on a display
//! nobody sees, and a phone has no way to answer it.
//!
//! WebKit is set to draw without the GPU, which a server lacks (it would be
//! emulated in software at several times the memory), and to keep no cache
//! of pages and processes past the one in use: one web process a teammate,
//! where the defaults keep three and twice the memory.

#[allow(
    dead_code,
    reason = "the app's file: its picker refusal names the pane, which a server has none of"
)]
#[path = "../../hotline-app/src/browser/native/asks.rs"]
mod asks;
#[path = "../../hotline-app/src/browser/native/full_page.rs"]
mod full_page;
#[path = "../../hotline-app/src/browser/native/guard.rs"]
mod guard;
#[allow(dead_code, reason = "the app's file: following its pane's size")]
#[path = "../../hotline-app/src/browser/native/linux/hands.rs"]
mod hands;
#[allow(
    dead_code,
    reason = "the app's file: showing and hiding its pane, which a server has none of"
)]
#[path = "../../hotline-app/src/browser/native/linux.rs"]
mod linux;
#[allow(
    dead_code,
    reason = "the app's file: what its pane's bar shows of a certificate let through"
)]
#[path = "../../hotline-app/src/browser/native/trust.rs"]
mod trust;
mod views;

use async_trait::async_trait;
use hotline_browser_core::pipe::{Link, Up};
use hotline_browser_core::{Act, Acted, DeskBrowser, Field, Page, Reading, Showing, Step};
use hotline_contract::BrowserInput;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use tokio::sync::{oneshot, watch};
use url::Url;
use views::{Shared, gtk_do, on_view};

const WALK: &str = include_str!("../../hotline-app/src/browser/walk.js");
const READ_TEXT: &str = concat!(
    include_str!("../../hotline-app/src/browser/walk.js"),
    include_str!("../../hotline-app/src/browser/read_text.js")
);
const READ_LINKS: &str = include_str!("../../hotline-app/src/browser/read_links.js");
/// field.js goes in front of act.js: a secret's fill checks its field
/// again as it types.
const ACT: &str = concat!(
    include_str!("../../hotline-app/src/browser/field.js"),
    include_str!("../../hotline-app/src/browser/act.js")
);
const LOCATE: &str = include_str!("../../hotline-app/src/browser/locate.js");
/// What a field is, before a secret is typed into it: walk.js goes in
/// front, and `A` names the ref.
const FIELD: &str = concat!(
    include_str!("../../hotline-app/src/browser/field.js"),
    "const el = refs.get(A.ref);\n",
    "return JSON.stringify(el && el.isConnected ? { field: fieldFacts(el), next: globalThis.__hotlineNext } : { error: `${A.ref} is no longer on the page: read it again with text.` });"
);
const SHOWING: &str = include_str!("../../hotline-app/src/browser/showing.js");
/// The person's own input from a seat away from the desk: `A` names it.
const PERSON: &str = include_str!("../../hotline-app/src/browser/person.js");
const PAGE_SIZE: &str =
    "return JSON.stringify({ width: window.innerWidth, height: window.innerHeight });";
const PERSON_SETTLE: Duration = Duration::from_millis(120);
const HANDS_IDLE: Duration = Duration::from_secs(4);
const ARRIVED: &str = "return JSON.stringify(globalThis.__hotlineArrived !== false);";
const CLICK: &str = "const el = refs.get(A.ref); if (el) { el.focus?.(); el.click(); } return JSON.stringify(!!el);";
const ACT_SETTLE: Duration = Duration::from_millis(400);
const PAGE_INFO: &str =
    "return JSON.stringify({ title: document.title, url: location.href, text: '' });";
const NO_PAGE: &str = "Your browser has no page open yet: navigate to one first.";
const REMOVE_TRIES: usize = 5;
const REMOVE_WAIT: Duration = Duration::from_millis(400);
const POPUP_OPENED: &str = "The page opened a window of its own (a sign-in, say): your reads and acts are in it now. When it closes, as a sign-in's does once it is done, you are back on the page that opened it; navigating closes it.";
const POPUP_CLOSED: &str = "That window closed: you are back on the page that opened it.";
const LOAD_WAIT: Duration = Duration::from_secs(20);
const STEP_WAIT: Duration = Duration::from_secs(5);
const EMPTY_RETRIES: usize = 4;
const EMPTY_WAIT: Duration = Duration::from_millis(500);
const SCRIPT_WAIT: Duration = Duration::from_secs(15);
/// The page's size: a common laptop viewport, as the app's.
const WIDTH: f64 = 1280.0;
const HEIGHT: f64 = 800.0;
/// Where the person chooses Visit anyway on a server, which has no pane,
/// as a refused certificate's warning page (`trust.rs`) and the teammate
/// are told: wherever they drive the browser from.
const VISIT_ANYWAY_ON_PAGE: &str =
    "take control from your phone or a linked desktop and choose Visit anyway";
const VISIT_ANYWAY_FOR_TEAMMATE: &str = "they take control of your browser from their phone or a linked desktop and choose Visit anyway";
/// What the call that made a page ask for a file hears: a server's browser
/// opens no file picker, even for the person, so no file is uploaded there.
const PICKER_REFUSED: &str = "The page asked to choose a file to upload, and a server's browser never opens a file picker, for you or the person: nothing was chosen or uploaded. If the task needs a file uploaded there, tell the person; it cannot be done from this browser.";

/// One teammate's browser: its page and popup, as their web views' ids, and
/// what the desk keeps of them. The fields the app's files read are named
/// as the app names them.
struct Tab {
    persona: String,
    page: u64,
    loads: watch::Sender<Loads>,
    /// The address the page last finished loading or was read at.
    shown: Mutex<String>,
    seen: trust::Seen,
    moves: AtomicU64,
    refs_next: AtomicU64,
    popup: Mutex<Option<u64>>,
    page_shown: Mutex<String>,
    viewport: Mutex<Option<(f64, f64)>>,
    /// The page's window as a live screen, and its popup's while one is
    /// open (`screen.rs`).
    screen: Mutex<Option<Arc<crate::screen::Screen>>>,
    popup_screen: Mutex<Option<Arc<crate::screen::Screen>>>,
    /// Where the teammate's viewers are told which window is in front.
    screens: Arc<crate::screen::Screens>,
}

#[derive(Clone, Default)]
struct Loads {
    started: u64,
    finished: u64,
    blocked: Option<String>,
    failed: Option<String>,
    picker_refused: bool,
}

impl Loads {
    fn fresh(&mut self) {
        self.blocked = None;
        self.failed = None;
        self.picker_refused = false;
    }
}

#[derive(Deserialize)]
struct Spot {
    x: f64,
    y: f64,
    height: f64,
}

#[derive(Deserialize)]
struct Located {
    point: Option<Spot>,
}

#[derive(Deserialize)]
struct Read {
    title: String,
    url: String,
    text: String,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Runs the browser until the desk hangs up: the display, GTK on this
/// thread, and the pipe on a thread of its own.
pub fn run(data: &Path) -> Result<(), String> {
    let pipe = views::take_pipe()?;
    let display = crate::display::start()?;
    gtk::init().map_err(|error| format!("GTK did not start: {error}"))?;
    // A caret that blinks redraws its field twice a second, which is a
    // picture each time for every viewer of the page: it stays lit.
    if let Some(settings) = gtk::Settings::default() {
        use gtk::prelude::GtkSettingsExt;
        settings.set_gtk_cursor_blink(false);
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|error| format!("the browser's runtime did not start: {error}"))?;
    let screens = Arc::new(crate::screen::Screens::new(
        display.number,
        display.folder(),
        views::set_clipboard,
    )?);
    let socket = screens.socket();
    let listener = {
        let _entered = runtime.enter();
        tokio::net::UnixListener::from_std(crate::screen::listen(&socket)?)
            .map_err(|error| format!("the browser's screens could not listen: {error}"))?
    };
    runtime.spawn(crate::screen::serve(listener, screens.clone()));
    let (link, outbox) = Link::new();
    let native: Arc<dyn DeskBrowser> = Arc::new(Native::new(
        link.clone(),
        runtime.handle().clone(),
        data.to_path_buf(),
        screens.clone(),
    ));
    let ready = Up::Ready {
        pid: std::process::id(),
        display: display.number,
        screen: Some(socket.to_string_lossy().into_owned()),
    };
    // Calls come once GTK's loop runs, so every one is run by it.
    let (looping, looped) = std::sync::mpsc::channel();
    gtk::glib::idle_add_local_once(move || {
        let _ = looping.send(());
    });
    let answering = std::thread::spawn(move || {
        let _ = looped.recv();
        let answered = runtime.block_on(async move {
            let stream = tokio::net::UnixStream::from_std(pipe)?;
            hotline_browser_core::pipe::answer(stream, link, outbox, native, ready).await
        });
        if let Err(error) = answered {
            eprintln!("the desk's pipe failed: {error}");
        }
        // The desk hung up: the browser closes.
        gtk_do(gtk::main_quit);
    });
    gtk::main();
    let _ = answering.join();
    drop(screens);
    drop(display);
    Ok(())
}

/// The teammates' browsers in this helper.
pub(crate) struct Native {
    shared: Shared,
    data: PathBuf,
    tabs: Mutex<HashMap<String, Arc<Tab>>>,
    names: Mutex<HashMap<String, String>>,
}

impl Native {
    fn new(
        link: Link,
        runtime: tokio::runtime::Handle,
        data: PathBuf,
        screens: Arc<crate::screen::Screens>,
    ) -> Self {
        Self {
            shared: Shared::new(link, runtime, screens),
            data,
            tabs: Mutex::default(),
            names: Mutex::default(),
        }
    }

    fn tab(&self, persona_id: &str) -> Option<Arc<Tab>> {
        lock(&self.tabs).get(persona_id).cloned()
    }

    /// The teammate's page in front: its popup while one is open, the page
    /// otherwise. Reads, acts and pictures go here.
    fn window(&self, persona_id: &str) -> Result<u64, String> {
        let tab = self.tab(persona_id).ok_or(NO_PAGE)?;
        let popup = *lock(&tab.popup);
        Ok(popup.unwrap_or(tab.page))
    }

    fn store_dir(&self, persona_id: &str) -> PathBuf {
        hotline_browser_core::store_dir(&self.data, persona_id)
    }

    /// Closes the teammate's browser: its popup, its page, and all the desk
    /// kept for them; with `wipe`, its web storage, emptied through the
    /// open page first so what WebKit holds in memory goes too, then
    /// removed once no web view uses it.
    async fn close_tab(&self, persona_id: &str, wipe: bool) -> Result<(), String> {
        let emptied = if wipe {
            self.empty_store(persona_id).await
        } else {
            Ok(())
        };
        let tab = lock(&self.tabs).remove(persona_id);
        lock(&self.names).remove(persona_id);
        self.shared.exceptions.forget(persona_id);
        if let Some(tab) = &tab {
            shut_popup(tab);
            views::close(tab.page);
            views::release(tab, &self.shared.screens);
        }
        if !wipe {
            return Ok(());
        }
        let mut removed = Ok(());
        for _ in 0..REMOVE_TRIES {
            tokio::time::sleep(REMOVE_WAIT).await;
            removed = self.remove_store(persona_id).await;
            if removed.is_ok() {
                break;
            }
        }
        emptied?;
        removed.map_err(|error| {
            format!("Its storage was emptied, but the store itself was not removed: {error}")
        })
    }

    /// Empties the teammate's web storage of every kind of data: through its
    /// open page's, so what WebKit holds in memory goes too, or a manager
    /// of its own on the data directory.
    async fn empty_store(&self, persona_id: &str) -> Result<(), String> {
        let page = self.tab(persona_id).map(|tab| tab.page);
        let directory = self.store_dir(persona_id);
        let (reply, answer) = oneshot::channel();
        gtk_do(move || views::empty(page, &directory, reply));
        tokio::time::timeout(SCRIPT_WAIT, answer)
            .await
            .map_err(|_| "The browser's storage did not answer in time.".to_string())?
            .map_err(|_| "The browser's storage closed before it answered.".to_string())?
    }

    async fn remove_store(&self, persona_id: &str) -> Result<(), String> {
        let directory = self.store_dir(persona_id);
        let removed = {
            let directory = directory.clone();
            tokio::task::spawn_blocking(move || std::fs::remove_dir_all(directory))
                .await
                .map_err(|error| error.to_string())?
        };
        match removed {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                Err(format!("{}: {error}", directory.display()))
            }
            _ => Ok(()),
        }
    }

    /// Makes the teammate's page on `url`, its loads counted from nothing.
    async fn make(&self, persona_id: &str, url: &Url) -> Result<Arc<Tab>, String> {
        let (loads, _) = watch::channel(Loads::default());
        let tab = Arc::new(Tab {
            persona: persona_id.to_string(),
            page: views::next_id(),
            loads,
            shown: Mutex::new(String::new()),
            seen: trust::Seen::default(),
            moves: AtomicU64::new(0),
            refs_next: AtomicU64::new(0),
            popup: Mutex::new(None),
            page_shown: Mutex::new(String::new()),
            viewport: Mutex::new(None),
            screen: Mutex::new(None),
            popup_screen: Mutex::new(None),
            screens: self.shared.screens.clone(),
        });
        let made = {
            let (tab, shared, url) = (tab.clone(), self.shared.clone(), url.clone());
            let store = self.store_dir(persona_id);
            let (reply, answer) = oneshot::channel();
            gtk_do(move || {
                let _ = reply.send(views::page(&tab, &shared, &store, url));
            });
            answer
                .await
                .map_err(|_| "Your browser could not be opened.".to_string())?
        };
        made.map_err(|error| format!("Your browser could not be opened: {error}"))?;
        lock(&self.tabs).insert(persona_id.to_string(), tab.clone());
        Ok(tab)
    }

    /// Before an act: brings its element into view and moves the teammate's
    /// pointer onto it, as the app does for the person watching its pane.
    async fn reach(&self, persona_id: &str, tab: &Arc<Tab>, act: &Act, args: &str) -> bool {
        let watched = matches!(
            act,
            Act::Click { .. }
                | Act::Check { .. }
                | Act::Hover { .. }
                | Act::Fill { .. }
                | Act::FillSecret { .. }
                | Act::Select { .. }
                | Act::Scroll { .. }
        );
        if !watched {
            return false;
        }
        let Ok(args) = serde_json::from_str::<serde_json::Value>(args) else {
            return false;
        };
        let args = serde_json::json!({ "act": args["act"], "ref": args["ref"] });
        let Ok(answer) = self
            .run(persona_id, format!("const A = {args};\n{WALK}\n{LOCATE}"))
            .await
        else {
            return false;
        };
        let Ok(Located {
            point: Some(Spot { x, y, height }),
        }) = serde_json::from_str(&answer)
        else {
            return false;
        };
        let who = self.name_of(persona_id);
        let Ok(front) = self.window(persona_id) else {
            return false;
        };
        on_view(front, move |view| hands::point(view, x, y, height, &who));
        let moves = tab.moves.fetch_add(1, Ordering::SeqCst) + 1;
        let (page, still) = (tab.page, tab.clone());
        self.shared.runtime.spawn(async move {
            tokio::time::sleep(HANDS_IDLE).await;
            if still.moves.load(Ordering::SeqCst) == moves {
                on_view(page, hands::hide);
            }
        });
        tokio::time::sleep(Duration::from_secs_f64(hands::MOVE_SECONDS)).await;
        true
    }

    fn name_of(&self, persona_id: &str) -> String {
        lock(&self.names)
            .get(persona_id)
            .cloned()
            .unwrap_or_else(|| "A teammate".into())
    }

    /// Waits for the next load to finish after `before`, or a refused one.
    async fn settle(
        &self,
        mut loads: watch::Receiver<Loads>,
        before: u64,
        wait: Duration,
    ) -> Result<bool, String> {
        let settled = tokio::time::timeout(wait, async {
            loop {
                {
                    let now = loads.borrow_and_update();
                    if let Some(blocked) = &now.blocked {
                        return Err(format!(
                            "The page went on to {blocked}, which your browser never opens: not a web page, or one of Hotline's own ports."
                        ));
                    }
                    if let Some(failed) = &now.failed {
                        return Err(failed.clone());
                    }
                    if now.finished > before {
                        return Ok(());
                    }
                }
                if loads.changed().await.is_err() {
                    return Ok(());
                }
            }
        })
        .await;
        match settled {
            Ok(Ok(())) => Ok(true),
            Ok(Err(said)) => Err(said),
            Err(_) => Ok(false),
        }
    }

    async fn script(&self, persona_id: &str, body: &'static str) -> Result<Read, String> {
        let json = self.run(persona_id, body.to_string()).await?;
        serde_json::from_str(&json)
            .map_err(|error| format!("The page's answer was unreadable: {error}"))
    }

    /// Runs `body` in the desk's script world and answers its string.
    async fn run(&self, persona_id: &str, body: String) -> Result<String, String> {
        let tab = self.tab(persona_id);
        let base = tab
            .as_ref()
            .map_or(0, |tab| tab.refs_next.load(Ordering::SeqCst));
        let answer = self
            .run_bare(persona_id, format!("const BASE = {base};\n{body}"))
            .await?;
        if let Some(tab) = tab
            && let Ok(said) = serde_json::from_str::<serde_json::Value>(&answer)
            && let Some(next) = said.get("next").and_then(serde_json::Value::as_u64)
        {
            tab.refs_next.fetch_max(next, Ordering::SeqCst);
        }
        Ok(answer)
    }

    /// Runs `body` as it is, in the desk's script world.
    async fn run_bare(&self, persona_id: &str, body: String) -> Result<String, String> {
        let front = self.window(persona_id)?;
        let (sender, answer) = oneshot::channel();
        let sender = Arc::new(Mutex::new(Some(sender)));
        on_view(front, move |view| linux::call(view, &body, sender));
        tokio::time::timeout(
            SCRIPT_WAIT + Duration::from_millis(hotline_browser_core::WAIT_LIMIT_MS),
            answer,
        )
        .await
        .map_err(|_| "The page did not answer in time.".to_string())?
        .map_err(|_| "The page closed before it answered.".to_string())?
    }

    /// Sends `input` to the teammate's page in front as the person's own.
    fn input(&self, persona_id: &str, input: linux::Input) -> Result<(), String> {
        on_view(self.window(persona_id)?, move |view| {
            linux::input(view, &input);
        });
        Ok(())
    }

    async fn page(&self, persona_id: &str, loaded: bool) -> Result<Page, String> {
        let read = self.script(persona_id, PAGE_INFO).await?;
        if let Some(tab) = self.tab(persona_id) {
            lock(&tab.shown).clone_from(&read.url);
        }
        Ok(Page {
            url: read.url,
            title: read.title,
            loaded,
        })
    }
}

/// Closes the tab's popup, if it has one: the teammate is back on the page,
/// and a call waiting on the popup's load is answered.
fn shut_popup(tab: &Tab) {
    let Some(popup) = lock(&tab.popup).take() else {
        return;
    };
    views::close(popup);
    // The page is in front again, for its viewers too.
    lock(&tab.popup_screen).take();
    tab.screens.show(&tab.persona, lock(&tab.screen).clone());
    let page = lock(&tab.page_shown).clone();
    *lock(&tab.shown) = page;
    tab.loads.send_modify(|loads| loads.finished += 1);
}

/// The act as act.js reads it.
fn act_args(act: &Act) -> String {
    let value = match act {
        Act::Click { target } => serde_json::json!({ "act": "click", "ref": target }),
        Act::Hover { target } => serde_json::json!({ "act": "hover", "ref": target }),
        Act::Fill { target, text } => {
            serde_json::json!({ "act": "fill", "ref": target, "text": text })
        }
        Act::FillSecret {
            target,
            text,
            part,
            field,
        } => {
            serde_json::json!({ "act": "fill_secret", "ref": target, "text": text, "part": part, "field": field })
        }
        Act::Select { target, values } => {
            serde_json::json!({ "act": "select", "ref": target, "values": values })
        }
        Act::Check { target, on } => {
            serde_json::json!({ "act": "check", "ref": target, "on": on })
        }
        Act::Press { key } => serde_json::json!({ "act": "press", "key": key }),
        Act::Scroll { target, dy } => {
            serde_json::json!({ "act": "scroll", "ref": target, "dy": dy })
        }
        Act::WaitFor {
            text,
            target,
            timeout_ms,
        } => {
            serde_json::json!({ "act": "wait_for", "find": text, "ref": target, "timeout": timeout_ms })
        }
    };
    value.to_string()
}

#[async_trait]
impl DeskBrowser for Native {
    async fn open(&self, persona_id: &str, url: &Url) -> Result<Page, String> {
        let (receiver, before) = match self.tab(persona_id) {
            Some(tab) => {
                // The teammate's own navigate leaves a popup for the page.
                shut_popup(&tab);
                tab.loads.send_modify(Loads::fresh);
                let receiver = tab.loads.subscribe();
                let before = receiver.borrow().finished;
                let address = url.to_string();
                on_view(tab.page, move |view| {
                    use webkit2gtk::WebViewExt;
                    view.load_uri(&address);
                });
                (receiver, before)
            }
            None => (self.make(persona_id, url).await?.loads.subscribe(), 0),
        };
        let loaded = self.settle(receiver, before, LOAD_WAIT).await?;
        self.page(persona_id, loaded).await
    }

    async fn step(&self, persona_id: &str, step: Step) -> Result<Page, String> {
        let tab = self.tab(persona_id).ok_or(NO_PAGE)?;
        tab.loads.send_modify(Loads::fresh);
        let receiver = tab.loads.subscribe();
        let before = receiver.borrow().finished;
        on_view(self.window(persona_id)?, move |view| {
            linux::step(view, step);
        });
        let wait = if step == Step::Reload {
            LOAD_WAIT
        } else {
            STEP_WAIT
        };
        let loaded = self.settle(receiver, before, wait).await?;
        // A history move served from the cache loads nothing and is
        // still done.
        self.page(persona_id, loaded || step != Step::Reload).await
    }

    async fn read(&self, persona_id: &str, reading: Reading) -> Result<String, String> {
        let body = match reading {
            Reading::Text => READ_TEXT,
            Reading::Links => READ_LINKS,
            Reading::Title => return Ok(self.script(persona_id, PAGE_INFO).await?.title),
        };
        let mut read = self.script(persona_id, body).await?;
        for _ in 0..EMPTY_RETRIES {
            if !read.text.trim().is_empty() {
                break;
            }
            tokio::time::sleep(EMPTY_WAIT).await;
            read = self.script(persona_id, body).await?;
        }
        let text = if read.text.trim().is_empty() {
            "(The page has no readable text.)".to_string()
        } else {
            read.text
        };
        Ok(format!(
            "Page: {}\nURL: {}\n\n{text}",
            read.title.trim(),
            read.url
        ))
    }

    async fn act(&self, persona_id: &str, act: &Act) -> Result<Acted, String> {
        #[derive(Deserialize)]
        struct Answer {
            point: Option<Spot>,
            note: Option<String>,
            error: Option<String>,
        }
        let tab = self.tab(persona_id).ok_or(NO_PAGE)?;
        tab.loads.send_modify(Loads::fresh);
        let receiver = tab.loads.subscribe();
        let (started, finished) = {
            let loads = receiver.borrow();
            (loads.started, loads.finished)
        };
        let popup_before = *lock(&tab.popup);
        let args = act_args(act);
        let seen = self.reach(persona_id, &tab, act, &args).await;
        let body = format!("const A = {args};\n{WALK}\n{ACT}");
        let answer: Answer = serde_json::from_str(&self.run(persona_id, body).await?)
            .map_err(|error| format!("The page's answer was unreadable: {error}"))?;
        if let Some(error) = answer.error {
            return Err(error);
        }
        match (act, answer.point) {
            (Act::Click { .. } | Act::Check { .. }, Some(point)) => {
                let Spot { x, y, height } = point;
                if seen {
                    on_view(self.window(persona_id)?, move |view| {
                        hands::ring(view, x, y, height);
                    });
                }
                self.input(persona_id, linux::Input::Click(x, y, height))?;
                tokio::time::sleep(Duration::from_millis(150)).await;
                // A click that loaded a page arrived; one that reached
                // nothing is made by the page's own script instead.
                let navigated = receiver.borrow().started > started;
                if !navigated && self.run(persona_id, ARRIVED.into()).await? == "false" {
                    self.run(persona_id, format!("const A = {args};\n{WALK}\n{CLICK}"))
                        .await?;
                }
            }
            (Act::Hover { .. }, Some(point)) => {
                self.input(
                    persona_id,
                    linux::Input::Hover(point.x, point.y, point.height),
                )?;
            }
            (Act::Press { key }, _) => {
                self.input(persona_id, linux::Input::Key(key.clone()))?;
            }
            _ => {}
        }
        tokio::time::sleep(ACT_SETTLE).await;
        let began = {
            let loads = receiver.borrow();
            if loads.picker_refused {
                return Err(PICKER_REFUSED.into());
            }
            if let Some(blocked) = &loads.blocked {
                return Err(format!(
                    "The page tried to go to {blocked}, which your browser never opens: not a web page, or one of Hotline's own ports."
                ));
            }
            loads.started > started
        };
        let loaded = if began {
            self.settle(receiver, finished, LOAD_WAIT).await?
        } else {
            true
        };
        let popup_now = *lock(&tab.popup);
        let moved = match (popup_before, popup_now) {
            (_, Some(now)) if popup_before != Some(now) => Some(POPUP_OPENED),
            (Some(_), None) => Some(POPUP_CLOSED),
            _ => None,
        };
        let note = match (answer.note, moved) {
            (Some(note), Some(moved)) => Some(format!("{note}\n{moved}")),
            (note, moved) => note.or_else(|| moved.map(str::to_string)),
        };
        Ok(Acted {
            page: self.page(persona_id, loaded).await?,
            note,
        })
    }

    async fn screenshot(&self, persona_id: &str) -> Result<Vec<u8>, String> {
        let front = self.window(persona_id)?;
        let (sender, answer) = oneshot::channel();
        let sender = Arc::new(Mutex::new(Some(sender)));
        on_view(front, move |view| linux::snapshot(view, sender));
        tokio::time::timeout(SCRIPT_WAIT, answer)
            .await
            .map_err(|_| "The page could not be drawn in time.".to_string())?
            .map_err(|_| "The page closed before it was drawn.".to_string())?
    }

    fn size(&self, persona_id: &str) -> Option<(u32, u32)> {
        let tab = self.tab(persona_id)?;
        let asked = *lock(&tab.viewport);
        asked.map(|(width, height)| (width as u32, height as u32))
    }

    async fn viewport(&self, persona_id: &str, size: Option<(u32, u32)>) -> Result<Page, String> {
        let tab = self
            .tab(persona_id)
            .ok_or("Open a page first with navigate.")?;
        *lock(&tab.viewport) = size.map(|(width, height)| (f64::from(width), f64::from(height)));
        on_view(self.window(persona_id)?, move |view| linux::fit(view, size));
        let fitted = tab.clone();
        gtk_do(move || views::fit(&fitted));
        // The page reflows at its new width before it is read or pictured.
        tokio::time::sleep(Duration::from_millis(250)).await;
        self.page(persona_id, true).await
    }

    async fn full_page(
        &self,
        persona_id: &str,
        most: usize,
    ) -> Result<hotline_browser_core::FullPage, String> {
        full_page::look(self, persona_id, most).await
    }

    async fn field(&self, persona_id: &str, target: &str) -> Result<Field, String> {
        #[derive(Deserialize)]
        struct Answer {
            field: Option<Field>,
            error: Option<String>,
        }
        let args = serde_json::json!({ "ref": target });
        let answer: Answer = serde_json::from_str(
            &self
                .run(persona_id, format!("const A = {args};\n{WALK}\n{FIELD}"))
                .await?,
        )
        .map_err(|error| format!("The page's answer was unreadable: {error}"))?;
        match (answer.field, answer.error) {
            (Some(field), _) => Ok(field),
            (None, error) => Err(error.unwrap_or_else(|| "The page answered nothing.".into())),
        }
    }

    async fn showing(&self, persona_id: &str) -> Result<Showing, String> {
        serde_json::from_str(&self.run(persona_id, SHOWING.to_string()).await?)
            .map_err(|_| "The page's answer was unreadable.".to_string())
    }

    async fn page_size(&self, persona_id: &str) -> Option<(u32, u32)> {
        #[derive(Deserialize)]
        struct Size {
            width: u32,
            height: u32,
        }
        let answer = self.run_bare(persona_id, PAGE_SIZE.into()).await.ok()?;
        let size: Size = serde_json::from_str(&answer).ok()?;
        Some((size.width, size.height))
    }

    async fn person_input(&self, persona_id: &str, input: &BrowserInput) -> Result<(), String> {
        #[derive(Deserialize)]
        struct Said {
            height: f64,
            error: Option<String>,
        }
        let tab = self
            .tab(persona_id)
            .ok_or("That teammate's browser has no page open.")?;
        let args = match input {
            BrowserInput::Key { key } => {
                self.input(persona_id, linux::Input::Key(key.clone()))?;
                tokio::time::sleep(PERSON_SETTLE).await;
                return Ok(());
            }
            BrowserInput::Tap { x, y } => serde_json::json!({ "kind": "tap", "x": x, "y": y }),
            BrowserInput::Scroll { dx, dy, x, y } => {
                serde_json::json!({ "kind": "scroll", "dx": dx, "dy": dy, "x": x, "y": y })
            }
            BrowserInput::Text { text } => serde_json::json!({ "kind": "text", "text": text }),
        };
        let said: Said = serde_json::from_str(
            &self
                .run_bare(persona_id, format!("const A = {args};\n{PERSON}"))
                .await?,
        )
        .map_err(|error| format!("The page's answer was unreadable: {error}"))?;
        if let Some(error) = said.error {
            return Err(error);
        }
        if let BrowserInput::Tap { x, y } = *input {
            let started = tab.loads.borrow().started;
            self.input(persona_id, linux::Input::Click(x, y, said.height))?;
            tokio::time::sleep(Duration::from_millis(150)).await;
            let navigated = tab.loads.borrow().started > started;
            if !navigated && self.run_bare(persona_id, ARRIVED.into()).await? == "false" {
                let args = serde_json::json!({ "kind": "click", "x": x, "y": y });
                self.run_bare(persona_id, format!("const A = {args};\n{PERSON}"))
                    .await?;
            }
        }
        tokio::time::sleep(PERSON_SETTLE).await;
        Ok(())
    }

    /// The person took the browser over: the teammate's pointer goes, and
    /// the person's own input reaches the page's display until they hand it
    /// back.
    fn taken(&self, persona_id: &str, taken: bool) {
        self.shared.screens.taken(persona_id, taken);
        let Some(tab) = self.tab(persona_id) else {
            return;
        };
        if taken {
            for view in [Some(tab.page), *lock(&tab.popup)].into_iter().flatten() {
                on_view(view, hands::hide);
            }
        }
    }

    fn current(&self, persona_id: &str) -> Option<String> {
        let tab = self.tab(persona_id)?;
        if let Some(untrusted) = tab.seen.untrusted() {
            return Some(untrusted.url);
        }
        let shown = lock(&tab.shown).clone();
        (!shown.is_empty()).then_some(shown)
    }

    fn name_teammate(&self, persona_id: &str, name: &str) {
        lock(&self.names).insert(persona_id.to_string(), name.to_string());
    }

    fn needs(&self, persona_id: &str) -> Option<String> {
        self.tab(persona_id)?
            .seen
            .untrusted()
            .map(|untrusted| untrusted.label.to_string())
    }

    /// Visit anyway from the phone or a linked desktop, as the app's bar
    /// does it, by the same trust code: the certificate the page in front
    /// was refused on is let through in the teammate's own web context, if
    /// it is still the one with the hash the person was shown, and the page
    /// loads again where it was refused.
    async fn visit_anyway(&self, persona_id: &str) -> Result<(), String> {
        let tab = self
            .tab(persona_id)
            .ok_or("That teammate's browser has closed.")?;
        let untrusted = tab
            .seen
            .untrusted()
            .ok_or("There is no refused certificate to visit past.")?;
        let fingerprint = untrusted
            .fingerprint
            .ok_or("Hotline did not see that site's certificate. Reload and try again.")?;
        let url = Url::parse(&untrusted.url)
            .map_err(|_| "That address could not be read.".to_string())?;
        self.shared
            .exceptions
            .add(persona_id, &untrusted.origin, &fingerprint);
        let window = self.window(persona_id)?;
        let origin = untrusted.origin.clone();
        let (sender, answer) = oneshot::channel();
        on_view(window, move |view| {
            let _ = sender.send(trust::let_through(view, &origin, &fingerprint));
        });
        answer
            .await
            .map_err(|_| "The browser closed before it answered.".to_string())??;
        tab.seen.clear();
        tab.loads.send_modify(Loads::fresh);
        let receiver = tab.loads.subscribe();
        let before = receiver.borrow().finished;
        let address = url.to_string();
        on_view(window, move |view| {
            use webkit2gtk::WebViewExt;
            view.load_uri(&address);
        });
        self.settle(receiver, before, LOAD_WAIT).await.map(drop)
    }

    async fn close(&self, persona_id: &str, wipe: bool) -> Result<(), String> {
        self.close_tab(persona_id, wipe).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A server has no pane and no bar above its page: its warning and the
    /// teammate send the person to the seat they drive the browser from.
    #[test]
    fn a_servers_refused_certificate_sends_the_person_to_their_phone() {
        for words in [VISIT_ANYWAY_ON_PAGE, VISIT_ANYWAY_FOR_TEAMMATE] {
            assert!(words.contains("phone"), "{words}");
            assert!(!words.contains("above"), "{words}");
            assert!(!words.contains("pane"), "{words}");
        }
        assert!(!PICKER_REFUSED.contains("pane"));
    }
}
