//! Which directory a handle holds (H-64), read from the handle itself and
//! never by looking its path up again, in the form the desk records it
//! (`cwdIdentity`, a grant's `identity`): `<volume>:<inode>`.
//!
//! The volume part names the volume in a way that outlives a restart where
//! the system keeps one, and says which way it is:
//!
//! - macOS `uuid:<volume UUID>`, from `fgetattrlist`; `snapshot:<UUID>` for
//!   a volume mounted from a snapshot (which answers its source's UUID and
//!   inode numbers, so it is not taken for the live volume); a volume with
//!   no UUID `mount:<f_fstypename>:<f_mntfromname>`, which is what it was
//!   mounted from: a server and share, or a device node. When macOS will
//!   not say which volume a handle is on at all, there is no identity.
//! - Linux `fsid:ext4:<f_fsid>` on ext4, which folds its UUID into
//!   `f_fsid`; otherwise `dev:<st_dev>`, which lasts one boot.
//! - Windows the volume serial number, as before (`<serial>:<file index>`).
//!
//! The 0.50 betas recorded a Unix directory as `<st_dev>:<inode>`, digits
//! only. macOS numbers volumes as it mounts them, so after a restart that
//! record named a device the folder was no longer on, and every holder
//! refused it. A record in a form that does not outlive a restart (that
//! one, `dev:` and `mount:`) is rebound by a session start when the inode
//! still matches and no confined teammate could have put another directory
//! there (`session/workspace_root.rs`); until then every holder compares it
//! as it always did.

use cap_std::{ambient_authority, fs::Dir};
use std::path::Path;

/// What one open of a directory says about which it is.
pub struct DirectoryId {
    volume: String,
    inode: u64,
    /// `st_dev` on Unix: what the confined shell's `stat` prints and an
    /// older record names.
    #[cfg(unix)]
    device: u64,
    /// Whether the volume never gives an inode number to another file
    /// once it is used (APFS, whose inode numbers are object ids that only
    /// grow).
    keeps_inodes: bool,
}

impl DirectoryId {
    /// The directory `dir` holds, or `None` where it cannot be read, which
    /// every holder treats as a directory it cannot confirm.
    pub fn of(dir: &Dir) -> Option<Self> {
        #[cfg(unix)]
        {
            use cap_std::fs::MetadataExt as _;
            let metadata = dir.dir_metadata().ok()?;
            let (volume, keeps_inodes) = unix_volume(dir, metadata.dev())?;
            Some(Self {
                volume,
                inode: metadata.ino(),
                device: metadata.dev(),
                keeps_inodes,
            })
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::AsHandle;
            let found = hotline_support::file_identity_windows::of_handle(dir.as_handle())?;
            let (volume, index) = found.split_once(':')?;
            Some(Self {
                volume: volume.to_string(),
                inode: index.parse().ok()?,
                keeps_inodes: false,
            })
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = dir;
            None
        }
    }

    /// The directory `path` opens now (following links).
    pub fn at(path: &Path) -> Option<Self> {
        Self::of(&Dir::open_ambient_dir(path, ambient_authority()).ok()?)
    }

    /// The form the desk records.
    pub fn recorded(&self) -> String {
        format!("{}:{}", self.volume, self.inode)
    }

    /// The volume part of [`DirectoryId::recorded`].
    pub fn volume(&self) -> &str {
        &self.volume
    }

    /// Whether the volume never reuses an inode number.
    pub fn keeps_inodes(&self) -> bool {
        self.keeps_inodes
    }

    /// Whether `recorded` names this directory: it is this directory's
    /// recorded form, or the older Unix form with this directory's device
    /// and inode now, which is what that record was always compared with.
    pub fn is(&self, recorded: &str) -> bool {
        recorded == self.recorded() || self.legacy().as_deref() == Some(recorded)
    }

    /// `dev:ino` as the confined shell's `stat` prints it, for the check it
    /// runs inside its sandbox against this same handle in this same boot.
    pub(crate) fn checked(&self) -> String {
        #[cfg(unix)]
        {
            format!("{}:{}", self.device, self.inode)
        }
        #[cfg(not(unix))]
        {
            self.recorded()
        }
    }

    fn legacy(&self) -> Option<String> {
        #[cfg(unix)]
        {
            Some(format!("{}:{}", self.device, self.inode))
        }
        #[cfg(not(unix))]
        {
            None
        }
    }

    /// Whether `recorded`, in a form that does not outlive a restart
    /// ([`rebindable_inode`]), names this directory's inode.
    pub fn same_inode(&self, recorded: &str) -> bool {
        rebindable_inode(recorded) == Some(self.inode)
    }
}

/// The inode of a record in a form that does not outlive a restart: the
/// older Unix `dev:ino` (digits only), `dev:` and `mount:`. `None` for a
/// form that does (`uuid:`, `snapshot:`, `fsid:`, every Windows record),
/// which names another directory whenever it differs.
pub fn rebindable_inode(recorded: &str) -> Option<u64> {
    if !cfg!(unix) {
        return None;
    }
    let (volume, inode) = recorded.rsplit_once(':')?;
    let inode = inode.parse().ok()?;
    let legacy = !volume.is_empty() && volume.bytes().all(|b| b.is_ascii_digit());
    (legacy || volume.starts_with("dev:") || volume.starts_with("mount:")).then_some(inode)
}

/// The volume part of the identity of `dir` on Unix, and whether that
/// volume keeps inode numbers; `None` when macOS will not say which volume
/// it is. Never the bare device number: where nothing steadier is known it
/// says `dev:`, so a reader knows it lasts one boot.
#[cfg(unix)]
fn unix_volume(dir: &Dir, device: u64) -> Option<(String, bool)> {
    use std::os::fd::AsFd;
    #[cfg(target_os = "macos")]
    {
        let _ = device;
        let volume = hotline_support::file_identity_macos::volume_of(dir.as_fd())?;
        let named = match volume.uuid {
            Some(uuid) if volume.snapshot => format!("snapshot:{}", uuid_text(uuid)),
            Some(uuid) => format!("uuid:{}", uuid_text(uuid)),
            None => format!("mount:{}:{}", volume.fs_type, volume.mounted_from),
        };
        Some((named, volume.fs_type == "apfs"))
    }
    #[cfg(target_os = "linux")]
    {
        let named = hotline_support::file_identity_linux::stable_of(dir.as_fd()).map_or_else(
            || format!("dev:{device}"),
            |found| format!("fsid:{}:{:016x}", found.fs_type, found.fsid),
        );
        Some((named, false))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = dir.as_fd();
        Some((format!("dev:{device}"), false))
    }
}

/// A UUID as macOS spells it (`diskutil info`): upper-case hex in 8-4-4-4-12.
#[cfg(target_os = "macos")]
fn uuid_text(uuid: [u8; 16]) -> String {
    use std::fmt::Write as _;
    let mut text = String::with_capacity(36);
    for (at, byte) in uuid.iter().enumerate() {
        if matches!(at, 4 | 6 | 8 | 10) {
            text.push('-');
        }
        let _ = write!(text, "{byte:02X}");
    }
    text
}

/// [`DirectoryId::recorded`] for the directory `dir` holds.
pub fn dir_identity(dir: &Dir) -> Option<String> {
    DirectoryId::of(dir).map(|found| found.recorded())
}

/// [`dir_identity`] of the directory `path` opens now (following links), for
/// recording what the person chose when they choose it.
pub fn path_identity(path: &Path) -> Option<String> {
    DirectoryId::at(path).map(|found| found.recorded())
}

/// Whether the directory `dir` holds is the one `recorded` names
/// ([`DirectoryId::is`]), read from the handle.
pub fn is_recorded(dir: &Dir, recorded: &str) -> bool {
    DirectoryId::of(dir).is_some_and(|found| found.is(recorded))
}

/// [`is_recorded`] for the directory `path` opens now (following links).
pub fn path_is_recorded(path: &Path, recorded: &str) -> bool {
    DirectoryId::at(path).is_some_and(|found| found.is(recorded))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One directory has one identity, however often it is opened, and the
    /// two holders of it compare it the same way. On macOS it names the
    /// volume by its UUID, a form no older record has and none rebinds.
    #[test]
    fn one_directory_has_the_same_identity_on_every_open() {
        let root = tempfile::tempdir().unwrap();
        let work = root.path().join("work");
        std::fs::create_dir(&work).unwrap();
        let first = path_identity(&work).unwrap();
        let held = Dir::open_ambient_dir(&work, ambient_authority()).unwrap();
        assert_eq!(dir_identity(&held), Some(first.clone()));
        assert_eq!(path_identity(&work), Some(first.clone()));
        assert!(is_recorded(&held, &first) && path_is_recorded(&work, &first));
        let other = root.path().join("other");
        std::fs::create_dir(&other).unwrap();
        assert_ne!(path_identity(&other), Some(first.clone()));
        assert!(!path_is_recorded(&other, &first));
        if cfg!(target_os = "macos") {
            assert!(first.starts_with("uuid:"), "{first}");
            assert_eq!(rebindable_inode(&first), None);
        }
    }

    /// A record in the older Unix form (`dev:ino`, digits only) is still
    /// the directory with that device and inode now, as every holder always
    /// compared it, and no other: a device number one off (a restart) is
    /// not it until a session start rebinds it. Its inode is readable for
    /// that; the new forms that outlive a restart have none to rebind by.
    #[cfg(unix)]
    #[test]
    fn a_record_in_the_older_form_is_compared_as_before() {
        use std::os::unix::fs::MetadataExt;
        let root = tempfile::tempdir().unwrap();
        let found = std::fs::metadata(root.path()).unwrap();
        let older = format!("{}:{}", found.dev(), found.ino());
        let restarted = format!("{}:{}", found.dev().wrapping_add(1), found.ino());
        let here = DirectoryId::at(root.path()).unwrap();
        assert!(here.is(&older) && path_is_recorded(root.path(), &older));
        assert!(!here.is(&restarted) && !path_is_recorded(root.path(), &restarted));
        assert!(here.same_inode(&restarted));
        assert_eq!(here.checked(), older, "what the shell's stat prints");
        assert_eq!(rebindable_inode(&restarted), Some(found.ino()));
        assert_eq!(rebindable_inode("dev:5:42"), Some(42));
        assert_eq!(rebindable_inode("mount:smbfs://ada@nas/work:42"), Some(42));
        assert_eq!(
            rebindable_inode("uuid:A082621D-5C1E-4C5F-B315-392E662A42C5:42"),
            None
        );
        assert_eq!(rebindable_inode("fsid:ext4:5269c8a99871e3b7:42"), None);
        assert_eq!(
            rebindable_inode("snapshot:A082621D-5C1E-4C5F-B315-392E662A42C5:42"),
            None
        );
    }

    /// A volume macOS gives no UUID (devfs here; a FAT disk whose serial
    /// is 0, some network shares) is named by its file system and what it
    /// was mounted from, never by the bare device number, and is the same
    /// on every open. That form is rebound by a start like an older one.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_volume_without_a_uuid_is_named_by_what_it_was_mounted_from() {
        use std::os::unix::fs::MetadataExt;
        let dev = Path::new("/dev");
        let named = path_identity(dev).unwrap();
        let inode = std::fs::metadata(dev).unwrap().ino();
        assert_eq!(named, format!("mount:devfs:devfs:{inode}"));
        assert_eq!(path_identity(dev), Some(named.clone()));
        assert_eq!(rebindable_inode(&named), Some(inode));
        assert!(!DirectoryId::at(dev).unwrap().keeps_inodes());
    }
}
