#!/usr/bin/env bash
# Build the public repository's history from the internal one (design D3 of
# the publish-publicly-for-adoption change; ADR 0097). Run once, at the export;
# a rehearsal into a scratch directory comes first.
#
# Usage: tools/release/build-public-history.sh SRC DEST
#   SRC   the internal repository (any clone with the four vendor tags)
#   DEST  where the public repository is created; must not exist
# Environment:
#   SRC_REF        the scrubbed internal commit to export (default: main)
#   PUBLIC_IDENTITY  author and committer of every public commit
#                  (default: "Christophe Pettus <xof@thebuild.com>")
#   KEEP_PARTIAL=1 keep the work directory of a refused build for inspection
#
# Why a reconstructed history rather than one squashed commit: the vendor
# tooling (export-patches.sh, test-check-patches.sh section 3, the licensing
# spec) needs the four pristine tags and one commit per hayro patch between
# them. So DEST gets, in order and all authored by the public identity:
#   1. commit A, the three hayro crates as vendor/hayro-pristine-1 holds them,
#      tagged with that name;
#   2. commit B, Tesseract and Leptonica as vendor/tesseract-pristine-1 holds
#      them, tagged vendor/tesseract-pristine-1 and vendor/leptonica-pristine-1
#      (internally both tags name one commit; the script refuses otherwise);
#   3. the range-1 patches, replayed with `git am` from the committed files;
#   4. commit C, the two codec crates as vendor/hayro-pristine-2 holds them,
#      tagged with that name (one parent, as export-patches.sh requires);
#   5. the range-2 patches;
#   6. one snapshot commit: SRC_REF's tree, with vendor/patches/hayro/
#      rewritten by DEST's own export-patches.sh.
# The import commits carry only the crate directories, never the tag's
# vendor/PROVENANCE.md or vendor/patches/: those files at the tag are internal
# history (the old series names a work address). The patches are replayed from
# the committed files, not from internal commits, so no internal commit,
# message or test change outside vendor/ reaches DEST. Each patch first goes
# through public-patch.pl, which names the public identity in its From: line and
# qualifies internal issue references in its message; that filtered file is
# also exactly what the regenerated series must equal.
#
# Every commit's committer date is its author date, and the import and
# snapshot commits take their dates from SRC, so two builds of the same SRC_REF
# give the same commit ids: the rehearsal predicts the export exactly.
#
# The build happens in a sibling work directory that is renamed to DEST only
# after every check passes, so a refused build never leaves a DEST that could
# be pushed by mistake. The checks:
#   1. DEST's head tree equals SRC_REF's tree, except for the bytes of
#      vendor/patches/hayro/*.patch (same file names), and the snapshot commit
#      changes no vendored crate (so vendor/ in DEST is pristine + patches by
#      construction, as the `vendor-patches` CI job proves for SRC);
#   2. the regenerated series equals the committed one after public-patch.pl,
#      byte for byte: only the From: lines and the qualifiers differ;
#   3. tools/vendor/test-check-patches.sh passes in DEST;
# plus the history's shape: the four tags on the import commits, 3 imports +
# one commit per patch + 1 snapshot, and one author and committer identity.
#
# Git configuration is isolated (GIT_CONFIG_GLOBAL=/dev/null, and
# core.attributesFile=/dev/null for the user-level attributes file git reads
# even then): a user's core.autocrlf, hooks, signing, am/apply settings or
# export-ignore attributes would otherwise change the bytes committed. The
# repository-local variables (GIT_DIR, GIT_INDEX_FILE, GIT_WORK_TREE and the
# rest of `git rev-parse --local-env-vars`) are unset first: inherited from a
# hook or a caller, they would point every `git -C` below at SRC and write the
# build into it. SRC is read with safe.directory set to itself, which
# GIT_CONFIG_GLOBAL=/dev/null would otherwise drop. SRC's .gitattributes is installed as DEST's
# .git/info/attributes while the imports run, so the vendored C trees' -text
# rule holds before the snapshot commits the file itself.
set -euo pipefail

usage() { echo "usage: $0 SRC DEST" >&2; exit 2; }
[ $# -eq 2 ] || usage
refuse() { echo "build-public-history: refused: $*" >&2; exit 1; }

unset CDPATH
# shellcheck disable=SC2046 # one variable name per word, by design
unset $(git rev-parse --local-env-vars)
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
identity=${PUBLIC_IDENTITY:-Christophe Pettus <xof@thebuild.com>}
[[ $identity =~ ^([^\<\>]+)\ \<([^\<\>[:space:]]+)\>$ ]] ||
  refuse "PUBLIC_IDENTITY must read 'Name <email>': $identity"
pub_name=${BASH_REMATCH[1]} pub_email=${BASH_REMATCH[2]}
export GIT_AUTHOR_NAME=$pub_name GIT_AUTHOR_EMAIL=$pub_email
export GIT_COMMITTER_NAME=$pub_name GIT_COMMITTER_EMAIL=$pub_email

here=$(cd "$(dirname "$0")" && pwd)
filter=$here/public-patch.pl
src=$(cd "$1" 2>/dev/null && pwd) || refuse "SRC $1 is not a directory"
sg() { git -c safe.directory="$src" -c core.attributesFile=/dev/null -C "$src" "$@"; }
sg rev-parse --git-dir >/dev/null 2>&1 || refuse "SRC $src is not a git repository"
dest=$2
[ ! -e "$dest" ] && [ ! -L "$dest" ] || refuse "DEST $dest already exists"
dest_parent=$(cd "$(dirname "$dest")" 2>/dev/null && pwd) ||
  refuse "the parent of DEST $dest does not exist"
dest=$dest_parent/$(basename "$dest")

ref=${SRC_REF:-main}
src_commit=$(sg rev-parse --verify --quiet "$ref^{commit}") ||
  refuse "SRC_REF $ref is not a commit in $src"
tag_h1=vendor/hayro-pristine-1 tag_h2=vendor/hayro-pristine-2
tag_t=vendor/tesseract-pristine-1 tag_l=vendor/leptonica-pristine-1
for t in "$tag_h1" "$tag_h2" "$tag_t" "$tag_l"; do
  sg rev-parse --verify --quiet "refs/tags/$t^{commit}" >/dev/null ||
    refuse "tag $t not found in $src (tags are local; fetch them)"
done
[ "$(sg rev-parse "$tag_t^{commit}")" = "$(sg rev-parse "$tag_l^{commit}")" ] ||
  refuse "$tag_t and $tag_l name different commits; D3 imports both C trees in one commit"

work=$(mktemp -d "$dest.partial.XXXXXX")
scratch=$(mktemp -d)
cleanup() {
  local rc=$?
  rm -rf "$scratch"
  if [ "$rc" -ne 0 ] && [ -d "$work" ]; then
    if [ "${KEEP_PARTIAL:-}" = 1 ]; then
      echo "build-public-history: refused build kept at $work" >&2
    else
      rm -rf "$work"
    fi
  fi
}
trap cleanup EXIT

# --- The committed series, made public ---------------------------------------

patches=vendor/patches/hayro
mkdir -p "$scratch/committed" "$scratch/public"
sg archive --format=tar "$src_commit" -- "$patches" | tar -x -C "$scratch/committed"
names=()
while IFS= read -r f; do names+=("$f"); done < <(cd "$scratch/committed/$patches" && ls -- *.patch)
n=${#names[@]}
[ "$n" -gt 0 ] || refuse "no patches under $patches at $ref"
# Range 1's files count within their range ("[PATCH k/N1]"); range 2's carry
# the series total (export-patches.sh's --start-number), each number padded to
# the width of its total. Read N1 from 0001.
n1=$(sed -n 's/^Subject: \[PATCH 0*1\/\([0-9][0-9]*\)\].*/\1/p' "$scratch/committed/$patches/${names[0]}")
[ -n "$n1" ] && [ "$n1" -le "$n" ] || refuse "cannot read range 1's length from ${names[0]}"
for i in "${!names[@]}"; do
  k=$((i + 1)) total=$n
  [ "$k" -le "$n1" ] && total=$n1
  printf -v k '%0*d' "${#total}" "$k"
  grep -q "^Subject: \[PATCH $k/$total\]" "$scratch/committed/$patches/${names[$i]}" ||
    refuse "${names[$i]} is not [PATCH $k/$total]; the series is not as export-patches.sh writes it"
  "$filter" "$identity" < "$scratch/committed/$patches/${names[$i]}" > "$scratch/public/${names[$i]}" ||
    refuse "public-patch.pl rejected ${names[$i]}"
done

# --- History ------------------------------------------------------------------

git init -q -b main "$work"
g() { git -C "$work" "$@"; }
g config core.autocrlf false
g config core.hooksPath /dev/null
g config commit.gpgSign false
g config tag.gpgSign false
g config core.attributesFile /dev/null
mkdir -p "$work/.git/info"
sg show "$src_commit:.gitattributes" > "$work/.git/info/attributes" 2>/dev/null || true

# import TAG PATH...: commit PATHs exactly as TAG holds them, dated as TAG's
# commit, with its subject. The subject is the one piece of internal commit
# text that reaches DEST unfiltered, so a merge (whose subject names a branch
# and a pull request) or any "#" in it is refused.
import() {
  local tag=$1; shift
  local p date subject
  [ "$(sg rev-list --parents -1 "$tag^{commit}" | wc -w | tr -d ' ')" -le 2 ] ||
    refuse "$tag is a merge commit"
  subject=$(sg log -1 --format=%s "$tag^{commit}")
  [[ $subject != *'#'* ]] || refuse "$tag's subject contains '#': $subject"
  for p in "$@"; do
    [ "$(sg cat-file -t "$tag:$p" 2>/dev/null)" = tree ] || refuse "$tag holds no $p"
    [ ! -e "$work/$p" ] || refuse "$p already exists before its import"
  done
  sg archive --format=tar "$tag" -- "$@" | tar -x -C "$work"
  g add -A -f -- "$@"
  date=$(sg log -1 --format=%aI "$tag^{commit}")
  GIT_AUTHOR_DATE=$date GIT_COMMITTER_DATE=$date g commit -q --cleanup=verbatim \
    -m "$subject" -m "The upstream sources, unmodified (tag $tag)."
  for p in "$@"; do
    [ "$(sg rev-parse "$tag:$p")" = "$(g rev-parse "HEAD:$p")" ] ||
      refuse "the import of $p differs from $tag"
  done
}

# replay FROM TO: git am of public patches FROM..TO (1-based, inclusive).
replay() {
  local k
  for ((k = $1; k <= $2; k++)); do
    g am -q --keep-non-patch --no-scissors --no-message-id --no-signoff \
      --no-3way --whitespace=nowarn --committer-date-is-author-date \
      --directory=vendor "$scratch/public/${names[$((k - 1))]}" ||
      refuse "git am failed on ${names[$((k - 1))]}"
  done
}

import "$tag_h1" vendor/hayro-syntax vendor/hayro-interpret vendor/hayro-postscript
g tag "$tag_h1"
import "$tag_t" vendor/tesseract vendor/leptonica
g tag "$tag_t"
g tag "$tag_l"
replay 1 "$n1"
import "$tag_h2" vendor/hayro-jbig2 vendor/hayro-jpeg2000
g tag "$tag_h2"
replay "$((n1 + 1))" "$n"

# The snapshot: SRC_REF's tree replaces the working tree, then DEST's own
# exporter rewrites the series from the replayed commits (HEAD is still the
# last patch commit, so the snapshot itself is outside its range).
g rm -rq -- .
sg archive --format=tar "$src_commit" | tar -x -C "$work"
(cd "$work" && tools/vendor/export-patches.sh >/dev/null) || refuse "export-patches.sh failed in DEST"
g add -A -f
rm -f "$work/.git/info/attributes"
date=$(sg log -1 --format=%cI "$src_commit")
GIT_AUTHOR_DATE=$date GIT_COMMITTER_DATE=$date g commit -q --cleanup=verbatim \
  -m "Public snapshot of internal commit $(sg rev-parse --short=12 "$src_commit")"

# --- Checks -------------------------------------------------------------------

# 1. The tree equals SRC_REF's, outside the patch files' bytes; and the
#    snapshot changes no vendored crate.
tree_list() { git -C "$1" ls-tree -r --full-tree "$2" | grep -v -E $'\t'"$patches/[^/]*\.patch$" || true; }
diff <(tree_list "$src" "$src_commit") <(tree_list "$work" HEAD) > "$scratch/tree.diff" ||
  refuse "DEST's tree differs from $ref outside $patches:"$'\n'"$(head -20 "$scratch/tree.diff")"
diff <(cd "$scratch/committed/$patches" && ls) <(cd "$work/$patches" && ls) > "$scratch/names.diff" ||
  refuse "the regenerated series has different files:"$'\n'"$(cat "$scratch/names.diff")"
crates=(vendor/hayro-syntax vendor/hayro-interpret vendor/hayro-postscript
  vendor/hayro-jbig2 vendor/hayro-jpeg2000 vendor/tesseract vendor/leptonica)
changed=$(g diff --name-only HEAD^ HEAD -- "${crates[@]}")
[ -z "$changed" ] ||
  refuse "the snapshot changes vendored files, so vendor/ at $ref is not pristine + patches:"$'\n'"$changed"

# The history's shape, before the series: an in-body From: line in a patch
# message would make `git am` author that commit as someone else, and this
# names that cause, where the series comparison would only report a diff.
expect_commits=$((3 + n + 1))
[ "$(g rev-list --count HEAD)" = "$expect_commits" ] ||
  refuse "DEST has $(g rev-list --count HEAD) commits, expected $expect_commits"
[ "$(g tag -l | wc -l | tr -d ' ')" = 4 ] || refuse "DEST has tags other than the four vendor tags"
[ "$(g rev-list --count "$tag_h1")" = 1 ] || refuse "$tag_h1 is not the root import"
[ "$(g rev-parse "$tag_t^")" = "$(g rev-parse "$tag_h1")" ] || refuse "$tag_t does not follow $tag_h1"
[ "$(g rev-list --count "$tag_t..$tag_h2^")" = "$n1" ] || refuse "range 1 does not hold $n1 commits"
[ "$(g rev-list --count "$tag_h2..HEAD^")" = "$((n - n1))" ] || refuse "range 2 does not hold $((n - n1)) commits"
people=$(g log --format='%an <%ae>%n%cn <%ce>' | sort -u)
[ "$people" = "$identity" ] || refuse "DEST names identities other than $identity:"$'\n'"$people"

# 2. The regenerated series is the committed one, made public.
for f in "${names[@]}"; do
  cmp -s "$scratch/public/$f" "$work/$patches/$f" ||
    refuse "regenerated $f differs from the committed patch beyond From: and the qualifiers:"$'\n'"$(diff "$scratch/public/$f" "$work/$patches/$f" | head -20)"
done

# 3. The vendor patch tooling's own tests, in DEST.
(cd "$work" && tools/vendor/test-check-patches.sh) > "$scratch/tcp.log" 2>&1 ||
  refuse "tools/vendor/test-check-patches.sh failed in DEST:"$'\n'"$(tail -20 "$scratch/tcp.log")"

[ ! -e "$dest" ] && [ ! -L "$dest" ] || refuse "DEST $dest appeared during the build"
mv "$work" "$dest"
echo "built $dest from $ref = $src_commit: $expect_commits commits (3 imports, $n patches = $n1 + $((n - n1)), 1 snapshot), 4 tags, author $identity"
