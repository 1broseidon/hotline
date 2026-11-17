//! Word, Excel and PowerPoint files, known by their name and by what they
//! hold. The desk offers to hand one to the system, and the system picks the
//! program by the name, so a file is one of these only when its name and its
//! contents agree: a script or an archive named `.docx` is never handed over.
//!
//! Each kind is a ZIP whose entries say what it is: `[Content_Types].xml`
//! beside the kind's own folder (`word/`, `xl/` or `ppt/`). Only the names
//! in the ZIP's central directory are read; no entry is ever extracted,
//! which keeps clear of ZIP's extraction advisories (see `deny.toml`).
//!
//! The macro-enabled kinds (`.docm`, `.xlsm`, `.pptm`) are never one, by
//! their names, and neither is a package that carries macros
//! (`vbaProject.bin`) under a plain name.

use std::io::{Read, Seek, SeekFrom};

/// Each kind: the name's ending, the folder only that kind's package has,
/// and its type.
const KINDS: [(&str, &str, &str); 3] = [
    (
        "docx",
        "word/",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    ),
    (
        "xlsx",
        "xl/",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    ),
    (
        "pptx",
        "ppt/",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    ),
];

/// The first bytes of a ZIP that starts with its first entry, as every
/// Office file does.
const ZIP_START: &[u8; 4] = b"PK\x03\x04";

/// Whether `mime_type` is one of the three Office types.
pub fn is_office_type(mime_type: &str) -> bool {
    KINDS.iter().any(|(_, _, kind)| *kind == mime_type)
}

/// The Office type of a file called `name` with these contents, when the
/// name ends in `.docx`, `.xlsx` or `.pptx` and the file is a ZIP from its
/// first byte whose entries are that kind's; none otherwise.
pub fn office_type(name: &str, file: &mut (impl Read + Seek)) -> Option<&'static str> {
    let (_, extension) = name.rsplit_once('.')?;
    let extension = extension.to_ascii_lowercase();
    let (_, folder, mime_type) = KINDS.iter().find(|(kind, ..)| *kind == extension)?;
    let mut start = [0u8; 4];
    file.seek(SeekFrom::Start(0)).ok()?;
    file.read_exact(&mut start).ok()?;
    if &start != ZIP_START {
        return None;
    }
    // Reads the central directory and nothing past it.
    let archive = zip::ZipArchive::new(file).ok()?;
    if archive.offset() != 0 {
        return None;
    }
    let mut types = false;
    let mut parts = false;
    for entry in archive.file_names() {
        let entry = entry.ok()?;
        if entry
            .to_ascii_lowercase()
            .rsplit('/')
            .next()
            .is_some_and(|last| last == "vbaproject.bin")
        {
            return None;
        }
        types |= entry == "[Content_Types].xml";
        parts |= entry.starts_with(folder);
    }
    (types && parts).then_some(*mime_type)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};

    /// A ZIP holding these entries, each a few bytes, stored as they are.
    fn zip_of(entries: &[&str]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for entry in entries {
            writer.start_file(*entry, options).unwrap();
            writer.write_all(b"<x/>").unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    fn word() -> Vec<u8> {
        zip_of(&["[Content_Types].xml", "_rels/.rels", "word/document.xml"])
    }

    fn kind(name: &str, bytes: &[u8]) -> Option<&'static str> {
        office_type(name, &mut Cursor::new(bytes))
    }

    #[test]
    fn a_word_excel_or_powerpoint_file_is_one_by_its_name_and_its_entries() {
        assert_eq!(kind("brief.docx", &word()), Some(KINDS[0].2));
        assert_eq!(kind("BRIEF.DOCX", &word()), Some(KINDS[0].2));
        let sheet = zip_of(&["[Content_Types].xml", "xl/workbook.xml"]);
        assert_eq!(kind("q3.xlsx", &sheet), Some(KINDS[1].2));
        let deck = zip_of(&["[Content_Types].xml", "ppt/presentation.xml"]);
        assert_eq!(kind("pitch.pptx", &deck), Some(KINDS[2].2));
        assert!(is_office_type(KINDS[1].2));
        assert!(!is_office_type("application/zip"));
    }

    #[test]
    fn a_macro_enabled_kind_is_never_one() {
        // By its name, even with a plain package inside.
        assert_eq!(kind("brief.docm", &word()), None);
        assert_eq!(
            kind(
                "q3.xlsm",
                &zip_of(&["[Content_Types].xml", "xl/workbook.xml"])
            ),
            None
        );
        assert_eq!(
            kind(
                "pitch.pptm",
                &zip_of(&["[Content_Types].xml", "ppt/presentation.xml"])
            ),
            None
        );
        // And by its macros, under a plain name.
        let macros = zip_of(&[
            "[Content_Types].xml",
            "word/document.xml",
            "word/vbaProject.bin",
        ]);
        assert_eq!(kind("brief.docx", &macros), None);
    }

    #[test]
    fn a_zip_renamed_docx_is_not_one() {
        assert_eq!(
            kind("brief.docx", &zip_of(&["notes.txt", "photos/1.jpg"])),
            None
        );
        // A package of one kind named as another.
        assert_eq!(
            kind(
                "brief.docx",
                &zip_of(&["[Content_Types].xml", "xl/workbook.xml"])
            ),
            None
        );
        // The kind's folder without the package's list of types.
        assert_eq!(kind("brief.docx", &zip_of(&["word/document.xml"])), None);
    }

    #[test]
    fn a_docx_that_is_not_a_zip_is_not_one() {
        assert_eq!(kind("brief.docx", b"#!/bin/sh\necho hi\n"), None);
        assert_eq!(kind("brief.docx", b"PK\x03\x04 and then nothing"), None);
        assert_eq!(kind("brief.docx", b""), None);
        // A real package behind something else's first bytes.
        let mut prefixed = b"MZ\x90\x00".to_vec();
        prefixed.extend(word());
        assert_eq!(kind("brief.docx", &prefixed), None);
        // The right entries under a name that is not one of the three.
        assert_eq!(kind("brief.zip", &word()), None);
        assert_eq!(kind("docx", &word()), None);
    }
}
