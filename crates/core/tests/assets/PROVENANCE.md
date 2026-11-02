# Test assets: provenance

Role: records where each third-party file in this directory came from and
under which license, so the tests of the vendored hayro fork can embed real
font programs and images that the test kit cannot generate (it has no
TrueType or CFF writer and no JPEG 2000 encoder). Files here are read by the
tests in `crates/core/tests/` only; they are never part of a built artifact.

| file | source | commit | license | sha256 |
|---|---|---|---|---|
| `tinos_subset.ttf` | <https://github.com/googlefonts/fontations/blob/cc90033f73577ab8476c76d692a5101385c584eb/font-test-data/test_data/ttf/tinos_subset.ttf> | `cc90033f73577ab8476c76d692a5101385c584eb` | Apache-2.0 | `f443f99843977ab03549bfc31cb6f2413416f04a06a0eacae8815b537ec5cca0` |
| `LICENSE-APACHE-2.0.txt` | <https://github.com/googlefonts/fontations/blob/cc90033f73577ab8476c76d692a5101385c584eb/LICENSE-APACHE> | `cc90033f73577ab8476c76d692a5101385c584eb` | (license text) | `eba684204073ed715c4abb48129acdfd2bff8ae48339e6a7da3b341d3027b7f0` |
| `foxit_sans.cff` | `hayro-interpret` 0.7.0, `assets/FoxitSans.pfb` (this repo: `vendor/hayro-interpret/assets/FoxitSans.pfb`), itself extracted from PDFium | n/a (crates.io `hayro-interpret` 0.7.0) | BSD-3-Clause | `a02339e95d20db14cbdfc1c6baf359cd36a091d5434925c3a4040f60e1a1af9e` |
| `LICENSE-BSD-3-PDFium.txt` | `hayro-interpret` 0.7.0, `assets/LICENSE_FOXIT` | n/a | (license text) | `b578cdd2345840ada550bd12519533812320d5f1d21cf4c1c7e1b1b0a31c98b7` |
| `precinct1_gray_128.j2k` | our own: written with OpenJPEG 2.5.4 `opj_compress` (see below) | n/a | GPL-3.0-or-later (our work) | `52ecc20aecd0f337f0f581365a0cfa802f8702894df934e27066b1c08d946135` |

## `tinos_subset.ttf`

A subset of Tinos Regular whose name table reads "Digitized data copyright
(c) 2010-2012 Google Corporation." That release was distributed under the
Apache License 2.0: Google Fonts listed it as `license: "APACHE2"` with that
copyright line in
<https://github.com/google/fonts/blob/d4dc6ad9d1b9a307b5a3bb6a1633c30fa5813491/apache/tinos/METADATA.pb>
(later Tinos releases, from 2026, are under the OFL and are not what this
file contains). fontations' `font-test-data/test_data/README.md` records the
same license and the command that made the subset, `pyftsubset
tinos-regular.ttf --text=aAbB --gids=3,5`. The file is copied unchanged
(11,196 bytes). The Apache License 2.0 text is `LICENSE-APACHE-2.0.txt`.

What the tests rely on: a TrueType (`glyf`) font with a Windows Unicode BMP
`cmap` and seven glyphs, `.notdef`, `space`, `quotedbl`, `A`, `B`, `a`, `b`
(glyph IDs 0-6). The `cmap` maps U+0020 and U+00A0 both to glyph 1, U+0022 to
2, U+0041/42 to 3/4 and U+0061/62 to 5/6.

Used by `h6_cid_unicode.rs` as the embedded `/FontFile2` of CIDFontType2
fonts: with `Identity` ordering and no `ToUnicode`, Unicode comes from the
reversed `cmap` (H6b); with `Japan1` ordering, from the ordering's UCS2 CMap
whether or not a program is embedded (H6a).

Chosen over the fonts in hayro's own test suite (`hayro-tests/assets`, the
Liberation family, 300-415 KB each and under the OFL with a Reserved Font
Name, which a subset would have to drop) and over the CID-font PDFs there,
which embed commercial fonts. Apache-2.0 is in the `deny.toml` allow-list.

## `foxit_sans.cff`

A copy, byte for byte (15,025 bytes), of the Foxit Sans face that hayro ships
for the standard-14 Helvetica substitute. Despite its `.pfb` name in hayro it
is a bare CFF font program (it starts with a CFF header and the name
`ChromSansOTF`), so it is stored here under the extension that matches its
format. It is copied rather than `include_bytes!`d from `vendor/` so that the
tests do not depend on the layout of a directory that is re-vendored. hayro's
`assets/README.md` records that these files were extracted from PDFium
(Copyright 2014 PDFium Authors, original code Copyright 2014 Foxit Software
Inc.) under the BSD-3-Clause license in `LICENSE-BSD-3-PDFium.txt`, which is on
the `deny.toml` allow-list.

Used by `h5_glyph_info.rs` as the embedded `/FontFile3` (`/Type1C`) of a simple
font: the tests need an embedded CFF program to prove that patch 0007 gives it
no AFM ascent/descent. The embedded Type 1 (`/FontFile`) and TrueType
(`tinos_subset.ttf`, above) programs of the same tests need no further files:
the Type 1 program is generated in the test itself.

## `precinct1_gray_128.j2k`

A 128 × 128 8-bit gray JPEG 2000 codestream (8,023 bytes) with precinct
exponent 1 at every one of its six resolutions, so every code-block above the
lowest resolution is 1 × 1. Our own image, encoded once with OpenJPEG 2.5.4
and committed unchanged (its `COM` marker names the encoder). The source
raster is a diagonal gradient, `(x + y) * 255 // 254`, with a dark square
(value 20) over `48 <= x, y < 80`, written as a binary PGM by a few lines of
Python, then

    opj_compress -i src128.pgm -o precinct1_gray_128.j2k \
        -c '[2,2],[2,2],[2,2],[2,2],[2,2],[2,2]' -n 6

(reversible 5/3 wavelet, lossless: `opj_decompress` returns the source
raster exactly). Each precinct size must be given: with one `[2,2]` OpenJPEG
halves it for the lower resolutions and writes a precinct exponent of 0 at
the highest, which decoders reject. At 256 × 256 `opj_compress` fails to
encode the same geometry, hence 128.

Used by `h15_jpx_budget.rs` (run 0003 H15) as the `Ok` leg of the 1 × 1
code-block shape with real content: the test kit's own codestreams of that
shape carry no packets and decode flat, so this is the file that shows the
budget leaves real pixels unchanged where the decoder's structural records
cost the most (about 242 bytes per pixel).
