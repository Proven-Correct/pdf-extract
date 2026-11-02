#!/usr/bin/env bash
# Tests for the hayro patch tooling (CI: the `rust` job; needs full history
# and the pristine tags):
#
#   1. check-patches.sh itself, on a stand-in repository (stub import.sh, one
#      patch): with TMPDIR inside a git repository it applies the series and
#      passes; when `git apply --numstat` prints nothing (what git escaping to
#      the outer repository looked like; simulated with a stub git, since
#      current git no longer does it) the empty-numstat guard stops the run.
#   2. export-patches.sh, two ranges (run 0003 §3.2): in a synthetic history
#      with both pristine tags, the export numbers the two ranges continuously,
#      never exports the second import commit, exports each later commit once
#      (including a side branch merged across the second tag), drops the parts
#      of a commit outside vendor/, re-exports byte-identically, and the series
#      applied to the two pristine imports reproduces vendor/.
#   2b. The exporters' guards: BASE2 a merge, BASE1 not an ancestor of BASE2,
#      and a pristine tag that is not an ancestor of HEAD (a squash-merged
#      import) each fail loudly instead of exporting the wrong series; and a
#      hostile user git config (format.*, diff.*, core.abbrev) cannot change
#      the exported bytes. Its one-line appends give diff.orderFile,
#      core.attributesFile and diff.suppressBlankEmpty nothing to act on;
#      2c and 3 cover those three.
#   2c. format_patch_pinned on inputs each pin exists for (a rename, diffs
#      that the algorithm, indent heuristic, inter-hunk context, blank-line
#      suppression, file order and attributes driver change, a
#      note, a non-ASCII subject): first a check that git's unpinned output
#      really changes with each setting, then that the hostile config does not
#      change the pinned output, which also has zero hashes, no To:/Cc:, and
#      a/ b/ prefixes.
#   3. export-patches.sh on this repository: regenerating the series at HEAD
#      (in a throwaway clone, with the working-tree export-patches.sh and
#      lib-format-patch.sh copied in; the working tree is never touched), with and
#      without that hostile config, gives byte-identical files
#      (`git diff --exit-code`), after a check that git's unpinned output of the
#      real range does change under the order file, the attributes file and
#      diff.suppressBlankEmpty. Needs both pristine tags locally.

set -euo pipefail

# The caller's git configuration must not reach these tests: it would change
# the synthetic history (signing, default branch) and the bytes the export
# scripts write. Identity comes from the environment instead.
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
export GIT_AUTHOR_NAME="Test User" GIT_AUTHOR_EMAIL="test@example.com"
export GIT_COMMITTER_NAME="Test User" GIT_COMMITTER_EMAIL="test@example.com"

repo=$(cd "$(dirname "$0")/../.." && pwd)
test_root=$(mktemp -d)
trap 'rm -rf "$test_root"' EXIT
fail() { echo "  ✗ FAILED: $*" >&2; exit 1; }

# --- 1. check-patches.sh -----------------------------------------------------

echo "Test: check-patches.sh applies a series, and refuses a patch that matches no files"

# A stand-in repository: the real check-patches.sh, an import.sh that writes a
# one-file pristine tree (the real one fetches crates.io), and a vendor/ that
# is that tree plus the one patch. It is a git repository with TMPDIR inside
# it, the layout in which git once resolved the patch against the outer repo.
fake="$test_root/fake"
mkdir -p "$fake/tools/vendor" "$fake/vendor/patches/hayro" \
  "$fake/vendor/hayro-syntax/src" "$fake/tmp" "$test_root/bin"
cp "$repo/tools/vendor/check-patches.sh" "$fake/tools/vendor/"
cat > "$fake/tools/vendor/import.sh" <<'EOF'
#!/usr/bin/env bash
mkdir -p "$1/hayro-syntax/src"
echo "target file" > "$1/hayro-syntax/src/lib.rs"
EOF
chmod +x "$fake/tools/vendor/import.sh"
echo "provenance" > "$fake/vendor/PROVENANCE.md"
echo "modified file" > "$fake/vendor/hayro-syntax/src/lib.rs"
cat > "$fake/vendor/patches/hayro/0001-change.patch" <<'EOF'
--- a/hayro-syntax/src/lib.rs
+++ b/hayro-syntax/src/lib.rs
@@ -1 +1 @@
-target file
+modified file
EOF
git -C "$fake" init -q -b main
TMPDIR="$fake/tmp" "$fake/tools/vendor/check-patches.sh" |
  grep -q '^vendor/ matches pristine + 1 patch(es)$' ||
  fail "check-patches.sh did not pass a series that reproduces vendor/"
echo "  ✓ a correct series passes, with TMPDIR inside a git repository"

# A git whose `apply --numstat` prints nothing and succeeds; every other
# command is the real git. Without the guard, the real `git apply` that
# follows would succeed and the run would pass.
real_git=$(command -v git)
cat > "$test_root/bin/git" <<EOF
#!/usr/bin/env bash
for arg in "\$@"; do [[ \$arg == --numstat ]] && exit 0; done
exec "$real_git" "\$@"
EOF
chmod +x "$test_root/bin/git"
if err=$(PATH="$test_root/bin:$PATH" TMPDIR="$fake/tmp" \
  "$fake/tools/vendor/check-patches.sh" 2>&1 >/dev/null); then
  fail "check-patches.sh accepted a patch whose numstat was empty"
fi
grep -q 'patch did not match any files: vendor/patches/hayro/0001-change.patch' <<<"$err" ||
  fail "check-patches.sh failed without the empty-numstat error: $err"
echo "  ✓ an empty numstat stops check-patches.sh"

# The GIT_CEILING_DIRECTORIES export in check-patches.sh is not tested: on
# current git the escape it guards against no longer reproduces, so no test
# could fail without it (see the comment there).

# --- 2. two-range export ---------------------------------------------------

echo ""
echo "Test: export-patches.sh exports two ranges, numbered continuously"

# Fixed dates and no signing, so the synthetic history is reproducible.
tick=1700000000
# commit MESSAGE: commit everything staged, one second after the last commit,
# so commit order is date order.
commit() {
  tick=$((tick + 1))
  GIT_AUTHOR_DATE="@$tick +0000" GIT_COMMITTER_DATE="@$tick +0000" \
    git -c commit.gpgsign=false commit -q -m "$1"
}

synth="$test_root/synth"
mkdir -p "$synth/tools/vendor"
cd "$synth"
git init -q -b main
cp "$repo/tools/vendor/export-patches.sh" "$repo/tools/vendor/export-c-patches.sh" \
  "$repo/tools/vendor/lib-format-patch.sh" tools/vendor/
for crate in hayro-syntax hayro-interpret hayro-postscript; do
  mkdir -p "vendor/$crate/src"
  printf 'pristine %s\n' "$crate" > "vendor/$crate/src/lib.rs"
done
git add -A
commit "Vendor pristine old crates"
git tag vendor/hayro-pristine-1

mkdir -p crates/core/tests
echo "H1 syntax" >> vendor/hayro-syntax/src/lib.rs
echo "test of H1" > crates/core/tests/h1.rs
git add -A
commit "H1: change syntax"
echo "not vendor" > README
git add -A
commit "Outside vendor only"
echo "H2 interpret" >> vendor/hayro-interpret/src/lib.rs
git add -A
commit "H2: change interpret"

# A side branch forked before the second import and merged after it: its
# commit is not an ancestor of the import commit, so it belongs to range 2.
git checkout -q -b side
echo "H3 postscript" >> vendor/hayro-postscript/src/lib.rs
git add -A
commit "H3: change postscript on a side branch"
git checkout -q -

for crate in hayro-jbig2 hayro-jpeg2000; do
  mkdir -p "vendor/$crate/src"
  printf 'pristine %s\n' "$crate" > "vendor/$crate/src/lib.rs"
done
git add -A
commit "Vendor pristine codec crates"
git tag vendor/hayro-pristine-2

echo "H4 jbig2" >> vendor/hayro-jbig2/src/lib.rs
echo "H4 syntax" >> vendor/hayro-syntax/src/lib.rs
git add -A
commit "H4: change jbig2 and syntax"
tick=$((tick + 1))
GIT_AUTHOR_DATE="@$tick +0000" GIT_COMMITTER_DATE="@$tick +0000" \
  git -c commit.gpgsign=false merge -q --no-ff -m "Merge side" side
echo "H5 jpeg2000" >> vendor/hayro-jpeg2000/src/lib.rs
git add -A
commit "H5: change jpeg2000"

out=vendor/patches/hayro
tools/vendor/export-patches.sh >/dev/null

# Two in range 1 (H1, H2), three in range 2 (H3, H4, H5); numbering continues.
names=()
while IFS= read -r f; do names+=("$f"); done < <(
  find "$out" -maxdepth 1 -name '*.patch' -exec basename {} \; | LC_ALL=C sort)
[[ ${#names[@]} -eq 5 ]] || fail "expected 5 patches, got ${#names[@]}: ${names[*]}"
want=(0001-H1-change-syntax.patch 0002-H2-change-interpret.patch)
[[ ${names[0]} == "${want[0]}" && ${names[1]} == "${want[1]}" ]] ||
  fail "range 1 is not 0001 H1, 0002 H2: ${names[*]}"
for f in "${names[@]:2}"; do
  [[ $f == 000[345]-H[345]-* ]] || fail "range 2 file $f is not numbered 0003-0005"
done
echo "  ✓ five patches, 0001-0002 from range 1, 0003-0005 from range 2"

# Every patch, not a sample: file NNNN must say [PATCH N/total] with the total
# of its own range (range 2's total includes the range-1 offset).
for f in "${names[@]}"; do
  n=$((10#${f%%-*}))
  total=2
  if [[ $n -gt 2 ]]; then total=5; fi
  grep -q "^Subject: \[PATCH $n/$total\] " "$out/$f" ||
    fail "$f header is not [PATCH $n/$total]: $(grep -m1 '^Subject:' "$out/$f")"
done
echo "  ✓ headers count within each range (range 1 n/2, range 2 n/5)"

# Each later commit is exported exactly once; the import commit and the
# commit outside vendor/ are not exported; the vendor/ prefix is stripped.
for change in "H1 syntax" "H2 interpret" "H3 postscript" "H4 jbig2" "H4 syntax" "H5 jpeg2000"; do
  n=$(cat "$out"/*.patch | grep -c "^+$change\$" || true)
  [[ $n -eq 1 ]] || fail "change '$change' exported $n times"
done
if grep -l 'new file mode' "$out"/*.patch; then fail "an import was exported"; fi
if grep -l 'crates/core\|README' "$out"/*.patch; then fail "a non-vendor path was exported"; fi
for crate in hayro-jbig2 hayro-jpeg2000; do
  # Not `cat ... | grep -q`: grep -q exits at its first match, and under
  # pipefail cat's SIGPIPE on the files it has not yet written fails the
  # pipeline intermittently. grep reads the files itself.
  grep -q "^+++ b/$crate/src/lib.rs\$" "$out"/0003-* "$out"/0004-* "$out"/0005-* ||
    fail "range 2 paths for $crate are not relative to vendor/"
done
echo "  ✓ each change exported once; imports and non-vendor paths are not"

git add -A
commit "Export patches"
tools/vendor/export-patches.sh >/dev/null
git diff --exit-code --quiet -- "$out" || fail "re-export is not byte-identical"
[[ -z $(git status --porcelain -- "$out") ]] || fail "re-export left untracked files"
echo "  ✓ re-export is byte-identical"

# Reproduce vendor/ the way check-patches.sh does: the old crates as of
# tag 1, the codec crates as of tag 2, then every patch in name order.
pristine="$test_root/pristine"
mkdir -p "$pristine"
git archive vendor/hayro-pristine-1 vendor/hayro-syntax vendor/hayro-interpret \
  vendor/hayro-postscript | tar -x -C "$pristine"
git archive vendor/hayro-pristine-2 vendor/hayro-jbig2 vendor/hayro-jpeg2000 |
  tar -x -C "$pristine"
head_tree="$test_root/head"
mkdir -p "$head_tree"
git archive HEAD vendor | tar -x -C "$head_tree"
rm -r "$head_tree/vendor/patches"
for patch in "$synth/$out"/*.patch; do
  GIT_CEILING_DIRECTORIES="$test_root" git -C "$pristine/vendor" apply "$patch"
done
diff -r "$pristine/vendor" "$head_tree/vendor" || fail "series does not reproduce vendor/"
echo "  ✓ the two pristine imports plus the series reproduce vendor/"

tag2=$(git rev-parse vendor/hayro-pristine-2)
git tag -d vendor/hayro-pristine-2 >/dev/null
if tools/vendor/export-patches.sh >/dev/null 2>&1; then
  fail "export succeeded without the second pristine tag"
fi
echo "  ✓ a missing pristine tag is an error"
git tag vendor/hayro-pristine-2 "$tag2"

echo ""
echo "Test passed: two-range export"

# --- 2b. guards and config independence ------------------------------------

echo ""
echo "Test: export scripts' guards and independence from the caller's git config"

# expect_error PATTERN CMD...: CMD must exit non-zero with PATTERN on stderr,
# and must leave the exported series untouched (a guard fires before the
# series is rewritten).
expect_error() {
  local pat=$1 err
  shift
  if err=$("$@" 2>&1 >/dev/null); then fail "expected failure: $*"; fi
  grep -q -- "$pat" <<<"$err" || fail "error lacks '$pat': $err"
  [[ -z $(git status --porcelain -- "$out") ]] || fail "a refused export changed $out"
}

# Every setting that changes `git format-patch` output, set in a user-level
# config. The exporters must produce the bytes they produce without it.
# diff.renames is false, not copies: the inputs in 2c have a rename, not a
# copy. diff.srcPrefix/dstPrefix (git 2.45+) are the prefix settings current
# format-patch honours; on git 2.56 noprefix and mnemonicPrefix alone leave
# it on a/ b/ (but see 2c: mnemonicPrefix overrides srcPrefix).
hostile="$test_root/hostile.gitconfig"
cat > "$hostile" <<'EOF'
[format]
	thread = deep
	subjectPrefix = RFC
	coverLetter = true
	headers = X-Hostile: 1
	from = Hostile <hostile@example.com>
	signOff = true
	attach = BOUNDARY
	signature = hostile signature
	numbered = false
	to = a@example.com
	cc = b@example.com
	suffix = .txt
	useAutoBase = true
	notes = true
	filenameMaxLength = 20
	encodeEmailHeaders = false
[diff]
	noprefix = true
	mnemonicPrefix = true
	algorithm = patience
	context = 1
	interHunkContext = 5
	indentHeuristic = false
	renames = false
	srcPrefix = x/
	dstPrefix = y/
	suppressBlankEmpty = true
[core]
	abbrev = 12
EOF
# The two file-valued keys need real files (so, not the quoted heredoc above).
# The order file moves new.txt, *.toml, *.md and */lib.rs ahead of the
# alphabetical order (an order file's `*` also matches `/`, so */lib.rs moves
# a crate's lib.rs ahead of its src/decode/... files in the real series); the
# attributes file gives *.rs git's rust funcname pattern, which picks a
# different hunk header than the default one.
order_file="$test_root/hostile.orderfile"
printf '%s\n' new.txt '*.toml' '*.md' '*/lib.rs' > "$order_file"
attrs_file="$test_root/hostile.attributes"
echo '*.rs diff=rust' > "$attrs_file"
git config --file "$hostile" diff.orderFile "$order_file"
git config --file "$hostile" core.attributesFile "$attrs_file"
GIT_CONFIG_GLOBAL="$hostile" tools/vendor/export-patches.sh >/dev/null
git diff --exit-code --quiet -- "$out" || fail "a user git config changed the exported bytes"
[[ -z $(git status --porcelain -- "$out") ]] || fail "a user git config changed the exported file set"
echo "  ✓ format.*, diff.* and core.abbrev in the user's config do not change the export"

# A pristine tag on a commit outside HEAD's history: what a squash-merged or
# rebased import leaves behind. BASE1 is an ancestor of it and it is not a
# merge, so only the HEAD guard can catch it; without that guard the export
# succeeds and writes the wrong series.
home=$(git symbolic-ref --short HEAD)
git checkout -q --detach vendor/hayro-pristine-1
for crate in hayro-jbig2 hayro-jpeg2000; do
  mkdir -p "vendor/$crate/src"
  printf 'pristine %s\n' "$crate" > "vendor/$crate/src/lib.rs"
done
git add -A
commit "Squashed codec import, not in HEAD's history"
stranded=$(git rev-parse HEAD)
git checkout -q "$home"
git tag -f vendor/hayro-pristine-2 "$stranded" >/dev/null
expect_error "not an ancestor of HEAD.*merge commit" tools/vendor/export-patches.sh
git tag -f vendor/hayro-pristine-2 "$tag2" >/dev/null
echo "  ✓ export-patches.sh refuses a BASE2 outside HEAD's history"

# BASE2 a merge commit: BASE2^ would name only its first parent.
merge=$(git rev-list --merges -1 HEAD)
expect_error "is a merge" tools/vendor/export-patches.sh vendor/hayro-pristine-1 "$merge"
echo "  ✓ export-patches.sh refuses a merge commit as BASE2"

# BASE1 not an ancestor of BASE2: the side branch forked before the second
# import but is not part of its history.
expect_error "side is not an ancestor of vendor/hayro-pristine-2" \
  tools/vendor/export-patches.sh side vendor/hayro-pristine-2
echo "  ✓ export-patches.sh refuses a BASE1 that is not an ancestor of BASE2"

# The C exporter has the same HEAD guard. An empty series (no vendor/tesseract
# in this synthetic history) is valid, so a tag inside HEAD's history succeeds.
git tag vendor/tesseract-pristine-1 "$stranded"
expect_error "not an ancestor of HEAD.*merge commit" tools/vendor/export-c-patches.sh tesseract
git tag -f vendor/tesseract-pristine-1 vendor/hayro-pristine-1 >/dev/null
tools/vendor/export-c-patches.sh tesseract | grep -q '^exported 0 patch(es)' ||
  fail "export-c-patches.sh did not export an empty series from an ancestor tag"
echo "  ✓ export-c-patches.sh refuses a base outside HEAD's history"

echo ""
echo "Test passed: guards and config independence"

# --- 2c. inputs that every output-affecting pin acts on ----------------------

echo ""
echo "Test: format_patch_pinned on inputs its pins act on"

# The 2/2b history has no renames, multi-hunk diffs, notes or non-ASCII
# subjects, so removing the pins for those changed nothing there. Each file
# here is a minimal input on which git's output differs with one setting:
# algo.txt (myers vs patience), indent.txt (indent heuristic on vs off),
# hunks.txt (two changes 9 lines apart: two hunks, or one under
# interHunkContext=5), old.txt renamed to new.txt; canary.rs has a blank
# context line (diff.suppressBlankEmpty) and a hunk header that the default
# funcname pattern and the rust driver (core.attributesFile) pick differently.
# The commit touches several files, which diff.orderFile reorders.
pins="$test_root/pins"
mkdir -p "$pins"
cd "$pins"
git init -q -b main
printf '%s\n' d b c c > algo.txt
printf '%s\n' a b a '  y' > indent.txt
seq 1 20 > hunks.txt
echo "renamed" > old.txt
canary() {
  printf '%s\n' 'fn outer() {' '    fn inner() {' '        let a = 1;' '        let b = 2;' \
    '        let c = 3;' '        let d = 4;' '        let e = 5;' '' "        let f = $1;" \
    '    }' '}' > canary.rs
}
canary 6
git add -A
commit "Base"
base=$(git rev-parse HEAD)
printf '%s\n' d c b c > algo.txt
printf '%s\n' a '  y' > indent.txt
seq 1 20 | sed 's/^2$/two/;s/^12$/twelve/' > hunks.txt
git mv old.txt new.txt
canary 7
git add -A
commit "Change the diff shapes and rename"
echo "added" > ascii.txt
git add -A
commit "Ünïcödé subject"
git notes add -m "a note"

# git's own output must change with each setting, or the comparison below
# proves nothing about that pin. Only the settings whose effect depends on the
# input are checked; prefix and recipient settings act on any patch (and
# diff.srcPrefix needs git 2.45+). Each output is assigned before it is
# compared, so a failing git stops the test instead of differing vacuously.
plain=$(git format-patch --stdout "$base..HEAD")
for setting in diff.algorithm=patience diff.indentHeuristic=false \
  diff.interHunkContext=5 diff.renames=false format.notes=true \
  diff.suppressBlankEmpty=true "diff.orderFile=$order_file" \
  "core.attributesFile=$attrs_file"; do
  out=$(git -c "$setting" format-patch --stdout "$base..HEAD")
  [[ $out != "$plain" ]] || fail "the 2c inputs do not exercise $setting"
done
# Compared with each other, not with the default, which is one of the two.
out=$(git -c format.encodeEmailHeaders=true format-patch --stdout "$base..HEAD")
raw=$(git -c format.encodeEmailHeaders=false format-patch --stdout "$base..HEAD")
[[ $out != "$raw" ]] || fail "the 2c inputs do not exercise format.encodeEmailHeaders"
echo "  ✓ git's unpinned output changes with each setting on these inputs"

# shellcheck source=tools/vendor/lib-format-patch.sh
. "$repo/tools/vendor/lib-format-patch.sh"
# --numbered, as the exporters pass it (numbering is the caller's choice).
format_patch_pinned --numbered -o "$test_root/pins-plain" "$base..HEAD"
# diff.mnemonicPrefix=true makes format-patch write a/ b/ even over
# diff.srcPrefix/dstPrefix, so under the full hostile config a run with no
# prefix pin at all still writes a/ b/. Without it, the prefix pins are tested.
hostile_pins="$test_root/hostile-pins.gitconfig"
printf '[include]\n\tpath = %s\n[diff]\n\tmnemonicPrefix = false\n' "$hostile" > "$hostile_pins"
GIT_CONFIG_GLOBAL="$hostile_pins" format_patch_pinned --numbered -o "$test_root/pins-hostile" "$base..HEAD"
# Explicit checks first, so a failure names the pin. The two pairs that cover
# one another (lib-format-patch.sh) are checked as pairs: To:/Cc: absent,
# a/ b/ prefixes present.
for f in "$test_root"/pins-hostile/*.patch; do
  [[ $(head -1 "$f") == "From 0000000000000000000000000000000000000000 Mon Sep 17 00:00:00 2001" ]] ||
    fail "${f##*/} does not start with a zero commit hash"
done
if grep -l -E '^(To|Cc):' "$test_root"/pins-hostile/*.patch; then
  fail "format.to/format.cc reached the patch headers"
fi
for line in '--- a/algo.txt' '+++ b/algo.txt'; do
  grep -qxF -- "$line" "$test_root"/pins-hostile/0001-*.patch ||
    fail "the hostile config changed the a/ b/ prefixes (no '$line')"
done
diff -r "$test_root/pins-plain" "$test_root/pins-hostile" ||
  fail "the hostile config changed format_patch_pinned's output"
echo "  ✓ the hostile config changes none of it: zero hashes, no To:/Cc:, a/ b/ prefixes"

echo ""
echo "Test passed: pins on inputs they act on"

# --- 3. this repository's series regenerates byte-identically --------------

echo ""
echo "Test: export-patches.sh regenerates this repository's series unchanged"
cd "$repo"
for tag in vendor/hayro-pristine-1 vendor/hayro-pristine-2; do
  git rev-parse --verify --quiet "$tag^{commit}" >/dev/null ||
    fail "tag $tag missing (git fetch --tags)"
done
clone="$test_root/clone"
git clone -q --no-checkout "$repo" "$clone"
git -C "$clone" checkout -q --detach "$(git rev-parse HEAD)"
# The scripts under test are the working-tree ones, not HEAD's, so an
# uncommitted change to the exporter or its pins is checked against the
# committed series too. Only vendor/patches/hayro is compared below.
cp "$repo/tools/vendor/export-patches.sh" "$repo/tools/vendor/lib-format-patch.sh" \
  "$clone/tools/vendor/"
"$clone/tools/vendor/export-patches.sh"
if ! git -C "$clone" diff --exit-code --stat -- vendor/patches/hayro; then
  fail "regenerated series differs from the committed one (HEAD $(git rev-parse --short HEAD))"
fi
[[ -z $(git -C "$clone" status --porcelain -- vendor/patches/hayro) ]] ||
  fail "regeneration left untracked patch files"
echo "  ✓ committed series at HEAD regenerates byte-identically"
# The same, from a user config that sets every output-affecting option. As in
# 2c, git's unpinned output of the real range 2 must first change under the
# three settings whose effect depends on the input, or their pins go untested.
plain=$(git -C "$clone" format-patch --stdout vendor/hayro-pristine-2..HEAD -- 'vendor/hayro-*')
for setting in "diff.orderFile=$order_file" "core.attributesFile=$attrs_file" \
  diff.suppressBlankEmpty=true; do
  real=$(git -C "$clone" -c "$setting" format-patch --stdout \
    vendor/hayro-pristine-2..HEAD -- 'vendor/hayro-*')
  [[ $real != "$plain" ]] || fail "the real series does not exercise $setting"
done
GIT_CONFIG_GLOBAL="$hostile" "$clone/tools/vendor/export-patches.sh" >/dev/null
if ! git -C "$clone" diff --exit-code --stat -- vendor/patches/hayro; then
  fail "a user git config changed the regenerated series"
fi
[[ -z $(git -C "$clone" status --porcelain -- vendor/patches/hayro) ]] ||
  fail "a user git config changed the regenerated file set"
echo "  ✓ ... also under a hostile user git config"
echo ""
echo "All tests passed"
