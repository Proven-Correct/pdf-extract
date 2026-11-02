#!/usr/bin/env bash
# Lint of the sanitizer suppression files (sanitizer-checks spec,
# "Sanitizer suppressions are explicit and justified"; design D9). Run by
# tools/sanitizers/run.sh before any test, so a bad entry fails the run before
# it starts, and by ci.yml's `shellcheck` job on every push.
#
# Usage: tools/sanitizers/check-suppressions.sh [FILE...]
#        tools/sanitizers/check-suppressions.sh --self-test
#
# With no FILE, checks tools/sanitizers/lsan.supp and tsan.supp. The rules,
# per entry (every line that is neither blank nor a comment):
#
# 1. Syntax: `<kind>:<pattern>`, with a kind the file's sanitizer knows
#    (lsan.supp: leak; tsan.supp: race, race_top, thread, mutex, signal,
#    deadlock, called_from_lib) and a pattern with no whitespace.
# 2. Justification: the comment block directly above the entry (contiguous
#    `#` lines, no blank line between) contains `# why: <reason>` with a
#    non-empty reason. One block may justify several consecutive entries
#    only by repeating it; each entry carries its own.
# 3. Scope: the pattern must not be able to cover this workspace's own code.
#    It may not name a pdf_extract crate (`pdf_extract`, `pdf-extract`), the
#    tesseract-sys Rust symbols (`tesseract_sys`) or the C++ shim's `pxt_`
#    entry points, and may not be wildcards alone (`*`, `*:*`), which would
#    match everything including our frames. Suppressions exist for
#    third-party false positives (uninstrumented system libraries) only; a
#    report in our code is a bug to fix, not to silence.
#
# --self-test runs the checks on planted good and bad files in a scratch
# directory and fails unless every bad file is rejected and the good one
# passes.
set -euo pipefail
export LC_ALL=C

here=$(cd "$(dirname "$0")" && pwd)

# check FILE: prints one line per violation to stderr, returns 1 if any.
check() {
    local file=$1 kinds
    case $(basename "$file") in
        lsan*.supp) kinds='leak' ;;
        tsan*.supp) kinds='race|race_top|thread|mutex|signal|deadlock|called_from_lib' ;;
        *)
            echo "$file: unknown suppression file (expected lsan*.supp or tsan*.supp)" >&2
            return 1
            ;;
    esac
    # awk keeps the "is there a why: in the block just above" state; a blank
    # line or an entry ends the block.
    awk -v file="$file" -v kinds="$kinds" '
        function fail(msg) { printf "%s:%d: %s\n", file, NR, msg > "/dev/stderr"; bad = 1 }
        /^[[:space:]]*$/ { why = 0; next }
        /^[[:space:]]*#/ {
            if ($0 ~ /^[[:space:]]*#[[:space:]]*why:[[:space:]]*[^[:space:]]/) why = 1
            next
        }
        {
            line = $0
            sub(/[[:space:]]+$/, "", line)
            if (line !~ ("^(" kinds "):[^[:space:]]+$")) {
                fail("not a valid entry for this file (" kinds "): " line)
            } else {
                pat = line
                sub(/^[^:]*:/, "", pat)
                low = tolower(pat)
                if (low ~ /pdf[_-]extract/ || low ~ /tesseract_sys/ || low ~ /(^|[^a-z0-9_])pxt_/)
                    fail("names this workspace'"'"'s own code: " line)
                if (pat ~ /^[*:]+$/)
                    fail("pattern matches everything: " line)
            }
            if (!why) fail("no \"# why:\" justification directly above: " line)
            why = 0
        }
        END { exit bad }
    ' "$file"
}

self_test() {
    local dir status=0
    dir=$(mktemp -d)
    # shellcheck disable=SC2064 # expand now: the trap must remove this dir.
    trap "rm -rf '$dir'" EXIT
    mkdir -p "$dir/good" "$dir/bad"
    printf '%s\n' '# Header text.' '' \
        '# libdispatch worker teardown.' \
        '# why: uninstrumented system library; frees on process exit.' \
        'leak:libdispatch.dylib' > "$dir/good/lsan.supp"
    printf '%s\n' '# why: libstdc++ is not built with TSan.' 'race:std::__1::basic_string' \
        '' '# why: second entry, own block.' 'called_from_lib:libc++.1.dylib' > "$dir/good/tsan.supp"
    # One planted violation per file.
    printf '%s\n' 'leak:libfoo.so' > "$dir/bad/lsan.supp"                                      # no why
    mkdir -p "$dir/bad2" "$dir/bad3" "$dir/bad4" "$dir/bad5" "$dir/bad6" "$dir/bad7" "$dir/bad8"
    printf '%s\n' '# why:' 'leak:libfoo.so' > "$dir/bad2/lsan.supp"                            # empty why
    printf '%s\n' '# why: x' '' 'race:libfoo' > "$dir/bad3/tsan.supp"                          # blank line breaks the block
    printf '%s\n' '# why: x' 'race:pdf_extract_core::document' > "$dir/bad4/tsan.supp"        # our crate
    printf '%s\n' '# why: x' 'leak:pdf_extract_tesseract_sys::Instance' > "$dir/bad5/lsan.supp" # our FFI crate
    printf '%s\n' '# why: x' 'race:pxt_recognize' > "$dir/bad6/tsan.supp"                      # our shim
    printf '%s\n' '# why: x' 'race:*' > "$dir/bad7/tsan.supp"                                  # everything
    printf '%s\n' '# why: x' 'race:libfoo' > "$dir/bad8/lsan.supp"                             # wrong kind for LSan
    if ! check "$dir/good/lsan.supp" || ! check "$dir/good/tsan.supp"; then
        echo "self-test: the good files were rejected" >&2
        status=1
    fi
    local f
    for f in "$dir"/bad*/*.supp; do
        if check "$f" 2>/dev/null; then
            echo "self-test: $f was accepted" >&2
            status=1
        fi
    done
    if [ "$status" -eq 0 ]; then
        echo "check-suppressions self-test: ok"
    fi
    return "$status"
}

if [ "${1-}" = "--self-test" ]; then
    self_test
    exit
fi

if [ "$#" -eq 0 ]; then
    set -- "$here/lsan.supp" "$here/tsan.supp"
fi
status=0
for f in "$@"; do
    check "$f" || status=1
done
exit "$status"
