//! Embeds official control styles with supported CSS-module exports.
use lightningcss::{
    css_modules::{Config, Pattern},
    stylesheet::{ParserOptions, PrinterOptions, StyleSheet},
};
use std::{env, fmt::Write, fs, path::PathBuf};

/// Returns whether `byte` can continue a CSS identifier (a letter, digit, `_`, or `-`),
/// distinguishing identifier continuation from a leading numeric sign.
fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
}

/// Rewrites every bare `<n>rem` length in `source` into `calc(<n> * var(--adam-rem, 1rem))`,
/// letting a host scale every embedded control length by setting `--adam-rem` on or above
/// `.adam-theme`.
///
/// A single forward scan copies through non-matching bytes and replaces each numeric literal
/// immediately followed by `rem` and a non-identifier character (so `rem` is a whole unit, not
/// a prefix of a longer identifier). A digit or `.` does not start a literal when it continues
/// an identifier (preceded by a letter, digit, `_`, or `-`); a leading `-` or `+` sign is part
/// of the literal only when that sign itself is not continuing an identifier (i.e. the
/// character before the sign is absent or not an identifier character), so `--adam-rem: 1rem`
/// is left alone but a bare length like `-0.5rem` becomes
/// `calc(-0.5 * var(--adam-rem, 1rem))`. The scan does not special-case existing `calc(...)`
/// wrappers, so a length already inside one (e.g. `calc(1.15rem - 2px)`) nests into
/// `calc(calc(1.15 * var(--adam-rem, 1rem)) - 2px)`; Lightning CSS accepts and minifies nested
/// `calc()`, so this remains valid CSS.
///
/// - Complexity: O(b) time and space for source byte length `b`.
fn scale_rem(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut output = String::with_capacity(source.len());
    let mut copied_to = 0;
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte.is_ascii_digit() || byte == b'.' {
            let preceding = bytes.get(index.wrapping_sub(1)).copied();
            let number_start = match preceding {
                Some(b'-') | Some(b'+')
                    if !bytes
                        .get(index.wrapping_sub(2))
                        .is_some_and(|b| is_identifier_byte(*b)) =>
                {
                    Some(index - 1)
                }
                Some(b) if is_identifier_byte(b) => None,
                _ => Some(index),
            };
            if let Some(number_start) = number_start {
                let mut cursor = index;
                while bytes
                    .get(cursor)
                    .is_some_and(|b| b.is_ascii_digit() || *b == b'.')
                {
                    cursor += 1;
                }
                let number_end = cursor;
                let unit_end = cursor + 3;
                let followed_by_rem = bytes.get(cursor..unit_end) == Some(b"rem");
                let unit_is_whole_word =
                    followed_by_rem && !bytes.get(unit_end).is_some_and(|b| is_identifier_byte(*b));
                if unit_is_whole_word {
                    output.push_str(&source[copied_to..number_start]);
                    let literal = &source[number_start..number_end];
                    write!(output, "calc({literal} * var(--adam-rem, 1rem))").unwrap();
                    index = unit_end;
                    copied_to = unit_end;
                    continue;
                }
            }
        }
        index += 1;
    }
    output.push_str(&source[copied_to..]);
    output
}

/// Transforms each official module and writes its class exports and combined stylesheet.
///
/// - Complexity: O(b + k log k) time and O(b + k) storage for stylesheet/output
///   bytes b and exported names k, including deterministic export ordering.
fn main() {
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo provides OUT_DIR"));
    let mut combined = String::new();
    for name in [
        "theme",
        "button",
        "input",
        "checkbox",
        "switch",
        "slider",
        "label",
        "separator",
    ] {
        let path = if name == "theme" {
            PathBuf::from("assets/dx-components-theme.css")
        } else {
            PathBuf::from(format!("src/components/{name}/style.css"))
        };
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
