//! Task runner for the cel-rs workspace.
//!
//! Run tasks with `cargo xtask <task>`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

mod live_book_assets;

/// A single vendored asset entry from `begin/assets/versions.toml`.
#[derive(Deserialize)]
struct Asset {
    /// Pinned version string (informational only).
    version: String,
    /// Full URL to download the asset from.
    url: String,
    /// Destination filename within `begin/assets/`.
    file: String,
}

/// Returns the workspace root (one directory above the `xtask` manifest).
///
/// - Complexity: O(p) time and space for p manifest-path bytes.
fn project_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(1)
        .unwrap()
        .to_path_buf()
}

/// Downloads every asset listed in `begin/assets/versions.toml` into `begin/assets/`.
///
/// - Complexity: manifest parsing plus O(b + p) time and space for downloaded
///   asset bytes b and manifest/path bytes p, excluding network latency.
///
/// # Errors
/// Returns an error for unreadable/malformed manifests, unsafe filenames,
/// failed downloads or output writes. Failure can leave earlier assets updated.
fn fetch_assets() -> Result<(), Box<dyn std::error::Error>> {
    let root = project_root();
    fetch_assets_from(&root.join("begin/assets"))
}

/// Downloads the manifest's assets into the supplied assets directory.
///
/// - Complexity: manifest parsing plus O(b + p) time and space for downloaded
///   asset bytes b and manifest/path bytes p, excluding network latency.
///
/// # Errors
/// Returns an error for unreadable/malformed manifests, unsafe filenames,
/// failed downloads or output writes. Failure can leave earlier assets updated.
fn fetch_assets_from(assets_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let manifest_path = assets_dir.join("versions.toml");
    let manifest_str = std::fs::read_to_string(&manifest_path)?;
    let assets: HashMap<String, Asset> = toml::from_str(&manifest_str)?;

    for (name, asset) in &assets {
        // `asset.file` comes from the versions.toml config — reject any path that could
        // escape the assets directory.
        if asset.file.contains("..") || asset.file.contains('/') || asset.file.contains('\\') {
            return Err(format!("invalid asset filename in versions.toml: {}", asset.file).into());
        }
        let dest = assets_dir.join(&asset.file);
        println!("Fetching {name} v{} ...", asset.version);
        let body = ureq::get(&asset.url).call()?.into_body().read_to_vec()?;
        std::fs::write(&dest, &body)?;
        println!("  -> {} ({} bytes)", dest.display(), body.len());
    }

    Ok(())
}

/// Dispatches a named task and reports failures with a nonzero process status.
///
/// - Complexity: the selected task's IO and child-process costs plus argument bytes.
fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("fetch-assets") => {
            if let Err(e) = fetch_assets() {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        }
        Some("prepare-live-book-assets") => {
            if let Err(e) = live_book_assets::prepare_live_book_assets() {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        }
        Some("prepare-live-slides-assets") => {
            let destination = std::env::args_os().nth(2).map(PathBuf::from);
            if let Err(e) = live_book_assets::prepare_live_slides_assets(destination.as_deref()) {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        }
        _ => {
            eprintln!(
                "Usage: cargo xtask <fetch-assets|prepare-live-book-assets|prepare-live-slides-assets [destination]>"
            );
            std::process::exit(1);
        }
    }
}

/// Verifies file-based configuration and real child-process sequencing.
#[cfg(test)]
mod tests {
    use super::*;

    /// Creates a uniquely owned fixture under this workspace's ignored target directory.
    ///
    /// - Complexity: O(p) time and space for p fixture-path bytes.
    fn fixture() -> PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let parent = Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/test-fixtures");
        std::fs::create_dir_all(&parent).unwrap();
        let root = parent.join(format!(
            "xtask-command-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        root
    }

    /// Reports missing/malformed manifests and unsafe filenames before downloading.
    #[test]
    fn asset_fetch_rejects_invalid_local_inputs() {
        let root = fixture();
        assert!(fetch_assets_from(&root).is_err());
        std::fs::write(root.join("versions.toml"), "not valid toml").unwrap();
        assert!(fetch_assets_from(&root).is_err());
        for file in ["../escape", "folder/file", r"folder\file"] {
            std::fs::write(
                root.join("versions.toml"),
                format!("[asset]\nversion = '1'\nurl = 'https://unused.invalid'\nfile = '{file}'"),
            )
            .unwrap();
            let error = fetch_assets_from(&root).unwrap_err().to_string();
            assert!(error.contains("invalid asset filename"), "{error}");
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
