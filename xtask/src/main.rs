//! Task runner for the cel-rs workspace.
//!
//! Run tasks with `cargo xtask <task>`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

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
fn project_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(1)
        .unwrap()
        .to_path_buf()
}

/// Downloads every asset listed in `begin/assets/versions.toml` into `begin/assets/`.
fn fetch_assets() -> Result<(), Box<dyn std::error::Error>> {
    let root = project_root();
    let manifest_path = root.join("begin").join("assets").join("versions.toml");
    let manifest_str = std::fs::read_to_string(&manifest_path)?;
    let assets: HashMap<String, Asset> = toml::from_str(&manifest_str)?;

    let assets_dir = root.join("begin").join("assets");

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

/// Runs `npm ci` then `npm run build` in `begin/` to regenerate the vendored
/// Spectrum Web Components bundle (`begin/assets/swc.js`) from `begin/package.json`
/// and `begin/js/spectrum-entry.js`.
///
/// # Errors
/// Returns `Err` if `npm` is not on `PATH`, or if either command exits non-zero.
fn build_js() -> Result<(), Box<dyn std::error::Error>> {
    let root = project_root();
    let begin_dir = root.join("begin");
    let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };

    let steps: [&[&str]; 2] = [&["ci"], &["run", "build"]];
    for args in steps {
        println!(
            "Running `npm {}` in {} ...",
            args.join(" "),
            begin_dir.display()
        );
        let status = std::process::Command::new(npm)
            .args(args)
            .current_dir(&begin_dir)
            .status()?;
        if !status.success() {
            return Err(format!("npm {} failed with {status}", args.join(" ")).into());
        }
    }

    Ok(())
}

/// Entry point: dispatches to the named task.
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
        _ => {
            eprintln!("Usage: cargo xtask <fetch-assets|build-js>");
            std::process::exit(1);
        }
    }
}
