use super::*;
use std::time::Duration;
use tokio::time::timeout;
use windows_sys::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::System::JobObjects::{
    JOBOBJECT_BASIC_PROCESS_ID_LIST, JobObjectBasicProcessIdList, QueryInformationJobObject,
};
use windows_sys::Win32::System::Threading::{
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, TerminateProcess, WaitForSingleObject,
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
    let exited = timeout(Duration::from_secs(5), child.wait()).await;
    assert_eq!(waited, WAIT_OBJECT_0);
    exited.unwrap().unwrap();
}

#[tokio::test]
#[ignore = "needs a console window"]
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
