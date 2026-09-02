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

fn main() -> ExitCode {
    let paths: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if paths.is_empty() {
        eprintln!("usage: adam-fmt <file.adm2>...");
        return ExitCode::FAILURE;
    }

    let mut had_error = false;
    for path in &paths {
        match adam_fmt::format_file(path) {
            Ok(true) => println!("formatted {}", path.display()),
            Ok(false) => {}
            Err(error) => {
                eprint!("{}", adam_fmt::render_error(path, &error));
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
