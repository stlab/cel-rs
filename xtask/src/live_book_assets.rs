//! `prepare-live-book-assets`: assembles everything `adam-lang-book`'s live examples need
//! before `mdbook build` runs — the per-example source manifest, the vendored Spectrum Web
//! Components bundle, and the compiled `adam-lang-book-live` wasm/js bundle — into
//! `adam-lang-book/book-src/theme/`, where mdBook's theme-directory mechanism copies it
//! verbatim into `book-dist/theme/` (see `adam-live-bootstrap.js` for how those files are
//! then fetched at runtime).
//!
//! This must run after both `cargo install --path adam-lang-book-preprocessor` (so `mdbook
//! build` can find the `mdbook-live-examples` binary on `PATH`) and `wasm-pack build --target
//! web --release` inside `adam-lang-book-live/` (so its `pkg/` output exists to copy from).

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

use adam_lang_book_live_config::NO_LIVE_MOUNT;

use crate::project_root;

/// Walks `adam-lang-book/book-src/examples/<chapter>/<name>.adm2`, building the
/// `{"<chapter>/<name>": "<source>"}` map the live-mount bootstrap script looks each example
/// up in by its mount div's `data-example` attribute.
///
/// - Postcondition: no key in the returned map is present in [`NO_LIVE_MOUNT`].
/// - Complexity: O(n) in the total size of every `.adm2` file under `examples_dir`.
///
/// # Errors
/// Returns `Err` if `examples_dir` (or a chapter subdirectory within it) can't be read, or if
/// an `.adm2` file can't be read as UTF-8.
fn build_manifest(
    examples_dir: &Path,
) -> Result<BTreeMap<String, String>, Box<dyn std::error::Error>> {
    let mut manifest = BTreeMap::new();

    for chapter_entry in fs::read_dir(examples_dir)? {
        let chapter_entry = chapter_entry?;
        let chapter_path = chapter_entry.path();
        if !chapter_path.is_dir() {
            continue;
        }
        let chapter_name = chapter_entry.file_name();
        let chapter_name = chapter_name.to_string_lossy();

        for file_entry in fs::read_dir(&chapter_path)? {
            let file_entry = file_entry?;
            let file_path = file_entry.path();
            if file_path.extension().and_then(|e| e.to_str()) != Some("adm2") {
                continue;
            }
            let stem = file_path
                .file_stem()
                .ok_or("example file has no stem")?
                .to_string_lossy();
            let key = format!("{chapter_name}/{stem}");
            if NO_LIVE_MOUNT.contains(&key.as_str()) {
                continue;
            }
            let source = fs::read_to_string(&file_path)?;
            manifest.insert(key, source);
        }
    }

    Ok(manifest)
}

/// Recursively copies every file and subdirectory under `src` into `dst`, creating `dst` (and
/// any nested destination directories) as needed.
///
/// Used to merge `adam-lang-book-live/pkg/`'s contents (the `.js`/`.wasm` bundle plus its
/// `snippets/` subdirectory, if wasm-bindgen generated one) directly into
/// `adam-lang-book/book-src/theme/`, rather than nesting a `pkg/` subdirectory inside it — the
/// bootstrap script expects `theme/adam_lang_book_live.js`, not `theme/pkg/adam_lang_book_live.js`.
///
/// - Postcondition: an existing file at a destination path is overwritten.
/// - Complexity: O(n) in the total size of every file under `src`.
///
/// # Errors
/// Returns `Err` if `src` cannot be read, or if `dst` (or any nested destination directory or
/// file) cannot be created or written to.
fn copy_dir_contents(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir_contents(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// Generates `adam-live-examples.json` and stages the Spectrum CSS/JS bundle and the compiled
/// `adam-lang-book-live` wasm/js output into `adam-lang-book/book-src/theme/`, so a subsequent
/// `mdbook build adam-lang-book` has everything the live examples need.
///
/// - Precondition: `wasm-pack build --target web --release` has already been run inside
///   `adam-lang-book-live/`, producing its `pkg/` output directory.
///
/// # Errors
/// Returns `Err` if the examples directory can't be walked, the manifest can't be serialized
/// or written, `begin/assets/swc.js`/`inspector.css` are missing, or `adam-lang-book-live/pkg/`
/// doesn't exist.
pub fn prepare_live_book_assets() -> Result<(), Box<dyn std::error::Error>> {
    let root = project_root();
    let examples_dir = root
        .join("adam-lang-book")
        .join("book-src")
        .join("examples");
    let theme_dir = root.join("adam-lang-book").join("book-src").join("theme");
    fs::create_dir_all(&theme_dir)?;

    println!(
        "Building live-example manifest from {} ...",
        examples_dir.display()
    );
    let manifest = build_manifest(&examples_dir)?;
    let manifest_path = theme_dir.join("adam-live-examples.json");
    let manifest_json = serde_json::to_string_pretty(&manifest)?;
    fs::write(&manifest_path, manifest_json)?;
    println!(
        "  -> {} ({} examples)",
        manifest_path.display(),
        manifest.len()
    );

    let begin_assets = root.join("begin").join("assets");
    for name in ["swc.js", "inspector.css"] {
        let from = begin_assets.join(name);
        let to = theme_dir.join(name);
        fs::copy(&from, &to)?;
        println!("Copied {} -> {}", from.display(), to.display());
    }

    let pkg_dir = root.join("adam-lang-book-live").join("pkg");
    if !pkg_dir.is_dir() {
        return Err(format!(
            "{} not found -- run `wasm-pack build --target web --release` in adam-lang-book-live/ first",
            pkg_dir.display()
        )
        .into());
    }
    copy_dir_contents(&pkg_dir, &theme_dir)?;
    println!("Copied {} -> {}", pkg_dir.display(), theme_dir.display());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_example(dir: &Path, chapter: &str, name: &str, source: &str) {
        let chapter_dir = dir.join(chapter);
        fs::create_dir_all(&chapter_dir).unwrap();
        fs::write(chapter_dir.join(format!("{name}.adm2")), source).unwrap();
    }

    #[test]
    fn build_manifest_maps_chapter_slash_name_to_source() {
        let tmp = std::env::temp_dir().join(format!("xtask-live-book-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        write_example(&tmp, "cells", "tuple_typed_cell", "cell x: Int = 1;");

        let manifest = build_manifest(&tmp).unwrap();

        assert_eq!(
            manifest.get("cells/tuple_typed_cell").map(String::as_str),
            Some("cell x: Int = 1;")
        );
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn build_manifest_skips_excluded_examples() {
        let tmp =
            std::env::temp_dir().join(format!("xtask-live-book-test-excl-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        write_example(&tmp, "expressions", "no_standard_library", "cell x = 1;");
        write_example(
            &tmp,
            "expressions",
            "initializer_sees_no_cells",
            "cell y = 2;",
        );

        let manifest = build_manifest(&tmp).unwrap();

        assert!(!manifest.contains_key("expressions/no_standard_library"));
        assert!(manifest.contains_key("expressions/initializer_sees_no_cells"));
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn copy_dir_contents_recreates_nested_structure() {
        let base =
            std::env::temp_dir().join(format!("xtask-live-book-copy-test-{}", std::process::id()));
        let src = base.join("src");
        let dst = base.join("dst");
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(src.join("snippets/abc123")).unwrap();
        fs::write(src.join("bundle.js"), "console.log(1);").unwrap();
        fs::write(src.join("snippets/abc123/inline.js"), "export {};").unwrap();

        copy_dir_contents(&src, &dst).unwrap();

        assert_eq!(
            fs::read_to_string(dst.join("bundle.js")).unwrap(),
            "console.log(1);"
        );
        assert_eq!(
            fs::read_to_string(dst.join("snippets/abc123/inline.js")).unwrap(),
            "export {};"
        );
        fs::remove_dir_all(&base).unwrap();
    }
}
