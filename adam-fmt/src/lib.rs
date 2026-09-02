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

/// Error returned by [`format_file`].
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
///
/// # Errors
///
/// Returns [`FormatFileError::Io`] if `path` can't be read, or (only after formatting succeeds
/// and differs from the original) can't be written back. Returns [`FormatFileError::Format`] if
/// `path`'s contents can't be safely formatted (see [`adam_lang::format_source`]'s `# Errors`).
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
/// - Postcondition: the returned string ends with `\n` and is ready to print directly (via
///   `eprint!`, not `eprintln!`).
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

    /// Creates a fresh, empty temp directory under `std::env::temp_dir()` for one test, named
    /// after `test_name` plus the current time for uniqueness across concurrent test threads.
    fn temp_dir(test_name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "adam-fmt-test-{test_name}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn format_file_rewrites_an_unformatted_file_and_reports_a_change() {
        let dir = temp_dir("rewrites");
        let path = dir.join("sheet.adm2");
        std::fs::write(&path, "sheet   s{cell x:i32=1;}").unwrap();

        assert!(matches!(format_file(&path), Ok(true)));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "sheet s {\n    cell x: i32 = 1;\n}\n"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn format_file_leaves_an_already_formatted_file_untouched() {
        let dir = temp_dir("already-formatted");
        let path = dir.join("sheet.adm2");
        let formatted = "sheet s {\n    cell x: i32 = 1;\n}\n";
        std::fs::write(&path, formatted).unwrap();

        assert!(matches!(format_file(&path), Ok(false)));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), formatted);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn format_file_errors_on_a_missing_path() {
        let dir = temp_dir("missing");
        let path = dir.join("does-not-exist.adm2");

        assert!(matches!(format_file(&path), Err(FormatFileError::Io(_))));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn format_file_errors_on_unparsable_source_without_writing() {
        let dir = temp_dir("unparsable");
        let path = dir.join("sheet.adm2");
        std::fs::write(&path, "not a sheet at all").unwrap();

        assert!(matches!(
            format_file(&path),
            Err(FormatFileError::Format { .. })
        ));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "not a sheet at all"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn render_error_reports_the_path_for_an_io_error() {
        let error = FormatFileError::Io(io::Error::other("boom"));
        let rendered = render_error(Path::new("missing.adm2"), &error);
        assert!(rendered.contains("missing.adm2"));
        assert!(rendered.contains("boom"));
        assert!(rendered.ends_with('\n'));
    }

    #[test]
    fn render_error_reports_a_source_snippet_for_a_format_error() {
        let source = "not a sheet at all".to_string();
        let error = adam_lang::format_source(&source).unwrap_err();
        let rendered = render_error(
            Path::new("sheet.adm2"),
            &FormatFileError::Format { source, error },
        );
        assert!(rendered.contains("sheet.adm2"));
    }

    #[test]
    fn render_error_reports_a_source_snippet_for_each_recovered_error() {
        // Exercises FormatSourceError::Recovered specifically (as opposed to ::Parse, covered
        // above), since that's the variant whose `.collect()` a reviewer once mistakenly flagged
        // as failing to compile (`String` does implement `FromIterator<String>`).
        let source = "sheet s { cell x unknown_syntax }".to_string();
        let error = adam_lang::format_source(&source).unwrap_err();
        assert!(matches!(error, adam_lang::FormatSourceError::Recovered(_)));
        let rendered = render_error(
            Path::new("sheet.adm2"),
            &FormatFileError::Format { source, error },
        );
        assert!(rendered.contains("sheet.adm2"));
    }
}
