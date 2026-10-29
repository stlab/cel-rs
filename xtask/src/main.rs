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

/// Runs `npm ci` then `npm run build` in `begin/` and `ez-adam/` to regenerate each
/// crate's vendored Spectrum Web Components bundle (`assets/swc.js`) from that
/// crate's own `package.json` and `js/spectrum-entry.js`.
///
/// - Complexity: npm dependency installation and bundling costs for both crates.
///
/// # Errors
/// Returns an error if a child cannot start in its crate directory or exits nonzero.
fn build_js() -> Result<(), Box<dyn std::error::Error>> {
    let root = project_root();
    let npm = std::ffi::OsStr::new(if cfg!(windows) { "npm.cmd" } else { "npm" });
    for crate_name in ["begin", "ez-adam"] {
        build_js_for(&root.join(crate_name), npm)?;
    }
    Ok(())
}

/// Runs `npm ci` then `npm run build` in `crate_dir` to regenerate that crate's
/// vendored Spectrum Web Components bundle (`crate_dir/assets/swc.js`) from
/// `crate_dir/package.json` and `crate_dir/js/spectrum-entry.js`.
///
/// Accepts the npm-compatible program to execute; runs build only after ci succeeds.
/// - Complexity: the child programs' installation and bundling costs, plus
///   O(p) path storage for p path bytes.
///
/// # Errors
/// Returns an error if the program cannot start in `crate_dir` or exits nonzero.
fn build_js_for(crate_dir: &Path, npm: &std::ffi::OsStr) -> Result<(), Box<dyn std::error::Error>> {
    let steps: [&[&str]; 2] = [&["ci"], &["run", "build"]];
    for args in steps {
        println!(
            "Running `npm {}` in {} ...",
            args.join(" "),
            crate_dir.display()
        );
        let status = std::process::Command::new(npm)
            .args(args)
            .current_dir(crate_dir)
            .status()?;
        if !status.success() {
            return Err(format!("npm {} failed with {status}", args.join(" ")).into());
        }
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
        Some("build-js") => {
            if let Err(e) = build_js() {
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
                "Usage: cargo xtask <fetch-assets|build-js|prepare-live-book-assets|prepare-live-slides-assets [destination]>"
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

    /// Runs dependency installation before bundling and stops at either failed process.
    #[cfg(unix)]
    #[test]
    fn js_build_orders_steps_and_propagates_process_failures() {
        use std::os::unix::fs::PermissionsExt;
        let root = fixture().canonicalize().unwrap();
        let program = root.join("fixture-npm");
        for (failing_step, expected) in [
            ("none", "ci\nrun build\n"),
            ("ci", "ci\n"),
            ("run build", "ci\nrun build\n"),
        ] {
            std::fs::write(&program, format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> commands\nif [ \"$*\" = '{failing_step}' ]; then exit 7; fi\n"
            )).unwrap();
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
            let result = build_js_for(&root, program.as_os_str());
            assert_eq!(result.is_ok(), failing_step == "none");
            assert_eq!(
                std::fs::read_to_string(root.join("commands")).unwrap(),
                expected
            );
            std::fs::remove_file(root.join("commands")).unwrap();
        }
        assert!(build_js_for(&root.join("missing"), program.as_os_str()).is_err());
        assert!(build_js_for(&root, root.join("missing-program").as_os_str()).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
