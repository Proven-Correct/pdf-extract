#!/usr/bin/env bash
# Tests for tools/release/build-public-history.sh and public-patch.pl (CI: the
# `rust` job; git, bash and perl only).
#
#   1. public-patch.pl on a crafted patch: the header's From: line names the
#      public identity and the mbox line is kept; references in the message
#      are qualified by ADR 0097's rules (runs, PR/issue words, already
#      qualified, HTML entities, numbers above 200); the diff is untouched;
#      a reference in the Subject, a second From: header and a file with no
#      "---" separator are refused.
#   2. The build on a synthetic internal repository (the real export-patches.sh
#      and lib-format-patch.sh, a stub test-check-patches.sh, two pristine
#      hayro imports, one C import, a patch in each range, a work author):
#      DEST gets the 3 + 2 + 1 commits, the four tags in place, one identity,
#      the internal author nowhere, qualified messages, SRC's tree outside the
#      patch files, and the same commit ids when built twice.
#   3. Each refusal, which must leave neither DEST nor a partial work directory:
#      DEST exists, a bad PUBLIC_IDENTITY, a missing tag, split C tags, a
#      series not numbered as export-patches.sh writes it, a reference in a
#      patch subject, a patch that does not apply, a vendored file changed
#      without a re-export, a tree that differs (an export-ignore attribute),
#      a patch file not named as export-patches.sh names it, an in-body From:
#      that authors a commit as someone else, a committed patch that does not
#      regenerate byte-identically, and a failing test-check-patches.sh;
#      KEEP_PARTIAL=1 keeps the refused work directory.
#   4. An inherited GIT_DIR (as a hook sets it) neither redirects the build
#      into SRC nor changes SRC.

set -euo pipefail

# The caller's git configuration must not reach the synthetic repositories.
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
export GIT_AUTHOR_NAME="Work Person" GIT_AUTHOR_EMAIL="work@corp.example"
export GIT_COMMITTER_NAME="Work Person" GIT_COMMITTER_EMAIL="work@corp.example"
export PUBLIC_IDENTITY="Pub Lic <pub@example.org>"

repo=$(cd "$(dirname "$0")/../.." && pwd)
build=$repo/tools/release/build-public-history.sh
filter=$repo/tools/release/public-patch.pl
test_root=$(mktemp -d)
trap 'rm -rf "$test_root"' EXIT
fail() { echo "  ✗ FAILED: $*" >&2; exit 1; }
pass() { echo "  ✓ $*"; }

# --- 1. public-patch.pl -------------------------------------------------------

echo "Test: public-patch.pl rewrites the From: header and qualifies message references"

cat > "$test_root/in.patch" <<'EOF'
From 0000000000000000000000000000000000000000 Mon Sep 17 00:00:00 2001
From: Work Person <work@corp.example>
Date: Thu, 1 Oct 2026 10:00:00 -0700
Subject: [PATCH 1/1] H9: fix the thing

Refs #12, #13 and PR #14. Issue #4 and issue #5/#6 too.
Already internal #7; non-internal #15; not &#8; nor #12345; see [x](#9) and ${#10}.
#11 opens a line; Part of #121, #77, #56; #250, or #251
---
 a.txt | 2 +-
 1 file changed, 1 insertion(+), 1 deletion(-)

diff --git a/a.txt b/a.txt
--- a/a.txt
+++ b/a.txt
@@ -1 +1 @@
-old #20
+new #21
EOF
cat > "$test_root/want.patch" <<'EOF'
From 0000000000000000000000000000000000000000 Mon Sep 17 00:00:00 2001
From: Pub Lic <pub@example.org>
Date: Thu, 1 Oct 2026 10:00:00 -0700
Subject: [PATCH 1/1] H9: fix the thing

Refs internal #12, #13 and internal PR #14. Internal issue #4 and internal issue #5/#6 too.
Already internal #7; non-internal internal #15; not &#8; nor #12345; see [x](#9) and ${#10}.
internal #11 opens a line; Part of internal #121, #77, #56; internal #250, or #251
---
 a.txt | 2 +-
 1 file changed, 1 insertion(+), 1 deletion(-)

diff --git a/a.txt b/a.txt
--- a/a.txt
+++ b/a.txt
@@ -1 +1 @@
-old #20
+new #21
EOF
"$filter" "$PUBLIC_IDENTITY" < "$test_root/in.patch" > "$test_root/got.patch"
diff "$test_root/want.patch" "$test_root/got.patch" || fail "public-patch.pl output differs"
pass "From: replaced, mbox line kept, message qualified, diff untouched"

# A commit with a subject and no body: the message is empty, so a "---" line
# inside the diff must not be taken for the separator.
cat > "$test_root/nobody.patch" <<'EOF'
From 0000000000000000000000000000000000000000 Mon Sep 17 00:00:00 2001
From: Work Person <work@corp.example>
Date: Thu, 1 Oct 2026 10:00:00 -0700
Subject: [PATCH 1/1] H9: subject only

---
 a.txt | 2 +-

diff --git a/a.txt b/a.txt
--- a/a.txt
+++ b/a.txt
@@ -1,2 +1,2 @@
 x #5
---
+--- #6
EOF
"$filter" "$PUBLIC_IDENTITY" < "$test_root/nobody.patch" > "$test_root/got.patch"
diff <(sed 's/^From: Work Person <work@corp.example>$/From: Pub Lic <pub@example.org>/' "$test_root/nobody.patch") "$test_root/got.patch" ||
  fail "a subject-only patch changed beyond its From: line"
pass "a subject-only patch keeps its diff untouched"

# expect_filter_refusal NAME MESSAGE-SUBSTRING: the filter on $test_root/bad.patch.
expect_filter_refusal() {
  local err
  if err=$("$filter" "$PUBLIC_IDENTITY" < "$test_root/bad.patch" 2>&1 >/dev/null); then
    fail "$1: public-patch.pl accepted it"
  fi
  [[ $err == *"$2"* ]] || fail "$1: wrong refusal: $err"
  pass "refuses $1"
}
sed 's/^Subject: .*/Subject: [PATCH 1\/1] Fix #3/' "$test_root/in.patch" > "$test_root/bad.patch"
expect_filter_refusal "a reference in the Subject" "reference in the Subject"
perl -pe 's/^Date: /From: Other <o\@x.example>\nDate: /' "$test_root/in.patch" > "$test_root/bad.patch"
expect_filter_refusal "a second From: header" "exactly one From: header"
grep -v '^---$' "$test_root/in.patch" > "$test_root/bad.patch"
expect_filter_refusal "a file with no --- separator" "not a format-patch file"
if "$filter" "no brackets" < "$test_root/in.patch" >/dev/null 2>&1; then
  fail "public-patch.pl accepted an identity without <email>"
fi
pass "refuses an identity that is not 'Name <email>'"

# --- 2. The build on a synthetic internal repository --------------------------

# mk_src DIR [TCP_RC]: a stand-in internal repository whose main is a scrubbed
# snapshot with a committed two-patch series. TCP_RC is the stub
# test-check-patches.sh's exit status (default 0).
mk_src() {
  local d=$1 rc=${2:-0}
  git init -q -b main "$d"
  (
    cd "$d"
    mkdir -p vendor/hayro-syntax vendor/hayro-interpret vendor/hayro-postscript
    printf 'syntax 1\nsyntax 2\nsyntax 3\n' > vendor/hayro-syntax/lib.rs
    echo interpret > vendor/hayro-interpret/lib.rs
    echo postscript > vendor/hayro-postscript/lib.rs
    echo "internal provenance" > vendor/PROVENANCE.md
    git add -A && git commit -qm "Vendor pristine hayro 1"
    git tag vendor/hayro-pristine-1
    mkdir -p vendor/tesseract vendor/leptonica
    printf 'int t;\r\n' > vendor/tesseract/t.c
    echo 'int l;' > vendor/leptonica/l.c
    printf 'vendor/tesseract/** -text\n' > .gitattributes
    git add -A && git commit -qm "Vendor pristine C"
    git tag vendor/tesseract-pristine-1
    git tag vendor/leptonica-pristine-1
    sed -i.bak 's/syntax 2/syntax two/' vendor/hayro-syntax/lib.rs && rm vendor/hayro-syntax/lib.rs.bak
    mkdir -p crates && echo "test" > crates/t.rs
    git add -A && git commit -qm "H1: fix syntax" -m "Refs #12, #13 and PR #14."
    mkdir -p vendor/hayro-jbig2 vendor/hayro-jpeg2000
    printf 'jbig2 a\njbig2 b\n' > vendor/hayro-jbig2/lib.rs
    echo jpx > vendor/hayro-jpeg2000/lib.rs
    git add -A && git commit -qm "Vendor pristine codecs"
    git tag vendor/hayro-pristine-2
    sed -i.bak 's/jbig2 b/jbig2 bee/' vendor/hayro-jbig2/lib.rs && rm vendor/hayro-jbig2/lib.rs.bak
    git add -A && git commit -qm "H2: fix jbig2" -m "Fixes #7"
    mkdir -p tools/vendor
    cp "$repo/tools/vendor/export-patches.sh" "$repo/tools/vendor/lib-format-patch.sh" tools/vendor/
    printf '#!/usr/bin/env bash\necho stub; exit %s\n' "$rc" > tools/vendor/test-check-patches.sh
    chmod +x tools/vendor/test-check-patches.sh
    echo "public readme" > README.md
    echo "scrubbed" > vendor/PROVENANCE.md
    tools/vendor/export-patches.sh >/dev/null
    git add -A && git commit -qm "Scrubbed snapshot"
  )
}

echo "Test: the build reconstructs the history and passes its checks"
src=$test_root/src
mk_src "$src"
dest=$test_root/dest
"$build" "$src" "$dest" > "$test_root/build.log" || fail "build refused: $(cat "$test_root/build.log")"
g() { git -C "$dest" "$@"; }
[ "$(g rev-list --count HEAD)" = 6 ] || fail "expected 6 commits, got $(g rev-list --count HEAD)"
[ "$(g tag -l | tr '\n' ' ')" = "vendor/hayro-pristine-1 vendor/hayro-pristine-2 vendor/leptonica-pristine-1 vendor/tesseract-pristine-1 " ] ||
  fail "tags: $(g tag -l)"
[ "$(g rev-list --count vendor/hayro-pristine-1)" = 1 ] || fail "hayro-pristine-1 is not the root"
[ "$(g rev-parse vendor/tesseract-pristine-1^)" = "$(g rev-parse vendor/hayro-pristine-1)" ] || fail "C import placement"
[ "$(g rev-parse vendor/leptonica-pristine-1)" = "$(g rev-parse vendor/tesseract-pristine-1)" ] || fail "C tags differ"
[ "$(g rev-list --count vendor/tesseract-pristine-1..vendor/hayro-pristine-2^)" = 1 ] || fail "range 1 placement"
[ "$(g rev-list --count vendor/hayro-pristine-2..HEAD)" = 2 ] || fail "range 2 + snapshot placement"
[ "$(g log --format='%an <%ae>%n%cn <%ce>' | sort -u)" = "$PUBLIC_IDENTITY" ] || fail "identities: $(g log --format='%an <%ae> %cn <%ce>' | sort -u)"
pass "6 commits, the four tags on the imports, one identity"
# To a file first: `git log | grep -q` under pipefail fails when grep exits
# early and git takes SIGPIPE, which would read as "not found".
g log --all -p --format='%an %ae %cn %ce%n%B' > "$test_root/history.txt"
if grep -q -e 'work@corp' -e 'Work Person' -e 'internal provenance' "$test_root/history.txt"; then
  fail "internal author or tag-time PROVENANCE.md reached DEST"
fi
[ "$(g show -s --format=%b vendor/hayro-pristine-2^)" = "Refs internal #12, #13 and internal PR #14." ] ||
  fail "range-1 message: $(g show -s --format=%b vendor/hayro-pristine-2^)"
[ "$(g show -s --format=%b HEAD^)" = "Fixes internal #7" ] || fail "range-2 message: $(g show -s --format=%b HEAD^)"
g show vendor/hayro-pristine-1 --stat --format= | grep -q PROVENANCE && fail "import commit carries PROVENANCE.md"
grep -q '^From: Pub Lic <pub@example.org>$' "$dest"/vendor/patches/hayro/0001-*.patch || fail "0001 From: line"
pass "internal author absent; messages qualified; imports hold only the crates"
diff <(git -C "$src" ls-tree -r main | grep -v 'vendor/patches/hayro/') <(g ls-tree -r HEAD | grep -v 'vendor/patches/hayro/') ||
  fail "tree differs outside the patch files"
[ "$(git -C "$src" rev-parse main:vendor/tesseract/t.c)" = "$(g rev-parse HEAD:vendor/tesseract/t.c)" ] || fail "CRLF file changed"
pass "head tree equals SRC's outside the patch files (CRLF intact)"
"$build" "$src" "$test_root/dest2" >/dev/null || fail "second build refused"
[ "$(g rev-parse HEAD)" = "$(git -C "$test_root/dest2" rev-parse HEAD)" ] || fail "two builds give different commit ids"
pass "two builds give the same commit ids"

# --- 3. Refusals --------------------------------------------------------------

# expect_refusal NAME MESSAGE-SUBSTRING SRC [DEST]: the build must exit 1 with
# the message and leave neither DEST nor a partial directory beside it.
expect_refusal() {
  local name=$1 want=$2 s=$3 d=${4:-$test_root/out} err
  if err=$("$build" "$s" "$d" 2>&1 >/dev/null); then fail "$name: build accepted it"; fi
  [[ $err == *"$want"* ]] || fail "$name: wrong refusal: $err"
  if [ "$d" = "$test_root/out" ] && [ -e "$d" ]; then fail "$name: DEST left behind"; fi
  if compgen -G "$d.partial.*" >/dev/null; then fail "$name: partial directory left behind"; fi
  pass "refuses $name"
}
# variant NAME: a fresh clone of the good SRC (all tags) to break.
variant() {
  git clone -q "$src" "$test_root/$1"
  echo "$test_root/$1"
}

echo "Test: refusal paths"
expect_refusal "an existing DEST" "already exists" "$src" "$dest"
PUBLIC_IDENTITY="nobody" expect_refusal "a bad PUBLIC_IDENTITY" "must read 'Name <email>'" "$src"

v=$(variant notag)
git -C "$v" tag -d vendor/hayro-pristine-2 >/dev/null
expect_refusal "a missing tag" "tag vendor/hayro-pristine-2 not found" "$v"

v=$(variant splitc)
git -C "$v" tag -f vendor/leptonica-pristine-1 vendor/hayro-pristine-1 >/dev/null
expect_refusal "split C tags" "name different commits" "$v"

# commit_edit VARIANT PERL-EXPR: apply PERL-EXPR to the committed patch files
# in the variant's main and commit the result.
commit_edit() {
  (cd "$1" && perl -pi -e "$2" vendor/patches/hayro/*.patch && git commit -qam "edit")
}
v=$(variant numbering)
commit_edit "$v" 's{\[PATCH 2/2\]}{[PATCH 2/9]}'
expect_refusal "a series numbered unlike export-patches.sh" "is not [PATCH 2/2]" "$v"

v=$(variant subject)
# shellcheck disable=SC2016 # $1 is perl's capture, not a shell variable
commit_edit "$v" 's{^(Subject: \[PATCH 2/2\] H2: fix jbig2)$}{$1 #7}'
expect_refusal "a reference in a patch subject" "reference in the Subject" "$v"

v=$(variant noapply)
commit_edit "$v" 's{^-jbig2 b$}{-jbig2 zzz}'
expect_refusal "a patch that does not apply" "git am failed on 0002" "$v"

v=$(variant drift)
(cd "$v" && echo drift >> vendor/hayro-syntax/lib.rs && git commit -qam "unexported vendor change")
expect_refusal "a vendored change without a re-export" "the snapshot changes vendored files" "$v"

v=$(variant stat)
commit_edit "$v" 's{^ 1 file changed}{ 9 files changed}'
expect_refusal "a committed patch that does not regenerate" "beyond From: and the qualifiers" "$v"

v=$(variant ignore)
(cd "$v" && echo 'README.md export-ignore' >> .gitattributes && git commit -qam "export-ignore")
expect_refusal "a tree that differs from SRC's" "DEST's tree differs" "$v"

v=$(variant rename)
(cd "$v" && git mv vendor/patches/hayro/0002-*.patch vendor/patches/hayro/0002-renamed.patch && git commit -qm "rename")
expect_refusal "a patch file named unlike export-patches.sh" "different files" "$v"

v=$(variant inbody)
commit_edit "$v" 's{^Fixes #7$}{From: Other Person <other\@corp.example>\n\nFixes #7}'
expect_refusal "an in-body From: authoring a commit" "identities other than" "$v"

v=$test_root/tcpfail
mk_src "$v" 1
expect_refusal "a failing test-check-patches.sh" "test-check-patches.sh failed in DEST" "$v"
if KEEP_PARTIAL=1 "$build" "$v" "$test_root/kept" >/dev/null 2>&1; then fail "KEEP_PARTIAL build accepted"; fi
compgen -G "$test_root/kept.partial.*" >/dev/null || fail "KEEP_PARTIAL=1 did not keep the work directory"
[ ! -e "$test_root/kept" ] || fail "KEEP_PARTIAL=1 created DEST"
pass "KEEP_PARTIAL=1 keeps the refused work directory, and still no DEST"

echo "Test: an inherited GIT_DIR does not reach SRC"
before=$(git -C "$src" rev-parse HEAD; git -C "$src" config --local --list; git -C "$src" status --porcelain)
GIT_DIR=$src/.git GIT_WORK_TREE=$src GIT_INDEX_FILE=$src/.git/index \
  "$build" "$src" "$test_root/envdest" >/dev/null || fail "build refused under an inherited GIT_DIR"
after=$(git -C "$src" rev-parse HEAD; git -C "$src" config --local --list; git -C "$src" status --porcelain)
[ "$before" = "$after" ] || fail "SRC changed: $before vs $after"
[ "$(git -C "$test_root/envdest" rev-parse HEAD)" = "$(g rev-parse HEAD)" ] || fail "the build under GIT_DIR differs"
pass "SRC unchanged, and the same DEST"

echo
echo "All tests passed"
