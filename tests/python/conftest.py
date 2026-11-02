"""Shared fixtures of the Python test suite (run spec §4.11).

Generated PDFs come from the Rust test kit, so both suites test the same
bytes: the ``pytest_fixtures`` example of ``pdf-extract-testkit`` is run once
per session (``cargo run``; the Rust toolchain is needed anyway to build the
extension) and writes every file into a temporary directory. Set
``PDF_EXTRACT_PYTEST_FIXTURES`` to a directory that example already filled to
skip the cargo step. Committed fixtures (the encrypted files) are read from
``tests/fixtures/corpus-c``.
"""

from __future__ import annotations

import os
import subprocess
from pathlib import Path

import pytest

from pdfs import REPO, Fixtures


@pytest.fixture(scope="session")
def fixtures(tmp_path_factory: pytest.TempPathFactory) -> Fixtures:
    preset = os.environ.get("PDF_EXTRACT_PYTEST_FIXTURES")
    if preset:
        root = Path(preset)
    else:
        root = tmp_path_factory.mktemp("generated")
        subprocess.run(
            [
                "cargo",
                "run",
                "--locked",
                "--quiet",
                "-p",
                "pdf-extract-testkit",
                "--example",
                "pytest_fixtures",
                "--",
                str(root),
            ],
            cwd=REPO,
            check=True,
        )
    if not (root / "index.tsv").is_file():
        raise RuntimeError(f"{root} is not a complete fixture directory (no index.tsv)")
    return Fixtures(root)
