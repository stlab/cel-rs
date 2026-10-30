//! `adam-fmt`'s core: reads one adam-lang `.adm2` file, formats it with
//! [`adam_lang::format_source`] — the same logic `adam-lsp`'s `textDocument/formatting` handler
//! uses, so `adam-fmt` and an editor's "Format Document" agree exactly — and writes the result
//! back in place, only touching the file when its contents actually change.
//!
//! # Example
//!
//! ```no_run
//! use std::path::Path;
//!
//! let path = Path::new("sheet.adm2");
//! match adam_fmt::format_file(path) {
//!     Ok(true) => println!("formatted"),
//!     Ok(false) => println!("already formatted"),
//!     Err(error) => eprint!("{}", adam_fmt::render_error(path, &error)),
//! }
//! ```

use std::io;
use std::path::Path;

use adam_lang::FormatSourceError;
use annotate_snippets::Renderer;

/// Describes an I/O failure or retains the source and diagnostic for refused formatting.
///
/// [`format_file`] stores the original source in [`Self::Format`] without changing the file.
///
/// # Examples
///
/// ```
/// use adam_fmt::{FormatFileError, render_error};
/// use std::{io, path::Path};
///
/// let error = FormatFileError::Io(io::Error::other("unavailable"));
/// assert_eq!(
///     render_error(Path::new("sheet.adm2"), &error),
///     "adam-fmt: sheet.adm2: unavailable\n"
/// );
/// ```
#[derive(Debug)]
pub enum FormatFileError {
    /// `path` could not be read, or (only after formatting succeeds and differs) written back.
    Io(io::Error),
    /// `path`'s contents could not be safely formatted.
    Format {
        /// `path`'s original contents, needed by [`render_error`] to render a source-anchored
        /// diagnostic.
        source: String,
        /// Why formatting was refused.
        error: FormatSourceError,
    },
}

/// Formats the adam-lang source file at `path` in place.
///
/// - Postcondition: on `Ok(true)`, `path`'s contents are replaced with
///   [`adam_lang::format_source`]'s output. On `Ok(false)`, `path` is left untouched because it
///   already matched that output exactly (no write performed).
/// - Complexity: for n input bytes and m output bytes, reads and compares O(n + m)
///   bytes and writes O(m) bytes when changed. Uses O(n + m) space, in addition to
///   [`adam_lang::format_source`]'s time and space.
///
/// # Errors
///
/// Returns [`FormatFileError::Io`] if `path` can't be read, or (only after formatting succeeds
/// and differs from the original) can't be written back. Returns [`FormatFileError::Format`] if
/// `path`'s contents can't be safely formatted (see [`adam_lang::format_source`]'s `# Errors`).
/// A read or format error leaves the file untouched; a write error may leave a partial write.
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
///
/// let path = Path::new("sheet.adm2");
/// let changed = adam_fmt::format_file(path)?;
/// assert!(!adam_fmt::format_file(path)?);
/// # Ok::<(), adam_fmt::FormatFileError>(())
/// ```
pub fn format_file(path: &Path) -> Result<bool, FormatFileError> {
    let source = std::fs::read_to_string(path).map_err(FormatFileError::Io)?;
    match adam_lang::format_source(&source) {
        Ok(formatted) if formatted == source => Ok(false),
        Ok(formatted) => {
            std::fs::write(path, formatted).map_err(FormatFileError::Io)?;
            Ok(true)
        }
        Err(error) => Err(FormatFileError::Format { source, error }),
    }
}

/// Renders `error` as a human-readable diagnostic naming `path`, using `rustc`-style source
/// snippets for a [`FormatFileError::Format`].
///
/// I/O diagnostics end with `\n`. Format diagnostics preserve the source renderer's
/// styling and line endings. Print the returned string directly with `eprint!`.
///
/// - Complexity: copies O(p) path bytes and returns O(r) rendered bytes. Additional
///   time and space follow the source diagnostic renderer for each format error.
///
/// # Examples
///
/// ```
/// use adam_fmt::{FormatFileError, render_error};
/// use std::{io, path::Path};
///
/// let error = FormatFileError::Io(io::Error::other("cannot read"));
/// let diagnostic = render_error(Path::new("sheet.adm2"), &error);
/// assert_eq!(diagnostic, "adam-fmt: sheet.adm2: cannot read\n");
/// ```
#[must_use]
pub fn render_error(path: &Path, error: &FormatFileError) -> String {
    match error {
        FormatFileError::Io(io_error) => format!("adam-fmt: {}: {io_error}\n", path.display()),
        FormatFileError::Format { source, error } => {
            let renderer = Renderer::styled();
            let filename = path.display().to_string();
            match error {
                FormatSourceError::Parse(parse_error) => {
                    parse_error.format_rustc_style(source, &filename, 1, &renderer)
                }
                FormatSourceError::Recovered(errors) => errors
                    .iter()
                    .map(|e| e.format_rustc_style(source, &filename, 1, &renderer))
                    .collect(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Classifies denied rewrites as I/O errors when the filesystem enforces read-only modes.
    #[cfg(unix)]
    #[test]
    fn format_file_classifies_write_permission_failures() {
        let directory = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
        let path = directory.path().join("readonly.adm2");
        std::fs::write(&path, "sheet   s{}").unwrap();
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&path, permissions).unwrap();
        if std::fs::OpenOptions::new().write(true).open(&path).is_ok() {
            return;
        }
        assert!(matches!(
            format_file(&path),
            Err(FormatFileError::Io(error)) if error.kind() == io::ErrorKind::PermissionDenied
        ));
    }

    /// Retains every recovered diagnostic and leaves malformed declarations untouched on disk.
    #[test]
    fn format_file_preserves_source_with_recovered_errors() {
        let directory = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
        let path = directory.path().join("recovered.adm2");
        let original = "sheet s {\ncell first broken;\nsource second broken;\n}";
        std::fs::write(&path, original).unwrap();
        assert!(matches!(
            format_file(&path),
            Err(FormatFileError::Format {
                source,
                error: FormatSourceError::Recovered(errors),
            }) if source == original && errors.len() == 2
        ));
        assert_eq!(std::fs::read_to_string(path).unwrap(), original);
    }

    /// Replaces unformatted source with canonical output and reports a change.
    #[test]
    fn format_file_rewrites_an_unformatted_file_and_reports_a_change() {
        let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
        let path = dir.path().join("sheet.adm2");
        std::fs::write(&path, "sheet   s{cell x:i32=1;}").unwrap();

        assert!(matches!(format_file(&path), Ok(true)));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "sheet s {\n    cell x: i32 = 1;\n}\n"
        );
    }

    /// Reports no change for canonical source and preserves its contents and modification time.
    #[test]
    fn format_file_leaves_an_already_formatted_file_untouched() {
        let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
        let path = dir.path().join("sheet.adm2");
        let formatted = "sheet s {\n    cell x: i32 = 1;\n}\n";
        std::fs::write(&path, formatted).unwrap();
        let modified = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(modified)
            .unwrap();
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();

        assert!(matches!(format_file(&path), Ok(false)));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), formatted);
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            modified
        );
    }

    /// Reports an I/O error for a missing path without creating the file.
    #[test]
    fn format_file_errors_on_a_missing_path() {
        let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
        let path = dir.path().join("does-not-exist.adm2");

        assert!(matches!(format_file(&path), Err(FormatFileError::Io(_))));
        assert!(!path.exists());
    }

    /// Retains unparsable source in the error and leaves the file untouched.
    #[test]
    fn format_file_errors_on_unparsable_source_without_writing() {
        let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
        let path = dir.path().join("sheet.adm2");
        std::fs::write(&path, "not a sheet at all").unwrap();

        assert!(matches!(
            format_file(&path),
            Err(FormatFileError::Format { source, .. }) if source == "not a sheet at all"
        ));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "not a sheet at all"
        );
    }

    /// Rejects non-UTF-8 input as an I/O error without changing its bytes.
    #[test]
    fn format_file_errors_on_non_utf8_source_without_writing() {
        let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
        let path = dir.path().join("sheet.adm2");
        std::fs::write(&path, [0xff]).unwrap();

        assert!(matches!(format_file(&path), Err(FormatFileError::Io(_))));
        assert_eq!(std::fs::read(&path).unwrap(), [0xff]);
    }

    /// Names the path and I/O failure and terminates the diagnostic with a newline.
    #[test]
    fn render_error_reports_the_path_for_an_io_error() {
        let error = FormatFileError::Io(io::Error::other("boom"));
        let rendered = render_error(Path::new("missing.adm2"), &error);
        assert!(rendered.contains("missing.adm2"));
        assert!(rendered.contains("boom"));
        assert!(rendered.ends_with('\n'));
    }

    /// Names the path and shows the rejected source in a parse diagnostic.
    #[test]
    fn render_error_reports_a_source_snippet_for_a_format_error() {
        let source = "not a sheet at all".to_string();
        let error = adam_lang::format_source(&source).unwrap_err();
        let rendered = render_error(
            Path::new("sheet.adm2"),
            &FormatFileError::Format { source, error },
        );
        assert!(rendered.contains("sheet.adm2"));
        assert!(rendered.contains("not a sheet at all"));
    }

    /// Includes each recovered diagnostic with its path and source.
    #[test]
    fn render_error_reports_a_source_snippet_for_each_recovered_error() {
        // Exercises FormatSourceError::Recovered specifically (as opposed to ::Parse, covered
        // above), since that's the variant whose `.collect()` a reviewer once mistakenly flagged
        // as failing to compile (`String` does implement `FromIterator<String>`).
        let source = "sheet s {\ncell x unknown_syntax;\ncell y unknown_syntax;\n}".to_string();
        let error = adam_lang::format_source(&source).unwrap_err();
        assert!(
            matches!(&error, adam_lang::FormatSourceError::Recovered(errors) if errors.len() == 2)
        );
        let rendered = render_error(
            Path::new("sheet.adm2"),
            &FormatFileError::Format { source, error },
        );
        assert!(rendered.contains("sheet.adm2"));
        assert!(rendered.contains("cell x unknown_syntax"));
        assert!(rendered.contains("cell y unknown_syntax"));
    }
}
