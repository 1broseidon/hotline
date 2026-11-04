//! Each teammate's own browser on the desk: a web view in a window of its own,
//! hidden while the teammate works in it, that the core drives through
//! [`DeskBrowser`] (`hotline_core::browser`). It is macOS only for now: WebKit
//! answers a script run in a content world of the desk's own, out of the
//! page's sight, and draws a snapshot of itself; the other platforms' web
//! views get the same seam later. Elsewhere nothing is installed and no
//! teammate is offered the tool.
//!
//! What it may load is the core's [`Gate`]: every top-level load, the
//! agent's and the page's own, is asked first, so a page cannot send the
//! browser to a local address the person has not allowed. A window is never
//! opened for a page, and downloads are refused. Each teammate's cookies
//! and sign-ins live in a web data store of its own, apart from Hotline's
//! window and every other teammate's. No capability names these windows, so
//! a page in one reaches none of the shell's commands.

use hotline_core::desk::Desk;
use tauri::AppHandle;

/// Installs the browser on this desk, where the platform has one. The
/// Door's port and, in development, the window's own dev server, are the
/// ports it never opens.
pub(crate) fn install(app: &AppHandle, desk: &Desk, door_port: u16) {
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
    #[cfg(target_os = "macos")]
    {
        use tauri::Manager;
        let native = std::sync::Arc::new(native::Native::new(app.clone(), desk.browser_gate()));
        desk.install_browser(native.clone(), &never);
        app.manage(native);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (desk, never);
}

#[cfg(target_os = "macos")]
type Browsers<'a> = tauri::State<'a, std::sync::Arc<native::Native>>;
type DeskState<'a> = tauri::State<'a, std::sync::Arc<Desk>>;

#[cfg(not(target_os = "macos"))]
const NOT_HERE: &str = "The desk browser is not built for this platform yet.";

/// The PIP's click: the teammate's browser pane, opened over the desk.
#[tauri::command]
pub fn browser_pane_open(app: AppHandle, persona_id: String, name: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use tauri::Manager;
        app.state::<std::sync::Arc<native::Native>>()
            .open_pane(&persona_id, &name)
    }
    #[cfg(not(target_os = "macos"))]
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
    #[cfg(target_os = "macos")]
    {
        use tauri::Manager;
        let view = desk.browser_now(&persona_id);
        let taken = view
            .as_ref()
            .is_some_and(|view| view.phase == hotline_core::contract::BrowserPhase::Person);
        let mut state = serde_json::to_value(
            app.state::<std::sync::Arc<native::Native>>()
                .bar_state(&persona_id, taken)?,
        )
        .map_err(|error| error.to_string())?;
        state["title"] = view.map(|view| view.title).unwrap_or_default().into();
        Ok(state)
    }
    #[cfg(not(target_os = "macos"))]
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
    #[cfg(target_os = "macos")]
    {
        use tauri::Manager;
        desk.take_browser(&persona_id, taken).await?;
        let browsers: Browsers<'_> = app.state();
        browsers.shield(&persona_id, !taken)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, desk, persona_id, taken);
        Err(NOT_HERE.into())
    }
}

/// Back, forward or reload from the bar, or a typed address: only while the
/// person has the page.
#[tauri::command]
pub fn browser_bar_go(
    app: AppHandle,
    desk: DeskState<'_>,
    persona_id: String,
    step: Option<String>,
    url: Option<String>,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use hotline_core::browser::Step;
        use tauri::Manager;
        let taken = desk
            .browser_now(&persona_id)
            .is_some_and(|view| view.phase == hotline_core::contract::BrowserPhase::Person);
        if !taken {
            return Err("Take over first: the teammate is driving.".into());
        }
        let browsers: Browsers<'_> = app.state();
        match (step.as_deref(), url) {
            (Some("back"), _) => browsers.person_step(&persona_id, Step::Back),
            (Some("forward"), _) => browsers.person_step(&persona_id, Step::Forward),
            (Some("reload"), _) => browsers.person_step(&persona_id, Step::Reload),
            (_, Some(url)) => {
                let typed = url.trim();
                let url = tauri::Url::parse(typed)
                    .or_else(|_| tauri::Url::parse(&format!("https://{typed}")))
                    .map_err(|_| "That is not a web address.".to_string())?;
                browsers.person_go(&persona_id, &url)
            }
            _ => Err("Nothing to go to.".into()),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, desk, persona_id, step, url);
        Err(NOT_HERE.into())
    }
}

#[cfg(target_os = "macos")]
mod native {
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

    const WALK: &str = include_str!("browser/walk.js");
    const READ_TEXT: &str = concat!(
        include_str!("browser/walk.js"),
        include_str!("browser/read_text.js")
    );
    const READ_LINKS: &str = include_str!("browser/read_links.js");
    const ACT: &str = include_str!("browser/act.js");
    /// Whether the last native click or hover reached the page.
    const ARRIVED: &str = "return JSON.stringify(globalThis.__hotlineArrived !== false);";
    /// The click the page's own script makes, when a native one did not
    /// arrive: walk.js goes in front, and `A` names the ref.
    const CLICK: &str = "const el = refs.get(A.ref); if (el) { el.focus?.(); el.click(); } return JSON.stringify(!!el);";
    /// How long after an act a load it started is looked for.
    const ACT_SETTLE: Duration = Duration::from_millis(400);
    const PAGE_INFO: &str =
        "return JSON.stringify({ title: document.title, url: location.href, text: '' });";
    /// How long a load is waited for before the call answers that the page
    /// is still loading.
    const LOAD_WAIT: Duration = Duration::from_secs(20);
    /// How long a move through history is waited for: one served from the
    /// back-forward cache, or within the page, loads nothing.
    const STEP_WAIT: Duration = Duration::from_secs(5);
    const SCRIPT_WAIT: Duration = Duration::from_secs(15);
    /// The page's size: a common laptop viewport.
    const WIDTH: f64 = 1280.0;
    const HEIGHT: f64 = 800.0;
    /// Hotline's bar above the page in the browser pane.
    const BAR: f64 = 44.0;
    /// The window's own title bar above it: the traffic lights, the title
    /// and the place the person drags the window by, all macOS's.
    const TITLE: f64 = 28.0;

    /// One teammate's browser: its window, the page and the bar in it, who
    /// is driving, and its loads so far.
    struct Tab {
        window: String,
        page: String,
        bar: String,
        /// The teammate's name, for the pane's title.
        name: Mutex<String>,
        loads: watch::Sender<Loads>,
    }

    /// What the bar above the page shows.
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    pub(crate) struct BarState {
        name: String,
        url: String,
        title: String,
        taken: bool,
    }

    #[derive(Clone, Default)]
    struct Loads {
        started: u64,
        finished: u64,
        /// The last load the gate refused, since the call that cleared it.
        blocked: Option<String>,
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
            let _ = page.with_webview(|view| apple::shield_follow(view.inner()));
        }
    }

    impl Native {
        pub(super) fn new(app: AppHandle, gate: Arc<Gate>) -> Self {
            Self {
                app,
                gate,
                tabs: Mutex::default(),
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
                .with_webview(|view| apple::reveal(view.inner()))
                .map_err(|error| error.to_string())?;
            let _ = window.set_focus();
            Ok(())
        }

        /// What the pane's bar shows now.
        pub(crate) fn bar_state(&self, persona_id: &str, taken: bool) -> Result<BarState, String> {
            let tab = self
                .tab(persona_id)
                .ok_or("That teammate's browser has closed.")?;
            let page = self.window(persona_id)?;
            Ok(BarState {
                name: tab
                    .name
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone(),
                url: page.url().map(|url| url.to_string()).unwrap_or_default(),
                title: String::new(),
                taken,
            })
        }

        /// The person took the page over or handed it back: the shield that
        /// keeps a watcher's clicks off the page goes or comes back.
        pub(crate) fn shield(&self, persona_id: &str, on: bool) -> Result<(), String> {
            self.window(persona_id)?
                .with_webview(move |view| apple::shield(view.inner(), on))
                .map_err(|error| error.to_string())
        }

        /// The person's own move, from the bar, while they have the page.
        pub(crate) fn person_step(&self, persona_id: &str, step: Step) -> Result<(), String> {
            self.window(persona_id)?
                .with_webview(move |view| apple::step(view.inner(), step))
                .map_err(|error| error.to_string())
        }

        pub(crate) fn person_go(&self, persona_id: &str, url: &Url) -> Result<(), String> {
            if !self.gate.admits(persona_id, url) {
                return Err("That address is not a web page, or is on this computer or your network: ask the teammate to open it, so you are asked first.".into());
            }
            self.window(persona_id)?
                .navigate(url.clone())
                .map_err(|error| error.to_string())
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
            });
            let show = std::env::var_os("HOTLINE_BROWSER_SHOW").is_some();
            let navigating = {
                let gate = self.gate.clone();
                let persona = persona_id.to_string();
                let tab = tab.clone();
                move |url: &Url| {
                    let admitted = gate.admits(&persona, url);
                    if !admitted {
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
            let window = WindowBuilder::new(&self.app, &window_label)
                .title("Hotline browser")
                .title_bar_style(tauri::TitleBarStyle::Overlay)
                .theme(Some(tauri::Theme::Dark))
                .background_color(tauri::window::Color(28, 28, 31, 255))
                .inner_size(WIDTH, TITLE + BAR + HEIGHT)
                .min_inner_size(480.0, 320.0)
                .visible(show)
                .focused(false)
                .build()
                .map_err(|error| format!("Your browser could not be opened: {error}"))?;
            let page = WebviewBuilder::new(&page_label, WebviewUrl::External(url.clone()))
                .background_throttling(BackgroundThrottlingPolicy::Disabled)
                .data_store_identifier(hotline_core::browser::store_id(persona_id))
                .on_navigation(navigating)
                .on_new_window(popup)
                .on_download(|_, _| false)
                .on_page_load(loaded);
            window
                .add_child(
                    page,
                    LogicalPosition::new(0.0, TITLE + BAR),
                    LogicalSize::new(WIDTH, HEIGHT),
                )
                .map_err(|error| format!("Your browser could not be opened: {error}"))?;
            let hash = format!("persona={}", urlencode(persona_id));
            let bar = WebviewBuilder::new(
                &bar_label,
                WebviewUrl::App(format!("browser.html#{hash}").into()),
            );
            window
                .add_child(
                    bar,
                    LogicalPosition::new(0.0, TITLE),
                    LogicalSize::new(WIDTH, BAR),
                )
                .map_err(|error| format!("Your browser could not be opened: {error}"))?;
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
                            let _ = page.with_webview(|view| apple::conceal(view.inner()));
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
                            return Err(blocked.clone());
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
                Ok(Err(blocked)) => Err(format!(
                    "The page went on to {blocked}, which your browser may not open: a local address the person has not allowed, or not a web page."
                )),
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
            let window = self.window(persona_id)?;
            let (sender, answer) = oneshot::channel();
            let sender = Arc::new(Mutex::new(Some(sender)));
            window
                .with_webview(move |view| apple::call(view.inner(), &body, sender))
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
        fn input(&self, persona_id: &str, input: apple::Input) -> Result<(), String> {
            self.window(persona_id)?
                .with_webview(move |view| apple::input(view.inner(), &input))
                .map_err(|error| error.to_string())
        }

        async fn page(&self, persona_id: &str, loaded: bool) -> Result<Page, String> {
            let read = self.script(persona_id, PAGE_INFO).await?;
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
                    tab.loads.send_modify(|loads| loads.blocked = None);
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
            tab.loads.send_modify(|loads| loads.blocked = None);
            let receiver = tab.loads.subscribe();
            let before = receiver.borrow().finished;
            let window = self.window(persona_id)?;
            window
                .with_webview(move |view| apple::step(view.inner(), step))
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
            let read = self.script(persona_id, body).await?;
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
            tab.loads.send_modify(|loads| loads.blocked = None);
            let receiver = tab.loads.subscribe();
            let (started, finished) = {
                let loads = receiver.borrow();
                (loads.started, loads.finished)
            };
            let args = act_args(act);
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
                    self.input(
                        persona_id,
                        apple::Input::Click(point.x, point.y, point.height),
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
                        apple::Input::Hover(point.x, point.y, point.height),
                    )?;
                }
                (Act::Press { key }, _) => {
                    self.input(persona_id, apple::Input::Key(key.clone()))?;
                }
                _ => {}
            }
            // A load the act started is waited for, as navigate waits.
            tokio::time::sleep(ACT_SETTLE).await;
            let began = {
                let loads = receiver.borrow();
                if let Some(blocked) = &loads.blocked {
                    return Err(format!(
                        "The page tried to go to {blocked}, which your browser may not open. If it is a local address, navigate to it to ask the person."
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
                .with_webview(move |view| apple::snapshot(view.inner(), sender))
                .map_err(|error| error.to_string())?;
            tokio::time::timeout(SCRIPT_WAIT, answer)
                .await
                .map_err(|_| "The page could not be drawn in time.".to_string())?
                .map_err(|_| "The page closed before it was drawn.".to_string())?
        }

        fn current(&self, persona_id: &str) -> Option<String> {
            self.window(persona_id)
                .ok()
                .and_then(|window| window.url().ok())
                .map(|url| url.to_string())
        }
    }

    /// WebKit itself, on the main thread `with_webview` runs on.
    mod apple {
        use super::Step;
        use block2::RcBlock;
        use objc2::runtime::AnyObject;
        use objc2::{MainThreadMarker, MainThreadOnly};
        use objc2_app_kit::{
            NSBitmapImageFileType, NSBitmapImageRep, NSEvent, NSEventModifierFlags, NSEventType,
            NSImage, NSView, NSWindow, NSWindowCollectionBehavior, NSWindowOrderingMode,
        };
        use objc2_foundation::{NSDictionary, NSError, NSPoint, NSProcessInfo, NSString};
        use objc2_web_kit::{WKContentWorld, WKWebView};
        use std::ffi::c_void;
        use std::sync::{Arc, Mutex, PoisonError};
        use tokio::sync::oneshot;

        /// The content world the desk's scripts run in: the page's own
        /// scripts never see its variables, and it never sees theirs.
        const WORLD: &str = "hotline-desk-browser";

        thread_local! {
            /// Held for the app's life: a named world lives only while it is
            /// retained, and its variables with it.
            static KEPT: std::cell::OnceCell<objc2::rc::Retained<WKContentWorld>> =
                const { std::cell::OnceCell::new() };
        }

        type Reply<T> = Arc<Mutex<Option<oneshot::Sender<Result<T, String>>>>>;

        fn send<T>(reply: &Reply<T>, answer: Result<T, String>) {
            if let Some(sender) = reply.lock().unwrap_or_else(PoisonError::into_inner).take() {
                let _ = sender.send(answer);
            }
        }

        fn said(error: *mut NSError) -> String {
            // SAFETY: WebKit hands a valid NSError or null.
            unsafe { error.as_ref() }.map_or_else(
                || "WebKit gave no reason.".into(),
                |error| error.localizedDescription().to_string(),
            )
        }

        /// The web view `with_webview` hands over, as WebKit's type.
        ///
        /// SAFETY: the caller is `with_webview`'s closure, on the main
        /// thread, where `view` is the live `WKWebView` of a Tauri window.
        unsafe fn view<'a>(view: *mut c_void) -> &'a WKWebView {
            // SAFETY: as the function's own contract above.
            unsafe { &*view.cast::<WKWebView>() }
        }

        /// Runs `body` as an async function in the desk's content world and
        /// answers its string.
        pub(super) fn call(view_ptr: *mut c_void, body: &str, reply: Reply<String>) {
            let Some(main) = MainThreadMarker::new() else {
                return send(
                    &reply,
                    Err("The browser was asked off the main thread.".into()),
                );
            };
            // SAFETY: see `view`.
            let view = unsafe { view(view_ptr) };
            let world = KEPT.with(|kept| {
                kept.get_or_init(|| {
                    // SAFETY: a named world, retained here for the app's life.
                    unsafe { WKContentWorld::worldWithName(&NSString::from_str(WORLD), main) }
                })
                .clone()
            });
            let body = NSString::from_str(body);
            let done = RcBlock::new(move |result: *mut AnyObject, error: *mut NSError| {
                if !error.is_null() {
                    return send(&reply, Err(said(error)));
                }
                // SAFETY: WebKit hands the script's value or null.
                let answer = unsafe { result.as_ref() }
                    .and_then(|value| value.downcast_ref::<NSString>())
                    .map(ToString::to_string)
                    .ok_or_else(|| "The page answered nothing.".to_string());
                send(&reply, answer);
            });
            // SAFETY: no arguments, the main frame, and a block that lives
            // as long as WebKit keeps it.
            unsafe {
                view.callAsyncJavaScript_arguments_inFrame_inContentWorld_completionHandler(
                    &body,
                    None,
                    None,
                    &world,
                    Some(&done),
                );
            }
        }

        pub(super) fn step(view_ptr: *mut c_void, step: Step) {
            // SAFETY: see `view`.
            let view = unsafe { view(view_ptr) };
            // SAFETY: plain WebKit navigation on its own thread.
            unsafe {
                match step {
                    Step::Back => {
                        let _ = view.goBack();
                    }
                    Step::Forward => {
                        let _ = view.goForward();
                    }
                    Step::Reload => {
                        let _ = view.reload();
                    }
                }
            }
        }

        /// What the desk sends the web view as the person's own input: a
        /// click or a hover at a point in the page's CSS pixels, with the
        /// page's viewport height, or one key.
        pub(super) enum Input {
            Click(f64, f64, f64),
            Hover(f64, f64, f64),
            Key(String),
        }

        /// A named key as AppKit spells it: its characters, its virtual key
        /// code and whether Shift is held.
        fn key_of(key: &str) -> (String, u16, bool) {
            let function = |code: u32| char::from_u32(code).unwrap_or(' ').to_string();
            match key {
                "Enter" => ("\r".into(), 36, false),
                "Tab" => ("\t".into(), 48, false),
                "Shift+Tab" => ("\u{19}".into(), 48, true),
                "Escape" => ("\u{1b}".into(), 53, false),
                "Backspace" => ("\u{7f}".into(), 51, false),
                "Delete" => (function(0xF728), 117, false),
                "Space" => (" ".into(), 49, false),
                "ArrowUp" => (function(0xF700), 126, false),
                "ArrowDown" => (function(0xF701), 125, false),
                "ArrowLeft" => (function(0xF702), 123, false),
                "ArrowRight" => (function(0xF703), 124, false),
                "Home" => (function(0xF729), 115, false),
                "End" => (function(0xF72B), 119, false),
                "PageUp" => (function(0xF72C), 116, false),
                "PageDown" => (function(0xF72D), 121, false),
                other => (other.to_string(), 0, false),
            }
        }

        /// Puts a hidden browser window on screen, invisible, at the back and
        /// out of the person's way: WebKit takes input only in a window that
        /// is ordered in. Showing the pane makes it opaque again.
        fn cloak(window: &NSWindow) {
            if window.isVisible() {
                return;
            }
            window.setAlphaValue(0.0);
            window.setIgnoresMouseEvents(true);
            window.setCollectionBehavior(
                NSWindowCollectionBehavior::Transient
                    | NSWindowCollectionBehavior::IgnoresCycle
                    | NSWindowCollectionBehavior::FullScreenAuxiliary,
            );
            window.orderBack(None);
        }

        /// Hides the pane again, as `cloak` keeps a never-shown one: the
        /// teammate's browser goes on behind everything, invisible.
        pub(super) fn conceal(view_ptr: *mut c_void) {
            // SAFETY: see `view`.
            let view = unsafe { view(view_ptr) };
            if let Some(window) = view.window() {
                window.setAlphaValue(0.0);
                window.setIgnoresMouseEvents(true);
                window.setCollectionBehavior(
                    NSWindowCollectionBehavior::Transient
                        | NSWindowCollectionBehavior::IgnoresCycle
                        | NSWindowCollectionBehavior::FullScreenAuxiliary,
                );
                window.orderBack(None);
            }
        }

        /// Shows the pane: the window opaque, in front, taking the person's
        /// clicks, and the page shielded until they take it over.
        pub(super) fn reveal(view_ptr: *mut c_void) {
            // SAFETY: see `view`.
            let page = unsafe { view(view_ptr) };
            if let Some(window) = page.window() {
                window.setAlphaValue(1.0);
                window.setIgnoresMouseEvents(false);
                window.setCollectionBehavior(NSWindowCollectionBehavior::Default);
                window.makeKeyAndOrderFront(None);
            }
            shield(view_ptr, true);
        }

        thread_local! {
            /// The shield over each page a watcher sees, by its web view.
            static SHIELDS: std::cell::RefCell<std::collections::HashMap<usize, objc2::rc::Retained<NSView>>> =
                std::cell::RefCell::default();
        }

        /// A clear view over the page that takes a watcher's clicks and
        /// scrolls, so watching never drives the page. The desk's own input
        /// goes to the page directly and is not stopped by it.
        pub(super) fn shield(view_ptr: *mut c_void, on: bool) {
            let Some(main) = MainThreadMarker::new() else {
                return;
            };
            // SAFETY: see `view`.
            let page = unsafe { view(view_ptr) };
            SHIELDS.with(|shields| {
                let mut shields = shields.borrow_mut();
                let key = view_ptr as usize;
                if !on {
                    if let Some(shield) = shields.remove(&key) {
                        shield.removeFromSuperview();
                    }
                    return;
                }
                if shields.contains_key(&key) {
                    return;
                }
                // SAFETY: the page's own superview, on the main thread.
                let Some(parent) = (unsafe { page.superview() }) else {
                    return;
                };
                let shield = NSView::initWithFrame(NSView::alloc(main), page.frame());
                parent.addSubview_positioned_relativeTo(
                    &shield,
                    NSWindowOrderingMode::Above,
                    Some(page),
                );
                shields.insert(key, shield);
            });
        }

        /// Keeps a page's shield over it after the window is resized.
        pub(super) fn shield_follow(view_ptr: *mut c_void) {
            // SAFETY: see `view`.
            let page = unsafe { view(view_ptr) };
            SHIELDS.with(|shields| {
                if let Some(shield) = shields.borrow().get(&(view_ptr as usize)) {
                    shield.setFrame(page.frame());
                }
            });
        }

        pub(super) fn input(view_ptr: *mut c_void, input: &Input) {
            // SAFETY: see `view`.
            let view = unsafe { view(view_ptr) };
            let Some(window) = view.window() else {
                return;
            };
            cloak(&window);
            let number = window.windowNumber();
            let time = NSProcessInfo::processInfo().systemUptime();
            // The page's viewport sits below the window's title bar inside
            // the view, so a point in it is lower in the view by the height
            // the bar hides.
            let at = |x: f64, y: f64, viewport: f64| {
                let height = view.bounds().size.height;
                let y = y + (height - viewport).max(0.0);
                let local = NSPoint::new(x, if view.isFlipped() { y } else { height - y });
                view.convertPoint_toView(local, None)
            };
            let mouse = |kind: NSEventType, point: NSPoint| {
                NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
                    kind,
                    point,
                    NSEventModifierFlags::empty(),
                    time,
                    number,
                    None,
                    0,
                    1,
                    1.0,
                )
            };
            match input {
                Input::Click(x, y, viewport) => {
                    let point = at(*x, *y, *viewport);
                    if let Some(moved) = mouse(NSEventType::MouseMoved, point) {
                        view.mouseMoved(&moved);
                    }
                    if let (Some(down), Some(up)) = (
                        mouse(NSEventType::LeftMouseDown, point),
                        mouse(NSEventType::LeftMouseUp, point),
                    ) {
                        view.mouseDown(&down);
                        view.mouseUp(&up);
                    }
                }
                Input::Hover(x, y, viewport) => {
                    if let Some(moved) = mouse(NSEventType::MouseMoved, at(*x, *y, *viewport)) {
                        view.mouseMoved(&moved);
                    }
                }
                Input::Key(key) => {
                    let (characters, code, shift) = key_of(key);
                    let flags = if shift {
                        NSEventModifierFlags::Shift
                    } else {
                        NSEventModifierFlags::empty()
                    };
                    let characters = NSString::from_str(&characters);
                    window.makeFirstResponder(Some(view));
                    for kind in [NSEventType::KeyDown, NSEventType::KeyUp] {
                        let Some(event) = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
                            kind,
                            NSPoint::new(0.0, 0.0),
                            flags,
                            time,
                            number,
                            None,
                            &characters,
                            &characters,
                            false,
                            code,
                        ) else {
                            continue;
                        };
                        if kind == NSEventType::KeyDown {
                            view.keyDown(&event);
                        } else {
                            view.keyUp(&event);
                        }
                    }
                }
            }
        }

        /// The visible page as a PNG.
        pub(super) fn snapshot(view_ptr: *mut c_void, reply: Reply<Vec<u8>>) {
            // SAFETY: see `view`.
            let view = unsafe { view(view_ptr) };
            let done = RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
                // SAFETY: WebKit hands a valid image or null.
                let answer = match unsafe { image.as_ref() } {
                    Some(image) => png(image),
                    None => Err(said(error)),
                };
                send(&reply, answer);
            });
            // SAFETY: the default configuration is the visible viewport.
            unsafe { view.takeSnapshotWithConfiguration_completionHandler(None, &done) };
        }

        fn png(image: &NSImage) -> Result<Vec<u8>, String> {
            let tiff = image
                .TIFFRepresentation()
                .ok_or("The page's picture could not be read.")?;
            let bitmap = NSBitmapImageRep::imageRepWithData(&tiff)
                .ok_or("The page's picture could not be read.")?;
            // SAFETY: an empty property dictionary is valid for PNG.
            let png = unsafe {
                bitmap.representationUsingType_properties(
                    NSBitmapImageFileType::PNG,
                    &NSDictionary::new(),
                )
            }
            .ok_or("The page's picture could not be made a PNG.")?;
            Ok(png.to_vec())
        }
    }
}
