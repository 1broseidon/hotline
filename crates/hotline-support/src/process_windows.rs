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
/// timeout end its tree through `Job::terminate`, and a caller whose attach
/// fails kills the still-suspended child itself.
pub fn prepare_visible(command: &mut tokio::process::Command) {
    command.creation_flags(CREATE_SUSPENDED | CREATE_NEW_CONSOLE);
    command.kill_on_drop(false);
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
        // descendant can race assignment. Failure leaves it stopped; the
        // caller's kill_on_drop and our job both clean up their owned process.
        // A caller of `prepare_visible` has no kill_on_drop, so it kills the
        // child itself when this fails.
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
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::timeout;
    use windows_sys::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows_sys::Win32::System::JobObjects::{
        JOBOBJECT_BASIC_PROCESS_ID_LIST, JobObjectBasicProcessIdList, QueryInformationJobObject,
    };
    use windows_sys::Win32::System::Threading::{
        PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, TerminateProcess,
        WaitForSingleObject,
    };

    /// The process ids the job holds right now.
    fn members(job: &Job) -> Vec<u32> {
        // Two counts, then the ids; a buffer too small for every id still
        // reports the ones that fit.
        let mut buffer = [0usize; 64];
        let mut returned = 0u32;
        // SAFETY: Win32 writes at most the buffer's size into it, and reads
        // back only the ids it says it wrote, capped to the buffer.
        unsafe {
            QueryInformationJobObject(
                job.0.as_raw_handle(),
                JobObjectBasicProcessIdList,
                buffer.as_mut_ptr().cast(),
                size_of_val(&buffer) as u32,
                &raw mut returned,
            );
            let list = &*(buffer.as_ptr() as *const JOBOBJECT_BASIC_PROCESS_ID_LIST);
            let count = (list.NumberOfProcessIdsInList as usize).min(buffer.len() - 1);
            std::slice::from_raw_parts(list.ProcessIdList.as_ptr(), count)
                .iter()
                .map(|id| *id as u32)
                .collect()
        }
    }

    /// The first process the job holds besides `child`. The child was resumed
    /// inside the job, so the `ping` it starts is a member too: a descendant
    /// the job can reach.
    async fn descendant(job: &Job, child: u32) -> u32 {
        timeout(Duration::from_secs(30), async {
            loop {
                if let Some(found) = members(job).into_iter().find(|member| *member != child) {
                    break found;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the resumed child never started a descendant inside the job")
    }

    /// A `cmd.exe` held in a login's job, the `ping` it starts below it, and a
    /// handle on that ping that can wait for it and end it.
    async fn login_tree() -> (tokio::process::Child, Job, OwnedHandle) {
        let mut command = tokio::process::Command::new("cmd.exe");
        command.raw_arg("/d /s /c \"ping -n 60 127.0.0.1 > nul\"");
        prepare(&mut command);
        let child = command.spawn().unwrap();
        let id = child.id().unwrap();
        let job = Job::attach_without_kill_on_close(child.id()).unwrap();
        let ping = descendant(&job, id).await;
        // SAFETY: OpenProcess takes no memory; owned() checks the handle.
        let handle = owned(unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE | PROCESS_TERMINATE,
                0,
                ping,
            )
        })
        .unwrap();
        (child, job, handle)
    }

    /// Ends the one process a test started, whatever the test found: the ping
    /// outlives its test by a minute otherwise.
    fn end(process: &OwnedHandle) {
        // SAFETY: the handle is the live one the caller owns; a process that
        // is already gone makes this fail, which is the answer wanted.
        unsafe { TerminateProcess(process.as_raw_handle(), 1) };
    }

    #[tokio::test]
    async fn children_wait_for_the_job_and_descendants_die_when_it_closes() {
        let mut command = tokio::process::Command::new("cmd.exe");
        command.raw_arg("/d /s /c \"ping -n 60 127.0.0.1 > nul\"");
        prepare(&mut command);
        let mut child = command.spawn().unwrap();
        let id = child.id().unwrap();
        let job = Job::attach(Some(id)).unwrap();
        let ping = descendant(&job, id).await;
        // SAFETY: OpenProcess takes no memory; owned() checks the handle.
        let handle = owned(unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                0,
                ping,
            )
        })
        .unwrap();
        drop(job);
        assert_eq!(
            // SAFETY: the handle is the live one owned above.
            unsafe { WaitForSingleObject(handle.as_raw_handle(), 5000) },
            WAIT_OBJECT_0
        );
        tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn closing_a_login_job_leaves_its_tree_running() {
        let (mut child, job, handle) = login_tree().await;
        drop(job);
        // A floor, not a race: a tree the close did not reach is still alive
        // after a second.
        // SAFETY: the handle is the live one owned above.
        let waited = unsafe { WaitForSingleObject(handle.as_raw_handle(), 1000) };
        end(&handle);
        child.kill().await.unwrap();
        assert_eq!(waited, WAIT_TIMEOUT);
    }

    #[tokio::test]
    async fn terminating_a_login_job_ends_its_whole_tree() {
        let (mut child, job, handle) = login_tree().await;
        job.terminate().unwrap();
        // SAFETY: the handle is the live one owned above.
        let waited = unsafe { WaitForSingleObject(handle.as_raw_handle(), 5000) };
        end(&handle);
        child.kill().await.unwrap();
        assert_eq!(waited, WAIT_OBJECT_0);
    }

    #[tokio::test]
    #[ignore = "opens a console window"]
    async fn a_visible_child_waits_suspended_until_its_job_resumes_it() {
        let mut command = tokio::process::Command::new("cmd.exe");
        command.args(["/d", "/c", "exit", "3"]);
        prepare_visible(&mut command);
        let mut child = command.spawn().unwrap();
        assert!(child.try_wait().unwrap().is_none());
        let _job = Job::attach_without_kill_on_close(child.id()).unwrap();
        let status = timeout(Duration::from_secs(30), child.wait())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(status.code(), Some(3));
    }
}
