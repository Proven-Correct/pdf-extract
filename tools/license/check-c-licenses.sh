#!/usr/bin/env bash
# Licence check of the vendored C/C++ trees, vendor/tesseract and
# vendor/leptonica (spec §3.7, CI job `deny`). cargo-deny and cargo-about see
# only Rust crates; this is their counterpart for the C/C++ sources, and
# gen-notices.sh names the holders it reports in THIRD_PARTY_NOTICES Part 4.
#
# Usage: tools/license/check-c-licenses.sh [--root DIR]
#        tools/license/check-c-licenses.sh --holders SOURCE [--root DIR]
#        tools/license/check-c-licenses.sh --self-test
#
# Two checks over every text file of DIR/tesseract and DIR/leptonica (DIR
# defaults to vendor/):
#
# 1. Copyright holders. Every line that reads as a copyright notice (the word
#    "copyright", "(c)" or "©" together with a year) must name one of the
#    holders expected for its tree (the "(c)" and "©" forms only directly
#    before a year), and the set of holders found must equal
#    the expected set exactly: a new holder fails, and so does an expected one
#    that has gone (the notices heading would then be stale). Tesseract has a
#    few notices that give a year and no name, all under Apache headers; they
#    are their own expected category.
# 2. Licence keywords. No line may name a licence other than Apache-2.0 or
#    BSD (GPL/LGPL/AGPL, MPL, CDDL, EPL, CC, public domain, MIT, ISC, zlib,
#    ...). Two accepted phrases (Tesseract's Apache SPDX tag and its Apache
#    header line) and a short list of exact (file, text) pairs are removed
#    from a line before it is re-tested, so a line that pairs an accepted
#    phrase with another licence still fails.
#
# --holders SOURCE prints the named holders of one tree, one per line, after
# both checks pass. --self-test runs the checks on a scratch copy of vendor/
# with a planted GPL string, a planted foreign holder, a planted dual SPDX tag
# and a removed holder, and fails unless each plant is caught and the clean
# copy passes.
set -euo pipefail
# Byte-wise matching and sorting whatever the caller's locale; "©" is matched
# as its UTF-8 bytes.
export LC_ALL=C

repo=$(cd "$(dirname "$0")/../.." && pwd)

# The tables below are exported for the awk programs, which read ENVIRON.
export HOLDERS KEYWORDS ACCEPT_ANY ACCEPT_AT

# Expected holders: "source|label|pattern", the pattern an awk ERE matched
# against the lower-cased notice line. Order is the order --holders prints.
# The label "-" is the year-only category (notices that name nobody).
HOLDERS='tesseract|Google Inc.|google,? inc
tesseract|Hewlett-Packard|hewlett-packard
tesseract|Artifex Software Inc.|artifex software
tesseract|Institute of Software, Chinese Academy of Sciences (ISCAS)|institute of software chinese academy of sciences
tesseract|-|
leptonica|Leptonica|leptonica\.
leptonica|Milner Technologies, Inc.|milner technologies'

# Licence keywords, an ERE for `grep -i` and for awk on the lower-cased line.
# Word boundaries are spelled as [^a-z] so the pattern means the same to GNU
# grep, BSD grep and awk ("gplot" and "CopyLeftTab" must not match).
KEYWORDS='(^|[^a-z])(l|a)?gpl(v?[0-9]|[^a-z]|$)|general public licen|lesser general|affero|mozilla public|(^|[^a-z])mpl-?[12]|cddl|eclipse public|creative commons|public domain|(^|[^a-z])cc0|cc-by|unlicense|wtfpl|mit licen|isc licen|zlib licen|boost software|artistic licen|(^|[^a-z])copyleft([^a-z]|$)|openssl licen|not evil|commons clause|business source|server side public|eupl|european union public|spdx-license-identifier|licen[cs]ed under'

# Phrases accepted anywhere (removed before the line is re-tested).
ACCEPT_ANY='SPDX-License-Identifier: Apache-2.0
Licensed under the Apache License, Version 2.0'

# Exact "path|text" pairs accepted in one file only. Leptonica's red-black
# tree is adapted from code its author placed in the public domain (CC0 1.0);
# the files themselves carry Leptonica's BSD-2-Clause header, and CC0 asks
# nothing of a redistributor.
ACCEPT_AT='leptonica/src/rbtree.c|which has been placed in the public domain under the Creative Commons
leptonica/src/rbtree.c|CC0 1.0 waiver (http://creativecommons.org/publicdomain/zero/1.0/).
leptonica/src/rbtree.h|which has been placed in the public domain under the Creative Commons
leptonica/src/rbtree.h|CC0 1.0 waiver (http://creativecommons.org/publicdomain/zero/1.0/).'

# check ROOT [HOLDERS_OF]: run both checks on ROOT/{tesseract,leptonica};
# print a report, or with HOLDERS_OF only that tree's named holders. Returns 1
# on any finding.
check() {
  local root=$1 holders_of=${2:-} status=0 src
  for src in tesseract leptonica; do
    [[ -d $root/$src ]] || { echo "error: $root/$src is not a directory" >&2; return 1; }
  done

  # 1. Copyright holders. grep -I skips binary files; the paths it prints are
  # relative to ROOT because it runs there.
  local notices
  notices=$(cd "$root" && grep -rIniE 'copyright|\(c\)|©' tesseract leptonica || true)
  # The tables reach awk through ENVIRON, not -v, which would interpret the
  # backslash escapes in the patterns.
  if ! WANT=$holders_of awk '
      BEGIN {
        want = ENVIRON["WANT"]
        n = split(ENVIRON["HOLDERS"], rows, "\n")
        for (i = 1; i <= n; i++) {
          split(rows[i], f, "|")
          src[i] = f[1]; label[i] = f[2]; pat[i] = f[3]
        }
      }
      {
        line = tolower($0)
        sub(/^[^:]*:[0-9]*:/, "", line)
        tree = $0; sub(/\/.*/, "", tree)
        if (line !~ /(19|20)[0-9][0-9]/) next
        # "(c)" or "©" counts only directly before a year, so a URL such as
        # ".../Red-black_tree_(C)?oldid=19567" is not a notice.
        if (line !~ /copyright|\(c\) *(19|20)[0-9][0-9]|© *(19|20)[0-9][0-9]/) next
        hit = 0
        for (i = 1; i <= n; i++) {
          if (src[i] != tree || label[i] == "-") continue
          if (line ~ pat[i]) { hit = i; break }
        }
        if (!hit) {
          # Year-only: nothing but comment marks, "(c)", "copyright", years
          # and punctuation once the holder-less forms are stripped.
          rest = line
          gsub(/copyright|\(c\)|©|(19|20)[0-9][0-9]|[^a-z]/, "", rest)
          for (i = 1; i <= n; i++)
            if (src[i] == tree && label[i] == "-" && rest == "") hit = i
        }
        if (hit) count[hit]++
        else { print "error: unexpected copyright holder: " $0 > "/dev/stderr"; bad = 1 }
      }
      END {
        for (i = 1; i <= n; i++) {
          if (!count[i]) {
            print "error: expected holder not found in " src[i] ": " label[i] > "/dev/stderr"
            bad = 1
          }
        }
        if (bad) exit 1
        if (want != "") {
          for (i = 1; i <= n; i++) if (src[i] == want && label[i] != "-") print label[i]
          exit 0
        }
        for (i = 1; i <= n; i++) {
          name = label[i] == "-" ? "(year only, no holder named)" : label[i]
          printf "%s: %s: %d notice line(s)\n", src[i], name, count[i]
        }
      }' <<<"$notices"; then
    status=1
  fi

  # 2. Licence keywords.
  local hits
  hits=$(cd "$root" && grep -rIniE "$KEYWORDS" tesseract leptonica || true)
  if ! QUIET=$holders_of awk '
      # strip(s, t): s with every literal occurrence of t removed.
      function strip(s, t,   i, out) {
        out = ""
        while ((i = index(s, t)) > 0) { out = out substr(s, 1, i - 1); s = substr(s, i + length(t)) }
        return out s
      }
      BEGIN {
        keywords = ENVIRON["KEYWORDS"]; quiet = ENVIRON["QUIET"]
        na = split(ENVIRON["ACCEPT_ANY"], anys, "\n")
        nt = split(ENVIRON["ACCEPT_AT"], ats, "\n")
        for (i = 1; i <= nt; i++) { j = index(ats[i], "|"); atp[i] = substr(ats[i], 1, j - 1); att[i] = substr(ats[i], j + 1) }
      }
      $0 != "" {
        path = $0; sub(/:.*/, "", path)
        text = $0; sub(/^[^:]*:[0-9]*:/, "", text)
        for (i = 1; i <= na; i++) text = strip(text, anys[i])
        for (i = 1; i <= nt; i++) if (atp[i] == path) { t = strip(text, att[i]); if (t != text) { used[i] = 1; text = t } }
        if (tolower(text) ~ keywords) { print "error: licence keyword: " $0 > "/dev/stderr"; bad = 1 }
      }
      END {
        # A stale exception is a finding too: the file it excused has changed.
        for (i = 1; i <= nt; i++) if (!used[i]) { print "error: unused exception: " ats[i] > "/dev/stderr"; bad = 1 }
        if (bad) exit 1
        if (quiet == "") printf "licence keywords: none besides Apache-2.0 / BSD (%d excepted line(s))\n", nt
      }' <<<"$hits"; then
    status=1
  fi
  return "$status"
}

# expect_fail WHAT ROOT: the check must reject ROOT.
expect_fail() {
  if check "$2" >/dev/null 2>&1; then
    echo "self-test FAILED: $1 was not caught" >&2
    exit 1
  fi
  echo "self-test: $1 caught"
}

self_test() {
  # Global, not local: the EXIT trap runs after this function has returned.
  scratch=$(mktemp -d)
  trap 'rm -rf "$scratch"' EXIT
  cp -R "$repo/vendor/tesseract" "$repo/vendor/leptonica" "$scratch/"
  check "$scratch" >/dev/null || { echo "self-test FAILED: the clean copy does not pass" >&2; exit 1; }
  echo "self-test: clean copy passes"

  local target=$scratch/leptonica/src/pix1.c
  cp "$target" "$scratch/pix1.c.orig"
  echo ' * This file is distributed under the GNU GPL, version 3.' >>"$target"
  expect_fail "a planted GPL string" "$scratch"
  cp "$scratch/pix1.c.orig" "$target"

  echo ' * Copyright (C) 2026 Example Corporation.' >>"$target"
  expect_fail "a planted foreign copyright holder" "$scratch"
  cp "$scratch/pix1.c.orig" "$target"

  echo '// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-only' >>"$scratch/tesseract/src/api/capi.cpp"
  expect_fail "an Apache SPDX tag paired with GPL" "$scratch"
  cp "$repo/vendor/tesseract/src/api/capi.cpp" "$scratch/tesseract/src/api/capi.cpp"

  rm "$scratch/leptonica/src/pngio.c"
  expect_fail "a removed expected holder (Milner, pngio.c)" "$scratch"
  cp "$repo/vendor/leptonica/src/pngio.c" "$scratch/leptonica/src/pngio.c"

  check "$scratch" >/dev/null || { echo "self-test FAILED: the restored copy does not pass" >&2; exit 1; }
  echo "self-test passed"
}

root=$repo/vendor
holders_of=
while [[ $# -gt 0 ]]; do
  case $1 in
    --root) root=${2:?--root needs a directory}; shift 2 ;;
    --holders)
      holders_of=${2:?--holders needs a source}; shift 2
      case $holders_of in tesseract | leptonica) ;; *) echo "error: unknown source $holders_of" >&2; exit 2 ;; esac
      ;;
    --self-test) self_test; exit 0 ;;
    *) echo "usage: $0 [--root DIR] [--holders SOURCE] | --self-test" >&2; exit 2 ;;
  esac
done
check "$root" "$holders_of"
