//! Embeds official control styles with supported CSS-module exports.
use lightningcss::{
    css_modules::{Config, Pattern},
    stylesheet::{ParserOptions, PrinterOptions, StyleSheet},
};
use std::{env, fmt::Write, fs, path::PathBuf};

#[path = "build/scale_rem.rs"]
mod scale_rem;
use scale_rem::scale_rem;

/// Transforms each official module and writes its class exports and combined stylesheet.
///
/// - Complexity: O(b + k log k) time and O(b + k) storage for stylesheet/output
///   bytes b and exported names k, including deterministic export ordering.
fn main() {
    println!("cargo:rerun-if-changed=build/scale_rem.rs");
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo provides OUT_DIR"));
    let mut combined = String::new();
    for (name, path) in [
        ("theme", "assets/dx-components-theme.css"),
        ("button", "src/components/button/style.css"),
        ("input", "src/components/input/style.css"),
        ("checkbox", "src/components/checkbox/style.css"),
        ("switch", "src/components/switch/style.css"),
        ("slider", "src/components/slider/style.css"),
        ("label", "src/components/label/style.css"),
        ("separator", "src/components/separator/style.css"),
        ("card", "src/components/card/style.css"),
        ("numeric_editor", "src/components/numeric_editor/style.css"),
        ("inspector", "src/inspector/style.css"),
    ] {
        let path = PathBuf::from(path);
        println!("cargo:rerun-if-changed={}", path.display());
        let source = fs::read_to_string(&path).expect("official stylesheet is readable");
        let source = scale_rem(&source);
        let pattern = format!("adam_{name}_[local]");
        let sheet = StyleSheet::parse(
            &source,
            ParserOptions {
                filename: path.to_string_lossy().into_owned(),
                css_modules: Some(Config {
                    pattern: Pattern::parse(&pattern).expect("valid module naming pattern"),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .expect("official stylesheet parses");
        let result = sheet
            .to_css(PrinterOptions::default())
            .expect("official stylesheet transforms");
        combined.push_str(&result.code);
        combined.push('\n');
        let mut generated = String::from(
            "/// Contains supported CSS-module class exports.\n#[allow(non_upper_case_globals)]\nstruct Styles;\n#[allow(non_upper_case_globals, dead_code)]\nimpl Styles {\n",
        );
        let mut exports: Vec<_> = result
            .exports
            .expect("module exports")
            .into_iter()
            .collect();
        exports.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        for (class, export) in exports {
            writeln!(
                generated,
                "/// Names the scoped `{class}` selector.\nconst {}: &'static str = {:?};",
                class.replace('-', "_"),
                export.name
            )
            .unwrap();
        }
        generated.push_str("}\n");
        fs::write(output.join(format!("{name}-styles.rs")), generated)
            .expect("write module exports");
    }
    fs::write(output.join("adam-controls.css"), combined).expect("write embedded styles");
}
