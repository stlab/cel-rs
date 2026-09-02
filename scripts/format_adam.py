#!/usr/bin/env python3
"""Recursively formats every .adm2 file under one or more paths using adam-fmt.

adam-fmt itself only formats the single file(s) named on its command line (see
adam-fmt/src/main.rs); this script supplies the recursive directory walk `cargo
fmt` gets for free from rustfmt, then hands every discovered .adm2 file to
adam-fmt in one batch.

Usage:
    python scripts/format_adam.py [path ...]

With no arguments, formats every .adm2 file under the current directory.
"""

import os
import subprocess
import sys
from pathlib import Path

# scripts/ sits directly under the workspace root.
WORKSPACE_ROOT = Path(__file__).resolve().parent.parent
IGNORED_DIR_NAMES = {"target", "node_modules"}


def find_adm2_files(start: Path) -> list[Path]:
    """Returns every `.adm2` file at or under `start`, in sorted order.

    A `start` that is itself a file is returned as a single-element list if it has an
    `.adm2` extension, otherwise an empty list. Directories named in IGNORED_DIR_NAMES
    or starting with '.' are not descended into.
    """
    if start.is_file():
        return [start] if start.suffix == ".adm2" else []

    found = []
    for dirpath, dirnames, filenames in os.walk(start):
        dirnames[:] = [
            d for d in dirnames if d not in IGNORED_DIR_NAMES and not d.startswith(".")
        ]
        for name in filenames:
            if name.endswith(".adm2"):
                found.append(Path(dirpath) / name)
    return sorted(found)


def main() -> int:
    paths = [Path(p) for p in sys.argv[1:]] or [Path(".")]

    files: list[Path] = []
    seen: set[Path] = set()
    for path in paths:
        if not path.exists():
            print(f"format_adam.py: {path}: no such file or directory", file=sys.stderr)
            return 1
        for found in find_adm2_files(path):
            resolved = found.resolve()
            if resolved not in seen:
                seen.add(resolved)
                files.append(found)

    if not files:
        print("format_adam.py: no .adm2 files found")
        return 0

    result = subprocess.run(
        [
            "cargo",
            "run",
            "--manifest-path",
            str(WORKSPACE_ROOT / "Cargo.toml"),
            "--package",
            "adam-fmt",
            "--quiet",
            "--",
            *(str(f) for f in files),
        ]
    )
    return result.returncode


if __name__ == "__main__":
    sys.exit(main())
