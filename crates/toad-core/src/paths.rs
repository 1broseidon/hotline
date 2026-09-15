//! Where Toad keeps its state, spelled exactly the way the Bun main spells it.
//!
//! Every path here names a file the main process already reads and writes.
//! Byte-for-byte agreement on names is what lets a method move to Rust while
//! the files stay where they are.

use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// The data directory: `TOAD_DATA_DIR`, or the platform's application
/// support directory.
pub fn data_root() -> PathBuf {
    if let Ok(dir) = env::var("TOAD_DATA_DIR")
        && !dir.trim().is_empty()
    {
        return PathBuf::from(dir);
    }
    let home = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_default();
    if cfg!(target_os = "macos") {
        home.join("Library")
            .join("Application Support")
            .join("Toad")
    } else if cfg!(target_os = "windows") {
        env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData").join("Roaming"))
            .join("Toad")
    } else {
        env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local").join("share"))
            .join("toad")
    }
}

fn percent(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("%{byte:02X}")).collect()
}

/// A logical id as one portable filesystem component.
///
/// `%` is escaped first, making the encoding reversible without a prefix and
/// leaving the UUID-only names already on disk unchanged. The set is Windows'
/// forbidden filename set plus control bytes, trailing dots or spaces, and
/// device names; applying it on every OS makes moved data keep its names.
pub fn encode_file_component(value: &str) -> String {
    assert!(!value.is_empty(), "an empty value cannot name a file");
    let reserved = is_reserved_device_name(value);
    let mut encoded = String::new();
    for character in value.chars() {
        let forbidden = matches!(
            character,
            '%' | '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' | '\u{0}'
                ..='\u{1f}' | '\u{7f}'
        );
        if forbidden {
            encoded.push_str(&percent(character.to_string().as_bytes()));
        } else {
            encoded.push(character);
        }
    }
    let trimmed_len = encoded.trim_end_matches([' ', '.']).len();
    if trimmed_len < encoded.len() {
        let tail = percent(&encoded.as_bytes()[trimmed_len..]);
        encoded.truncate(trimmed_len);
        encoded.push_str(&tail);
    }
    if reserved {
        let first = encoded.chars().next().unwrap_or_default();
        encoded = format!("%{:02X}{}", first as u32, &encoded[first.len_utf8()..]);
    }
    encoded
}

fn is_reserved_device_name(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    let name = lower.split('.').next().unwrap_or("");
    let stem_is_whole =
        lower.len() == name.len() || lower.as_bytes().get(name.len()) == Some(&b'.');
    stem_is_whole
        && (matches!(name, "con" | "prn" | "aux" | "nul")
            || (name.len() == 4
                && (name.starts_with("com") || name.starts_with("lpt"))
                && name.as_bytes()[3].is_ascii_digit()
                && name.as_bytes()[3] != b'0'))
}

/// New managed names use the portable component. A path-safe raw predecessor
/// still wins when it already exists, so files written before this encoding
/// continue in place rather than splitting their history.
fn managed_path(directory: &Path, logical: &str, suffix: &str) -> PathBuf {
    let encoded = directory.join(format!("{}{suffix}", encode_file_component(logical)));
    if encoded == directory.join(format!("{logical}{suffix}")) {
        return encoded;
    }
    if !logical.contains(['/', '\\']) {
        let legacy = directory.join(format!("{logical}{suffix}"));
        if legacy.exists() && !encoded.exists() {
            return legacy;
        }
    }
    encoded
}

/// The room's own stream: the roster, the settings, and everything else the
/// room remembers. One file and no epochs — only a tape is shipped by segment,
/// so only a tape needs them. It replaces the previous Toad's `store.sqlite`
/// and `settings.json`, which the importer will read from a directory it is
/// given rather than from here.
pub fn room_path(root: &Path) -> PathBuf {
    root.join("room.jsonl")
}

/// The search index: FTS5 over every teammate's messages and chapters.
///
/// Not a source of truth. The main builds it from the transcripts and rebuilds
/// it whenever the two disagree, which is what makes it safe to delete.
pub fn index_path(root: &Path) -> PathBuf {
    root.join("index.sqlite")
}

/// The ACP registry's published catalogue, as last fetched. A cache and
/// nothing more: deleting it costs one fetch, and an empty one still leaves
/// the agents Toad was taught by hand.
pub fn acp_registry_path(root: &Path) -> PathBuf {
    root.join("cache").join("acp-registry.json")
}

/// The skills gateway: the operator's folder of skills, one directory each,
/// granted per teammate. Made on first use; empty until a skill is added.
pub fn skills_path(root: &Path) -> PathBuf {
    root.join("skills")
}

/// The absolute path of a command, looking on `PATH` and the directories a
/// packaged Mac app's GUI environment does not include.
///
/// Finder and a `.desktop` file spawn with `/usr/bin:/bin:/usr/sbin:/sbin`.
/// Homebrew, `/usr/local`, and `~/.local/bin` are where `docker` and `podman`
/// actually live, and a computer that cannot start because Toad was opened
/// from the dock is the bug the previous edition shipped.
pub fn resolve_command(name: &str) -> Option<PathBuf> {
    resolve_command_in(name, env::var_os("PATH").as_deref(), true)
}

/// Resolve `name` against an explicit `PATH` value. `extras` is the
/// packaged-Mac directories; tests that hand in a fake PATH turn them off
/// so a real runtime on this machine cannot leak into the probe.
pub fn resolve_command_in(name: &str, path: Option<&OsStr>, extras: bool) -> Option<PathBuf> {
    if name.contains(['/', '\\']) {
        let path = PathBuf::from(name);
        return path.is_file().then_some(path);
    }
    let mut dirs: Vec<PathBuf> = path
        .map(|path| env::split_paths(path).collect())
        .unwrap_or_default();
    if extras {
        for extra in extra_bin_dirs() {
            if !dirs.contains(&extra) {
                dirs.push(extra);
            }
        }
    }
    let extensions: Vec<String> = if cfg!(windows) {
        env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string())
            .split(';')
            .map(str::to_string)
            .collect()
    } else {
        vec![String::new()]
    };
    dirs.into_iter()
        .flat_map(|directory| {
            extensions
                .iter()
                .map(move |extension| directory.join(format!("{name}{extension}")))
        })
        .find(|candidate| candidate.is_file())
}

fn extra_bin_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/opt/homebrew/bin"),
    ];
    if let Some(home) = env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".local").join("bin"));
    }
    dirs
}

pub fn workspaces_dir(root: &Path) -> PathBuf {
    root.join("workspaces")
}

/// Where a teammate works when nobody named a directory for it.
pub fn default_workspace(root: &Path, persona_id: &str) -> PathBuf {
    managed_path(&workspaces_dir(root), persona_id, "")
}

pub fn transcripts_dir(root: &Path) -> PathBuf {
    root.join("transcripts")
}

/// The legacy flat transcript, which readers keep consulting as the epoch-1 segment.
pub fn transcript_path(root: &Path, persona_id: &str) -> PathBuf {
    managed_path(&transcripts_dir(root), persona_id, ".jsonl")
}

/// Directory of a teammate's epoch segments, a sibling of the flat file.
pub fn transcript_segments_dir(root: &Path, persona_id: &str) -> PathBuf {
    managed_path(&transcripts_dir(root), persona_id, "")
}

pub fn transcript_segment_path(root: &Path, persona_id: &str, epoch: u64) -> PathBuf {
    transcript_segments_dir(root, persona_id).join(format!("{epoch}.jsonl"))
}

/// One conversation between two teammates: its tape and its metadata.
pub fn threads_dir(root: &Path) -> PathBuf {
    root.join("threads")
}

/// Undoes `encode_file_component`, one escaped byte at a time.
///
/// Byte at a time and not sequence at a time because that is what the main
/// does: it encodes a character as its UTF-8 bytes and decodes each `%XX` with
/// `String.fromCharCode`, so a multi-byte character does not survive the round
/// trip. The names this is asked about are thread keys, whose participants are
/// ids with no escapable character in them, and matching the main matters more
/// here than being right on a name neither of them can produce.
pub fn decode_file_component(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = String::new();
    let mut index = 0;
    while index < bytes.len() {
        let escape = bytes.get(index) == Some(&b'%')
            && bytes[index + 1..]
                .iter()
                .take(2)
                .filter(|byte| byte.is_ascii_hexdigit())
                .count()
                == 2;
        if escape {
            let hex = &value[index + 1..index + 3];
            decoded.push(char::from(u8::from_str_radix(hex, 16).unwrap_or(b'?')));
            index += 3;
        } else {
            let character = value[index..].chars().next().unwrap_or_default();
            decoded.push(character);
            index += character.len_utf8();
        }
    }
    decoded
}

/// A participant id that can stand in a thread key. `~` separates the two, `/`
/// and `.` would let a key climb out of the threads directory.
fn safe_thread_id(id: &str) -> bool {
    !id.is_empty() && !id.contains(['~', '/', '.'])
}

/// The key naming the conversation between two teammates, whoever asked.
///
/// Sorted by UTF-16 code unit rather than by byte, because the main sorts with
/// `Array.prototype.sort` and a key the two processes spelled differently would
/// be two files for one conversation.
pub fn thread_key(a: &str, b: &str) -> Option<String> {
    if !safe_thread_id(a) || !safe_thread_id(b) {
        return None;
    }
    if a.encode_utf16().gt(b.encode_utf16()) {
        Some(format!("{b}~{a}"))
    } else {
        Some(format!("{a}~{b}"))
    }
}

/// Splits a key back into its two participants, refusing anything that does
/// not spell itself the same way again.
///
/// The first is the thread's `user` side and the second its `agent` side —
/// that is how the sidecar is written, here and in the previous Toad — so
/// this is also what decides which way round a pair's messages are stored.
pub fn thread_participants(key: &str) -> Option<(&str, &str)> {
    let (a, b) = key.split_once('~')?;
    if b.contains('~') || thread_key(a, b).as_deref() != Some(key) {
        return None;
    }
    Some((a, b))
}

pub fn thread_path(root: &Path, key: &str) -> Option<PathBuf> {
    thread_participants(key)?;
    Some(managed_path(&threads_dir(root), key, ".jsonl"))
}

pub fn thread_meta_path(root: &Path, key: &str) -> Option<PathBuf> {
    thread_participants(key)?;
    Some(managed_path(&threads_dir(root), key, ".json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_names_are_unchanged() {
        let id = "89bbac55-e7ac-4409-a709-115f33bb12f0";
        assert_eq!(encode_file_component(id), id);
    }

    #[test]
    fn a_thread_key_names_the_pair_the_same_way_whoever_asked() {
        assert_eq!(thread_key("bob", "ada").as_deref(), Some("ada~bob"));
        assert_eq!(thread_key("ada", "bob").as_deref(), Some("ada~bob"));
        // `~` separates, and `/` or `.` would let a key climb out of the
        // threads directory, so an id holding one is not an id.
        for bad in ["", "a~b", "a/b", "a.b"] {
            assert_eq!(thread_key(bad, "ada"), None);
            assert_eq!(thread_key("ada", bad), None);
        }
    }

    #[test]
    fn a_path_is_refused_for_anything_that_does_not_spell_itself_again() {
        let root = Path::new("/data");
        assert!(thread_path(root, "ada~bob").is_some());
        assert!(thread_meta_path(root, "ada~bob").is_some());
        for bad in ["bob~ada", "ada~bob~cal", "ada", "..~ada", "~ada"] {
            assert!(thread_path(root, bad).is_none(), "{bad}");
            assert!(thread_meta_path(root, bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn decoding_undoes_an_encoded_name_one_escaped_byte_at_a_time() {
        assert_eq!(decode_file_component("a%2Fb%3Ac"), "a/b:c");
        assert_eq!(decode_file_component("100%25"), "100%");
        assert_eq!(decode_file_component("%43ON"), "CON");
        // Not an escape: a `%` with nothing legible after it stands as itself.
        assert_eq!(decode_file_component("50%~ada"), "50%~ada");
        assert_eq!(decode_file_component("%zz"), "%zz");
        assert_eq!(decode_file_component("%2"), "%2");
    }

    #[test]
    fn forbidden_characters_and_trailing_dots_are_escaped_like_the_main_does() {
        assert_eq!(encode_file_component("a/b:c"), "a%2Fb%3Ac");
        assert_eq!(encode_file_component("100%"), "100%25");
        assert_eq!(encode_file_component("name. "), "name%2E%20");
        assert_eq!(encode_file_component("CON"), "%43ON");
        assert_eq!(encode_file_component("com1.txt"), "%63om1.txt");
        assert_eq!(encode_file_component("console"), "console");
    }
}
