//! Which volume an open macOS file descriptor is on, read from the
//! descriptor itself (`fgetattrlist` and `fstatfs`), so the answer is about
//! what was opened and never a second lookup of its path.
//!
//! `st_dev` is no name for a volume across a restart: macOS numbers volumes
//! as it mounts them, and the same disk came back as another number after a
//! reboot (16777232, then 16777229). The volume's UUID is kept on the disk
//! with it. APFS, HFS+, FAT and exFAT volumes all answer one for any
//! directory on them, not only their root; a FAT volume whose serial number
//! is 0, devfs and some network shares answer none.
use std::os::fd::{AsRawFd, BorrowedFd};

/// The volume a descriptor is on, as far as macOS says.
pub struct Volume {
    /// The UUID macOS gives the volume (`ATTR_VOL_UUID`), when it has one
    /// that is not all zeros.
    pub uuid: Option<[u8; 16]>,
    /// Whether the volume is mounted from a snapshot (`MNT_SNAPSHOT`): the
    /// sealed system volume at `/`, or an APFS snapshot of another volume,
    /// which answers that volume's UUID and its inode numbers.
    pub snapshot: bool,
    /// The file system's name (`f_fstypename`): `apfs`, `hfs`, `msdos`,
    /// `exfat`, `smbfs`, `nfs`, `devfs`.
    pub fs_type: String,
    /// What it was mounted from (`f_mntfromname`): a device such as
    /// `/dev/disk3s5`, or a server and share such as `//ada@nas/projects`.
    pub mounted_from: String,
}

/// What `fgetattrlist` writes for `ATTR_CMN_RETURNED_ATTRS` and
/// `ATTR_VOL_UUID`: the length, which attributes it returned, then the
/// UUID, each on a four-byte boundary, which `repr(C)` lays out with no
/// padding (4 + 20 + 16 bytes).
#[repr(C)]
struct UuidReply {
    length: u32,
    returned: libc::attribute_set_t,
    uuid: [u8; 16],
}

/// The volume the open `fd` is on, or `None` when `fstatfs` fails.
pub fn volume_of(fd: BorrowedFd<'_>) -> Option<Volume> {
    // SAFETY: an all-zero `statfs` is a valid value of this plain C
    // struct, filled in by the call below.
    let mut stats: libc::statfs = unsafe { std::mem::zeroed() };
    // SAFETY: `fd` is borrowed open for this call, and `stats` is a
    // writable struct of the type the call fills.
    if unsafe { libc::fstatfs(fd.as_raw_fd(), &raw mut stats) } != 0 {
        return None;
    }
    Some(Volume {
        uuid: uuid_of(fd),
        snapshot: stats.f_flags & libc::MNT_SNAPSHOT.cast_unsigned() != 0,
        fs_type: text(&stats.f_fstypename),
        mounted_from: text(&stats.f_mntfromname),
    })
}

/// The volume UUID `fd` is on. A call that fails is a volume without one:
/// a file system that does not answer volume attributes still has a name
/// to fall back on.
fn uuid_of(fd: BorrowedFd<'_>) -> Option<[u8; 16]> {
    let mut request = libc::attrlist {
        bitmapcount: libc::ATTR_BIT_MAP_COUNT,
        reserved: 0,
        commonattr: libc::ATTR_CMN_RETURNED_ATTRS,
        volattr: libc::ATTR_VOL_INFO | libc::ATTR_VOL_UUID,
        dirattr: 0,
        fileattr: 0,
        forkattr: 0,
    };
    // SAFETY: an all-zero `UuidReply` is a valid value of this plain C
    // struct, filled in by the call below.
    let mut reply: UuidReply = unsafe { std::mem::zeroed() };
    // SAFETY: `fd` is borrowed open for this call, `request` is the
    // `attrlist` the call reads, and `reply` is writable for the size given.
    let failed = unsafe {
        libc::fgetattrlist(
            fd.as_raw_fd(),
            (&raw mut request).cast(),
            (&raw mut reply).cast(),
            std::mem::size_of::<UuidReply>(),
            0,
        )
    } != 0;
    let whole = usize::try_from(reply.length).is_ok_and(|n| n >= std::mem::size_of::<UuidReply>());
    if failed || !whole || reply.returned.volattr & libc::ATTR_VOL_UUID == 0 {
        return None;
    }
    (reply.uuid != [0; 16]).then_some(reply.uuid)
}

fn text(field: &[libc::c_char]) -> String {
    let bytes: Vec<u8> = field
        .iter()
        .map(|&c| c.cast_unsigned())
        .take_while(|&b| b != 0)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsFd;

    /// An ordinary directory, not a volume's root, answers its volume's
    /// UUID, the same on every open; devfs answers none and says what it is.
    #[test]
    fn a_directory_answers_its_volumes_uuid_and_devfs_none() {
        let root = tempfile::tempdir().unwrap();
        let deeper = root.path().join("a/b");
        std::fs::create_dir_all(&deeper).unwrap();
        let open = |path: &std::path::Path| std::fs::File::open(path).unwrap();
        let here = volume_of(open(&deeper).as_fd()).unwrap();
        let again = volume_of(open(&deeper).as_fd()).unwrap();
        let parent = volume_of(open(root.path()).as_fd()).unwrap();
        assert!(here.uuid.is_some(), "{} has no UUID", here.fs_type);
        assert_eq!(here.uuid, again.uuid);
        assert_eq!(here.uuid, parent.uuid);
        assert!(!here.snapshot);
        let dev = volume_of(open(std::path::Path::new("/dev")).as_fd()).unwrap();
        assert_eq!((dev.uuid, dev.fs_type.as_str()), (None, "devfs"));
    }
}
