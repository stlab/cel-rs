//! File I/O operations: open, save, and export via rfd. See
//! `docs/superpowers/specs/2026-08-26-ez-adam-ui-design.md` §6.

use std::path::{Path, PathBuf};

use crate::model::document::Document;
use crate::persistence::{from_json, to_json};

/// Reads and deserializes `path` as an `ez-adam` native-format document.
///
/// # Errors
///
/// Returns a human-readable message if `path` cannot be read or its
/// contents aren't valid `ez-adam` JSON.
///
/// - Complexity: O(serialized bytes + document contents) expected time and space;
///   filesystem latency is unbounded.
///
/// # Examples
///
/// `read_document_file(Path::new("resize.ez-adam"))` loads and validates an editor document.
pub fn read_document_file(path: &Path) -> Result<Document, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    from_json(&text).map_err(|e| format!("failed to parse {}: {e}", path.display()))
}

/// Serializes `doc` and writes it to `path`.
///
/// # Errors
///
/// Returns a human-readable message if `doc` fails to serialize, or if
/// `path` cannot be written.
///
/// Invalid documents leave an existing destination unchanged.
///
/// - Complexity: O(serialized bytes + document contents) expected time and space;
///   filesystem latency is unbounded.
///
/// # Examples
///
/// `write_document_file(Path::new("resize.ez-adam"), &document)` saves a valid editor snapshot.
pub fn write_document_file(path: &Path, doc: &Document) -> Result<(), String> {
    let text = to_json(doc).map_err(|e| format!("failed to serialize document: {e}"))?;
    std::fs::write(path, text).map_err(|e| format!("failed to write {}: {e}", path.display()))
}

/// Writes already-generated `.adm2` text to `path`.
///
/// # Errors
///
/// Returns a human-readable message if `path` cannot be written.
///
/// # Examples
///
/// Pass the result of `generate_adm2(&document)` to
/// `write_adm2_file(Path::new("resize.adm2"), &source)`.
///
/// - Complexity: O(text.len()) write work; failures also format the path bytes.
pub fn write_adm2_file(path: &Path, text: &str) -> Result<(), String> {
    std::fs::write(path, text).map_err(|e| format!("failed to write {}: {e}", path.display()))
}

/// Opens the native "Open" dialog restricted to `ez-adam`'s native JSON
/// format, returning the picked path or `None` if cancelled.
///
/// - Complexity: awaits user interaction; no upper bound on wall-clock time.
///
/// # Examples
///
/// If `pick_open_path().await` returns a path, load it with `read_document_file`;
/// `None` leaves the current document selected.
pub async fn pick_open_path() -> Option<PathBuf> {
    let handle = rfd::AsyncFileDialog::new()
        .add_filter("ez-adam document", &["json"])
        .pick_file()
        .await?;
    Some(handle.path().to_path_buf())
}

/// Opens the native "Save" dialog for `ez-adam`'s native JSON format,
/// returning the picked path or `None` if cancelled.
///
/// - Complexity: awaits user interaction; no upper bound on wall-clock time.
///
/// # Examples
///
/// Pass a path returned by `pick_save_path().await` to `write_document_file`.
pub async fn pick_save_path() -> Option<PathBuf> {
    let handle = rfd::AsyncFileDialog::new()
        .add_filter("ez-adam document", &["json"])
        .save_file()
        .await?;
    Some(handle.path().to_path_buf())
}

/// Opens the native "Export" dialog for `.adm2` files, returning the
/// picked path or `None` if cancelled.
///
/// - Complexity: awaits user interaction; no upper bound on wall-clock time.
///
/// # Examples
///
/// After successful source generation, pass the path from `pick_export_path().await`
/// to `write_adm2_file`.
pub async fn pick_export_path() -> Option<PathBuf> {
    let handle = rfd::AsyncFileDialog::new()
        .add_filter("adm2", &["adm2"])
        .save_file()
        .await?;
    Some(handle.path().to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::cell::CellType;
    use crate::ops::cells::add_cell;

    /// Creates the build-tree fixture directory and returns a process-local path.
    ///
    /// - Complexity: O(constructed path bytes) time and space, plus directory creation.
    fn temp_path(name: &str) -> PathBuf {
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target");
        std::fs::create_dir_all(&directory).unwrap();
        directory.join(format!("{}-{name}", std::process::id()))
    }

    #[test]
    /// File persistence preserves a valid document through write and read.
    fn write_then_read_document_file_round_trips() {
        let mut doc = Document::new("demo");
        let _ = add_cell(&mut doc, "a", CellType::i64());
        let path = temp_path("ez_adam_file_io_test_round_trip.json");

        write_document_file(&path, &doc).unwrap();
        let read_back = read_document_file(&path).unwrap();

        std::fs::remove_file(&path).unwrap();
        assert_eq!(doc, read_back);
    }

    #[test]
    /// Reading a missing document returns an I/O diagnostic.
    fn read_document_file_missing_file_returns_err() {
        let path = temp_path("ez_adam_file_io_test_does_not_exist.json");
        let _ = std::fs::remove_file(&path);
        assert!(read_document_file(&path).is_err());
    }

    #[test]
    /// Text export writes the exact supplied Adam source.
    fn write_adm2_file_writes_the_given_text() {
        let path = temp_path("ez_adam_file_io_test_export.adm2");
        write_adm2_file(&path, "sheet s {}\n").unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(contents, "sheet s {}\n");
    }

    /// Invalid coordinates fail serialization before an existing file is overwritten.
    #[test]
    fn invalid_document_preserves_existing_destination() {
        let path = temp_path("ez_adam_invalid_save.json");
        std::fs::write(&path, "retained").unwrap();
        let mut doc = Document::new("invalid");
        let cell = add_cell(&mut doc, "a", CellType::i64());
        let _ = crate::ops::cells::add_cell_node(
            &mut doc,
            cell,
            crate::model::geometry::Point::new(f64::NAN, 0.0),
        );
        let result = write_document_file(&path, &doc);
        let contents = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert!(result.unwrap_err().contains("failed to serialize"));
        assert_eq!(contents, "retained");
    }
}
