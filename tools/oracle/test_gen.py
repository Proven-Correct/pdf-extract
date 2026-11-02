"""Tests for gen.py's argument guards (internal #56 B26). Run by hand or by CI:

    cd tools/oracle && uv run python -m unittest test_gen

Only the guards that fire before the oracle runs are covered, so no model or
fixture is read and nothing is written. CI runs this file (the `oracle-tests`
job) without installing this project's dependencies.
"""

from __future__ import annotations

import contextlib
import io
import sys
import tempfile
import tomllib
import types
import unittest
from pathlib import Path

# gen.py imports pymupdf at module level, but the guards under test fire before
# the oracle is touched. Library CI must never need PyMuPDF (AGPL; ADR 0006), so
# the module is replaced with an empty stub, unconditionally: the tests then
# behave the same with or without the real package installed. This must run
# before `import gen`.
sys.modules["pymupdf"] = types.ModuleType("pymupdf")

import gen  # noqa: E402


def run(argv: list[str]) -> tuple[int, str]:
    """(exit status, stderr and exit message) of `gen.main(argv)`."""
    err = io.StringIO()
    with contextlib.redirect_stderr(err):
        try:
            return gen.main(argv), err.getvalue()
        except SystemExit as exc:
            # `raise SystemExit("message")` carries the message in `code`.
            if isinstance(exc.code, int):
                return exc.code, err.getvalue()
            return 1, err.getvalue() + str(exc.code)


class PruneGuard(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.out = Path(self.tmp.name) / "expectations"
        self.out.mkdir()
        manifest = tomllib.loads((gen.FIXTURES / "manifest.toml").read_text(encoding="utf-8"))
        self.pin: str = manifest["built"][0]["sha256"]
        self.argv = ["--ocr", "--tessdata", self.tmp.name, "--prune", "--out", str(self.out)]
        self.argv.append(self.tmp.name)

    def test_refuses_to_prune_built_expectations_without_built(self) -> None:
        (self.out / f"{self.pin}.json").write_text("{}\n", encoding="utf-8")
        status, err = run(self.argv)
        self.assertEqual(status, 2)
        self.assertIn("--prune without --built", err)
        self.assertTrue((self.out / f"{self.pin}.json").is_file())

    def test_a_directory_without_built_expectations_is_not_refused(self) -> None:
        # The guard is silent; the run then stops at the (absent) model, which
        # proves it got past argument validation without touching anything.
        status, err = run(self.argv)
        self.assertNotIn("--prune without --built", err)
        self.assertIn("does not exist", err)
        self.assertEqual(status, 1)


if __name__ == "__main__":
    unittest.main()
