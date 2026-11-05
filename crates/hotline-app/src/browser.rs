//! Each teammate's own browser on the desk: a web view in a window of its own,
//! hidden while the teammate works in it, that the core drives through
//! [`DeskBrowser`] (`hotline_core::browser`). It is built on macOS (WKWebView)
//! and Linux (WebKitGTK): both answer a script run in a world of the desk's
//! own, out of the page's sight, and draw a snapshot of themselves. Windows
//! installs nothing yet and no teammate there is offered the tool.
//!
//! The tabs, the [`DeskBrowser`] impl, how a load is waited for, a read, an
//! act and the pane's bar commands are one code for both. Only the calls
//! that need the platform's web view go through a small seam, `platform`:
//! `apple` (`browser/native/apple.rs`) on macOS, `linux`
//! (`browser/native/linux.rs`) on Linux. Each answers a script from its
//! isolated world, steps through history, hides and shows the pane, puts the
//! shield over a watched page, sends input as the person's own, and draws a
//! snapshot; the pointer overlay (`hands`), certificate trust (`trust`), the
//! first load (`guard`), lent cookies (`cookies`) and what a page may ask of
//! the person's computer (`asks`) have a side for each.
//!
//! What it may load is the core's [`Gate`]: every top-level load, the
//! agent's and the page's own, is asked first, so a page cannot send the
//! browser to a local address the person has not allowed. A window is never
//! opened for a page, and downloads are refused. Each teammate's cookies
//! and sign-ins live in a web data store of its own, apart from Hotline's
//! window and every other teammate's: WebKit's store by an id on macOS, a
//! data directory of its own under Hotline's on Linux. No capability names
//! these windows, so a page in one reaches none of the shell's commands.
//! A page is never given the microphone, the camera or any other permission
//! it asks for, and its file picker opens only while the person has
//! control. A Mac before macOS 14 keeps no desk browser at all: its WebKit
//! has no store by id, so every teammate would share Hotline's window's
//! (`hotline_core::browser::kept_here`).
//! Passkeys are answered on macOS only: WebKitGTK's UI-process API does not
//! tell a script message which frame's origin sent it (only a web-process
//! extension could, and none is built), and the desk takes an origin only
//! from the web view, never from the page (`docs/security.md`).

use hotline_core::desk::Desk;
use tauri::AppHandle;

/// Installs the browser on this desk, where the system keeps one
/// (`hotline_core::browser::kept_here`): macOS 14 and later, and Linux.
/// Elsewhere nothing is installed and no teammate is offered the tool. The
/// Door's port and, in development, the window's own dev server, are the
/// ports it never opens.
pub(crate) fn install(app: &AppHandle, desk: &Desk, door_port: u16) {
    if !hotline_core::browser::kept_here() {
        return;
    }
    let mut never = vec![door_port];
    if let Some(port) = app
        .config()
        .build
        .dev_url
        .as_ref()
        .filter(|_| tauri::is_dev())
        .and_then(tauri::Url::port_or_known_default)
    {
        never.push(port);
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        use tauri::Manager;
        let native = std::sync::Arc::new(native::Native::new(app.clone(), desk.browser_gate()));
        desk.install_browser(native.clone(), &never);
        app.manage(native);
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let _ = (desk, never);
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
type Browsers<'a> = tauri::State<'a, std::sync::Arc<native::Native>>;
type DeskState<'a> = tauri::State<'a, std::sync::Arc<Desk>>;

const NOT_HERE: &str = "This desk keeps no browser: it needs macOS 14 or later, or Linux.";

/// The desk's browsers, where it keeps them.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn browsers(app: &AppHandle) -> Result<Browsers<'_>, String> {
    use tauri::Manager;
    app.try_state().ok_or_else(|| NOT_HERE.to_string())
}

/// Whether the person has control of the teammate's browser now, as the
/// core keeps it: from the pane's Take control until Give it back.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn person_has_it(desk: &Desk, persona_id: &str) -> bool {
    desk.browser_now(persona_id)
        .is_some_and(|view| view.phase == hotline_core::contract::BrowserPhase::Person)
}

/// The PIP's click: the teammate's browser pane, opened over the desk.
#[tauri::command]
pub fn browser_pane_open(app: AppHandle, persona_id: String, name: String) -> Result<(), String> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        browsers(&app)?.open_pane(&persona_id, &name)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (app, persona_id, name);
        Err(NOT_HERE.into())
    }
}

/// What the pane's bar shows: whose browser, the page, and whether the
/// person has it.
#[tauri::command]
pub fn browser_bar_state(
    app: AppHandle,
    desk: DeskState<'_>,
    persona_id: String,
) -> Result<serde_json::Value, String> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let taken = person_has_it(&desk, &persona_id);
        let mut state = serde_json::to_value(browsers(&app)?.bar_state(&persona_id, taken)?)
            .map_err(|error| error.to_string())?;
        state["title"] = desk
            .browser_now(&persona_id)
            .map(|view| view.title)
            .unwrap_or_default()
            .into();
        Ok(state)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (app, desk, persona_id);
        Err(NOT_HERE.into())
    }
}

/// Take over (`taken`) or Hand back, from the pane's bar.
#[tauri::command]
pub async fn browser_bar_take(
    app: AppHandle,
    desk: DeskState<'_>,
    persona_id: String,
    taken: bool,
) -> Result<(), String> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let browsers = browsers(&app)?;
        desk.take_browser(&persona_id, taken).await?;
        browsers.shield(&persona_id, !taken)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (app, desk, persona_id, taken);
        Err(NOT_HERE.into())
    }
}

/// Visit anyway, past a certificate the page was refused for: only from the
/// bar, and only while the person has the page.
#[tauri::command]
pub async fn browser_bar_trust(
    app: AppHandle,
    desk: DeskState<'_>,
    persona_id: String,
) -> Result<(), String> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        if !person_has_it(&desk, &persona_id) {
            return Err("Take control first: only you can visit past a certificate.".into());
        }
        let browsers = browsers(&app)?;
        let visited = browsers.visit_anyway(&persona_id).await;
        desk.browser_person_moved(&persona_id).await;
        visited
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (app, desk, persona_id);
        Err(NOT_HERE.into())
    }
}

/// Back, forward or reload from the bar, or a typed address: only while the
/// person has the page.
#[tauri::command]
pub async fn browser_bar_go(
    app: AppHandle,
    desk: DeskState<'_>,
    persona_id: String,
    step: Option<String>,
    url: Option<String>,
) -> Result<(), String> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        use hotline_core::browser::Step;
        if !person_has_it(&desk, &persona_id) {
            return Err("Take over first: the teammate is driving.".into());
        }
        let browsers = browsers(&app)?;
        match (step.as_deref(), url) {
            (Some("back"), _) => browsers.person_step(&persona_id, Step::Back),
            (Some("forward"), _) => browsers.person_step(&persona_id, Step::Forward),
            (Some("reload"), _) => browsers.person_step(&persona_id, Step::Reload),
            (_, Some(url)) => {
                let typed = url.trim();
                let url = tauri::Url::parse(typed)
                    .or_else(|_| tauri::Url::parse(&format!("https://{typed}")))
                    .map_err(|_| "That is not a web address.".to_string())?;
                desk.browser_person_opens(&persona_id, &url).await?;
                let went = browsers.person_go(&persona_id, &url).await;
                desk.browser_person_moved(&persona_id).await;
                went
            }
            _ => Err("Nothing to go to.".into()),
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (app, desk, persona_id, step, url);
        Err(NOT_HERE.into())
    }
}

#[cfg(target_os = "macos")]
mod passkeys;

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod native {
    mod cookies;

    use async_trait::async_trait;
    use hotline_core::browser::{Act, Acted, DeskBrowser, Gate, Page, Reading, Step};
    use serde::Deserialize;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, PoisonError};
    use std::time::Duration;
    use tauri::utils::config::BackgroundThrottlingPolicy;
    use tauri::webview::WebviewBuilder;
    use tauri::webview::{NewWindowResponse, PageLoadEvent};
    use tauri::window::WindowBuilder;
    use tauri::{
        AppHandle, LogicalPosition, LogicalSize, Manager, Url, Webview, WebviewUrl, WindowEvent,
    };
    use tokio::sync::{oneshot, watch};

    mod asks;
    mod full_page;
    mod guard;
    #[cfg(target_os = "macos")]
    mod hands;
    #[cfg(target_os = "linux")]
    #[path = "linux/hands.rs"]
    mod hands;
    mod trust;

    #[cfg(target_os = "macos")]
    mod apple;
    #[cfg(target_os = "linux")]
    mod linux;
    /// The seam: the few calls that need the platform's own web view.
    #[cfg(target_os = "macos")]
    use apple as platform;
    #[cfg(target_os = "linux")]
    use linux as platform;

    const WALK: &str = include_str!("browser/walk.js");
    const READ_TEXT: &str = concat!(
        include_str!("browser/walk.js"),
        include_str!("browser/read_text.js")
    );
    const READ_LINKS: &str = include_str!("browser/read_links.js");
    const ACT: &str = include_str!("browser/act.js");
    const LOCATE: &str = include_str!("browser/locate.js");
    /// How long after its last move the teammate's pointer fades.
    const HANDS_IDLE: Duration = Duration::from_secs(4);
    /// Whether the last native click or hover reached the page.
    const ARRIVED: &str = "return JSON.stringify(globalThis.__hotlineArrived !== false);";
    /// The click the page's own script makes, when a native one did not
    /// arrive: walk.js goes in front, and `A` names the ref.
    const CLICK: &str = "const el = refs.get(A.ref); if (el) { el.focus?.(); el.click(); } return JSON.stringify(!!el);";
    /// How long after an act a load it started is looked for.
    const ACT_SETTLE: Duration = Duration::from_millis(400);
    const PAGE_INFO: &str =
        "return JSON.stringify({ title: document.title, url: location.href, text: '' });";

    /// The user agent Safari on this Mac sends: WebKit's own frozen prefix
    /// and Safari's version, read from Safari itself. A bare WKWebView sends
    /// the prefix alone, and sites that look for a browser they know (Gmail)
    /// call that an unsupported one. None when Safari cannot be read, and
    /// then the web view's own goes. On Linux the web view's own goes:
    /// WebKitGTK's standard agent is already Safari's form, on X11 with
    /// WebKitGTK's own version, the one Epiphany sends, and WebKit's own
    /// quirks answer the few sites that want more.
    #[cfg(target_os = "macos")]
    fn safari_user_agent() -> Option<&'static str> {
        static AGENT: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
        AGENT
            .get_or_init(|| {
                use objc2_foundation::{NSBundle, NSString};
                let safari = NSBundle::bundleWithPath(&NSString::from_str("/Applications/Safari.app"))?;
                let version = safari
                    .objectForInfoDictionaryKey(&NSString::from_str("CFBundleShortVersionString"))?
                    .downcast::<NSString>()
                    .ok()?
                    .to_string();
                Some(format!(
                    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/{version} Safari/605.1.15"
                ))
            })
            .as_deref()
    }
    /// How long a load is waited for before the call answers that the page
    /// is still loading.
    const LOAD_WAIT: Duration = Duration::from_secs(20);
    /// How long a move through history is waited for: one served from the
    /// back-forward cache, or within the page, loads nothing.
    const STEP_WAIT: Duration = Duration::from_secs(5);
    /// How often, and how far apart, a read of an empty page looks again.
    const EMPTY_RETRIES: usize = 4;
    const EMPTY_WAIT: Duration = Duration::from_millis(500);
    const SCRIPT_WAIT: Duration = Duration::from_secs(15);
    /// The page's size: a common laptop viewport.
    const WIDTH: f64 = 1280.0;
    const HEIGHT: f64 = 800.0;
    /// Hotline's bar above the page in the browser pane.
    const BAR: f64 = 44.0;
    /// The window's own title bar above it: the traffic lights, the title
    /// and the place the person drags the window by, all macOS's. On Linux
    /// the window manager draws the title bar outside the window.
    #[cfg(target_os = "macos")]
    const TITLE: f64 = 28.0;
    #[cfg(target_os = "linux")]
    const TITLE: f64 = 0.0;

    /// One teammate's browser: its window, the page and the bar in it, who
    /// is driving, and its loads so far.
    struct Tab {
        window: String,
        page: String,
        bar: String,
        /// The teammate's name, for the pane's title.
        name: Mutex<String>,
        loads: watch::Sender<Loads>,
        /// The address the page last finished loading or was read at, kept
        /// here because wry's own `url()` panics on a page that has none
        /// yet, as one whose first load was refused.
        shown: Mutex<String>,
        /// Certificates its loads offered, and the last one refused.
        seen: trust::Seen,
        /// Counts the pointer's moves, so it fades only after the last.
        moves: std::sync::atomic::AtomicU64,
        /// The highest ref number any page of this tab has given, so a new
        /// page's refs go on from it (walk.js's `BASE`).
        refs_next: std::sync::atomic::AtomicU64,
    }

    /// What the bar above the page shows.
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    pub(crate) struct BarState {
        name: String,
        url: String,
        title: String,
        taken: bool,
        /// The bar's label for a load refused for its certificate, as
        /// "Certificate expired", which the person may visit anyway.
        untrusted: Option<&'static str>,
        /// The page is on a certificate the person chose to visit anyway.
        excepted: bool,
    }

    #[derive(Clone, Default)]
    struct Loads {
        started: u64,
        finished: u64,
        /// The last load the gate refused, since the call that cleared it.
        blocked: Option<String>,
        /// Why the last load failed to start, said in full, since the call
        /// that cleared it.
        failed: Option<String>,
        /// The page asked for a file picker while the teammate drove, and
        /// was refused, since the call that cleared it (`asks`).
        picker_refused: bool,
    }

    impl Loads {
        /// A new load from the teammate or the person: nothing refused or
        /// failed yet.
        fn fresh(&mut self) {
            self.blocked = None;
            self.failed = None;
            self.picker_refused = false;
        }
    }

    /// A point in the page's viewport, in CSS pixels, with the viewport's
    /// height, as act.js and locate.js answer it.
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

    pub(super) struct Native {
        app: AppHandle,
        gate: Arc<Gate>,
        tabs: Mutex<HashMap<String, Arc<Tab>>>,
        exceptions: Arc<trust::Exceptions>,
        /// Each teammate's name, as its pointer is tagged.
        names: Mutex<HashMap<String, String>>,
    }

    /// A label is letters, digits and `-`, so the teammate's id is written
    /// in hex: the window, the page in it and Hotline's bar above the page.
    fn labels_of(persona_id: &str) -> (String, String, String) {
        let hex: String = persona_id
            .bytes()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        (
            format!("browser-{hex}"),
            format!("browser-{hex}-page"),
            format!("browser-{hex}-bar"),
        )
    }

    /// Lays the bar and the page out in the window's current size.
    fn lay_out(app: &AppHandle, tab: &Tab) {
        let Some(window) = app.get_window(&tab.window) else {
            return;
        };
        let (Ok(size), Ok(scale)) = (window.inner_size(), window.scale_factor()) else {
            return;
        };
        let size = size.to_logical::<f64>(scale);
        if let Some(bar) = app.get_webview(&tab.bar) {
            let _ = bar.set_position(LogicalPosition::new(0.0, TITLE));
            let _ = bar.set_size(LogicalSize::new(size.width, BAR));
        }
        if let Some(page) = app.get_webview(&tab.page) {
            let _ = page.set_position(LogicalPosition::new(0.0, TITLE + BAR));
            let _ = page.set_size(LogicalSize::new(
                size.width,
                (size.height - TITLE - BAR).max(1.0),
            ));
            let _ = page.with_webview(|view| {
                platform::shield_follow(view.inner());
                hands::follow(view.inner());
            });
        }
    }

    impl Native {
        pub(super) fn new(app: AppHandle, gate: Arc<Gate>) -> Self {
            Self {
                app,
                gate,
                tabs: Mutex::default(),
                exceptions: Arc::default(),
                names: Mutex::default(),
            }
        }

        fn tab(&self, persona_id: &str) -> Option<Arc<Tab>> {
            let tabs = self.tabs.lock().unwrap_or_else(PoisonError::into_inner);
            tabs.get(persona_id)
                .filter(|tab| self.app.get_webview(&tab.page).is_some())
                .cloned()
        }

        /// The teammate's page.
        fn window(&self, persona_id: &str) -> Result<Webview, String> {
            self.tab(persona_id)
                .and_then(|tab| self.app.get_webview(&tab.page))
                .ok_or_else(|| "Your browser has no page open yet: navigate to one first.".into())
        }

        /// Opens the teammate's browser pane: the window made visible over
        /// the desk, watching. `name` titles it.
        pub(crate) fn open_pane(&self, persona_id: &str, name: &str) -> Result<(), String> {
            let tab = self
                .tab(persona_id)
                .ok_or("That teammate's browser has no page open.")?;
            *tab.name.lock().unwrap_or_else(PoisonError::into_inner) = name.to_string();
            let window = self
                .app
                .get_window(&tab.window)
                .ok_or("That teammate's browser has closed.")?;
            let _ = window.set_title(&format!("{name}'s browser"));
            lay_out(&self.app, &tab);
            self.window(persona_id)?
                .with_webview(|view| platform::reveal(view.inner()))
                .map_err(|error| error.to_string())?;
            let _ = window.set_focus();
            Ok(())
        }

        /// What the pane's bar shows now.
        pub(crate) fn bar_state(&self, persona_id: &str, taken: bool) -> Result<BarState, String> {
            let tab = self
                .tab(persona_id)
                .ok_or("That teammate's browser has closed.")?;
            self.window(persona_id)?;
            let shown = tab
                .shown
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            Ok(BarState {
                name: tab
                    .name
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone(),
                url: tab
                    .seen
                    .untrusted()
                    .map_or_else(|| shown.clone(), |untrusted| untrusted.url),
                title: String::new(),
                taken,
                untrusted: tab.seen.untrusted().map(|untrusted| untrusted.label),
                excepted: tauri::Url::parse(&shown)
                    .ok()
                    .as_ref()
                    .and_then(hotline_core::browser::origin_key)
                    .is_some_and(|origin| self.exceptions.any_on(persona_id, &origin)),
            })
        }

        /// Visit anyway, from the bar while the person has control: lets the
        /// certificate the last load was refused for through, for this
        /// teammate and that host and port, until the desk stops, and loads
        /// the page again.
        pub(crate) async fn visit_anyway(&self, persona_id: &str) -> Result<(), String> {
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
            self.exceptions
                .add(persona_id, &untrusted.origin, &fingerprint);
            // WebKitGTK asks nothing at load time: the certificate it refused
            // is let through in the teammate's own context here, by its hash.
            #[cfg(target_os = "linux")]
            {
                let origin = untrusted.origin.clone();
                let (sender, answer) = oneshot::channel();
                self.window(persona_id)?
                    .with_webview(move |view| {
                        let _ =
                            sender.send(trust::let_through(view.inner(), &origin, &fingerprint));
                    })
                    .map_err(|error| error.to_string())?;
                answer
                    .await
                    .map_err(|_| "The browser closed before it answered.".to_string())??;
            }
            tab.seen.clear();
            self.person_go(persona_id, &url).await
        }

        /// The person took the page over or handed it back: the shield that
        /// keeps a watcher's clicks off the page goes or comes back.
        pub(crate) fn shield(&self, persona_id: &str, on: bool) -> Result<(), String> {
            self.window(persona_id)?
                .with_webview(move |view| {
                    platform::shield(view.inner(), on);
                    // The person driving has their own pointer.
                    if !on {
                        hands::hide(view.inner());
                    }
                })
                .map_err(|error| error.to_string())
        }

        /// The person's own move, from the bar, while they have the page.
        pub(crate) fn person_step(&self, persona_id: &str, step: Step) -> Result<(), String> {
            self.window(persona_id)?
                .with_webview(move |view| platform::step(view.inner(), step))
                .map_err(|error| error.to_string())
        }

        pub(crate) async fn person_go(&self, persona_id: &str, url: &Url) -> Result<(), String> {
            if !self.gate.admits(url) {
                return Err(
                    "That address is not a web page, or is one of Hotline's own ports.".into(),
                );
            }
            let tab = self
                .tab(persona_id)
                .ok_or("That teammate's browser has no page open.")?;
            tab.loads.send_modify(Loads::fresh);
            let receiver = tab.loads.subscribe();
            let before = receiver.borrow().finished;
            self.window(persona_id)?
                .navigate(url.clone())
                .map_err(|error| error.to_string())?;
            self.settle(receiver, before, LOAD_WAIT).await.map(|_| ())
        }

        /// Makes the teammate's window on `url`: Hotline's bar above the
        /// page, hidden until the person opens the pane, or always shown
        /// when `HOTLINE_BROWSER_SHOW` is set, which development uses.
        fn make(&self, persona_id: &str, url: &Url) -> Result<Arc<Tab>, String> {
            let (window_label, page_label, bar_label) = labels_of(persona_id);
            let (loads, _) = watch::channel(Loads::default());
            let tab = Arc::new(Tab {
                window: window_label.clone(),
                page: page_label.clone(),
                bar: bar_label.clone(),
                name: Mutex::new(String::from("A teammate")),
                loads,
                shown: Mutex::new(String::new()),
                seen: trust::Seen::default(),
                moves: std::sync::atomic::AtomicU64::new(0),
                refs_next: std::sync::atomic::AtomicU64::new(0),
            });
            let show = std::env::var_os("HOTLINE_BROWSER_SHOW").is_some();
            // Nothing loads until the desk's delegate is in front of wry's.
            let started = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let navigating = {
                let gate = self.gate.clone();
                let tab = tab.clone();
                let started = started.clone();
                move |url: &Url| {
                    if !started.load(std::sync::atomic::Ordering::Acquire) {
                        return false;
                    }
                    let admitted = gate.admits(url);
                    if admitted {
                        if url.as_str() != "about:blank" {
                            *tab.seen
                                .attempted
                                .lock()
                                .unwrap_or_else(PoisonError::into_inner) = url.to_string();
                        }
                    } else {
                        tab.loads
                            .send_modify(|loads| loads.blocked = Some(url.to_string()));
                    }
                    admitted
                }
            };
            let loaded = {
                let tab = tab.clone();
                move |_: Webview, payload: tauri::webview::PageLoadPayload<'_>| match payload
                    .event()
                {
                    PageLoadEvent::Started => tab.loads.send_modify(|loads| loads.started += 1),
                    PageLoadEvent::Finished => {
                        // The warning a refused certificate leaves is a blank
                        // page of the desk's: the refusal stands behind it.
                        if payload.url().as_str() != "about:blank" {
                            tab.seen.clear();
                        }
                        *tab.shown.lock().unwrap_or_else(PoisonError::into_inner) =
                            payload.url().to_string();
                        tab.loads.send_modify(|loads| loads.finished += 1);
                    }
                }
            };
            // A page that opens a window gets the same view instead, if the
            // gate lets it load there.
            let popup = {
                let app = self.app.clone();
                let page_label = page_label.clone();
                move |url: Url, _| {
                    let app = app.clone();
                    let page_label = page_label.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Some(page) = app.get_webview(&page_label) {
                            let _ = page.navigate(url);
                        }
                    });
                    NewWindowResponse::Deny
                }
            };
            // Nothing covers the top strip, so it stays the window's own
            // title bar, dark like the bar below it.
            let window = WindowBuilder::new(&self.app, &window_label).title("Hotline browser");
            #[cfg(target_os = "macos")]
            let window = window.title_bar_style(tauri::TitleBarStyle::Overlay);
            let window = window
                .theme(Some(tauri::Theme::Dark))
                .background_color(tauri::window::Color(28, 28, 31, 255))
                .inner_size(WIDTH, TITLE + BAR + HEIGHT)
                .min_inner_size(480.0, 320.0)
                .visible(show)
                .focused(false)
                .build()
                .map_err(|error| format!("Your browser could not be opened: {error}"))?;
            let blank = Url::parse("about:blank").map_err(|error| error.to_string())?;
            let page = WebviewBuilder::new(&page_label, WebviewUrl::External(blank))
                .background_throttling(BackgroundThrottlingPolicy::Disabled)
                .on_navigation(navigating)
                .on_new_window(popup)
                .on_download(|_, _| false)
                .on_page_load(loaded);
            #[cfg(target_os = "macos")]
            let page = {
                let mut page = page
                    .data_store_identifier(hotline_core::browser::store_id(persona_id))
                    .initialization_script_for_all_frames(super::passkeys::SCRIPT);
                if let Some(agent) = safari_user_agent() {
                    page = page.user_agent(agent);
                }
                page
            };
            // A web context of its own, on a data directory of its own.
            #[cfg(target_os = "linux")]
            let page = page.data_directory(linux::store_dir(persona_id));
            let page = window
                .add_child(
                    page,
                    LogicalPosition::new(0.0, TITLE + BAR),
                    LogicalSize::new(WIDTH, HEIGHT),
                )
                .map_err(|error| format!("Your browser could not be opened: {error}"))?;
            // GTK packs the page in the window's box: it goes in an overlay
            // of the desk's, kept off screen until the pane opens.
            #[cfg(target_os = "linux")]
            page.with_webview(move |view| linux::seat(view.inner(), show))
                .map_err(|error| format!("Your browser could not be opened: {error}"))?;
            {
                let (persona, tab, exceptions) =
                    (persona_id.to_string(), tab.clone(), self.exceptions.clone());
                page.with_webview(move |view| {
                    trust::attach(view.inner(), persona, tab, exceptions);
                })
                .map_err(|error| format!("Your browser could not be opened: {error}"))?;
            }
            {
                // No microphone or camera, ever, and a file picker only while
                // the person has the page.
                let (app, persona, tab) = (self.app.clone(), persona_id.to_string(), tab.clone());
                page.with_webview(move |view| {
                    asks::attach(view.inner(), tab, move || {
                        app.try_state::<Arc<hotline_core::desk::Desk>>()
                            .is_some_and(|desk| super::person_has_it(&desk, &persona))
                    });
                })
                .map_err(|error| format!("Your browser could not be opened: {error}"))?;
            }
            {
                let url = url.clone();
                page.with_webview(move |view| guard::start(view.inner(), url, started))
                    .map_err(|error| format!("Your browser could not be opened: {error}"))?;
            }
            #[cfg(target_os = "macos")]
            super::passkeys::attach(&page, persona_id);
            let hash = format!("persona={}", urlencode(persona_id));
            let bar = WebviewBuilder::new(
                &bar_label,
                WebviewUrl::App(format!("browser.html#{hash}").into()),
            );
            let bar = window
                .add_child(
                    bar,
                    LogicalPosition::new(0.0, TITLE),
                    LogicalSize::new(WIDTH, BAR),
                )
                .map_err(|error| format!("Your browser could not be opened: {error}"))?;
            #[cfg(target_os = "linux")]
            bar.with_webview(|view| linux::seat_bar(view.inner(), BAR))
                .map_err(|error| format!("Your browser could not be opened: {error}"))?;
            #[cfg(target_os = "macos")]
            let _ = bar;
            {
                let app = self.app.clone();
                let tab = tab.clone();
                window.on_window_event(move |event| match event {
                    WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                        lay_out(&app, &tab);
                    }
                    // Closing the pane hides it: the teammate's browser goes
                    // on in the background.
                    WindowEvent::CloseRequested { api, .. } => {
                        api.prevent_close();
                        if let Some(page) = app.get_webview(&tab.page) {
                            let _ = page.with_webview(|view| platform::conceal(view.inner()));
                        }
                    }
                    _ => {}
                });
            }
            self.tabs
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(persona_id.to_string(), tab.clone());
            Ok(tab)
        }

        /// Before an act the person can watch: brings its element into view
        /// and moves the teammate's pointer onto it, then waits for it to
        /// arrive. Answers whether the pointer is there; an act goes on
        /// either way.
        async fn reach(&self, persona_id: &str, tab: &Arc<Tab>, act: &Act, args: &str) -> bool {
            let watched = matches!(
                act,
                Act::Click { .. }
                    | Act::Check { .. }
                    | Act::Hover { .. }
                    | Act::Fill { .. }
                    | Act::Select { .. }
                    | Act::Scroll { .. }
            );
            if !watched {
                return false;
            }
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
            let Ok(page) = self.window(persona_id) else {
                return false;
            };
            if page
                .with_webview(move |view| hands::point(view.inner(), x, y, height, &who))
                .is_err()
            {
                return false;
            }
            let moves = tab.moves.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            let (app, label, still) = (self.app.clone(), tab.page.clone(), tab.clone());
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(HANDS_IDLE).await;
                if still.moves.load(std::sync::atomic::Ordering::SeqCst) == moves
                    && let Some(page) = app.get_webview(&label)
                {
                    let _ = page.with_webview(|view| hands::hide(view.inner()));
                }
            });
            tokio::time::sleep(Duration::from_secs_f64(hands::MOVE_SECONDS)).await;
            true
        }

        /// The teammate's name for its pointer's tag.
        fn name_of(&self, persona_id: &str) -> String {
            self.names
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
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

        /// Runs `body` in the desk's content world and answers its string.
        async fn run(&self, persona_id: &str, body: String) -> Result<String, String> {
            let tab = self.tab(persona_id);
            let base = tab.as_ref().map_or(0, |tab| {
                tab.refs_next.load(std::sync::atomic::Ordering::SeqCst)
            });
            let answer = self
                .run_bare(persona_id, format!("const BASE = {base};\n{body}"))
                .await?;
            // A walk says how far its page has numbered refs.
            if let Some(tab) = tab
                && let Ok(said) = serde_json::from_str::<serde_json::Value>(&answer)
                && let Some(next) = said.get("next").and_then(serde_json::Value::as_u64)
            {
                tab.refs_next
                    .fetch_max(next, std::sync::atomic::Ordering::SeqCst);
            }
            Ok(answer)
        }

        /// Runs `body` as it is, in the desk's content world.
        async fn run_bare(&self, persona_id: &str, body: String) -> Result<String, String> {
            let window = self.window(persona_id)?;
            let (sender, answer) = oneshot::channel();
            let sender = Arc::new(Mutex::new(Some(sender)));
            window
                .with_webview(move |view| platform::call(view.inner(), &body, sender))
                .map_err(|error| error.to_string())?;
            tokio::time::timeout(
                SCRIPT_WAIT + Duration::from_millis(hotline_core::browser::WAIT_LIMIT_MS),
                answer,
            )
            .await
            .map_err(|_| "The page did not answer in time.".to_string())?
            .map_err(|_| "The page closed before it answered.".to_string())?
        }

        /// Sends `input` to the teammate's web view as the person's own.
        fn input(&self, persona_id: &str, input: platform::Input) -> Result<(), String> {
            self.window(persona_id)?
                .with_webview(move |view| platform::input(view.inner(), &input))
                .map_err(|error| error.to_string())
        }

        async fn page(&self, persona_id: &str, loaded: bool) -> Result<Page, String> {
            let read = self.script(persona_id, PAGE_INFO).await?;
            if let Some(tab) = self.tab(persona_id) {
                tab.shown
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone_from(&read.url);
            }
            Ok(Page {
                url: read.url,
                title: read.title,
                loaded,
            })
        }
    }

    /// A teammate's id as it goes in the bar's address, which only ever
    /// carries ids Hotline made.
    fn urlencode(text: &str) -> String {
        text.bytes()
            .map(|byte| match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => {
                    (byte as char).to_string()
                }
                other => format!("%{other:02X}"),
            })
            .collect()
    }

    /// The act as act.js reads it.
    fn act_args(act: &Act) -> String {
        let value = match act {
            Act::Click { target } => serde_json::json!({ "act": "click", "ref": target }),
            Act::Hover { target } => serde_json::json!({ "act": "hover", "ref": target }),
            Act::Fill { target, text } => {
                serde_json::json!({ "act": "fill", "ref": target, "text": text })
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
                    tab.loads.send_modify(Loads::fresh);
                    let receiver = tab.loads.subscribe();
                    let before = receiver.borrow().finished;
                    self.window(persona_id)?
                        .navigate(url.clone())
                        .map_err(|error| error.to_string())?;
                    (receiver, before)
                }
                None => (self.make(persona_id, url)?.loads.subscribe(), 0),
            };
            let loaded = self.settle(receiver, before, LOAD_WAIT).await?;
            self.page(persona_id, loaded).await
        }

        async fn step(&self, persona_id: &str, step: Step) -> Result<Page, String> {
            let tab = self
                .tab(persona_id)
                .ok_or("Your browser has no page open yet: navigate to one first.")?;
            tab.loads.send_modify(Loads::fresh);
            let receiver = tab.loads.subscribe();
            let before = receiver.borrow().finished;
            let window = self.window(persona_id)?;
            window
                .with_webview(move |view| platform::step(view.inner(), step))
                .map_err(|error| error.to_string())?;
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
            // A page that draws itself after it loaded (a sign-in app, a
            // dashboard) is empty for a moment: look again before saying so.
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
            let tab = self
                .tab(persona_id)
                .ok_or("Your browser has no page open yet: navigate to one first.")?;
            tab.loads.send_modify(Loads::fresh);
            let receiver = tab.loads.subscribe();
            let (started, finished) = {
                let loads = receiver.borrow();
                (loads.started, loads.finished)
            };
            let args = act_args(act);
            // The person watching sees the pointer reach the element first.
            let seen = self.reach(persona_id, &tab, act, &args).await;
            let body = format!("const A = {args};\n{WALK}\n{ACT}");
            #[derive(Deserialize)]
            struct Point {
                x: f64,
                y: f64,
                height: f64,
            }
            #[derive(Deserialize)]
            struct Answer {
                point: Option<Point>,
                note: Option<String>,
                error: Option<String>,
            }
            let answer: Answer = serde_json::from_str(&self.run(persona_id, body).await?)
                .map_err(|error| format!("The page's answer was unreadable: {error}"))?;
            if let Some(error) = answer.error {
                return Err(error);
            }
            match (act, answer.point) {
                (Act::Click { .. } | Act::Check { .. }, Some(point)) => {
                    let (x, y, height) = (point.x, point.y, point.height);
                    if seen {
                        let _ = self
                            .window(persona_id)?
                            .with_webview(move |view| hands::ring(view.inner(), x, y, height));
                    }
                    self.input(
                        persona_id,
                        platform::Input::Click(point.x, point.y, point.height),
                    )?;
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
                        platform::Input::Hover(point.x, point.y, point.height),
                    )?;
                }
                (Act::Press { key }, _) => {
                    self.input(persona_id, platform::Input::Key(key.clone()))?;
                }
                _ => {}
            }
            // A load the act started is waited for, as navigate waits.
            tokio::time::sleep(ACT_SETTLE).await;
            let began = {
                let loads = receiver.borrow();
                if loads.picker_refused {
                    return Err(asks::PICKER_REFUSED.into());
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
            Ok(Acted {
                page: self.page(persona_id, loaded).await?,
                note: answer.note,
            })
        }

        async fn screenshot(&self, persona_id: &str) -> Result<Vec<u8>, String> {
            let window = self.window(persona_id)?;
            let (sender, answer) = oneshot::channel();
            let sender = Arc::new(Mutex::new(Some(sender)));
            window
                .with_webview(move |view| platform::snapshot(view.inner(), sender))
                .map_err(|error| error.to_string())?;
            tokio::time::timeout(SCRIPT_WAIT, answer)
                .await
                .map_err(|_| "The page could not be drawn in time.".to_string())?
                .map_err(|_| "The page closed before it was drawn.".to_string())?
        }

        async fn full_page(
            &self,
            persona_id: &str,
            most: usize,
        ) -> Result<hotline_core::browser::FullPage, String> {
            full_page::look(self, persona_id, most).await
        }

        fn current(&self, persona_id: &str) -> Option<String> {
            let tab = self.tab(persona_id)?;
            // A refused certificate's warning stands for the address it
            // refused, as the bar shows it.
            if let Some(untrusted) = tab.seen.untrusted() {
                return Some(untrusted.url);
            }
            let shown = tab
                .shown
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            (!shown.is_empty()).then_some(shown)
        }

        fn name_teammate(&self, persona_id: &str, name: &str) {
            self.names
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(persona_id.to_string(), name.to_string());
        }

        fn needs(&self, persona_id: &str) -> Option<String> {
            self.tab(persona_id)?
                .seen
                .untrusted()
                .map(|untrusted| untrusted.label.to_string())
        }

        async fn lend_cookies(
            &self,
            persona_id: &str,
            cookies: &[serde_json::Value],
        ) -> Result<(), String> {
            self.lend(persona_id, cookies).await
        }

        async fn forget_cookies(&self, persona_id: &str, site: &str) -> Result<(), String> {
            self.forget(persona_id, site).await
        }
    }
}
