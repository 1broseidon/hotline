//! Which file or directory an open Windows handle names: its volume serial
//! number and file index, read from the handle itself with
//! `GetFileInformationByHandle`, so the answer is about what was opened and
//! never a second lookup of its path.
use std::os::windows::io::{AsRawHandle, BorrowedHandle};
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
};

/// `volume:index` for the open `handle`, or `None` when Windows will not say.
pub fn of_handle(handle: BorrowedHandle<'_>) -> Option<String> {
    // SAFETY: an all-zero BY_HANDLE_FILE_INFORMATION is a valid value of
    // this plain C struct, filled in by the call below.
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: the handle is borrowed open for this call, and `info` is a
    // writable struct of the type the call fills.
    if unsafe { GetFileInformationByHandle(handle.as_raw_handle(), &raw mut info) } == 0 {
        return None;
    }
    let index = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
    Some(format!("{}:{index}", info.dwVolumeSerialNumber))
}
