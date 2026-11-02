"""The release documents (public-distribution spec "Release checklist" and
"Releases are public and installable without credentials"; ADR 0096).

`RELEASING.md` must keep the steps a release depends on, and the README must
show the credential-free install: by URL, pinned by hash. Every workspace
crate staying `publish = false` is checked by the licence test
(crates/testkit/tests/license.rs).
"""

from __future__ import annotations

import re
import sys

from pdfs import REPO

sys.path.insert(0, str(REPO / "tools" / "license"))
import sbom

RELEASING = (REPO / "RELEASING.md").read_text(encoding="utf-8")
README = (REPO / "README.md").read_text(encoding="utf-8")


def test_golden_rebless_is_on_the_checklist() -> None:
    assert "RENDER_GOLDEN_BLESS=1" in RELEASING
    for crate in ("hayro", "vello", "pic-scale"):
        assert crate in RELEASING
    assert "crates/render/tests/render-golden/" in RELEASING


def test_checklist_covers_the_release_steps() -> None:
    for step in (
        "[workspace.package] version",
        "docs/releases/v<version>.md",
        "tools/license/gen-notices.sh",
        "git push origin v<version>",
        "sha256sum --check SHA256SUMS",
    ):
        assert step in RELEASING, step


def test_readme_installs_by_url_and_hash_without_credentials() -> None:
    section = README[README.index("## Installing a release") : README.index("## Build")]
    assert re.search(r"releases/download/v[\d.]+/pdf_extract-[\d.]+-cp313-abi3-", section)
    assert "--hash=sha256:" in section
    assert "--require-hashes" in section
    assert "gh release download" not in section  # no authenticated download


def test_readme_install_urls_name_the_current_version() -> None:
    """RELEASING.md step 2: the example URLs follow the version bump."""
    version = sbom.workspace_version(REPO)
    urls = re.findall(r"releases/download/v([\d.]+)/pdf_extract-([\d.]+)-", README)
    assert urls
    assert all(tag == version and wheel == version for tag, wheel in urls), urls
