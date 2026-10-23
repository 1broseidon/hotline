use hotline_core::paths::data_root;
use hotline_core::{account_setup_items, private_dir};
use std::ffi::OsStr;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use tauri_plugin_opener::OpenerExt;

const REFUSED: &str = "That is not part of an agent account's setup.";

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    File,
    Folder,
}

// The system picks the program by the name, so nothing but a document or a
// folder is ever opened, whatever a row names.
fn kind(open: &str) -> Option<Kind> {
    if open.ends_with('/') {
        return Some(Kind::Folder);
    }
    let extension = Path::new(open).extension()?;
    (extension.eq_ignore_ascii_case("md") || extension.eq_ignore_ascii_case("json"))
        .then_some(Kind::File)
}

fn create_item(item: &Path, kind: Kind) -> io::Result<()> {
    match kind {
        Kind::Folder => fs::create_dir(item),
        Kind::File => {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(item)?;
            if item
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
            {
                file.write_all(b"{}\n")?;
            }
            Ok(())
        }
    }
}

fn setup_item(root: &Path, path: &str) -> Result<PathBuf, String> {
    let refused = || REFUSED.to_string();
    let path = Path::new(path);
    let name = path
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(refused)?;
    let kind = account_setup_items()
        .into_iter()
        .find(|open| open.trim_end_matches('/') == name)
        .and_then(kind)
        .ok_or_else(refused)?;
    let folder = path.parent().ok_or_else(refused)?;
    let id = folder
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(refused)?;
    let account = private_dir::checked_account_dir(root, id).map_err(|_| refused())?;
    if dunce::canonicalize(folder).ok().as_deref() != Some(account.as_path()) {
        return Err(refused());
    }
    let item = account.join(name);
    if let Err(error) = create_item(&item, kind)
        && error.kind() != io::ErrorKind::AlreadyExists
    {
        return Err(format!("It could not be made: {error}"));
    }
    let canonical = dunce::canonicalize(&item).map_err(|_| refused())?;
    let right_kind = match kind {
        Kind::Folder => canonical.is_dir(),
        Kind::File => canonical.is_file(),
    };
    // A link to another file of the folder, such as the sign-in, is refused too.
    let right_name = canonical
        .file_name()
        .is_some_and(|found| found.eq_ignore_ascii_case(name));
    if canonical.parent() != Some(account.as_path()) || !right_kind || !right_name {
        return Err(refused());
    }
    Ok(canonical)
}

#[tauri::command]
pub fn open_account_setup(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let item = setup_item(&data_root(), &path)?;
    app.opener()
        .open_path(item.to_string_lossy(), None::<&str>)
        .map_err(|error| format!("It could not be opened: {error}"))
}

#[cfg(test)]
mod tests;
