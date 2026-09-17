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
    SetInformationJobObject, TerminateJobObject,
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
    pub(crate) fn terminate(&self) -> io::Result<()> {
        // This handle names only the process job created for this child.
        if unsafe { TerminateJobObject(self.0.as_raw_handle(), 1) } == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

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
    use windows_sys::Win32::System::JobObjects::{
        JOBOBJECT_BASIC_PROCESS_ID_LIST, JobObjectBasicProcessIdList, QueryInformationJobObject,
    };
    use windows_sys::Win32::System::Threading::{
        PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    };

    /// The process ids the job holds right now.
    fn members(job: &Job) -> Vec<u32> {
        // Two counts, then the ids; a buffer too small for every id still
        // reports the ones that fit.
        let mut buffer = [0usize; 64];
        let mut returned = 0u32;
        unsafe {
            QueryInformationJobObject(
                job.0.as_raw_handle(),
                JobObjectBasicProcessIdList,
                buffer.as_mut_ptr().cast(),
                size_of_val(&buffer) as u32,
                &mut returned,
            );
            let list = &*(buffer.as_ptr() as *const JOBOBJECT_BASIC_PROCESS_ID_LIST);
            let count = (list.NumberOfProcessIdsInList as usize).min(buffer.len() - 1);
            std::slice::from_raw_parts(list.ProcessIdList.as_ptr(), count)
                .iter()
                .map(|id| *id as u32)
                .collect()
        }
    }

    #[tokio::test]
    async fn children_wait_for_the_job_and_descendants_die_when_it_closes() {
        let mut command = tokio::process::Command::new("cmd.exe");
        command.raw_arg("/d /s /c \"ping -n 60 127.0.0.1 > nul\"");
        prepare(&mut command);
        let mut child = command.spawn().unwrap();
        let id = child.id().unwrap();
        let job = Job::attach(Some(id)).unwrap();
        // The child was resumed inside the job, so the ping it starts is a
        // member too: a descendant the job can reach.
        let descendant = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                if let Some(found) = members(&job).into_iter().find(|member| *member != id) {
                    break found;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the resumed child never started a descendant inside the job");
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
