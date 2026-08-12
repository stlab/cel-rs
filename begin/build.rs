//! Generates a compile-time manifest of `examples/*.adm2` files, embedded as
//! source text for platforms/builds with no live filesystem to read from at
//! runtime.
//!
//! Desktop reads examples directly off disk at runtime instead (see
//! `available_examples`/`load_example_source` in `src/example_source.rs`), so
//! it needs nothing from this script beyond compiling. The web build has no
//! filesystem to read at runtime, and tests want a fixture that doesn't
//! depend on desktop asset bundling being available - both instead get every
//! example's content embedded at compile time via `include_str!`, generated
//! here into `$OUT_DIR/example_manifest.rs`, which `example_source.rs`
//! splices in via `include!`.
//!
//! Deliberately watches only `build.rs` itself (`cargo:rerun-if-changed=
//! build.rs` below), not the `examples/` directory: watching the directory
//! would make Cargo treat every `.adm2` file as a build-script input, so
//! editing an *existing* example's content would force a full rebuild on
//! every edit for the platforms that read this generated manifest. Adding or
//! removing an example file still requires a rebuild for those platforms
//! (unavoidable - they have no live filesystem to notice the change), but
//! that trade-off doesn't apply to desktop, which never reads this manifest
//! at all.
use std::env;
use std::fs;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("set by cargo");
    let examples_dir = Path::new(&manifest_dir).join("examples");

    let mut names: Vec<String> = fs::read_dir(&examples_dir)
        .expect("examples/ directory must exist")
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
    out.push_str("/// `(name, embedded source)` pairs for every `examples/*.adm2` file,\n");
    out.push_str("/// sorted by name. Used on platforms/builds with no live filesystem to\n");
    out.push_str("/// read from at runtime: the web build, and tests (a fixture that\n");
    out.push_str("/// doesn't depend on desktop asset bundling being available). Desktop\n");
    out.push_str("/// reads both the list and each file's content live from disk instead -\n");
    out.push_str("/// see `available_examples`/`load_example_source`.\n");
    out.push_str("#[cfg(any(not(feature = \"desktop\"), test))]\n");
    out.push_str("pub static EXAMPLES_WITH_SOURCE: &[(&str, &str)] = &[\n");
    for name in &names {
        let abs_path = examples_dir.join(format!("{name}.adm2"));
        let abs_path_str = abs_path.display().to_string();
        out.push_str(&format!(
            "    ({name:?}, include_str!({abs_path_str:?})),\n"
        ));
    }
    out.push_str("];\n");

    let out_dir = env::var("OUT_DIR").expect("set by cargo");
    fs::write(Path::new(&out_dir).join("example_manifest.rs"), out)
        .expect("write example_manifest.rs");
}
