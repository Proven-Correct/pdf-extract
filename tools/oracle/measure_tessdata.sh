#!/usr/bin/env bash
# Measures the three English language-data sets on corpus D (spec 0002 O35,
# §4.13) and rewrites the generated table in
# docs/reference/tessdata-measurement.md.
#
# Each model is downloaded by git blob sha from its GitHub repository into
# tools/oracle/.cache/ and verified against a pinned SHA-256 before use (a
# mismatch aborts: scores of unknown bytes are worthless). quality.rs then
# runs in its report-only measurement mode (PDF_EXTRACT_TESSDATA=<dir>),
# which never touches the pinned CI gate. Only the region between the
# measure:begin/end markers of the doc is replaced; the prose around it is
# hand-written.
#
# usage: measure_tessdata.sh [--cache DIR] [--doc FILE] [--no-write] [--reuse]
#   --cache DIR   model cache (default tools/oracle/.cache)
#   --doc FILE    document holding the markers (default the reference doc)
#   --reuse       reuse target/measure/*.json from an earlier run (no scoring)
#   --no-write    print the table to stdout instead of editing the doc
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cache="$root/tools/oracle/.cache"
doc="$root/docs/reference/tessdata-measurement.md"
write=1
reuse=0

while [ $# -gt 0 ]; do
    case "$1" in
        --cache) cache="$2"; shift 2 ;;
        --doc) doc="$2"; shift 2 ;;
        --no-write) write=0; shift ;;
        --reuse) reuse=1; shift ;;
        -h | --help) sed -n '2,/^set -e/p' "$0" | sed '$d;s/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

# name | repository | blob sha | SHA-256 of the file. The combined set is
# tesseract-ocr/tessdata (legacy + LSTM in one file; only the LSTM part is
# used, the vendored build has no legacy engine).
models=(
    "fast|tessdata_fast|bbef4675053b5b468cdb477053e28b1c698ba08e|7d4322bd2a7749724879683fc3912cb542f19906c83bcc1a52132556427170b2"
    "best|tessdata_best|176dc3220de7db34d3b3aecbfa42043a6038348b|8280aed0782fe27257a68ea10fe7ef324ca0f8d85bd2fd145d1c2b560bcb66ba"
    "combined|tessdata|f4744c201359d02ced6914c77f0de68ee9bbdd74|daa0c97d651c19fba3b25e81317cd697e9908c8208090c94c3905381c23fc047"
)

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d' ' -f1
    else
        shasum -a 256 "$1" | cut -d' ' -f1
    fi
}

mkdir -p "$cache" "$root/target/measure"
reports=()
for m in "${models[@]}"; do
    IFS='|' read -r name repo blob want <<<"$m"
    dir="$cache/measure-$name"
    file="$dir/eng.traineddata"
    mkdir -p "$dir"
    if [ ! -f "$file" ] || [ "$(sha256_of "$file")" != "$want" ]; then
        echo "fetching $repo eng.traineddata (blob $blob)" >&2
        curl -fsSL -H "Accept: application/vnd.github.raw" \
            "https://api.github.com/repos/tesseract-ocr/$repo/git/blobs/$blob" \
            -o "$file.part"
        mv "$file.part" "$file"
    fi
    got="$(sha256_of "$file")"
    if [ "$got" != "$want" ]; then
        echo "SHA-256 mismatch for $repo: expected $want, got $got" >&2
        exit 1
    fi
    if [ "$reuse" = 0 ] || [ ! -f "$root/target/measure/$name.json" ]; then
        echo "measuring $name ($repo)" >&2
        (
            cd "$root"
            PDF_EXTRACT_TESSDATA="$dir" cargo test --release -p pdf-extract-ocr \
                --test quality corpus_d_quality -- --include-ignored --nocapture >&2
        )
        cp "$root/target/ocr-report-measure.json" "$root/target/measure/$name.json"
    fi
    size="$(wc -c <"$file" | tr -d ' ')"
    reports+=("$name=$root/target/measure/$name.json=$size")
done

table="$(python3 - "${reports[@]}" <<'PY'
import json, sys

models = [a.split("=", 2) for a in sys.argv[1:]]
data = {n: (json.load(open(p)), int(size)) for n, p, size in models}
subsets = ["clean", "noise", "skew", "blur", "lowres", "jpeg"]
names = list(data)

def f(x, nd=4):
    return f"{x:.{nd}f}"

out = []
out.append("CER / WER as corpus means (ours; report-only, no gate).\n")
out.append("| Subset | Pages | Metric | " + " | ".join(names) + " |")
out.append("|---|---|---|" + "---|" * len(names))
for s in subsets:
    pages = data[names[0]][0]["summary"][s]["pages"]
    for label, key in [("CER@72", "cer_ours72"), ("CER@300", "cer_ours300"),
                       ("WER@72", "wer_ours72"), ("WER@300", "wer_ours300"),
                       ("ms/page@300", "ms300")]:
        vals = [data[n][0]["summary"][s][key] for n in names]
        cells = [f(v, 0) if key.startswith("ms") else f(v) for v in vals]
        out.append(f"| {s} | {pages} | {label} | " + " | ".join(cells) + " |")
out.append("")
out.append("| Property | " + " | ".join(names) + " |")
out.append("|---|" + "---|" * len(names))
rows = [
    ("Model size (MB)", lambda r, sz: f"{sz / 1e6:.1f}"),
    ("SHA-256", lambda r, sz: "`" + r["model_sha256"][:16] + "...`"),
    ("`Init` latency (ms)", lambda r, sz: f"{r['init_ms']:.0f}"),
    ("RSS per instance (MB, growth over run)", lambda r, sz: f"{r['rss']['per_instance_bytes'] / 1e6:.0f}"),
    ("Mean ms/page@300, all 52 pages", lambda r, sz: f"{sum(p['ms300'] for p in r['pages']) / len(r['pages']):.0f}"),
    ("Mean CER@300, all 52 pages", lambda r, sz: f"{sum(p['cer_ours300'] for p in r['pages']) / len(r['pages']):.4f}"),
]
for label, fn in rows:
    out.append(f"| {label} | " + " | ".join(fn(data[n][0], data[n][1]) for n in names) + " |")
out.append("")
out.append("`sparse_below_min_chars_noise` at 300 dpi:\n")
out.append("| Metric | " + " | ".join(names) + " |")
out.append("|---|" + "---|" * len(names))
def page(n):
    return next(p for p in data[n][0]["pages"] if p["name"].startswith("sparse_below_min_chars_noise"))
for label, key, nd in [("CER@72", "cer_ours72", 4), ("CER@300", "cer_ours300", 4),
                       ("ms@72", "ms72", 0), ("ms@300", "ms300", 0)]:
    out.append(f"| {label} | " + " | ".join(f(page(n)[key], nd) for n in names) + " |")
print("\n".join(out))
PY
)"

if [ "$write" = 0 ]; then
    printf '%s\n' "$table"
    exit 0
fi
if ! grep -q '<!-- measure:begin -->' "$doc" || ! grep -q '<!-- measure:end -->' "$doc"; then
    echo "$doc lacks the measure:begin/end markers" >&2
    exit 1
fi
tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT
# ENVIRON, not -v: BSD awk rejects newlines in -v values.
TABLE="$table" awk '
    /<!-- measure:begin -->/ { print; print ENVIRON["TABLE"]; skip = 1; next }
    /<!-- measure:end -->/ { skip = 0 }
    !skip { print }
' "$doc" >"$tmp"
cat "$tmp" >"$doc"
echo "updated $doc" >&2
