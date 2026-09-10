//! Private Windows files use a protected DACL, including Rig's token files.
use std::fs::File;
use std::io;
use std::os::windows::{ffi::OsStrExt, fs::MetadataExt, io::FromRawHandle};
use std::path::Path;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, INVALID_HANDLE_VALUE, LocalFree,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo,
    SE_FILE_OBJECT, SetSecurityInfo,
};
use windows_sys::Win32::Security::{
    DACL_SECURITY_INFORMATION, EqualSid, GetSecurityDescriptorDacl, GetTokenInformation,
    OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, SECURITY_ATTRIBUTES,
    TOKEN_OWNER, TOKEN_QUERY, TOKEN_USER, TokenOwner, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateDirectoryW, CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, READ_CONTROL,
    WRITE_DAC,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

struct Descriptor(*mut std::ffi::c_void);
impl Drop for Descriptor {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}

/// Create with a private DACL from the first instant. Existing directories
/// are repaired through a handle, so a path swap cannot redirect the ACL.
pub(crate) fn private_directory(path: &Path) -> io::Result<()> {
    if !path.exists() {
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::other("A vault directory needs a parent."))?;
        if !parent.exists() {
            private_directory(parent)?;
        }
    }
    protect(path, true)
}

pub(crate) fn private_file(path: &Path) -> io::Result<()> {
    protect(path, false)
}

fn protect(path: &Path, directory: bool) -> io::Result<()> {
    let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // Every pointer below refers to a live allocation; descriptors are freed
    // after the last Win32 call and the File owns the opened handle.
    unsafe {
        let mut token = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut len = 0;
        GetTokenInformation(token, TokenUser, null_mut(), 0, &mut len);
        let mut buffer = vec![0usize; (len as usize).div_ceil(size_of::<usize>())];
        let status =
            GetTokenInformation(token, TokenUser, buffer.as_mut_ptr().cast(), len, &mut len);
        let error = io::Error::last_os_error();
        if status == 0 {
            CloseHandle(token);
            return Err(error);
        }
        let user = &*buffer.as_ptr().cast::<TOKEN_USER>();
        GetTokenInformation(token, TokenOwner, null_mut(), 0, &mut len);
        let mut owner_buffer = vec![0usize; (len as usize).div_ceil(size_of::<usize>())];
        let status = GetTokenInformation(
            token,
            TokenOwner,
            owner_buffer.as_mut_ptr().cast(),
            len,
            &mut len,
        );
        let error = io::Error::last_os_error();
        CloseHandle(token);
        if status == 0 {
            return Err(error);
        }
        let default_owner = &*owner_buffer.as_ptr().cast::<TOKEN_OWNER>();
        let mut sid_text = null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut sid_text) == 0 {
            return Err(io::Error::last_os_error());
        }
        let sid_allocation = Descriptor(sid_text.cast());
        let mut sid_len = 0;
        while *sid_text.add(sid_len) != 0 {
            sid_len += 1;
        }
        let sid = String::from_utf16_lossy(std::slice::from_raw_parts(sid_text, sid_len));
        drop(sid_allocation);
        let sddl: Vec<u16> = format!("O:{sid}D:P(A;OICI;FA;;;{sid})(A;OICI;FA;;;SY)")
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let mut descriptor = null_mut();
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut descriptor,
            null_mut(),
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let descriptor = Descriptor(descriptor);
        if directory {
            let attributes = SECURITY_ATTRIBUTES {
                nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor.0,
                bInheritHandle: 0,
            };
            if CreateDirectoryW(name.as_ptr(), &attributes) == 0 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() != Some(ERROR_ALREADY_EXISTS as i32) {
                    return Err(error);
                }
            }
        }
        let handle = CreateFileW(
            name.as_ptr(),
            READ_CONTROL | WRITE_DAC | FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            null(),
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
            null_mut(),
        );
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let file = File::from_raw_handle(handle);
        let metadata = file.metadata()?;
        if metadata.file_attributes() & 0x400 != 0 || metadata.is_dir() != directory {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Vault storage cannot be a reparse point or the wrong file type.",
            ));
        }
        let mut owner = null_mut();
        let mut existing = null_mut();
        let status = GetSecurityInfo(
            handle,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            null_mut(),
            null_mut(),
            &mut existing,
        );
        if status != 0 {
            return Err(io::Error::from_raw_os_error(status as i32));
        }
        let _existing = Descriptor(existing);
        // An elevated token can create files owned by its default owner group.
        // Accept that token's owner too; the DACL still grants only user/SYSTEM.
        if EqualSid(owner, user.User.Sid) == 0 && EqualSid(owner, default_owner.Owner) == 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Vault storage belongs to another Windows user.",
            ));
        }
        let mut present = 0;
        let mut defaulted = 0;
        let mut dacl = null_mut();
        if GetSecurityDescriptorDacl(descriptor.0, &mut present, &mut dacl, &mut defaulted) == 0
            || present == 0
            || dacl.is_null()
        {
            return Err(io::Error::last_os_error());
        }
        let status = SetSecurityInfo(
            handle,
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            dacl,
            null(),
        );
        if status != 0 {
            return Err(io::Error::from_raw_os_error(status as i32));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Security::Authorization::ConvertSecurityDescriptorToStringSecurityDescriptorW;

    #[test]
    fn a_private_directory_has_a_protected_dacl_and_rejects_junctions() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("vault");
        private_directory(&directory).unwrap();
        let file = directory.join("auth.json");
        std::fs::write(&file, b"{}").unwrap();
        private_file(&file).unwrap();
        let handle = File::open(&file).unwrap();
        unsafe {
            let mut descriptor = null_mut();
            assert_eq!(
                GetSecurityInfo(
                    handle.as_raw_handle(),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION,
                    null_mut(),
                    null_mut(),
                    null_mut(),
                    null_mut(),
                    &mut descriptor
                ),
                0
            );
            let descriptor = Descriptor(descriptor);
            let mut text = null_mut();
            assert_ne!(
                ConvertSecurityDescriptorToStringSecurityDescriptorW(
                    descriptor.0,
                    1,
                    DACL_SECURITY_INFORMATION,
                    &mut text,
                    null_mut()
                ),
                0
            );
            let _text = Descriptor(text.cast());
            let mut len = 0;
            while *text.add(len) != 0 {
                len += 1;
            }
            let dacl = String::from_utf16_lossy(std::slice::from_raw_parts(text, len));
            assert!(dacl.starts_with("D:P"), "{dacl}");
            assert_eq!(dacl.matches("(A;").count(), 2, "{dacl}");
            assert!(dacl.contains(";;;SY)"), "{dacl}");
            assert!(dacl.contains(";;;S-"), "{dacl}");
        }
        let other = root.path().join("other");
        std::fs::create_dir(&other).unwrap();
        let junction = root.path().join("junction");
        let status = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(&junction)
            .arg(&other)
            .output()
            .unwrap()
            .status;
        assert!(status.success());
        assert!(private_directory(&junction).is_err());
    }
}
