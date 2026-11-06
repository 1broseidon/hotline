//! What Tauri and wry do for the app's browser, done on GTK's thread with
//! WebKit's own signals: the web views by id, each in a window that is never
//! shown and seated offscreen as the app seats its hidden pane
//! (`linux::seat`); a web context for each teammate on its own data
//! directory; the gate asked before every load (`decide-policy`), loads
//! counted (`load-changed`), a page's popup (`create`), and downloads
//! refused. Everything else reaches a view through [`on_view`], which runs
//! on GTK's thread, as the app's `with_webview` does.

use super::{Loads, Tab, asks, guard, hands, linux, lock, shut_popup, trust};
use gtk::glib;
use gtk::prelude::*;
use hotline_core::browser::Gate;
use hotline_core::browser::pipe::{Link, PIPE_FD};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tokio::sync::oneshot;
use url::Url;
use webkit2gtk::gio::Cancellable;
use webkit2gtk::{
    CacheModel, CookieManagerExt, CookiePersistentStorage, DownloadExt, HardwareAccelerationPolicy,
    LoadEvent, NavigationAction, NavigationPolicyDecision, NavigationPolicyDecisionExt,
    PolicyDecision, PolicyDecisionExt, PolicyDecisionType, SettingsExt, URIRequestExt, WebContext,
    WebContextExt, WebView, WebViewExt, WebsiteDataManager, WebsiteDataManagerExt,
    WebsiteDataManagerExtManual, WebsiteDataTypes,
};

/// What every view's handlers share: the pipe back to the desk, the
/// runtime its asks wait on, and the trust side's exceptions.
#[derive(Clone)]
pub(super) struct Shared {
    pub(super) link: Link,
    pub(super) runtime: tokio::runtime::Handle,
    pub(super) exceptions: Arc<trust::Exceptions>,
    /// What the trust side is handed as the app hands it; WebKitGTK's side
    /// asks the gate nothing at load time, and the desk's own is asked
    /// over the pipe.
    gate: Arc<Gate>,
}

impl Shared {
    pub(super) fn new(link: Link, runtime: tokio::runtime::Handle) -> Self {
        Self {
            link,
            runtime,
            exceptions: Arc::default(),
            gate: Arc::default(),
        }
    }
}

/// A web view and the window it was made in.
struct Held {
    view: WebView,
    window: gtk::Window,
}

static NEXT: AtomicU64 = AtomicU64::new(1);

thread_local! {
    static VIEWS: RefCell<HashMap<u64, Held>> = RefCell::default();
    /// Loads waiting on the gate's word, by ask.
    static DECISIONS: RefCell<HashMap<u64, PolicyDecision>> = RefCell::default();
}

/// A fresh id for a view or a decision.
pub(super) fn next_id() -> u64 {
    NEXT.fetch_add(1, Ordering::SeqCst)
}

/// Runs `work` on GTK's thread in a later turn of its loop, never inside
/// the call or signal that asked.
pub(super) fn gtk_do(work: impl FnOnce() + Send + 'static) {
    let mut work = Some(work);
    glib::source::idle_add_full(glib::Priority::DEFAULT, move || {
        if let Some(work) = work.take() {
            work();
        }
        glib::ControlFlow::Break
    });
}

fn view_of(id: u64) -> Option<WebView> {
    VIEWS.with(|views| views.borrow().get(&id).map(|held| held.view.clone()))
}

/// Runs `work` on GTK's thread with the view `id`; nothing runs if it has
/// closed, and a call waiting on `work`'s answer hears the page closed.
pub(super) fn on_view(id: u64, work: impl FnOnce(WebView) + Send + 'static) {
    gtk_do(move || {
        if let Some(view) = view_of(id) {
            work(view);
        }
    });
}

/// The desk's end of the pipe, on [`PIPE_FD`], kept from every process
/// this one starts.
pub(super) fn take_pipe() -> Result<std::os::unix::net::UnixStream, String> {
    use std::os::fd::FromRawFd;
    // SAFETY: a flags read and write on one descriptor.
    let held = unsafe {
        let flags = libc::fcntl(PIPE_FD, libc::F_GETFD);
        flags >= 0 && libc::fcntl(PIPE_FD, libc::F_SETFD, flags | libc::FD_CLOEXEC) == 0
    };
    if !held {
        return Err(
            "hotline-browser is started by hotline serve, with its pipe on descriptor 3".into(),
        );
    }
    // SAFETY: descriptor 3 is this process's, handed over by the desk, and
    // owned by this stream alone from here.
    let pipe = unsafe { std::os::unix::net::UnixStream::from_raw_fd(PIPE_FD) };
    pipe.set_nonblocking(true)
        .map_err(|error| format!("the desk's pipe could not be read: {error}"))?;
    Ok(pipe)
}

/// A teammate's website data on `directory`, made as wry makes the app's
/// (`WebContextImpl::new`), cookies kept as text beside it.
fn store_in(directory: &Path) -> WebsiteDataManager {
    let path = directory.to_string_lossy();
    let store = WebsiteDataManager::builder()
        .base_cache_directory(path.as_ref())
        .base_data_directory(path.as_ref())
        .build();
    if let Some(cookies) = store.cookie_manager() {
        cookies.set_persistent_storage(
            &directory.join("cookies").to_string_lossy(),
            CookiePersistentStorage::Text,
        );
    }
    store
}

/// A teammate's web context: its own store, no cache of pages or web
/// processes past the one in use, and every download refused.
fn context_on(directory: &Path) -> WebContext {
    let context = WebContext::with_website_data_manager(&store_in(directory));
    context.set_cache_model(CacheModel::DocumentViewer);
    context.set_use_system_appearance_for_scrollbars(false);
    context.connect_download_started(|_, download| download.cancel());
    context
}

/// Puts `view` in a window of its own, as Tauri does, then seats it
/// offscreen, where it lays out, draws and takes input unseen.
fn hold(id: u64, view: &WebView) {
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_default_size(super::WIDTH as i32, super::HEIGHT as i32);
    let home = gtk::Box::new(gtk::Orientation::Vertical, 0);
    home.pack_start(view, true, true, 0);
    window.add(&home);
    linux::seat(view.clone(), false);
    VIEWS.with(|views| {
        views.borrow_mut().insert(
            id,
            Held {
                view: view.clone(),
                window,
            },
        );
    });
}

/// Lets go of the view `id`: what the desk put on it, then its window.
pub(super) fn close(id: u64) {
    gtk_do(move || {
        let Some(held) = VIEWS.with(|views| views.borrow_mut().remove(&id)) else {
            return;
        };
        linux::forget(held.view.clone());
        hands::forget(held.view.clone());
        trust::forget(held.view);
        // SAFETY: the window this helper made for the view, referenced by
        // nothing of the desk's now.
        unsafe { held.window.destroy() };
    });
}

/// Empties a teammate's website data of every kind: its open page's, or
/// the store on its data directory.
pub(super) fn empty(
    page: Option<u64>,
    directory: &Path,
    reply: oneshot::Sender<Result<(), String>>,
) {
    let store = page
        .and_then(view_of)
        .and_then(|view| view.website_data_manager())
        .unwrap_or_else(|| store_in(directory));
    store.clear(
        WebsiteDataTypes::ALL,
        glib::TimeSpan(0),
        None::<&Cancellable>,
        move |cleared| {
            let _ = reply.send(cleared.map_err(|error| error.to_string()));
        },
    );
}

/// Makes the teammate's page and sends it to `url`: on GTK's thread.
pub(super) fn page(tab: &Arc<Tab>, shared: &Shared, store: &Path, url: Url) -> Result<(), String> {
    let context = context_on(store);
    let view = WebView::with_context(&context);
    let settings = WebViewExt::settings(&view).ok_or("WebKit gave the page no settings")?;
    // No GPU on a server: WebKit's software path, at a fraction of the
    // memory of an emulated one. The rest is what wry sets for the app.
    settings.set_hardware_acceleration_policy(HardwareAccelerationPolicy::Never);
    settings.set_enable_webgl(true);
    settings.set_enable_webaudio(true);
    settings.set_enable_page_cache(true);
    hold(tab.page, &view);
    // Nothing loads until every handler is on (`guard`).
    let started = Arc::new(AtomicBool::new(false));
    watch(&view, tab, shared, started.clone(), true);
    {
        let (tab, shared) = (tab.clone(), shared.clone());
        view.connect_create(move |page, action| popup(&tab, &shared, page, action));
    }
    guard::start(view, url, started);
    Ok(())
}

/// A page's `window.open`, or a link it opens in a new window: a popup of
/// the teammate's own, made from the page so it shares its process, store
/// and settings and keeps its `window.opener`, and the teammate's reads and
/// acts go to it until it closes. One at a time: a second goes into the
/// one open. A scheme that is not the web's is refused here; the gate's
/// word on the address comes at its load, as for every load.
fn popup(
    tab: &Arc<Tab>,
    shared: &Shared,
    page: &WebView,
    action: &NavigationAction,
) -> Option<gtk::Widget> {
    let uri = action.request()?.uri()?.to_string();
    let web = Url::parse(&uri)
        .is_ok_and(|url| matches!(url.scheme(), "http" | "https") || url.as_str() == "about:blank");
    if !web {
        tab.loads
            .send_modify(|loads: &mut Loads| loads.blocked = Some(uri));
        return None;
    }
    if let Some(open) = *lock(&tab.popup) {
        on_view(open, move |view| view.load_uri(&uri));
        return None;
    }
    let id = next_id();
    let popup = WebView::with_related_view(page);
    if let Some(settings) = WebViewExt::settings(page) {
        popup.set_settings(&settings);
    }
    hold(id, &popup);
    *lock(&tab.popup) = Some(id);
    watch(&popup, tab, shared, Arc::new(AtomicBool::new(true)), false);
    {
        let tab = tab.clone();
        popup.connect_close(move |_| shut_popup(&tab));
    }
    {
        let tab = tab.clone();
        popup.connect_create(move |_, action| {
            let uri = action.request()?.uri()?.to_string();
            if let Some(open) = *lock(&tab.popup) {
                on_view(open, move |view| view.load_uri(&uri));
            }
            None
        });
    }
    Some(popup.upcast())
}

/// What a page or popup does at each load: asks the gate, counts it, says
/// why it failed or what certificate it was refused (`trust`), and answers
/// what the page asks of the machine (`asks`).
fn watch(view: &WebView, tab: &Arc<Tab>, shared: &Shared, started: Arc<AtomicBool>, page: bool) {
    {
        let (tab, shared) = (tab.clone(), shared.clone());
        view.connect_decide_policy(move |_, decision, kind| {
            decide(&tab, &shared, &started, decision, kind)
        });
    }
    {
        let (tab, link) = (tab.clone(), shared.link.clone());
        view.connect_load_changed(move |view, event| match event {
            LoadEvent::Committed => tab.loads.send_modify(|loads| loads.started += 1),
            LoadEvent::Finished => {
                let url = view.uri().map(|uri| uri.to_string()).unwrap_or_default();
                // The warning a refused certificate leaves is a blank page of
                // the desk's: the refusal stands behind it.
                if url != "about:blank" {
                    tab.seen.clear();
                }
                lock(&tab.shown).clone_from(&url);
                if page {
                    *lock(&tab.page_shown) = url;
                }
                tab.loads.send_modify(|loads| loads.finished += 1);
                // Said once every handler of this load has run: the trust
                // side's, after this one, puts back what a failed load left.
                let (link, persona) = (link.clone(), tab.persona.clone());
                glib::idle_add_local_once(move || link.moved(&persona));
            }
            _ => {}
        });
    }
    trust::attach(
        view.clone(),
        tab.persona.clone(),
        tab.clone(),
        shared.exceptions.clone(),
        shared.gate.clone(),
    );
    // No file picker, ever: nobody could see it to answer it.
    asks::attach(view.clone(), tab.clone(), || false);
}

/// The gate's word on a load of any frame, as the Linux app asks it (every
/// load by the frame's rule, since WebKitGTK says not which frame it is
/// for): asked of the desk over the pipe while WebKit holds the load, then
/// used or ignored. Before the page has started, nothing loads.
fn decide(
    tab: &Arc<Tab>,
    shared: &Shared,
    started: &AtomicBool,
    decision: &PolicyDecision,
    kind: PolicyDecisionType,
) -> bool {
    if kind != PolicyDecisionType::NavigationAction {
        return false;
    }
    let Some(uri) = decision
        .downcast_ref::<NavigationPolicyDecision>()
        .and_then(NavigationPolicyDecisionExt::navigation_action)
        .and_then(|action| action.request())
        .and_then(|request| request.uri())
    else {
        return false;
    };
    if !started.load(Ordering::Acquire) {
        decision.ignore();
        return true;
    }
    let uri = uri.to_string();
    let ask = next_id();
    DECISIONS.with(|decisions| decisions.borrow_mut().insert(ask, decision.clone()));
    let (tab, link) = (tab.clone(), shared.link.clone());
    shared.runtime.spawn(async move {
        let url = Url::parse(&uri).ok();
        let admitted = url.is_some() && link.admits_frame(&uri).await;
        if !admitted {
            tab.loads
                .send_modify(|loads: &mut Loads| loads.blocked = Some(uri));
        } else if url.is_some_and(|url| matches!(url.scheme(), "http" | "https")) {
            *lock(&tab.seen.attempted) = uri;
        }
        gtk_do(move || {
            if let Some(decision) = DECISIONS.with(|decisions| decisions.borrow_mut().remove(&ask))
            {
                if admitted {
                    decision.use_();
                } else {
                    decision.ignore();
                }
            }
        });
    });
    true
}
