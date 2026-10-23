use super::*;
use crate::paths::{agent_account_dir, agent_accounts_dir};
use tempfile::TempDir;

const ID: &str = "89bbac55-e7ac-4409-a709-115f33bb12f0";

fn fixture() -> (TempDir, PathBuf, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("Conta Teste ção");
    let dir = agent_account_dir(&root, ID).unwrap();
    (tmp, root, dir)
}

fn canonical(root: &Path) -> PathBuf {
    agent_account_dir(&dunce::canonicalize(root).unwrap(), ID).unwrap()
}

fn link(target: &Path, at: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, at).unwrap();
    #[cfg(windows)]
    {
        let status = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(at)
            .arg(target)
            .output()
            .unwrap()
            .status;
        assert!(status.success(), "mklink /J {at:?} {target:?}");
    }
}

fn kind(root: &Path, id: &str) -> io::ErrorKind {
    checked_account_dir(root, id).unwrap_err().kind()
}

#[test]
fn a_real_account_folder_answers_its_canonical_path() {
    let (_tmp, root, dir) = fixture();
    make_private_directory(&dir).unwrap();
    let answer = checked_account_dir(&root, ID).unwrap();
    assert_eq!(answer, canonical(&root));
    assert!(!answer.to_string_lossy().starts_with(r"\\?\"));
}

#[test]
fn a_root_reached_through_a_link_still_finds_its_folder() {
    let (tmp, root, dir) = fixture();
    make_private_directory(&dir).unwrap();
    let via = tmp.path().join("via");
    link(&root, &via);
    assert_eq!(checked_account_dir(&via, ID).unwrap(), canonical(&root));
}

#[cfg(windows)]
#[test]
#[ignore = "needs a volume with 8.3 names"]
fn a_root_reached_by_its_8_3_short_name_still_finds_its_folder() {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows_sys::Win32::Storage::FileSystem::GetShortPathNameW;

    let (_tmp, root, dir) = fixture();
    make_private_directory(&dir).unwrap();
    let long: Vec<u16> = root.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: `long` is NUL-terminated and outlives both calls; the second
    // call is given the length the first asked for, and Win32 writes no more.
    let short = unsafe {
        let needed = GetShortPathNameW(long.as_ptr(), std::ptr::null_mut(), 0);
        let mut buffer = vec![0u16; needed as usize];
        let written = GetShortPathNameW(long.as_ptr(), buffer.as_mut_ptr(), needed);
        PathBuf::from(std::ffi::OsString::from_wide(&buffer[..written as usize]))
    };
    assert_ne!(short, root, "this volume has no 8.3 names");
    assert_eq!(checked_account_dir(&short, ID).unwrap(), canonical(&root));
}

#[test]
fn a_folder_that_is_missing_a_file_a_link_or_elsewhere_is_refused_by_kind() {
    let (_tmp, root, _dir) = fixture();
    assert_eq!(kind(&root, "not-a-uuid"), io::ErrorKind::InvalidInput);
    assert_eq!(kind(&root, ID), io::ErrorKind::NotFound);
    fs::create_dir_all(&root).unwrap();
    assert_eq!(kind(&root, ID), io::ErrorKind::NotFound);

    let (_tmp, root, dir) = fixture();
    fs::create_dir_all(agent_accounts_dir(&root)).unwrap();
    fs::write(&dir, b"not a folder").unwrap();
    assert_eq!(kind(&root, ID), io::ErrorKind::NotADirectory);

    let (tmp, root, dir) = fixture();
    let elsewhere = tmp.path().join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    fs::create_dir_all(agent_accounts_dir(&root)).unwrap();
    link(&elsewhere, &dir);
    assert_eq!(kind(&root, ID), io::ErrorKind::PermissionDenied);

    let (tmp, root, _dir) = fixture();
    let elsewhere = tmp.path().join("elsewhere");
    make_private_directory(&elsewhere.join(ID)).unwrap();
    fs::create_dir_all(&root).unwrap();
    link(&elsewhere, &agent_accounts_dir(&root));
    assert_eq!(kind(&root, ID), io::ErrorKind::PermissionDenied);
}

#[test]
fn a_root_that_is_an_alias_does_not_excuse_a_link_in_place_of_the_folder() {
    let (tmp, root, _dir) = fixture();
    let (via, elsewhere) = (tmp.path().join("via"), tmp.path().join("elsewhere"));
    make_private_directory(&elsewhere.join(ID)).unwrap();
    fs::create_dir_all(&root).unwrap();
    link(&root, &via);
    link(&elsewhere, &agent_accounts_dir(&root));
    assert_eq!(kind(&via, ID), io::ErrorKind::PermissionDenied);

    let (tmp, root, dir) = fixture();
    let (via, elsewhere) = (tmp.path().join("via"), tmp.path().join("elsewhere"));
    fs::create_dir_all(&elsewhere).unwrap();
    fs::create_dir_all(agent_accounts_dir(&root)).unwrap();
    link(&root, &via);
    link(&elsewhere, &dir);
    assert_eq!(kind(&via, ID), io::ErrorKind::PermissionDenied);
}

#[cfg(unix)]
#[test]
fn setting_the_directory_private_does_not_follow_a_symlink() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let target = tmp.path().join("elsewhere");
    fs::create_dir_all(&target).unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
    let vault_dir = tmp.path().join("vault");
    std::os::unix::fs::symlink(&target, &vault_dir).unwrap();

    assert!(make_private_directory(&vault_dir).is_err());
    let mode = fs::metadata(&target).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        mode, 0o755,
        "chmod followed the symlink and changed the target"
    );
}

#[cfg(unix)]
#[test]
fn a_private_directory_is_0700_and_makes_its_parents() {
    use std::os::unix::fs::PermissionsExt;

    let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
    let tmp = tempfile::tempdir().unwrap();
    let (parent, child) = (tmp.path().join("a"), tmp.path().join("a").join("b"));
    make_private_directory(&child).unwrap();
    assert_eq!((mode(&parent), mode(&child)), (0o700, 0o700));

    fs::set_permissions(&child, fs::Permissions::from_mode(0o755)).unwrap();
    make_private_directory(&child).unwrap();
    assert_eq!(mode(&child), 0o700);
}
