//! A Windows child cannot execute until it belongs to its own kill-on-close job.
//! Closing that job reaches launchers' descendants as well as the direct child.
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject,
};
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_SUSPENDED, OpenProcess, OpenThread, PROCESS_SET_QUOTA,
    PROCESS_TERMINATE, ResumeThread, THREAD_SUSPEND_RESUME,
};

pub(crate) fn prepare(command: &mut tokio::process::Command) {
    command.creation_flags(CREATE_SUSPENDED | CREATE_NO_WINDOW);
    command.kill_on_drop(true);
}

fn owned(handle: HANDLE) -> io::Result<OwnedHandle> {
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // Win32 returned a new owned handle; OwnedHandle closes it on every exit.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

pub(crate) struct Job(#[allow(dead_code)] OwnedHandle);

impl Job {
    pub(crate) fn attach(id: Option<u32>) -> io::Result<Self> {
        let id = id.ok_or_else(|| {
            io::Error::other("The child exited before its process job was established.")
        })?;
        // The child is suspended and its caller still owns the process. No
        // descendant can race assignment. Failure leaves it stopped; the
        // caller's kill_on_drop and our job both clean up their owned process.
        unsafe {
            let process = owned(OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, id))?;
            let job = owned(CreateJobObjectW(std::ptr::null(), std::ptr::null()))?;
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                job.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of_val(&limits) as u32,
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            if AssignProcessToJobObject(job.as_raw_handle(), process.as_raw_handle()) == 0 {
                return Err(io::Error::last_os_error());
            }
            let snapshot = owned(CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0))?;
            let mut entry: THREADENTRY32 = std::mem::zeroed();
            entry.dwSize = size_of::<THREADENTRY32>() as u32;
            let mut found = Thread32First(snapshot.as_raw_handle(), &mut entry);
            while found != 0 {
                if entry.th32OwnerProcessID == id {
                    let thread = owned(OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID))?;
                    if ResumeThread(thread.as_raw_handle()) == u32::MAX {
                        return Err(io::Error::last_os_error());
                    }
                    return Ok(Self(job));
                }
                found = Thread32Next(snapshot.as_raw_handle(), &mut entry);
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
    use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
    use windows_sys::Win32::System::Threading::{
        PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    };

    #[tokio::test]
    async fn children_wait_for_the_job_and_descendants_die_when_it_closes() {
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("child.pid");
        let marker_text = marker.to_string_lossy().replace('\'', "''");
        let script = format!(
            "$p = Start-Process -FilePath cmd.exe -ArgumentList '/d /c ping -n 60 127.0.0.1 > nul' -PassThru -NoNewWindow; $p.Id | Set-Content -Path '{marker_text}'; Start-Sleep -Seconds 60"
        );
        let mut command = tokio::process::Command::new("powershell.exe");
        command.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
        prepare(&mut command);
        let mut child = command.spawn().unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!marker.exists(), "the child ran before job assignment");
        let job = Job::attach(child.id()).unwrap();
        let descendant = tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                if let Ok(text) = std::fs::read_to_string(&marker)
                    && let Ok(id) = text.trim().parse::<u32>()
                {
                    break id;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        let handle = owned(unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                0,
                descendant,
            )
        })
        .unwrap();
        drop(job);
        assert_eq!(
            unsafe { WaitForSingleObject(handle.as_raw_handle(), 5000) },
            WAIT_OBJECT_0
        );
        tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .unwrap()
            .unwrap();
    }
}
