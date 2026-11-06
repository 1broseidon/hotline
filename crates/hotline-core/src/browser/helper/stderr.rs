//! What of the browser helper's standard error reaches the desk's log. The
//! helper's own lines and anything that looks like trouble go through as
//! they are, as `[browser]` lines. GTK, GLib and WebKit also write what they
//! would to a desktop session's terminal: warnings that this display has no
//! accessibility bus, GPU or settings portal, which are true of every server
//! and say nothing an operator can act on. Those known ones are dropped, and
//! any other warning or message of theirs is said once, then left out for
//! [`QUIET`] while it repeats, and said again with how many were left out:
//! WebKit's network process can repeat one for every page.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How long a library's warning is left out after it was said.
pub(super) const QUIET: Duration = Duration::from_secs(600);

/// Library warnings every headless display gives, by a part of their text.
const HARMLESS: [&str; 14] = [
    // No accessibility bus: nobody reads this screen with assistive tools.
    "accessibility bus",
    "AT-SPI",
    "atk-bridge",
    // GTK modules a desktop session loads and a server has not installed.
    "Failed to load module",
    "Not loading module",
    // No GPU, which WebKit is told not to use.
    "libEGL warning",
    "MESA:",
    "Failed to create GBM device",
    "hardware acceleration",
    // No desktop session's settings or portal.
    "GSettings backend",
    "portal",
    "dconf",
    // A cursor theme a server does not have, on a screen with no pointer.
    "cursor theme",
    // Two of a teammate's web processes opening its HSTS store at once.
    "soup_hsts_policies already exists",
];

/// How a line is let through.
#[derive(Debug, PartialEq, Eq)]
enum Kind {
    /// The helper's own, an error, or anything not known to be a library's
    /// chatter.
    Said,
    /// A library's warning every server gives.
    Harmless,
    /// Another warning or message of a library's, by what it says with its
    /// process, time and numbers taken out.
    Chatter(String),
}

/// What a line is. GLib's are `(process:pid): Domain-LEVEL **: time: text`
/// or `Domain-Message: time: text`; a CRITICAL or ERROR one is trouble.
fn kind(line: &str) -> Kind {
    let rest = line
        .strip_prefix('(')
        .and_then(|after| after.split_once("): ").map(|(_, rest)| rest))
        .unwrap_or(line);
    let Some((head, text)) = rest.split_once(": ") else {
        return Kind::Said;
    };
    let head = head.trim_end_matches(" **");
    let level = head.rsplit_once('-').map(|(_, level)| level);
    let library = match level {
        Some("WARNING" | "Message" | "INFO" | "DEBUG") => true,
        Some(_) => false,
        // `libEGL warning: …`, `MESA: …`: not GLib's shape, still a library's.
        None => HARMLESS.iter().any(|part| line.contains(part)),
    };
    if !library {
        return Kind::Said;
    }
    if HARMLESS.iter().any(|part| line.contains(part)) {
        return Kind::Harmless;
    }
    // The time GLib puts first, then every number, so repeats compare equal.
    let text = text
        .split_once(": ")
        .filter(|(time, _)| {
            time.chars()
                .all(|c| c.is_ascii_digit() || c == ':' || c == '.')
        })
        .map_or(text, |(_, text)| text);
    let mut said = String::new();
    for c in format!("{head}: {text}").chars() {
        if !c.is_ascii_digit() {
            said.push(c);
        } else if !said.ends_with('#') {
            said.push('#');
        }
    }
    Kind::Chatter(said)
}

/// The helper's standard error, a line at a time, as the log takes it.
#[derive(Default)]
pub(super) struct Lines {
    /// Each library warning said: when, and how many were left out since.
    said: HashMap<String, (Instant, u32)>,
}

impl Lines {
    /// What the log says for `line`, read at `now`, if anything.
    pub(super) fn take(&mut self, line: &str, now: Instant) -> Option<String> {
        let line = line.trim_end();
        if line.trim().is_empty() {
            return None;
        }
        match kind(line) {
            Kind::Said => Some(format!("[browser] {line}")),
            Kind::Harmless => None,
            Kind::Chatter(key) => match self.said.get_mut(&key) {
                Some((at, left_out)) if now.duration_since(*at) < QUIET => {
                    *left_out += 1;
                    None
                }
                Some((at, left_out)) => {
                    let more = std::mem::take(left_out);
                    *at = now;
                    Some(if more == 0 {
                        format!("[browser] {line}")
                    } else {
                        format!("[browser] {line} ({more} more like it left out)")
                    })
                }
                None => {
                    self.said.insert(key, (now, 0));
                    Some(format!("[browser] {line}"))
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_helpers_own_lines_and_errors_go_through_as_they_are() {
        let mut lines = Lines::default();
        let now = Instant::now();
        for line in [
            "the desk's pipe failed: Broken pipe (os error 32)",
            "Xvfb is not installed (the xvfb package)",
            "[browser] the desk sent a line this helper does not read",
            "(hotline-browser:4012): Gtk-CRITICAL **: 10:01:02.345: gtk_widget_show: assertion 'GTK_IS_WIDGET (widget)' failed",
            "(WebKitWebProcess:4100): GLib-ERROR **: 10:01:02.345: out of memory",
            "thread 'main' panicked at src/native.rs:12:5:",
        ] {
            assert_eq!(lines.take(line, now), Some(format!("[browser] {line}")));
            // Trouble is never left out, however often it comes.
            assert_eq!(lines.take(line, now), Some(format!("[browser] {line}")));
        }
        assert_eq!(lines.take("   ", now), None);
    }

    #[test]
    fn what_every_headless_display_warns_of_is_left_out() {
        let mut lines = Lines::default();
        let now = Instant::now();
        for line in [
            "(hotline-browser:4012): dbind-WARNING **: 10:01:02.345: Couldn't connect to accessibility bus: Failed to connect to socket /run/user/0/at-spi/bus_0: No such file or directory",
            "Gtk-Message: 10:01:02.345: Failed to load module \"canberra-gtk-module\"",
            "libEGL warning: DRI2: failed to authenticate",
            "MESA: error: ZINK: failed to choose pdev",
            "(WebKitWebProcess:4100): GLib-GIO-Message: 10:01:02.345: Using the 'memory' GSettings backend.  Your settings will not be saved or shared with other applications.",
            "(process:4200): libsoup-WARNING **: 10:01:02.345: Failed to execute query: table soup_hsts_policies already exists",
        ] {
            assert_eq!(lines.take(line, now), None, "{line}");
        }
    }

    #[test]
    fn another_library_warning_is_said_once_then_quiet_while_it_repeats() {
        let mut lines = Lines::default();
        let start = Instant::now();
        let one = "(process:4200): libsoup-WARNING **: 10:01:02.345: Failed to execute query: attempt to write a readonly database";
        let same = "(process:4377): libsoup-WARNING **: 10:09:12.003: Failed to execute query: attempt to write a readonly database";
        let other = "(WebKitWebProcess:4100): Gtk-WARNING **: 10:01:03.000: Theme parsing error: gtk.css:12:3: not a number";
        assert_eq!(lines.take(one, start), Some(format!("[browser] {one}")));
        for seconds in [1, 30, 599] {
            let at = start + Duration::from_secs(seconds);
            assert_eq!(
                lines.take(same, at),
                None,
                "another process and time, the same warning"
            );
        }
        assert_eq!(
            lines.take(other, start),
            Some(format!("[browser] {other}")),
            "a different warning is said"
        );
        let later = start + QUIET;
        assert_eq!(
            lines.take(same, later),
            Some(format!("[browser] {same} (3 more like it left out)"))
        );
        assert_eq!(
            lines.take(same, later + QUIET),
            Some(format!("[browser] {same}"))
        );
    }
}
