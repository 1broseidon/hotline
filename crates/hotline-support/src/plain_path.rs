//! A Windows path in the form people and programs write it.
//!
//! `std::fs::canonicalize` answers Windows' verbatim form, `\\?\C:\…` or
//! `\\?\UNC\server\share\…`, which no person types and many programs refuse.
//! `dunce::canonicalize` drops the prefix only where the plain spelling opens
//! the same file, so a very long path or one with a reserved name keeps it.
//! A path Hotline shows a person or a teammate is plain whatever it is, and a
//! path a teammate asks for is compared in the same plain form, so a path it
//! was shown is one it can ask for again. Opening still goes through the
//! canonical form; this only spells.

use std::path::{Path, PathBuf};

/// `path` without Windows' verbatim prefix: `\\?\C:\…` is `C:\…`, and
/// `\\?\UNC\server\share\…` is `\\server\share\…`. Any other text is
/// answered as it is, including a verbatim volume or device path, which has
/// no plain spelling. Takes the text, not a `Path`, so it is tested on every
/// platform; [`plain`] applies it only on Windows.
pub fn without_verbatim_prefix(path: &str) -> String {
    if let Some(share) = path.strip_prefix(r"\\?\UNC\") {
        let mut parts = share.split('\\');
        let server = parts.next().unwrap_or_default();
        let name = parts.next().unwrap_or_default();
        if server.is_empty() || name.is_empty() {
            return path.to_string();
        }
        return format!(r"\\{share}");
    }
    if let Some(disk) = path.strip_prefix(r"\\?\") {
        let bytes = disk.as_bytes();
        if bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && bytes[2] == b'\\'
        {
            return disk.to_string();
        }
    }
    path.to_string()
}

/// [`without_verbatim_prefix`] of `path` on Windows. Elsewhere `\\?\` is
/// only characters in a name, and `path` is answered as it is.
pub fn plain(path: &Path) -> PathBuf {
    if cfg!(windows)
        && let Some(text) = path.to_str()
    {
        return PathBuf::from(without_verbatim_prefix(text));
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A disk path and a share lose the prefix; every other spelling,
    /// verbatim or not, is left as it is.
    #[test]
    fn only_a_verbatim_disk_or_share_path_loses_its_prefix() {
        for (verbatim, plain) in [
            (r"\\?\C:\Users\ada\q3.csv", r"C:\Users\ada\q3.csv"),
            (r"\\?\d:\", r"d:\"),
            (
                r"\\?\UNC\server\share\work\x.txt",
                r"\\server\share\work\x.txt",
            ),
            (r"\\?\UNC\server\share", r"\\server\share"),
        ] {
            assert_eq!(without_verbatim_prefix(verbatim), plain, "{verbatim}");
        }
        for kept in [
            r"C:\Users\ada",
            r"\\server\share\x",
            r"\\?\Volume{0b1c2d3e-0000-0000-0000-100000000000}\x",
            r"\\?\GLOBALROOT\Device\HarddiskVolume1\x",
            r"\\?\C:",
            r"\\?\1:\x",
            r"\\?\UNC\server",
            r"\\?\UNC\\share\x",
            r"\\.\C:\x",
            r"\\.\pipe\name",
            "/Users/ada/q3.csv",
            r"relative\x",
            "",
        ] {
            assert_eq!(without_verbatim_prefix(kept), kept, "{kept}");
        }
    }

    /// Off Windows a name may begin with `\\?\`, and it is kept.
    #[cfg(not(windows))]
    #[test]
    fn elsewhere_a_path_is_kept_as_it_is() {
        let named = Path::new(r"\\?\C:\x");
        assert_eq!(plain(named), named);
    }

    #[cfg(windows)]
    #[test]
    fn on_windows_a_verbatim_disk_path_is_plain() {
        assert_eq!(plain(Path::new(r"\\?\C:\x")), Path::new(r"C:\x"));
    }
}
