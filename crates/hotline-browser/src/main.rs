//! `hotline-browser`: each teammate's desk browser on a server, for `hotline
//! serve`, which starts this beside itself at a teammate's first page and
//! talks to it over the socket it hands over on descriptor 3
//! (`hotline_core::browser::pipe`). It is the Linux desk browser of the app,
//! the same files compiled here (`native.rs`), in a WebKitGTK that draws on
//! an X display of its own nobody sees (`display.rs`), so the server's own
//! binary never links GTK and runs as before where WebKitGTK is not
//! installed. The phone or a linked desktop is its screen, through the
//! desk's pictures of it and the person's take-over commands.
//!
//! The process forks first: the parent only reaps (`reaper.rs`), so WebKit's
//! own processes and the display never outlive it as zombies, even under a
//! `hotline serve` that runs as a container's process 1. The child runs GTK
//! on its main thread and the pipe on a thread of its own, and exits when
//! the desk hangs up.
//!
//! `hotline-browser --probe` starts nothing: it runs only if its libraries
//! load, and says whether Xvfb is here, for the desk to decide whether to
//! offer a browser at all.
//!
//! It never depends on `hotline-app` or Tauri, and nothing depends on it.

#[cfg(target_os = "linux")]
mod display;
#[cfg(target_os = "linux")]
mod native;
#[cfg(target_os = "linux")]
mod reaper;

use std::process::ExitCode;

#[cfg(not(target_os = "linux"))]
fn main() -> ExitCode {
    eprintln!("hotline-browser runs on Linux only.");
    ExitCode::from(2)
}

#[cfg(target_os = "linux")]
fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let data = match (args.next().as_deref(), args.next()) {
        (Some("--probe"), None) => return probe(),
        (Some("--data"), Some(data)) => std::path::PathBuf::from(data),
        _ => {
            eprintln!("usage: hotline-browser --data <dir> (started by hotline serve), or --probe");
            return ExitCode::from(2);
        }
    };
    // Before anything else, while this process has one thread.
    reaper::split();
    match native::run(&data) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

/// Says what this browser is, its version first, when it can run here at
/// all: its libraries loaded, or the loader would have refused it before
/// `main`, and Xvfb is on `PATH`.
#[cfg(target_os = "linux")]
fn probe() -> ExitCode {
    if display::xvfb().is_none() {
        eprintln!("Xvfb is not installed (the xvfb package)");
        return ExitCode::FAILURE;
    }
    // SAFETY: pure version reads.
    let (major, minor, micro) = unsafe {
        (
            webkit2gtk::ffi::webkit_get_major_version(),
            webkit2gtk::ffi::webkit_get_minor_version(),
            webkit2gtk::ffi::webkit_get_micro_version(),
        )
    };
    println!(
        "hotline-browser {}, WebKitGTK {major}.{minor}.{micro}",
        env!("CARGO_PKG_VERSION")
    );
    ExitCode::SUCCESS
}
