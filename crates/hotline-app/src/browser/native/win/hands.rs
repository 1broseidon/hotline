//! A teammate's hands over its page on Windows: none are drawn. macOS draws
//! them in Core Animation layers and Linux in a GTK drawing area, both over
//! the page and never in its document (`hands.rs`, `linux/hands.rs`). Over a
//! WebView2 page the same would be a window of the desk's own above
//! WebView2's, which is another process's: a layered child window kept
//! clear of the page's input and moved with it, and nothing here can check
//! that it draws over Chromium's output. Drawn in the page's own document
//! instead, the page could read, move or hide it. So on Windows the person
//! watching sees the page scroll to what the teammate acts on, and its
//! click, but no pointer or name. These keep the shared calls' shape.

use super::win::View;

/// How long the pointer takes to reach a point: nothing moves.
pub(super) const MOVE_SECONDS: f64 = 0.0;

pub(super) fn point(_view: View, _x: f64, _y: f64, _viewport: f64, _who: &str) {}

pub(super) fn ring(_view: View, _x: f64, _y: f64, _viewport: f64) {}

pub(super) fn hide(_view: View) {}

pub(super) fn follow(_view: View) {}
