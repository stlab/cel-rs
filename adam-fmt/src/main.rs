//! `adam-fmt` binary entry point: formats each `.adm2` file named on the command line in place.
//!
//! # Examples
//!
//! ```sh
//! adam-fmt sheet.adm2                    # format one file
//! adam-fmt cells/a.adm2 cells/b.adm2     # format several
//! ```
//!
//! To format every `.adm2` file under a directory tree, walk the tree and pass the discovered
//! files to `adam-fmt` (see `scripts/format_adam.py` for a recursive wrapper).

use std::path::PathBuf;
use std::process::ExitCode;

/// Formats every supplied path in order, reporting errors without skipping later paths.
///
/// Returns failure after printing usage for no paths, or after any file fails.
/// Prints changed paths to stdout and diagnostics to stderr; unchanged files are silent.
///
/// - Complexity: in addition to argument retrieval and file formatting, O(a) iterator
///   steps for a arguments and O(p) space for the longest converted path.
fn main() -> ExitCode {
    let mut paths = std::env::args_os().skip(1).map(PathBuf::from).peekable();
    if paths.peek().is_none() {
        eprintln!("usage: adam-fmt <file.adm2>...");
        return ExitCode::FAILURE;
    }

    let mut had_error = false;
    // Process every path even when an earlier file fails.
    for path in paths {
        match adam_fmt::format_file(&path) {
            Ok(true) => println!("formatted {}", path.display()),
            Ok(false) => {}
            Err(error) => {
                eprint!("{}", adam_fmt::render_error(&path, &error));
                had_error = true;
            }
        }
    }

    if had_error {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
