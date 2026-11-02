#!/usr/bin/env bash
# Write THIRD_PARTY_NOTICES, the notices file the wheel ships via pyproject's
# `license-files`. Never edit THIRD_PARTY_NOTICES by hand: change this script,
# about.toml, tools/license/about.hbs or tools/license/agl-notice.txt and rerun.
# The `deny` CI job reruns it and fails on any diff.
#
# Five parts:
#   1. licences of every Rust crate linked into the extension module, from
#      cargo-about over crates/python's graph (about.toml);
#   2. the Adobe Glyph List notice (tools/license/agl-notice.txt, verbatim);
#   3. notices of data files that hayro-interpret, hayro-jpeg2000 and
#      hayro-cmap embed with include_bytes!, which cargo-about and cargo-deny
#      cannot see, copied verbatim from the crate sources at generation time;
#   4. the vendored C/C++ libraries, Tesseract and Leptonica: each licence file
#      verbatim, under a heading naming the version (from vendor/PROVENANCE.md)
#      and the copyright holders tools/license/check-c-licenses.sh finds, so
#      the heading cannot drift from the sources (and the script fails if the
#      licence check does);
#   5. the Rust standard library the binaries link statically, which neither
#      cargo-about nor cargo-deny sees: tools/license/rust_std.py lists its
#      crates from the pinned toolchain's rust-src and copies the crates.io
#      ones' licences. The Rust project's own notice for the in-tree crates,
#      the toolchain's COPYRIGHT-library.html, is copied verbatim to
#      tools/license/rust-std-COPYRIGHT.html, which ships beside this file.
#
# Usage: tools/license/gen-notices.sh
# Needs cargo-about 0.9.2 (pinned in the `deny` CI job), jq, rustup with the
# rust-toolchain.toml pin (rust-src included) and Python >= 3.11 ($PYTHON, or
# python3). Runs cargo-about offline after a locked fetch, so the output
# depends only on Cargo.lock and the toolchain pin.
set -euo pipefail
# Byte-order glob sorting and messages, whatever the caller's locale.
export LC_ALL=C

# sort_used_by: stdin to stdout, with each "Used by:" list of
# "    - NAME VERSION" lines in byte order (insertion sort; the lists are short),
# and identical lines collapsed. cargo-about adds one entry per licence file it
# matched, so a crate shipping the same text twice (pyo3's LICENSE-MIT and
# pyo3-runtime/LICENSE-MIT) would otherwise list itself twice.
# Defined before the source guard below so tools/license/test-gen-notices.sh
# can test it alone; it is the only definition that must stay above the guard.
sort_used_by() {
    awk '
        function flush(    i, j, t) {
            for (i = 2; i <= n; i++) {
                t = run[i]
                for (j = i - 1; j >= 1 && run[j] > t; j--) run[j + 1] = run[j]
                run[j + 1] = t
            }
            for (i = 1; i <= n; i++) if (i == 1 || run[i] != run[i - 1]) print run[i]
            n = 0
        }
        inlist && /^    - / { run[++n] = $0; next }
        inlist { flush(); inlist = 0 }
        { print }
        $0 == "Used by:" { inlist = 1 }
        END { if (inlist) flush() }
    '
}

# Sourced (by the test), stop here: everything below needs cargo, jq and the
# repository.
[[ "${BASH_SOURCE[0]}" == "$0" ]] || return 0

repo=$(cd "$(dirname "$0")/../.." && pwd)
cd "$repo"
manifest=crates/python/Cargo.toml
py=${PYTHON:-python3}
assets=vendor/hayro-interpret/assets

cargo fetch --locked --manifest-path "$manifest" >/dev/null
meta=$(cargo metadata --format-version 1 --locked --offline --manifest-path "$manifest")
# crate_field NAME FILTER: FILTER applied to the one locked package NAME.
crate_field() {
    jq -er --arg n "$1" '[.packages[] | select(.name == $n)]
        | if length == 1 then .[0] | '"$2"' else error("not one \($n)") end' <<<"$meta"
}
# Source directories: hayro-cmap's registry copy; hayro-jpeg2000 is vendored,
# so this finds vendor/hayro-jpeg2000.
cmap=$(dirname "$(crate_field hayro-cmap .manifest_path)")
jp2=$(dirname "$(crate_field hayro-jpeg2000 .manifest_path)")
# Labels name a crate's files as NAME-VERSION/..., as the registry names its
# directory, whether or not the crate is vendored: where a notice was read
# from is not part of its text, so vendoring a crate changes nothing here.
cmap_label=$(crate_field hayro-cmap '"\(.name)-\(.version)"')
jp2_label=$(crate_field hayro-jpeg2000 '"\(.name)-\(.version)"')
# c_version SOURCE: the version column of SOURCE's row in the C/C++ table of
# vendor/PROVENANCE.md (rows start "| tesseract |" / "| leptonica |").
c_version() {
    awk -F' *[|] *' -v s="$1" '$2 == s && $3 ~ /^[0-9]/ { print $3; n++ }
        END { if (n != 1) exit 1 }' vendor/PROVENANCE.md
}
# label LABEL FILE: FILE under a crate's label, e.g. hayro-cmap-0.1.0/x.
label() { echo "$1/$2"; }
# Build beside the targets and rename, so a failure never leaves either
# truncated. Part 5 is written first: it is the step that needs the toolchain.
out=$(mktemp THIRD_PARTY_NOTICES.XXXXXX)
part5=$(mktemp part5.XXXXXX)
copyright=$(mktemp tools/license/rust-std-COPYRIGHT.XXXXXX)
trap 'rm -f "$out" "$part5" "$copyright"' EXIT
"$py" -I tools/license/rust_std.py notices >"$part5"
"$py" -I tools/license/rust_std.py copyright "$copyright"

# heading TITLE CHAR: TITLE underlined with CHAR to its full width.
heading() { printf '%s\n%s\n\n' "$1" "$(printf '%*s' "${#1}" '' | tr ' ' "$2")"; }
# verbatim FILE: FILE's bytes, plus a newline if FILE does not end in one.
verbatim() { cat "$1"; if [[ -n $(tail -c1 "$1") ]]; then echo; fi; }

{
    heading "Third-party notices shipped with pdf-extract" "="
    cat <<'EOF'
pdf-extract itself is GPL-3.0-or-later (see LICENSE). This file carries the
notices of the third-party code and data compiled into its extension module.
Among the package's other licence files are the Adobe Core14 AFM notice
(Helvetica.afm), crates/testkit/data/README-AFM.txt, and the Rust project's
standard-library notice, tools/license/rust-std-COPYRIGHT.html.
It is written by tools/license/gen-notices.sh; do not edit it by hand.

EOF
    heading "Part 1. Rust crates linked into the extension module" "="
    # cargo-about lists each licence's crates in package-id order, which puts
    # path crates before registry ones; vendoring or un-vendoring a crate would
    # then reorder this file. Sorting by name and version makes the file
    # independent of where a crate comes from. Only the run directly after a
    # "Used by:" line is sorted, never licence text that happens to contain
    # indented bullets.
    cargo about generate --frozen --fail -c about.toml \
        --manifest-path "$manifest" tools/license/about.hbs | sort_used_by
    echo
    heading "Part 2. Adobe Glyph List" "="
    echo "read-fonts (Part 1) compiles in a table from the same list, under the"
    echo "same Adobe notice."
    echo
    verbatim tools/license/agl-notice.txt
    echo
    heading "Part 3. Data files embedded by hayro crates" "="

    heading "Foxit standard fonts (hayro-interpret, feature embed-fonts)" "-"
    echo "Embedded font programs, from $assets:"
    for f in "$assets"/*.pfb; do echo "    ${f##*/}"; done
    echo
    echo "Licence, $assets/LICENSE_FOXIT verbatim:"
    echo
    verbatim "$assets/LICENSE_FOXIT"
    echo
    heading "hayro-interpret asset provenance" "-"
    echo "$assets/README.md verbatim:"
    echo
    verbatim "$assets/README.md"
    echo
    heading "CMYK profile CGATS001Compat-v2-micro.icc (hayro-interpret)" "-"
    echo "Licence, $assets/CGATS_LICENSE.txt verbatim:"
    echo
    verbatim "$assets/CGATS_LICENSE.txt"
    echo
    heading "Lab profile LAB.icc (hayro-interpret)" "-"
    cat <<'EOF'
LAB.icc was generated with LCMS2 and states no licence of its own. It is
distributed under hayro-interpret's licence, Apache-2.0 OR MIT (Part 1).

EOF
    heading "ICC profiles (hayro-jpeg2000)" "-"
    echo "Embedded profiles, from $(label "$jp2_label" assets):"
    for f in "$jp2"/assets/*.icc; do echo "    ${f##*/}"; done
    echo
    echo "$(label "$jp2_label" assets/README.md) verbatim:"
    echo
    verbatim "$jp2/assets/README.md"
    echo
    echo "Licence, $(label "$jp2_label" assets/LICENSE.txt) verbatim:"
    echo
    verbatim "$jp2/assets/LICENSE.txt"
    echo
    heading "Adobe CMaps (hayro-cmap assets/cmaps.brotli, feature embed-cmaps)" "-"
    echo "Licence, $(label "$cmap_label" assets/LICENSE.txt) verbatim:"
    echo
    verbatim "$cmap/assets/LICENSE.txt"
    echo
    heading "Part 4. Vendored C/C++ libraries" "="
    cat <<'EOF'
These libraries are vendored under vendor/ and built from source for the OCR
engine.

EOF
    for lib in tesseract:Tesseract:LICENSE leptonica:Leptonica:leptonica-license.txt; do
        IFS=: read -r src title file <<<"$lib"
        # An assignment, so a failed lookup stops the script under set -e (a
        # substitution inside heading's argument would not).
        version=$(c_version "$src")
        heading "$title $version (vendor/$src)" "-"
        echo "Copyright holders named in the sources:"
        tools/license/check-c-licenses.sh --holders "$src" | sed 's/^/    /'
        echo
        echo "Licence, vendor/$src/$file verbatim:"
        echo
        verbatim "vendor/$src/$file"
        if [[ $src == tesseract ]]; then echo; fi
    done
    # leptonica-license.txt ends in a blank line, which separates Part 5.
    cat "$part5"
} >"$out"
chmod 644 "$out" "$copyright"
mv "$out" THIRD_PARTY_NOTICES
mv "$copyright" tools/license/rust-std-COPYRIGHT.html
