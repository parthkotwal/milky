//! Getting the text out of documents (DECISIONS 2026-10-16, document search,
//! beat D1).
//!
//! Plain text and code are read directly. PDFs go through PDFKit, the engine
//! Preview uses; Word, RTF, and OpenDocument through AppKit's document
//! readers, the machinery behind `textutil`. Data formats (`.json`, `.csv`,
//! `.html`) are left out: mostly datasets, three times the text of everything
//! else. At most [`MAX_TEXT_BYTES`] of a file's text is indexed.

use std::path::Path;

/// The most text indexed per file. Larger files are mostly generated (logs,
/// minified code); their first megabyte still makes them findable.
pub const MAX_TEXT_BYTES: usize = 1_000_000;

/// How a file's text is obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextKind {
    /// Read the file itself: text and code.
    Plain,
    /// PDFKit.
    Pdf,
    /// AppKit's document readers: Word, RTF, OpenDocument.
    Rich,
}

impl TextKind {
    /// Whether getting the text costs real time (tens of milliseconds to a
    /// second per file), so the result is worth caching. Plain files are
    /// re-read in microseconds and are not cached.
    pub fn is_expensive(self) -> bool {
        !matches!(self, TextKind::Plain)
    }

    /// A short name stored with cached text: `plain`, `pdf`, `rich`.
    pub fn name(self) -> &'static str {
        match self {
            TextKind::Plain => "plain",
            TextKind::Pdf => "pdf",
            TextKind::Rich => "rich",
        }
    }
}

/// Documents read as plain text.
const DOCUMENT_EXTENSIONS: &[&str] = &["txt", "text", "md", "markdown", "rst", "tex", "org", "adoc"];

/// Source code, read as plain text.
const CODE_EXTENSIONS: &[&str] = &[
    "py", "rs", "swift", "js", "mjs", "cjs", "ts", "tsx", "jsx", "java", "kt", "scala", "c", "h", "cc", "cpp",
    "hpp", "m", "mm", "go", "rb", "php", "cs", "lua", "r", "jl", "hs", "ml", "ex", "exs", "clj", "dart", "zig",
    "css", "scss", "sass", "less", "sql", "sh", "bash", "zsh", "fish", "ps1", "yaml", "yml", "toml", "ini", "cfg",
    "conf", "gradle", "cmake", "make", "mk", "proto", "graphql", "vue", "svelte", "ipynb",
];

const RICH_EXTENSIONS: &[&str] = &["doc", "docx", "rtf", "odt"];

/// How `path`'s text is obtained, or `None` if its contents are not indexed
/// (images, archives, data formats, files without an extension).
pub fn text_kind(path: &Path) -> Option<TextKind> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    let extension = extension.as_str();
    if DOCUMENT_EXTENSIONS.contains(&extension) || CODE_EXTENSIONS.contains(&extension) {
        Some(TextKind::Plain)
    } else if extension == "pdf" {
        Some(TextKind::Pdf)
    } else if RICH_EXTENSIONS.contains(&extension) {
        Some(TextKind::Rich)
    } else {
        None
    }
}

/// The text of `path`, extracted as `kind` says, capped at
/// [`MAX_TEXT_BYTES`] (cut at a character boundary). `None` if the file
/// cannot be read or holds no text (a scanned PDF: OCR, later).
pub fn extract(path: &Path, kind: TextKind) -> Option<String> {
    let text = match kind {
        TextKind::Plain => read_plain(path)?,
        TextKind::Pdf => apple::pdf_text(path)?,
        TextKind::Rich => apple::rich_text(path)?,
    };
    let text = cap(text);
    (!text.trim().is_empty()).then_some(text)
}

/// The first [`MAX_TEXT_BYTES`] of the file, as UTF-8 with invalid bytes
/// replaced (code and notes are almost always UTF-8; this never fails on a
/// stray byte).
fn read_plain(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path).ok()?.take(MAX_TEXT_BYTES as u64).read_to_end(&mut bytes).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn cap(mut text: String) -> String {
    if text.len() > MAX_TEXT_BYTES {
        let mut end = MAX_TEXT_BYTES;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}

#[cfg(target_os = "macos")]
mod apple {
    use std::path::Path;

    use objc2::AnyThread;
    use objc2::rc::autoreleasepool;
    use objc2_app_kit::NSAttributedStringDocumentFormats;
    use objc2_foundation::{NSAttributedString, NSDictionary, NSString, NSURL};
    use objc2_pdf_kit::PDFDocument;

    fn file_url(path: &Path) -> Option<objc2::rc::Retained<NSURL>> {
        Some(NSURL::fileURLWithPath(&NSString::from_str(path.to_str()?)))
    }

    /// PDFKit's text for the whole document. Runs inside an autorelease pool:
    /// a background thread has none of its own, and Apple frameworks hand back
    /// temporary objects that would otherwise never be freed.
    pub fn pdf_text(path: &Path) -> Option<String> {
        autoreleasepool(|_| {
            let url = file_url(path)?;
            // SAFETY: a valid file URL; PDFKit returns nil for anything it
            // cannot open, which becomes `None`.
            let document = unsafe { PDFDocument::initWithURL(PDFDocument::alloc(), &url) }?;
            // SAFETY: `document` is a live PDFDocument.
            Some(unsafe { document.string() }?.to_string())
        })
    }

    /// AppKit's reading of a Word, RTF, or OpenDocument file, as plain text.
    pub fn rich_text(path: &Path) -> Option<String> {
        autoreleasepool(|_| {
            let url = file_url(path)?;
            let options = NSDictionary::new();
            // SAFETY: a valid file URL and an empty options dictionary (let
            // AppKit detect the format); no document attributes requested.
            let text = unsafe {
                NSAttributedString::initWithURL_options_documentAttributes_error(
                    NSAttributedString::alloc(),
                    &url,
                    &options,
                    None,
                )
            }
            .ok()?;
            Some(text.string().to_string())
        })
    }
}

#[cfg(not(target_os = "macos"))]
mod apple {
    use std::path::Path;

    pub fn pdf_text(_path: &Path) -> Option<String> {
        None
    }

    pub fn rich_text(_path: &Path) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A fresh folder with `notes.txt`, plus Word, RTF, and PDF copies made
    /// by macOS's own converters (`textutil`, `cupsfilter`).
    fn samples(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("milky-extract-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let notes = dir.join("notes.txt");
        std::fs::write(&notes, "Milky test document.\nThe attention mechanism lets the decoder look back.\n").unwrap();
        for format in ["docx", "rtf", "odt"] {
            let status = std::process::Command::new("textutil")
                .args(["-convert", format, "-output"])
                .arg(dir.join(format!("notes.{format}")))
                .arg(&notes)
                .status()
                .unwrap();
            assert!(status.success(), "textutil {format}");
        }
        let pdf = std::process::Command::new("cupsfilter").arg(&notes).output().unwrap();
        assert!(pdf.status.success(), "cupsfilter");
        std::fs::write(dir.join("notes.pdf"), pdf.stdout).unwrap();
        dir
    }

    #[test]
    fn each_kind_of_document_gives_its_text() {
        let dir = samples("kinds");
        for name in ["notes.txt", "notes.docx", "notes.rtf", "notes.odt", "notes.pdf"] {
            let path = dir.join(name);
            let kind = text_kind(&path).unwrap();
            let text = extract(&path, kind).unwrap_or_else(|| panic!("no text from {name}"));
            assert!(text.contains("attention mechanism"), "{name}: {text:?}");
        }
    }

    #[test]
    fn extraction_works_off_the_main_thread() {
        let dir = samples("thread");
        let text = std::thread::spawn(move || extract(&dir.join("notes.pdf"), TextKind::Pdf)).join().unwrap();
        assert!(text.unwrap().contains("decoder"));
    }

    #[test]
    fn data_formats_images_and_bare_names_are_not_read() {
        for name in ["data.json", "table.csv", "page.html", "photo.png", "archive.zip", "Makefile"] {
            assert_eq!(text_kind(Path::new(name)), None, "{name}");
        }
        assert_eq!(text_kind(Path::new("Notes.MD")), Some(TextKind::Plain), "case does not matter");
        assert_eq!(text_kind(Path::new("main.rs")), Some(TextKind::Plain));
        assert_eq!(text_kind(Path::new("paper.pdf")), Some(TextKind::Pdf));
        assert!(TextKind::Pdf.is_expensive() && TextKind::Rich.is_expensive() && !TextKind::Plain.is_expensive());
    }

    #[test]
    fn large_files_are_capped_at_a_character_boundary() {
        let dir = std::env::temp_dir().join(format!("milky-extract-cap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("big.txt");
        std::fs::write(&path, "é".repeat(MAX_TEXT_BYTES)).unwrap();
        let text = extract(&path, TextKind::Plain).unwrap();
        assert!(text.len() <= MAX_TEXT_BYTES);
        assert!(text.chars().all(|c| c == 'é'), "no character cut in half");
    }

    #[test]
    fn missing_and_empty_files_have_no_text() {
        assert_eq!(extract(Path::new("/nonexistent/milky.pdf"), TextKind::Pdf), None);
        let dir = std::env::temp_dir().join(format!("milky-extract-empty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("empty.md"), "  \n").unwrap();
        assert_eq!(extract(&dir.join("empty.md"), TextKind::Plain), None);
    }
}
