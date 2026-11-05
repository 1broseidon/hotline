//! WebView2 itself, on the main thread `with_webview` runs on: the Windows
//! side of the desk browser's platform seam (`browser.rs`). `with_webview`
//! hands over the page's `ICoreWebView2Controller`, the types wry builds on.
//!
//! WebView2 has no content worlds of its own, and a script it runs with
//! `ExecuteScript` runs in the page's world, among the page's own scripts
//! and their changes to the built-ins. So the desk does not use it: its
//! reads and acts go through the DevTools protocol on the page's own
//! session (`CallDevToolsProtocolMethod`), into an isolated world of the
//! desk's (`Page.createIsolatedWorld`, the mechanism Chromium's extensions'
//! content scripts run in) in the main frame, and come back as JSON
//! (`Runtime.evaluate`). The world is found by its name, so every script in
//! one document meets the variables the last one left (a read's refs), and
//! a new document starts a fresh one, as WebKit's named world does. The
//! page's scripts never see the world's variables, and a change the page
//! makes to `JSON`, `Map` or a prototype is not the world's. Nothing is put
//! in the page's own world but the passkey refusal (`edge::NO_PASSKEYS`).
//!
//! Input goes the same way (`Input.dispatchMouseEvent` and
//! `Input.dispatchKeyEvent`): Chromium's own trusted events at a point in
//! the page, which reach it whether or not its window is on screen. The
//! snapshot is WebView2's `CapturePreview`, a PNG of the visible page.
//!
//! The page's window waits hidden while nobody watches. The environment it
//! is made in (`edge::BROWSER_ARGS`) keeps Chromium from counting a hidden
//! or covered window's page as hidden, and the controller itself stays
//! visible, so the page lays out, draws and runs at full speed. The pane
//! shows the window (`browser.rs`, through Tauri) and puts the shield up: the
//! page's own window, wry's `WRY_WEBVIEW` container, is disabled, which
//! stops every click, scroll and key of a watcher before it reaches the
//! page, and the desk's own input, which never goes through Windows' input,
//! is not stopped by it.

use super::Step;
use super::edge;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex, PoisonError};
use tokio::sync::oneshot;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG, ICoreWebView2, ICoreWebView2Controller,
    ICoreWebView2Settings4,
};
use webview2_com::{
    AddScriptToExecuteOnDocumentCreatedCompletedHandler,
    CallDevToolsProtocolMethodCompletedHandler, CapturePreviewCompletedHandler,
    WindowCloseRequestedEventHandler,
};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{IStream, STREAM_SEEK_SET};
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};
use windows::Win32::UI::Shell::SHCreateMemStream;
use windows::Win32::UI::WindowsAndMessaging::{GA_ROOT, GetAncestor};
use windows::core::{HSTRING, Interface};

/// What `with_webview` hands over: the page's WebView2 controller.
pub(super) type View = ICoreWebView2Controller;

/// `view.inner()`, as macOS and Linux spell the platform's web view, so the
/// shared code reads the same on Windows.
pub(super) trait Inner {
    fn inner(&self) -> View;
}

impl Inner for tauri::webview::PlatformWebview {
    fn inner(&self) -> View {
        self.controller()
    }
}

/// The isolated world the desk's scripts run in: the page's own scripts
/// never see its variables, and it never sees theirs.
const WORLD: &str = "hotline-desk-browser";

type Reply<T> = Arc<Mutex<Option<oneshot::Sender<Result<T, String>>>>>;

fn send<T>(reply: &Reply<T>, answer: Result<T, String>) {
    if let Some(sender) = reply.lock().unwrap_or_else(PoisonError::into_inner).take() {
        let _ = sender.send(answer);
    }
}

/// The controller's web view.
pub(super) fn webview(view: &View) -> Option<ICoreWebView2> {
    // SAFETY: a plain getter on the main thread, where `with_webview` runs.
    unsafe { view.CoreWebView2() }.ok()
}

/// The page's own window: wry's `WRY_WEBVIEW` container the controller is
/// in, which holds WebView2's windows and nothing else.
pub(super) fn container(view: &View) -> Option<HWND> {
    let mut window = HWND::default();
    // SAFETY: a plain getter writing a window handle.
    unsafe { view.ParentWindow(&raw mut window) }.ok()?;
    (!window.is_invalid()).then_some(window)
}

/// The top-level window the page is in: the teammate's browser window, or
/// its popup's.
pub(super) fn root(view: &View) -> Option<HWND> {
    let window = container(view)?;
    // SAFETY: a window handle the controller gave, on its own thread.
    let root = unsafe { GetAncestor(window, GA_ROOT) };
    (!root.is_invalid()).then_some(root)
}

/// One DevTools protocol call on the page's own session; `done` hears its
/// result object or what went wrong. Its answer comes back on this thread.
pub(super) fn devtools(
    webview: &ICoreWebView2,
    method: &str,
    params: &Value,
    done: impl FnOnce(Result<Value, String>) + 'static,
) {
    type Done = Box<dyn FnOnce(Result<Value, String>)>;
    let done: Rc<RefCell<Option<Done>>> = Rc::new(RefCell::new(Some(Box::new(done))));
    let answered = done.clone();
    let handler =
        CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |result, json| {
            if let Some(done) = answered.borrow_mut().take() {
                done(edge::devtools_answer(result.is_ok(), &json));
            }
            Ok(())
        }));
    // SAFETY: on the main thread, with a live web view, and a handler
    // WebView2 keeps until it answers.
    let sent = unsafe {
        webview.CallDevToolsProtocolMethod(
            &HSTRING::from(method),
            &HSTRING::from(params.to_string()),
            &handler,
        )
    };
    if let Err(error) = sent
        && let Some(done) = done.borrow_mut().take()
    {
        done(Err(error.message()));
    }
}

/// Runs `body` as an async function in the desk's isolated world in the
/// page's main frame, and answers its string. The fallback click (`CLICK`)
/// runs as the person's gesture, as a click is, so a link it follows may
/// open its window; a read or any other act does not, so a script the desk
/// runs never hands the page a gesture of the person's.
pub(super) fn call(view: View, body: &str, reply: Reply<String>) {
    let Some(page) = webview(&view) else {
        return send(&reply, Err("The browser has closed.".into()));
    };
    let gesture = body.ends_with(super::CLICK);
    let expression = edge::wrap(body);
    let in_frame = page.clone();
    devtools(&page, "Page.getFrameTree", &json!({}), move |tree| {
        let frame = match tree.and_then(|tree| edge::main_frame(&tree)) {
            Ok(frame) => frame,
            Err(error) => return send(&reply, Err(error)),
        };
        let in_world = in_frame.clone();
        let world = json!({ "frameId": frame, "worldName": WORLD, "grantUniveralAccess": false });
        devtools(&in_frame, "Page.createIsolatedWorld", &world, move |made| {
            let context = match made.and_then(|made| edge::world_of(&made)) {
                Ok(context) => context,
                Err(error) => return send(&reply, Err(error)),
            };
            let run = json!({
                "expression": expression,
                "contextId": context,
                "awaitPromise": true,
                "returnByValue": true,
                "userGesture": gesture,
            });
            devtools(&in_world, "Runtime.evaluate", &run, move |evaluated| {
                send(
                    &reply,
                    evaluated.and_then(|answer| edge::evaluated(&answer)),
                );
            });
        });
    });
}

pub(super) fn step(view: View, step: Step) {
    let Some(page) = webview(&view) else {
        return;
    };
    // SAFETY: plain WebView2 navigation on its own thread.
    let _ = unsafe {
        match step {
            Step::Back => page.GoBack(),
            Step::Forward => page.GoForward(),
            Step::Reload => page.Reload(),
        }
    };
}

/// What the desk sends the web view as the person's own input: a click or
/// a hover at a point in the page's CSS pixels, with the page's viewport
/// height, or one key.
#[allow(
    dead_code,
    reason = "the viewport's height places a point on macOS and Linux; the protocol counts from the viewport's own top"
)]
pub(super) enum Input {
    Click(f64, f64, f64),
    Hover(f64, f64, f64),
    Key(String),
}

/// Sends `events` to the page one after another, each once Chromium has
/// taken the one before.
fn dispatch(page: ICoreWebView2, method: &'static str, mut events: Vec<Value>) {
    if events.is_empty() {
        return;
    }
    let event = events.remove(0);
    let next = page.clone();
    devtools(&page, method, &event, move |_| {
        dispatch(next, method, events);
    });
}

/// Sends `input` to the page as Chromium's own trusted input events, at the
/// point in the viewport's CSS pixels act.js answered: the protocol counts
/// from the viewport's top, which on Windows is the web view's own top. A
/// click Chromium did not take is made by the page's own script instead
/// (`CLICK`), as on macOS.
pub(super) fn input(view: View, input: &Input) {
    let Some(page) = webview(&view) else {
        return;
    };
    match input {
        Input::Click(x, y, _) => {
            dispatch(page, "Input.dispatchMouseEvent", edge::click_events(*x, *y));
        }
        Input::Hover(x, y, _) => {
            dispatch(page, "Input.dispatchMouseEvent", edge::hover_events(*x, *y));
        }
        Input::Key(key) => {
            if let Some(events) = edge::key_events(key) {
                dispatch(page, "Input.dispatchKeyEvent", events);
            }
        }
    }
}

/// The visible page as a PNG.
pub(super) fn snapshot(view: View, reply: Reply<Vec<u8>>) {
    let Some(page) = webview(&view) else {
        return send(&reply, Err("The browser has closed.".into()));
    };
    // SAFETY: a fresh, empty stream in memory.
    let Some(stream) = (unsafe { SHCreateMemStream(None) }) else {
        return send(&reply, Err("The page's picture had nowhere to go.".into()));
    };
    let (drawn, answered) = (stream.clone(), reply.clone());
    let handler = CapturePreviewCompletedHandler::create(Box::new(move |result| {
        let answer = result
            .map_err(|error| error.message())
            .and_then(|()| read_all(&drawn));
        send(&answered, answer);
        Ok(())
    }));
    // SAFETY: on the main thread, a live stream and a handler WebView2 keeps
    // until it answers.
    if let Err(error) = unsafe {
        page.CapturePreview(
            COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
            &stream,
            &handler,
        )
    } {
        send(&reply, Err(error.message()));
    }
}

/// Everything WebView2 wrote into the stream.
fn read_all(stream: &IStream) -> Result<Vec<u8>, String> {
    // SAFETY: the stream is ours, in memory, read from its start.
    unsafe { stream.Seek(0, STREAM_SEEK_SET, None) }
        .map_err(|_| "The page's picture could not be read.".to_string())?;
    let mut png = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        let mut read = 0u32;
        // SAFETY: a buffer of the length given, and a count it writes.
        let result = unsafe {
            stream.Read(
                chunk.as_mut_ptr().cast(),
                chunk.len() as u32,
                Some(&raw mut read),
            )
        };
        if result.is_err() {
            return Err("The page's picture could not be read.".into());
        }
        if read == 0 {
            break;
        }
        png.extend_from_slice(&chunk[..read as usize]);
    }
    if png.is_empty() {
        return Err("The page's picture was empty.".into());
    }
    Ok(png)
}

/// Shows the pane's page: Tauri has shown its window (`browser.rs`), and
/// the page is shielded until the person takes it over.
pub(super) fn reveal(view: View) {
    shield(view, true);
}

/// Hides the pane again. Tauri hides the window (`browser.rs`); the page
/// goes on in it as before, its controller still visible.
pub(super) fn conceal(_view: View) {}

/// The shield over the page while a watcher sees it, so watching never
/// drives the page: the page's own window is disabled, so Windows hands it
/// no click, scroll or key of a watcher's, and the window above it takes the
/// keyboard. A disabled window's children are disabled with it, WebView2's
/// own among them, in its own process too. The desk's input goes to the page
/// through the protocol and is not stopped by it. With the shield the file
/// chooser is the desk's to refuse again (`asks/win.rs`); without it,
/// the person's.
pub(super) fn shield(view: View, on: bool) {
    if let Some(window) = container(&view) {
        // SAFETY: wry's container of the page, on its own thread.
        let _ = unsafe { EnableWindow(window, !on) };
        if on && let Some(root) = root(&view) {
            // SAFETY: the browser's own top-level window, on its thread.
            let _ = unsafe { SetFocus(Some(root)) };
        }
    }
    if let Some(page) = webview(&view) {
        super::asks::intercept_files(&page, on);
    }
}

/// A disabled window stays over the page whatever the window's size.
pub(super) fn shield_follow(_view: View) {}

/// Calls `closed` when the popup's own script closes it (`window.close()`,
/// as a sign-in popup does once it has answered its opener). wry's own
/// answer destroys only the web view's container, so the desk closes the
/// window around it.
pub(super) fn watch_close(view: View, closed: Box<dyn Fn()>) {
    let Some(page) = webview(&view) else {
        return;
    };
    let mut token = 0;
    // SAFETY: on the main thread; the handler lives as long as the web view.
    let _ = unsafe {
        page.add_WindowCloseRequested(
            &WindowCloseRequestedEventHandler::create(Box::new(move |_, _| {
                closed();
                Ok(())
            })),
            &raw mut token,
        )
    };
}

/// What the desk sets on each teammate page and popup before it loads: no
/// script messages to the shell at all (no capability names these windows,
/// and here there is no channel either), no passwords or form data saved
/// into the teammate's store, passkeys refused (`edge::NO_PASSKEYS`), and
/// the page kept focused for the desk's keys.
pub(super) fn prepare(view: &View) {
    let Some(page) = webview(view) else {
        return;
    };
    // SAFETY: WebView2's settings, on the main thread.
    unsafe {
        if let Ok(settings) = page.Settings() {
            let _ = settings.SetIsWebMessageEnabled(false);
            if let Ok(settings) = settings.cast::<ICoreWebView2Settings4>() {
                let _ = settings.SetIsPasswordAutosaveEnabled(false);
                let _ = settings.SetIsGeneralAutofillEnabled(false);
            }
        }
        let _ = page.AddScriptToExecuteOnDocumentCreated(
            &HSTRING::from(edge::NO_PASSKEYS),
            &AddScriptToExecuteOnDocumentCreatedCompletedHandler::create(Box::new(|_, _| Ok(()))),
        );
    }
    // The page behaves as the focused one it would be on screen, so a key
    // the desk sends reaches what has focus in it while its window is
    // hidden, as WebKit's first responder does on macOS.
    devtools(
        &page,
        "Emulation.setFocusEmulationEnabled",
        &json!({ "enabled": true }),
        |_| {},
    );
}
