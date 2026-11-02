# Vendored crates: provenance

Role: records where each crate under `vendor/` came from, so the pristine
tree can be reproduced and checked. `tools/vendor/import.sh` reads the table
below as its manifest (crate, version, expected checksum); keep each row on one
line in this exact column order.

The `sha256` column is the crates.io `package` checksum: the SHA-256 of the
published `.crate` tarball, as recorded by `cargo vendor` in
`.cargo-checksum.json` and in the crates.io index. Each directory is that
tarball's contents, byte for byte, minus `.cargo-checksum.json` (cargo would
otherwise verify per-file hashes and refuse to build our patched files).

| crate | version | sha256 | imported |
|---|---|---|---|
| hayro-syntax | 0.7.2 | 0edeafd70aa2db743de8ede8637d07ec87db05efe69cde371d03f1b185fcef27 | 2026-10-01 |
| hayro-interpret | 0.7.0 | f2613d0406898995042d0794c4245b2f2fba1246490c8ac593769bba6551129d | 2026-10-01 |
| hayro-postscript | 0.1.0 | 885c5ef0654933139a9b9546fc2c69e18d37f38aa2520f079092b1be18f1fcaa | 2026-10-01 |
| hayro-jbig2 | 0.3.0 | 69374b3668dd45aeb3d3145cda68f2c7b4f223aaa2511e67d076f1c7d741388d | 2026-10-05 |
| hayro-jpeg2000 | 0.3.5 | c75ab947623ef4ccaa7acf0579edf7cbb5a73838e3839a7be73335e522f433a1 | 2026-10-05 |

Upstream: <https://github.com/LaurenzV/hayro>, licensed `Apache-2.0 OR MIT`
(see each crate's `LICENSE-*`; hayro-interpret's and hayro-jpeg2000's
`assets/` carry their own notices).

The crates were imported pristine in two steps: hayro-syntax, hayro-interpret
and hayro-postscript at the commit tagged `vendor/hayro-pristine-1`; the image
codecs hayro-jbig2 and hayro-jpeg2000, once their decoders needed patching, at
the commit tagged `vendor/hayro-pristine-2` (the first three were already
patched by then, and stay so in that commit). Every later change to these
directories is one of the runs' H-changes and is exported to `patches/hayro/`
by `tools/vendor/export-patches.sh`, as one series numbered across both
imports: the changes made before tag 2 (to the first three crates), then those
made after it (to any of the five). `tools/vendor/check-patches.sh` (CI job
`vendor-patches`) proves that a fresh pristine import of every row above plus
that series reproduces this tree exactly.

## Vendored C/C++ sources

`tools/vendor/import-c.sh` reads the table below as its manifest, as
`import.sh` reads the crate table above (rows here start `| tesseract` /
`| leptonica`, so `import.sh`'s `| hayro-` filter never sees them). Keep each
row on one line in this exact column order.

Neither project publishes checksums, so `sha256` is the SHA-256 of the archive
at `url`, computed when it was pinned and re-verified by every import.
Leptonica's archive is the release asset its maintainer uploaded. Tesseract
publishes no source asset, so its pin is GitHub's archive of the tag; `commit`
is the commit that archive embeds (`git get-tar-commit-id`, which `import-c.sh`
checks), so the same tree can be re-fetched by commit id if GitHub ever
regenerates tag archives. Leptonica's release archive embeds no commit id; its
`commit` (the `1.87.0` tag) is informational only.

| source | version | url | sha256 | commit | imported |
|---|---|---|---|---|---|
| tesseract | 5.5.3 | https://github.com/tesseract-ocr/tesseract/archive/refs/tags/5.5.3.tar.gz | 9218e62793116d42a9f6d14cd9348518b27f382096eea3d0f2d1a24616bb5884 | db0ec62f81b0737fbbe184d8fea40af5738f8eef | 2026-10-03 |
| leptonica | 1.87.0 | https://github.com/DanBloomberg/leptonica/releases/download/1.87.0/leptonica-1.87.0.tar.gz | c73363397f96eb1295602bf44d708a994ad42046c791bf03ea0505d829bdb6a7 | 13275a278eb55b5746e33f95fbf5a2c8f604b3ab | 2026-10-03 |

Licences: Tesseract `Apache-2.0` (`tesseract/LICENSE`), Leptonica
`BSD-2-Clause` (`leptonica/leptonica-license.txt`). Both notices ship in
`THIRD_PARTY_NOTICES` (Part 4); `tools/license/check-c-licenses.sh` checks the
copyright holders and licence keywords of both trees.

Each tree is its archive's top-level directory with the paths below removed:
documentation, tests, autotools and packaging files, the training tools,
Tesseract's `tessdata/` configs (no models) and Leptonica's `prog/`. Nothing
the CMake library build reads is removed. `import-c.sh` deletes exactly these
paths (relative to the archive's top directory) and fails if one is missing,
so a typo cannot silently keep a file.

| prune | source | path |
|---|---|---|
| prune | tesseract | .clang-format |
| prune | tesseract | .gitattributes |
| prune | tesseract | .github |
| prune | tesseract | .gitignore |
| prune | tesseract | .gitmodules |
| prune | tesseract | .mailmap |
| prune | tesseract | AUTHORS |
| prune | tesseract | CITATIONS.bib |
| prune | tesseract | CONTRIBUTING.md |
| prune | tesseract | ChangeLog |
| prune | tesseract | INSTALL |
| prune | tesseract | INSTALL.GIT.md |
| prune | tesseract | Makefile.am |
| prune | tesseract | README.md |
| prune | tesseract | SECURITY.md |
| prune | tesseract | appveyor.yml |
| prune | tesseract | autogen.sh |
| prune | tesseract | configure.ac |
| prune | tesseract | doc |
| prune | tesseract | java |
| prune | tesseract | m4 |
| prune | tesseract | nsis |
| prune | tesseract | snap |
| prune | tesseract | src/training |
| prune | tesseract | sw.cpp |
| prune | tesseract | tessdata |
| prune | tesseract | tesseract.pc.in |
| prune | tesseract | test |
| prune | tesseract | unittest |
| prune | leptonica | Doxyfile |
| prune | leptonica | Makefile.am |
| prune | leptonica | Makefile.in |
| prune | leptonica | README.html |
| prune | leptonica | README.md |
| prune | leptonica | aclocal.m4 |
| prune | leptonica | autogen.sh |
| prune | leptonica | autom4te.cache |
| prune | leptonica | config |
| prune | leptonica | configure |
| prune | leptonica | configure.ac |
| prune | leptonica | doxygen.log |
| prune | leptonica | lept.pc.in |
| prune | leptonica | lok.lua |
| prune | leptonica | m4 |
| prune | leptonica | make-for-auto |
| prune | leptonica | make-for-local |
| prune | leptonica | moller52.jpg |
| prune | leptonica | prog |
| prune | leptonica | src/Makefile.am |
| prune | leptonica | src/Makefile.in |
| prune | leptonica | style-guide.txt |
| prune | leptonica | sw.cpp |
| prune | leptonica | version-notes.html |

What remains: Tesseract `CMakeLists.txt`, `LICENSE`, `VERSION`,
`tesseract.pc.cmake`, `cmake/`, `include/`, `src/` (without `src/training/`);
Leptonica `CMakeLists.txt`, `leptonica-license.txt`, `lept.pc.cmake`, `cmake/`,
`src/` (without its two automake files). Size at import: 677 files,
15,387,341 bytes (Tesseract 6,364,302; Leptonica 9,023,039), about 3.4 MB
gzip-compressed.

The pristine import is the commit tagged `vendor/tesseract-pristine-1` and
`vendor/leptonica-pristine-1` (one commit, both tags). Every later change to
`tesseract/` or `leptonica/` is exported to `patches/tesseract/` or
`patches/leptonica/` by `tools/vendor/export-c-patches.sh`;
`tools/vendor/check-c-sources.sh` (CI job `vendor-patches`) proves that the
checksummed, pruned archives plus those series reproduce the two trees exactly.
An absent patch directory is an empty series.
