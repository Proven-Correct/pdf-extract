# Language data measurement (spec 0002 O35, design spec §17.2 Q2)

Which `eng.traineddata` should deployments pin? Measured on corpus D (52 pages)
with `tools/oracle/measure_tessdata.sh`, which runs `crates/ocr/tests/quality.rs`
in its report-only measurement mode (`PDF_EXTRACT_TESSDATA=<dir>`: no model pin
check, no gates, no process-isolation pass; report in
`target/ocr-report-measure.json`). The pinned CI gate is unchanged and still
runs against `tessdata_fast` only.

## Models

All three are Apache-2.0 (the repositories' `LICENSE`), so redistribution
alongside a GPL-3.0-or-later project is unproblematic; the library bundles none
of them (D-14).

| Name in table | Source | Blob sha (git) | Size | SHA-256 |
|---|---|---|---|---|
| fast | `tesseract-ocr/tessdata_fast` (current Debian `tesseract-ocr-eng` (tesseract-lang 1:4.1.0 and later) ships this set; Ubuntu not verified) | `bbef4675053b5b468cdb477053e28b1c698ba08e` | 4,113,088 | `7d4322bd2a7749724879683fc3912cb542f19906c83bcc1a52132556427170b2` |
| best | `tesseract-ocr/tessdata_best` | `176dc3220de7db34d3b3aecbfa42043a6038348b` | 15,400,601 | `8280aed0782fe27257a68ea10fe7ef324ca0f8d85bd2fd145d1c2b560bcb66ba` |
| combined | `tesseract-ocr/tessdata` (the MacPorts default, `/opt/local/share/tessdata/eng.traineddata`, is byte-identical) | `f4744c201359d02ced6914c77f0de68ee9bbdd74` | 23,466,654 | `daa0c97d651c19fba3b25e81317cd697e9908c8208090c94c3905381c23fc047` |

The combined file also carries the legacy engine's data; the vendored build has
no legacy engine, so only its LSTM part is used (it loads under `LstmOnly` only).

## Results

Apple M3 Max, macOS 14 arm64, release build, in-process, one page at a time.
Latency and RSS are single-run, report-only figures on a developer machine;
treat differences under about 10% as noise. `Init` latency is engine
construction (resolution, checksum, `Init`, one 1 x 1 recognition) with a warm
file cache. "RSS per instance" is the growth in peak RSS over the whole run
(one instance plus page buffers, dominated by the 300 dpi render of the
largest page), so it compares models only roughly.

<!-- measure:begin -->
CER / WER as corpus means (ours; report-only, no gate).

| Subset | Pages | Metric | fast | best | combined |
|---|---|---|---|---|---|
| clean | 12 | CER@72 | 0.0315 | 0.0565 | 0.0544 |
| clean | 12 | CER@300 | 0.0084 | 0.0087 | 0.0087 |
| clean | 12 | WER@72 | 0.1406 | 0.1955 | 0.1962 |
| clean | 12 | WER@300 | 0.0181 | 0.0185 | 0.0185 |
| clean | 12 | ms/page@300 | 935 | 2428 | 1629 |
| noise | 12 | CER@72 | 0.0383 | 0.0551 | 0.0542 |
| noise | 12 | CER@300 | 0.0863 | 0.0892 | 0.0864 |
| noise | 12 | WER@72 | 0.1626 | 0.1931 | 0.1962 |
| noise | 12 | WER@300 | 0.1014 | 0.1019 | 0.1019 |
| noise | 12 | ms/page@300 | 2230 | 3803 | 2943 |
| skew | 12 | CER@72 | 0.0590 | 0.0989 | 0.1010 |
| skew | 12 | CER@300 | 0.0037 | 0.0037 | 0.0037 |
| skew | 12 | WER@72 | 0.2044 | 0.2919 | 0.2988 |
| skew | 12 | WER@300 | 0.0086 | 0.0082 | 0.0083 |
| skew | 12 | ms/page@300 | 877 | 2142 | 1428 |
| blur | 12 | CER@72 | 0.0491 | 0.0654 | 0.0649 |
| blur | 12 | CER@300 | 0.0085 | 0.0084 | 0.0084 |
| blur | 12 | WER@72 | 0.1691 | 0.2078 | 0.2022 |
| blur | 12 | WER@300 | 0.0181 | 0.0181 | 0.0181 |
| blur | 12 | ms/page@300 | 937 | 2422 | 1589 |
| lowres | 3 | CER@72 | 0.0200 | 0.0230 | 0.0250 |
| lowres | 3 | CER@300 | 0.0000 | 0.0000 | 0.0000 |
| lowres | 3 | WER@72 | 0.1049 | 0.1190 | 0.1294 |
| lowres | 3 | WER@300 | 0.0000 | 0.0000 | 0.0000 |
| lowres | 3 | ms/page@300 | 1002 | 2602 | 1704 |
| jpeg | 1 | CER@72 | 0.0097 | 0.0200 | 0.0220 |
| jpeg | 1 | CER@300 | 0.0000 | 0.0000 | 0.0000 |
| jpeg | 1 | WER@72 | 0.0717 | 0.0973 | 0.1041 |
| jpeg | 1 | WER@300 | 0.0000 | 0.0000 | 0.0000 |
| jpeg | 1 | ms/page@300 | 1294 | 3347 | 2194 |

| Property | fast | best | combined |
|---|---|---|---|
| Model size (MB) | 4.1 | 15.4 | 23.5 |
| SHA-256 | `7d4322bd2a774972...` | `8280aed0782fe272...` | `daa0c97d651c19fb...` |
| `Init` latency (ms) | 12 | 40 | 61 |
| RSS per instance (MB, growth over run) | 567 | 604 | 641 |
| Mean ms/page@300, all 52 pages | 1232 | 2706 | 1892 |
| Mean CER@300, all 52 pages | 0.0247 | 0.0254 | 0.0248 |

`sparse_below_min_chars_noise` at 300 dpi:

| Metric | fast | best | combined |
|---|---|---|---|
| CER@72 | 0.0667 | 0.0000 | 0.0000 |
| CER@300 | 0.9333 | 0.9667 | 0.9333 |
| ms@72 | 12 | 20 | 15 |
| ms@300 | 15765 | 16352 | 16428 |
<!-- measure:end -->

## Findings

- **Accuracy at 300 dpi is the same for all three.** Corpus means differ in the
  fourth decimal (clean 0.0084 / 0.0087 / 0.0087; all 52 pages 0.0247 / 0.0254 /
  0.0248 for fast / best / combined). `tessdata_best` is not more accurate on
  this corpus; it is slightly worse on clean pages.
- **`tessdata_fast` is the cheapest.** About 2.2x faster per page than best
  (1232 vs 2706 ms mean at 300 dpi) and 1.5x faster than combined (1892 ms), 4 to 6x
  smaller, and the quickest to `Init`.
- **At 72 dpi `tessdata_fast` is clearly the most accurate** (clean CER 0.0315
  against 0.054 to 0.057), which matters for the 72 dpi default of the text-path
  comparison but not for 300 dpi OCR.
- **Near-blank noisy page.** `sparse_below_min_chars_noise` at 300 dpi is
  misread by every model (CER 0.93 / 0.97 / 0.93) and takes about 16 s per
  page for each (about 12 to 19 ms at 72 dpi; 0.1 s at 150 dpi). Neither best nor
  combined behaves differently, so the cause is Tesseract's page-segmentation
  and noise handling on a near-empty page, not the model. It is also why the
  noise subset is the only one where 300 dpi is worse than 72 dpi. Deployments
  that OCR scans of unknown quality should keep `page_timeout` set (default
  60 s) and may prefer 150 dpi for such inputs.

## Recommendation (O35)

O35's rule: the model with the best 300 dpi CER, unless its per-page latency
exceeds the fastest by more than 2x, in which case `tessdata_fast`. Here
`tessdata_fast` has the best (lowest) mean 300 dpi CER on clean pages and over
all pages and is also the fastest, so the rule picks it with no tie-break
needed. Deployments should pin
`7d4322bd2a7749724879683fc3912cb542f19906c83bcc1a52132556427170b2` and can
verify it with `OcrEngine.info()` (`model_sha256`). The O34 expectations stay
those of `tessdata_fast`, the CI model; the gate is not re-tuned.

To reproduce: `tools/oracle/measure_tessdata.sh` (needs network for the first
download, `curl`, `python3`, and builds in release mode; about 8 minutes). It
refuses a model whose SHA-256 differs from the pins above.
