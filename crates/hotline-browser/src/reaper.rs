//! The helper's first process, which only reaps. WebKit starts processes of
//! its own for each teammate's pages and network, and the browser starts
//! its X display: when one of those is left behind by the process that
//! started it, Linux gives it to the nearest "subreaper" above it, else to
//! process 1. Under a `hotline serve` that is a container's process 1 and
//! never waits for children it did not start, each would stay a zombie. So
//! the helper forks before anything else: the parent becomes that subreaper
//! and waits for whatever comes to it, and the child runs the browser. When
//! the browser exits, the parent gives what it left two seconds to go,
//! kills the rest, reaps them, and exits with the browser's status.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// A stop was asked for (SIGTERM, SIGINT or SIGHUP), to pass to the browser.
static ASKED: AtomicBool = AtomicBool::new(false);

extern "C" fn stop_asked(_signal: libc::c_int) {
    ASKED.store(true, Ordering::SeqCst);
}

/// Forks. The child returns, to run the browser, and dies with the parent;
/// the parent reaps until the browser is gone and never returns.
pub fn split() {
    // SAFETY: plain process calls while this process has one thread, so
    // the child inherits no other thread's locks.
    let (parent, browser) = unsafe {
        libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0);
        (libc::getpid(), libc::fork())
    };
    match browser {
        -1 => {
            eprintln!(
                "the browser could not start: {}",
                std::io::Error::last_os_error()
            );
            std::process::exit(1);
        }
        0 => {
            // SAFETY: as above; a reaper gone before this ran is checked for.
            unsafe {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0);
                if libc::getppid() != parent {
                    std::process::exit(1);
                }
            }
        }
        browser => reap(browser),
    }
}

fn reap(browser: libc::pid_t) -> ! {
    // SAFETY: the pipe is the browser's: with this copy closed, the desk
    // hears the browser hang up, not a reaper still holding it.
    unsafe { libc::close(hotline_browser_core::pipe::PIPE_FD) };
    for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
        // SAFETY: the handler only stores to an atomic. No SA_RESTART, so
        // a wait is interrupted to pass the stop on.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = stop_asked as *const () as libc::sighandler_t;
            libc::sigaction(signal, &raw const action, std::ptr::null_mut());
        }
    }
    let mut passed = false;
    let code = loop {
        let mut status = 0;
        // SAFETY: a wait for any child, with a status to fill.
        let pid = unsafe { libc::waitpid(-1, &raw mut status, 0) };
        if pid == browser {
            break exit_code(status);
        }
        if pid < 0 {
            let error = std::io::Error::last_os_error();
            match error.raw_os_error() {
                Some(libc::EINTR) => {
                    if ASKED.load(Ordering::SeqCst) && !passed {
                        // SAFETY: the browser this process started.
                        unsafe { libc::kill(browser, libc::SIGTERM) };
                        passed = true;
                    }
                }
                _ => break 1,
            }
        }
    };
    finish();
    std::process::exit(code);
}

/// The browser's status as this process's exit code.
fn exit_code(status: libc::c_int) -> i32 {
    if libc::WIFEXITED(status) {
        libc::WEXITSTATUS(status)
    } else if libc::WIFSIGNALED(status) {
        128 + libc::WTERMSIG(status)
    } else {
        1
    }
}

/// Reaps what the browser left: two seconds for it to go by itself, as
/// WebKit's processes do once their browser is gone, then killed.
fn finish() {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if reap_waiting() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    // A killed process's own children come here next, so it goes round.
    for _ in 0..100 {
        for pid in children() {
            // SAFETY: a process whose parent is this one.
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
        std::thread::sleep(Duration::from_millis(20));
        if reap_waiting() {
            return;
        }
    }
}

/// Reaps every child that has exited; true once there are none at all.
fn reap_waiting() -> bool {
    loop {
        let mut status = 0;
        // SAFETY: as in `reap`, without waiting.
        let pid = unsafe { libc::waitpid(-1, &raw mut status, libc::WNOHANG) };
        if pid > 0 {
            continue;
        }
        return pid < 0;
    }
}

/// This process's children, from `/proc`.
fn children() -> Vec<libc::pid_t> {
    // SAFETY: a plain read of this process's id.
    let me = unsafe { libc::getpid() };
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| entry.file_name().to_str()?.parse::<libc::pid_t>().ok())
        .filter(|pid| {
            std::fs::read_to_string(format!("/proc/{pid}/stat"))
                .ok()
                .and_then(|stat| {
                    let rest = &stat[stat.rfind(')')? + 2..];
                    rest.split(' ').nth(1)?.parse::<libc::pid_t>().ok()
                })
                == Some(me)
        })
        .collect()
}
