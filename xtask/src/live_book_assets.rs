//! Stages shared live-example assets for mdBook and standalone slides.
//!
//! Both commands assemble the example-source manifest, Spectrum bundle,
//! inspector/graph assets and existing `adam-lang-book-live/pkg/` WASM/JS output.
//! The book command writes `adam-lang-book/book-src/theme/`; the slides command
//! accepts an output directory and defaults to `adam-slides/dist/theme`.
//! Neither command compiles the WASM bundle.
//!
//! ```text
//! cargo xtask prepare-live-book-assets
//! cargo xtask prepare-live-slides-assets staging/theme
//! ```

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
/// - Complexity: O(1 + e + b + p + k log(f + 1)) time and O(b + k + p) space
///   for e directory entries, b included source bytes, p total visited path
///   bytes, k included key bytes and f included files. Empty chapters and
///   excluded entries contribute traversal/path work.
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
/// - Complexity: O(1 + e + b + p) time and O(d + p) auxiliary space for e entries,
///   b copied bytes, depth d and total visited path bytes p, including root paths.
///   Empty directories also contribute entries and are recreated.
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
/// Requires the compiled `adam-lang-book-live/pkg/` input at runtime.
///
/// - Complexity: manifest construction, serialization and recursive copying costs;
///   space for the manifest, its serialized form and traversal paths.
///
/// # Errors
/// Returns `Err` if the examples directory can't be walked, the manifest can't be serialized
/// or written, `begin/assets/swc.js`/`inspector.css` are missing, or `adam-lang-book-live/pkg/`
/// doesn't exist.
///
/// # Examples
/// ```text
/// cargo xtask prepare-live-book-assets
/// ```
pub fn prepare_live_book_assets() -> Result<(), Box<dyn std::error::Error>> {
    let root = project_root();
    prepare_live_assets(&root, &root.join("adam-lang-book/book-src/theme"))
}

/// Stages the manifest and runtime assets for the standalone Adam slides.
///
/// Uses `destination` when provided, otherwise `adam-slides/dist/theme`.
///
/// - Complexity: manifest construction, serialization and recursive copying costs;
///   space for the manifest, its serialized form and traversal paths.
///
/// # Errors
/// Returns an error if a required input is missing or staging fails.
///
/// # Examples
/// ```text
/// cargo run -p xtask -- prepare-live-slides-assets
/// cargo run -p xtask -- prepare-live-slides-assets staging/theme
/// ```
pub fn prepare_live_slides_assets(
    destination: Option<&Path>,
) -> Result<(), Box<dyn std::error::Error>> {
    let root = project_root();
    match destination {
        Some(destination) => prepare_live_assets(&root, destination),
        None => prepare_live_assets(&root, &root.join("adam-slides/dist/theme")),
    }
}

/// Stages the live-example manifest and runtime assets into `destination`.
///
/// Checks required top-level files and constructs the complete manifest before
/// writing output. Later IO errors can leave partial output.
/// - Complexity: manifest construction, serialization and recursive copying costs;
///   space for the manifest, its serialized form and traversal paths.
///
/// # Errors
/// Returns an error for missing or unreadable inputs, serialization failure,
/// or a destination that cannot be created or written.
fn prepare_live_assets(root: &Path, destination: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let examples_dir = root
        .join("adam-lang-book")
        .join("book-src")
        .join("examples");
    let begin_assets = root.join("begin").join("assets");
    let asset_names = [
        "swc.js",
        "inspector.css",
        "graph.js",
        "graph.css",
        "d3.v7.min.js",
    ];
    for name in asset_names {
        if !begin_assets.join(name).is_file() {
            return Err(format!(
                "Required live asset missing: {}",
                begin_assets.join(name).display()
            )
            .into());
        }
    }
    let pkg_dir = root.join("adam-lang-book-live").join("pkg");
    for name in ["adam_lang_book_live.js", "adam_lang_book_live_bg.wasm"] {
        if !pkg_dir.join(name).is_file() {
            return Err(format!(
                "{} not found -- run `wasm-pack build --target web --release` in adam-lang-book-live/ first",
                pkg_dir.join(name).display()
            ).into());
        }
    }

    println!(
        "Building live-example manifest from {} ...",
        examples_dir.display()
    );
    let manifest = build_manifest(&examples_dir)?;
    let manifest_path = destination.join("adam-live-examples.json");
    let manifest_json = serde_json::to_string_pretty(&manifest)?;
    fs::create_dir_all(destination)?;
    fs::write(&manifest_path, manifest_json)?;
    println!(
        "  -> {} ({} examples)",
        manifest_path.display(),
        manifest.len()
    );

    for name in asset_names {
        let from = begin_assets.join(name);
        let to = destination.join(name);
        fs::copy(&from, &to)?;
        println!("Copied {} -> {}", from.display(), to.display());
    }

    copy_dir_contents(&pkg_dir, destination)?;
    println!("Copied {} -> {}", pkg_dir.display(), destination.display());

    Ok(())
}

/// Verifies source manifests, recursive copying and independent asset staging.
#[cfg(test)]
mod tests {
    use super::*;

    /// Creates an owned staging fixture with all required inputs.
    ///
    /// - Complexity: O(n + p) time and space for fixture bytes n and path bytes p.
    fn staging_fixture() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/test-fixtures");
        fs::create_dir_all(&fixtures).unwrap();
        let root = fixtures.join(format!(
            "adam-slides-staging-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&root).unwrap();
        let examples = root.join("adam-lang-book/book-src/examples");
        write_example(&examples, "tutorial", "first_sheet", "sheet hello {}");
        write_example(&examples, "expressions", "no_standard_library", "excluded");
        let assets = root.join("begin/assets");
        fs::create_dir_all(&assets).unwrap();
        for name in [
            "swc.js",
            "inspector.css",
            "graph.js",
            "graph.css",
            "d3.v7.min.js",
        ] {
            fs::write(assets.join(name), name).unwrap();
        }
        let pkg = root.join("adam-lang-book-live/pkg");
        fs::create_dir_all(pkg.join("snippets/example")).unwrap();
        fs::write(pkg.join("adam_lang_book_live.js"), "export {};").unwrap();
        fs::write(
            pkg.join("adam_lang_book_live_bg.wasm"),
            [0, 97, 115, 109, 255],
        )
        .unwrap();
        fs::write(
            pkg.join("snippets/example/inline.js"),
            "export const x = 1;",
        )
        .unwrap();
        root
    }

    /// Verifies the manifest and runtime assets reach an independent destination.
    #[test]
    fn prepare_live_assets_stages_manifest_and_runtime() {
        let root = staging_fixture();
        let destination = root.join("slides/theme");
        prepare_live_assets(&root, &destination).unwrap();
        let manifest: BTreeMap<String, String> = serde_json::from_str(
            &fs::read_to_string(destination.join("adam-live-examples.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest.len(), 1);
        assert_eq!(manifest["tutorial/first_sheet"], "sheet hello {}");
        for name in [
            "swc.js",
            "inspector.css",
            "graph.js",
            "graph.css",
            "d3.v7.min.js",
        ] {
            assert_eq!(fs::read_to_string(destination.join(name)).unwrap(), name);
        }
        assert_eq!(
            fs::read(destination.join("adam_lang_book_live_bg.wasm")).unwrap(),
            [0, 97, 115, 109, 255]
        );
        assert_eq!(
            fs::read_to_string(destination.join("snippets/example/inline.js")).unwrap(),
            "export const x = 1;"
        );
        fs::remove_dir_all(root).unwrap();
    }

    /// Verifies missing required inputs fail before replacing existing output.
    #[test]
    fn prepare_live_assets_rejects_missing_inputs_even_with_stale_output() {
        for missing in [
            "adam-lang-book/book-src/examples",
            "begin/assets/swc.js",
            "adam-lang-book-live/pkg",
        ] {
            let root = staging_fixture();
            fs::rename(root.join(missing), root.join("missing-input")).unwrap();
            let destination = root.join("slides/theme");
            fs::create_dir_all(&destination).unwrap();
            fs::write(destination.join("adam-live-examples.json"), "stale").unwrap();
            assert!(
                prepare_live_assets(&root, &destination).is_err(),
                "{missing}"
            );
            assert_eq!(
                fs::read_to_string(destination.join("adam-live-examples.json")).unwrap(),
                "stale"
            );
            fs::remove_dir_all(root).unwrap();
        }
    }

    /// Rejects invalid UTF-8 before replacing an existing manifest.
    #[test]
    fn prepare_live_assets_rejects_invalid_utf8_without_writing() {
        let root = staging_fixture();
        fs::write(
            root.join("adam-lang-book/book-src/examples/tutorial/first_sheet.adm2"),
            [0xff],
        )
        .unwrap();
        let destination = root.join("output");
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("adam-live-examples.json"), "original").unwrap();
        assert!(prepare_live_assets(&root, &destination).is_err());
        assert_eq!(
            fs::read_to_string(destination.join("adam-live-examples.json")).unwrap(),
            "original"
        );
        fs::remove_dir_all(root).unwrap();
    }

    /// Reports destination file and nested-directory conflicts as staging errors.
    #[test]
    fn prepare_live_assets_reports_destination_conflicts() {
        for conflict in ["", "adam-live-examples.json", "swc.js", "snippets"] {
            let root = staging_fixture();
            let destination = root.join("output");
            if conflict.is_empty() {
                fs::write(&destination, "not a directory").unwrap();
            } else {
                fs::create_dir(&destination).unwrap();
                if conflict == "snippets" {
                    fs::write(destination.join(conflict), "not a directory").unwrap();
                } else {
                    fs::create_dir(destination.join(conflict)).unwrap();
                }
            }
            assert!(
                prepare_live_assets(&root, &destination).is_err(),
                "{conflict}"
            );
            fs::remove_dir_all(root).unwrap();
        }
    }

    /// Replaces stale manifest and asset contents with current input bytes.
    #[test]
    fn prepare_live_assets_overwrites_previous_output() {
        let root = staging_fixture();
        let destination = root.join("output");
        prepare_live_assets(&root, &destination).unwrap();
        fs::write(root.join("begin/assets/swc.js"), "updated bundle").unwrap();
        write_example(
            &root.join("adam-lang-book/book-src/examples"),
            "tutorial",
            "first_sheet",
            "sheet updated {}",
        );
        prepare_live_assets(&root, &destination).unwrap();
        assert_eq!(
            fs::read_to_string(destination.join("swc.js")).unwrap(),
            "updated bundle"
        );
        let manifest: BTreeMap<String, String> = serde_json::from_str(
            &fs::read_to_string(destination.join("adam-live-examples.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["tutorial/first_sheet"], "sheet updated {}");
        fs::remove_dir_all(root).unwrap();
    }

    /// Ignores non-example entries and empty chapters while rejecting source IO failures.
    #[test]
    fn manifest_handles_empty_chapters_invalid_utf8_and_unreadable_sources() {
        let root = staging_fixture();
        let examples = root.join("examples");
        fs::create_dir_all(examples.join("empty")).unwrap();
        fs::write(examples.join("not-a-chapter"), "ignored").unwrap();
        fs::write(examples.join("empty/notes.txt"), "ignored").unwrap();
        assert!(build_manifest(&examples).unwrap().is_empty());
        assert!(build_manifest(&root.join("missing")).is_err());
        fs::write(examples.join("empty/invalid.adm2"), [0xff]).unwrap();
        assert!(build_manifest(&examples).is_err());
        fs::remove_file(examples.join("empty/invalid.adm2")).unwrap();
        fs::create_dir(examples.join("empty/directory.adm2")).unwrap();
        assert!(build_manifest(&examples).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    /// Copies empty directories, overwrites files and reports source/destination IO failures.
    #[test]
    fn copy_handles_empty_directories_overwrite_and_conflicts() {
        let root = staging_fixture();
        let source = root.join("copy-source");
        let destination = root.join("copy-destination");
        fs::create_dir_all(source.join("empty")).unwrap();
        fs::write(source.join("file"), [1, 2, 255]).unwrap();
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("file"), "stale").unwrap();
        copy_dir_contents(&source, &destination).unwrap();
        assert!(destination.join("empty").is_dir());
        assert_eq!(fs::read(destination.join("file")).unwrap(), [1, 2, 255]);
        fs::write(destination.join("file"), "destination change").unwrap();
        assert_eq!(fs::read(source.join("file")).unwrap(), [1, 2, 255]);
        fs::write(source.join("file"), "source change").unwrap();
        assert_eq!(
            fs::read(destination.join("file")).unwrap(),
            b"destination change"
        );
        assert!(copy_dir_contents(&root.join("missing"), &destination).is_err());
        assert!(copy_dir_contents(&source.join("file"), &destination).is_err());
        fs::remove_file(destination.join("file")).unwrap();
        fs::create_dir(destination.join("file")).unwrap();
        assert!(copy_dir_contents(&source, &destination).is_err());
        assert!(copy_dir_contents(&source, &source.join("file")).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    /// Writes one UTF-8 example into its chapter directory.
    ///
    /// - Complexity: O(p + n) time and O(p) space for p path and n source bytes.
    fn write_example(dir: &Path, chapter: &str, name: &str, source: &str) {
        let chapter_dir = dir.join(chapter);
        fs::create_dir_all(&chapter_dir).unwrap();
        fs::write(chapter_dir.join(format!("{name}.adm2")), source).unwrap();
    }

    /// Maps a chapter and file stem to the complete example source.
    #[test]
    fn build_manifest_maps_chapter_slash_name_to_source() {
        let tmp = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../target/test-fixtures")
            .join(format!("xtask-live-book-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        write_example(&tmp, "cells", "tuple_typed_cell", "cell x: Int = 1;");

        let manifest = build_manifest(&tmp).unwrap();

        assert_eq!(
            manifest.get("cells/tuple_typed_cell").map(String::as_str),
            Some("cell x: Int = 1;")
        );
        fs::remove_dir_all(&tmp).unwrap();
    }

    /// Omits excluded configurations while retaining neighboring examples.
    #[test]
    fn build_manifest_skips_excluded_examples() {
        let tmp = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../target/test-fixtures")
            .join(format!("xtask-live-book-test-excl-{}", std::process::id()));
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

    /// Recreates nested directories and preserves every copied file's bytes.
    #[test]
    fn copy_dir_contents_recreates_nested_structure() {
        let base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../target/test-fixtures")
            .join(format!("xtask-live-book-copy-test-{}", std::process::id()));
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
