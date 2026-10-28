"""Checks recursive formatting discovery, input selection, and visible failures."""

import contextlib
import errno
import io
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import format_adam


class FormatAdamTests(unittest.TestCase):
    """Checks the formatter script's filesystem and command-line contracts."""

    def test_discovery_sorts_sources_and_prunes_nested_build_directories(self):
        """Finds sources in order without traversing hidden or generated trees."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in (
                "z.adm2",
                "nested/a.adm2",
                "nested/notes.txt",
                ".hidden/private.adm2",
                "target/generated.adm2",
                "node_modules/dependency.adm2",
            ):
                file = root / name
                file.parent.mkdir(parents=True, exist_ok=True)
                file.write_text("sheet example {}", encoding="utf-8")
            self.assertEqual(
                format_adam.find_adm2_files(root),
                [root / "nested/a.adm2", root / "z.adm2"],
            )
            self.assertEqual(
                format_adam.find_adm2_files(root / "z.adm2"), [root / "z.adm2"]
            )
            self.assertEqual(
                format_adam.find_adm2_files(root / "nested/notes.txt"), []
            )

    def test_unreadable_subtree_reports_failure_without_running_formatter(self):
        """Reports incomplete discovery instead of formatting a successful subset."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            denied = root / "unreadable"
            denied.mkdir()
            (root / "valid.adm2").write_text("sheet example {}", encoding="utf-8")
            scan = format_adam.os.scandir

            def failing_scan(path):
                """Rejects the selected subtree and preserves other real directory IO."""
                if Path(path) == denied:
                    raise PermissionError(errno.EACCES, "unreadable directory", str(path))
                return scan(path)

            stderr = io.StringIO()
            with (
                patch.object(format_adam.os, "scandir", side_effect=failing_scan),
                patch.object(format_adam.sys, "argv", ["format_adam.py", str(root)]),
                patch.object(format_adam.subprocess, "run") as run,
                contextlib.redirect_stderr(stderr),
            ):
                self.assertEqual(format_adam.main(), 1)
                run.assert_not_called()
            self.assertIn("unreadable", stderr.getvalue())

    def test_overlapping_paths_format_each_source_once_and_return_child_status(self):
        """Deduplicates discovered files and preserves the formatter's exit status."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "example.adm2"
            source.write_text("sheet example {}", encoding="utf-8")
            with (
                patch.object(
                    format_adam.sys,
                    "argv",
                    ["format_adam.py", str(root), str(source)],
                ),
                patch.object(
                    format_adam.subprocess,
                    "run",
                    return_value=subprocess.CompletedProcess([], 7),
                ) as run,
            ):
                self.assertEqual(format_adam.main(), 7)
                command = run.call_args.args[0]
            self.assertEqual(command[command.index("--") + 1 :], [str(source)])

    def test_empty_input_is_success_but_missing_input_is_reported(self):
        """Distinguishes an empty search from a nonexistent requested path."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            stdout = io.StringIO()
            stderr = io.StringIO()
            with (
                patch.object(format_adam.sys, "argv", ["format_adam.py", str(root)]),
                patch.object(format_adam.subprocess, "run") as run,
                contextlib.redirect_stdout(stdout),
            ):
                self.assertEqual(format_adam.main(), 0)
                run.assert_not_called()
            with (
                patch.object(
                    format_adam.sys,
                    "argv",
                    ["format_adam.py", str(root / "missing")],
                ),
                contextlib.redirect_stderr(stderr),
            ):
                self.assertEqual(format_adam.main(), 1)
            self.assertIn("no .adm2 files found", stdout.getvalue())
            self.assertIn("no such file or directory", stderr.getvalue())

    def test_formatter_launch_failure_is_reported(self):
        """Reports an unavailable formatter command through the CLI error boundary."""
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "example.adm2"
            source.write_text("sheet example {}", encoding="utf-8")
            stderr = io.StringIO()
            with (
                patch.object(format_adam.sys, "argv", ["format_adam.py", str(source)]),
                patch.object(
                    format_adam.subprocess,
                    "run",
                    side_effect=FileNotFoundError(errno.ENOENT, "cargo not found"),
                ),
                contextlib.redirect_stderr(stderr),
            ):
                self.assertEqual(format_adam.main(), 1)
            self.assertIn("cargo not found", stderr.getvalue())


if __name__ == "__main__":
    unittest.main()
