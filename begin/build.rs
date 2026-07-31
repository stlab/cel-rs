//! Generates a compile-time manifest of `assets/*.adm2` demo files.
//!
//! `begin` lets the user pick between several example property models at
//! runtime (see `src/demo_source.rs`). The web build has no filesystem to
//! scan at runtime - `asset!()`-bundled files must be named as string
//! literals at compile time - so instead this script scans `assets/` once
//! and writes out the equivalent literal Rust source to
//! `$OUT_DIR/demo_manifest.rs`, which `demo_source.rs` splices in via
//! `include!`.
//!
//! Deliberately watches only `build.rs` itself (`cargo:rerun-if-changed=
//! build.rs` below), not the `assets/` directory: watching the directory
//! would make Cargo treat every `.adm2` file as a build-script input, so
//! editing an *existing* demo's content - the common case, and the one
//! `dx serve`'s asset-based hot reload (see `spawn_hot_reload`) exists to
//! handle without a rebuild - would force a full crate rebuild on every
//! edit. The trade-off: *adding or removing* a demo file requires a manual
//! nudge (e.g. `touch build.rs`, or just restart `dx serve`) before it shows
//! up - there's no Cargo primitive for "watch a directory's file list but
//! ignore existing files' content."
use std::env;
use std::fs;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("set by cargo");
    let assets_dir = Path::new(&manifest_dir).join("assets");

    let mut names: Vec<String> = fs::read_dir(&assets_dir)
        .expect("assets/ directory must exist")
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("adm2") {
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_string())
            } else {
                None
            }
        })
        .collect();
    names.sort();

    let mut out = String::new();

    out.push_str("/// Every `assets/*.adm2` demo file discovered at build time, by name,\n");
    out.push_str("/// sorted. Available on every platform: used to build the demo picker\n");
    out.push_str("/// UI. Doesn't carry each demo's source text - see `DEMOS_WITH_SOURCE`\n");
    out.push_str("/// for that (needed only on non-desktop builds; desktop reads content\n");
    out.push_str("/// live from disk instead, see `load_demo_source`).\n");
    out.push_str("pub static DEMO_NAMES: &[&str] = &[\n");
    for name in &names {
        out.push_str(&format!("    {name:?},\n"));
    }
    out.push_str("];\n\n");

    out.push_str("/// `(name, embedded source)` pairs for every demo. Gated out of ordinary\n");
    out.push_str("/// desktop builds: embedding each file's content via `include_str!` would\n");
    out.push_str("/// register it as a compile-time dependency of this crate, and `dx serve`\n");
    out.push_str("/// would then treat any edit to an existing demo's *content* as requiring\n");
    out.push_str("/// a full rebuild - defeating this file's own asset-based hot reload (see\n");
    out.push_str("/// `spawn_hot_reload`). Needed on non-desktop builds (no live filesystem to\n");
    out.push_str("/// read from at runtime) and in tests (a fixture that doesn't depend on\n");
    out.push_str("/// desktop asset bundling being available).\n");
    out.push_str("#[cfg(any(not(feature = \"desktop\"), test))]\n");
    out.push_str("pub static DEMOS_WITH_SOURCE: &[(&str, &str)] = &[\n");
    for name in &names {
        let abs_path = assets_dir.join(format!("{name}.adm2"));
        let abs_path_str = abs_path.display().to_string();
        out.push_str(&format!(
            "    ({name:?}, include_str!({abs_path_str:?})),\n"
        ));
    }
    out.push_str("];\n\n");

    out.push_str("/// One `asset!()` registration per demo file, so `dx`'s bundler tracks\n");
    out.push_str("/// and reports changes to all of them, not just whichever is active.\n");
    out.push_str("/// Desktop-only: see `spawn_hot_reload`'s doc comment for why these need\n");
    out.push_str("/// to be read at least once to avoid being compiled away. `asset!()`\n");
    out.push_str("/// doesn't have `include_str!`'s rebuild-on-content-edit problem above -\n");
    out.push_str("/// it's `dx`'s own bundler tracking a file for its live-reload system,\n");
    out.push_str("/// not a Rust compile-time dependency.\n");
    out.push_str("#[cfg(feature = \"desktop\")]\n");
    out.push_str("pub static DEMO_ASSETS: &[Asset] = &[\n");
    for name in &names {
        out.push_str(&format!("    asset!(\"/assets/{name}.adm2\"),\n"));
    }
    out.push_str("];\n");

    let out_dir = env::var("OUT_DIR").expect("set by cargo");
    fs::write(Path::new(&out_dir).join("demo_manifest.rs"), out).expect("write demo_manifest.rs");
}
