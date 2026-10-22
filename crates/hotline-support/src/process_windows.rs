//! A Windows child cannot execute until it belongs to a job of its own: it
//! starts suspended and runs only once its job holds it. The default job kills
//! on close, which reaches launchers' descendants as well as the direct child;
//! only a login's job does not, and only `Job::terminate` ends that one.
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Threading::{
    CREATE_NEW_CONSOLE, CREATE_NO_WINDOW, CREATE_SUSPENDED, OpenProcess, OpenThread,
    PROCESS_SET_QUOTA, PROCESS_TERMINATE, ResumeThread, THREAD_SUSPEND_RESUME,
};

/// A console program started by the windowed app gets a console window of its
/// own unless told not to: every `docker ps` would flash one on screen.
pub fn quiet(command: &mut tokio::process::Command) {
    command.creation_flags(CREATE_NO_WINDOW);
}

pub fn prepare(command: &mut tokio::process::Command) {
    command.creation_flags(CREATE_SUSPENDED | CREATE_NO_WINDOW);
    command.kill_on_drop(true);
}

/// `prepare` for a child that has to show a console, such as a login the person
/// types into. `CREATE_NO_WINDOW` excludes a console, so this takes
/// `CREATE_NEW_CONSOLE` instead. Dropping the child must not kill it: cancel and
/// timeout end its tree through `Job::terminate`, then kill the direct child,
/// and a caller whose attach fails kills the still-suspended child itself.
pub fn prepare_visible(command: &mut tokio::process::Command) {
    command.creation_flags(CREATE_SUSPENDED | CREATE_NEW_CONSOLE);
    command.kill_on_drop(false);
    // The app's stdout and stderr can be its log file. With all three null,
    // std passes no handles, and the new console gives the child its own.
    command
        .stdin(no_handle())
        .stdout(no_handle())
        .stderr(no_handle());
}

fn no_handle() -> std::process::Stdio {
    // SAFETY: std documents that an OwnedHandle may hold NULL, the value a
    // process without a console has, and passes it to the child as NULL.
    std::process::Stdio::from(unsafe { OwnedHandle::from_raw_handle(std::ptr::null_mut()) })
}

fn owned(handle: HANDLE) -> io::Result<OwnedHandle> {
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: Win32 returned a new owned handle; OwnedHandle closes it on every exit.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

pub struct Job(OwnedHandle);

impl Job {
    pub fn terminate(&self) -> io::Result<()> {
        // SAFETY: this handle names only the process job created for this child.
        if unsafe { TerminateJobObject(self.0.as_raw_handle(), 1) } == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    /// Holds the suspended child `id` in a new job that kills everything in it
    /// when the job is closed, then lets the child run.
    pub fn attach(id: Option<u32>) -> io::Result<Self> {
        Self::attach_with(id, true)
    }

    /// Like `attach`, but closing the job kills nothing. A login's browser can
    /// be a descendant, so a normal exit only closes the handle and leaves that
    /// browser open; the tree ends only through `terminate`.
    pub fn attach_without_kill_on_close(id: Option<u32>) -> io::Result<Self> {
        Self::attach_with(id, false)
    }

    fn attach_with(id: Option<u32>, kill_on_close: bool) -> io::Result<Self> {
        let id = id.ok_or_else(|| {
            io::Error::other("The child exited before its process job was established.")
        })?;
        // SAFETY: the child is suspended and its caller still owns the process. No
        // descendant can race assignment. Failure returns before the resume and
        // leaves it stopped: the caller's kill_on_drop ends it, as does a
        // kill-on-close job that already holds it when that job's handle drops.
        // A login's job ends nothing on close and `prepare_visible` sets no
        // kill_on_drop, so that caller kills the still-suspended child itself.
        unsafe {
            let process = owned(OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, id))?;
            let job = owned(CreateJobObjectW(std::ptr::null(), std::ptr::null()))?;
            if kill_on_close {
                let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                if SetInformationJobObject(
                    job.as_raw_handle(),
                    JobObjectExtendedLimitInformation,
                    (&raw const limits).cast(),
                    size_of_val(&limits) as u32,
                ) == 0
                {
                    return Err(io::Error::last_os_error());
                }
            }
            if AssignProcessToJobObject(job.as_raw_handle(), process.as_raw_handle()) == 0 {
                return Err(io::Error::last_os_error());
            }
            let snapshot = owned(CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0))?;
            let mut entry: THREADENTRY32 = std::mem::zeroed();
            entry.dwSize = size_of::<THREADENTRY32>() as u32;
            let mut found = Thread32First(snapshot.as_raw_handle(), &raw mut entry);
            while found != 0 {
                if entry.th32OwnerProcessID == id {
                    let thread = owned(OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID))?;
                    if ResumeThread(thread.as_raw_handle()) == u32::MAX {
                        return Err(io::Error::last_os_error());
                    }
                    return Ok(Self(job));
                }
                found = Thread32Next(snapshot.as_raw_handle(), &raw mut entry);
            }
            Err(io::Error::other(
                "The suspended child's initial thread could not be resumed.",
            ))
        }
    }
}

#[cfg(test)]
mod tests;
