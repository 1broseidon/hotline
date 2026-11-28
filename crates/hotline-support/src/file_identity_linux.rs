//! Which file system an open Linux file descriptor is on, read from the
//! descriptor itself (`fstatfs`), so the answer is about what was opened and
//! never a second lookup of its path.
//!
//! `st_dev` is no name for a file system across a restart wherever the
//! kernel hands out the number as it goes: a loop device's minor (an ext4
//! image remounted went from 7:0 to 7:2), a device-mapper volume's, and
//! every btrfs subvolume's anonymous `0:N` (0:62 to 0:63 on a remount).
//! Only ext4 is named here: it folds its UUID into `f_fsid`, which stayed
//! the same across those remounts on Linux 6.18 and 7.2. btrfs mixes the
//! device into `f_fsid` (it changed when its loop device did, on 7.2); xfs
//! derives it from the device number (`0x701` on 7:1), procfs too, and
//! overlayfs and tmpfs from an id made at mount; NFS and FUSE leave it 0.

use std::os::fd::{AsRawFd, BorrowedFd};

/// A file system whose `f_fsid` is kept on disk with it.
pub struct Stable {
    /// `ext4`.
    pub fs_type: &'static str,
    /// Its `f_fsid`, the two words as one number.
    pub fsid: u64,
}

/// The stable id of the file system the open `fd` is on, or `None` when it
/// has none or `fstatfs` fails.
pub fn stable_of(fd: BorrowedFd<'_>) -> Option<Stable> {
    // SAFETY: an all-zero `statfs` is a valid value of this plain C
    // struct, filled in by the call below.
    let mut stats: libc::statfs = unsafe { std::mem::zeroed() };
    // SAFETY: `fd` is borrowed open for this call, and `stats` is a
    // writable struct of the type the call fills.
    if unsafe { libc::fstatfs(fd.as_raw_fd(), &raw mut stats) } != 0 {
        return None;
    }
    if stats.f_type != libc::EXT4_SUPER_MAGIC {
        return None;
    }
    // SAFETY: `fsid_t` is two C ints whose fields libc keeps private; the
    // transmute reads the same eight bytes as that array.
    let words = unsafe { std::mem::transmute::<libc::fsid_t, [libc::c_int; 2]>(stats.f_fsid) };
    let fsid = (u64::from(words[0].cast_unsigned()) << 32) | u64::from(words[1].cast_unsigned());
    (fsid != 0).then_some(Stable {
        fs_type: "ext4",
        fsid,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsFd;

    /// The same directory answers the same id on two opens, and a file
    /// system without a kept id (procfs) answers none.
    #[test]
    fn the_same_directory_answers_the_same_id_and_procfs_none() {
        let root = tempfile::tempdir().unwrap();
        let open = |path: &std::path::Path| std::fs::File::open(path).unwrap();
        let first = stable_of(open(root.path()).as_fd());
        let second = stable_of(open(root.path()).as_fd());
        assert_eq!(
            first.as_ref().map(|found| (found.fs_type, found.fsid)),
            second.as_ref().map(|found| (found.fs_type, found.fsid))
        );
        assert!(stable_of(open(std::path::Path::new("/proc")).as_fd()).is_none());
    }
}
