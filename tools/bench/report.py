#!/usr/bin/env python3
"""Summarise a benchmark run and compare it with the checked-in baseline.

Role: the report half of the CI `bench` job (spec §3.7, §4.8). It reads
criterion's per-benchmark estimates and the peak-RSS file written by
`crates/core/benches/open.rs`, writes `bench.json` (same schema as
`crates/core/benches/baseline.json`, so re-recording the baseline is a copy of
a CI artefact), and prints GitHub annotations comparing the two.

It NEVER fails the build: shared runners are too noisy for wall-clock numbers
to gate anything, so every finding is an annotation and the exit status is
always 0, even when inputs are missing.

Usage:
    report.py --criterion target/criterion --rss bench-rss.json \
              --baseline crates/core/benches/baseline.json --out bench.json
"""

from __future__ import annotations

import argparse
import datetime
import json
import os
import platform
from pathlib import Path

# A result this much worse than the baseline gets a warning annotation
# instead of a notice. Generous on purpose: the point is to flag a trend for
# a human, not to catch noise.
WARN_RATIO = 1.25


def load_criterion(root: Path) -> dict[str, float]:
    """Mean wall-clock (ns) per benchmark, keyed by criterion's full id."""
    out: dict[str, float] = {}
    for meta in sorted(root.glob("**/new/benchmark.json")):
        try:
            full_id = json.loads(meta.read_text())["full_id"]
            est = json.loads((meta.parent / "estimates.json").read_text())
            out[full_id] = float(est["mean"]["point_estimate"])
        except (OSError, KeyError, ValueError) as e:
            print(f"::warning title=bench report::cannot read {meta}: {e}")
    return out


def load_json(path: Path) -> dict:
    try:
        return json.loads(path.read_text())
    except (OSError, ValueError) as e:
        print(f"::warning title=bench report::cannot read {path}: {e}")
        return {}


def build(criterion: dict[str, float], rss: dict) -> dict:
    """The bench.json document."""
    benches: dict[str, dict] = {}
    for full_id, mean_ns in criterion.items():
        # RSS entries are keyed by the function id (no group prefix).
        name = full_id.split("/", 1)[1] if "/" in full_id else full_id
        entry: dict = {"mean_ns": round(mean_ns)}
        if name in rss:
            entry.update(rss[name])
        benches[full_id] = entry
    return {
        "schema": 1,
        "recorded": {
            "date": datetime.date.today().isoformat(),
            "platform": platform.platform(),
            "machine": platform.machine(),
            "runner": os.environ.get("RUNNER_NAME", "local"),
        },
        "benches": benches,
    }


def fmt(value: float, unit: str) -> str:
    if unit == "ns":
        return f"{value / 1e6:.1f} ms"
    return f"{value / 2**20:.1f} MiB"


def compare(current: dict, baseline: dict) -> list[str]:
    """Annotation lines; also returned for the step summary."""
    lines = []
    base = baseline.get("benches", {})
    where = baseline.get("recorded", {})
    for name, entry in sorted(current["benches"].items()):
        old = base.get(name)
        if old is None:
            lines.append(f"::notice title={name}::no baseline entry")
            continue
        for key, unit in (("mean_ns", "ns"), ("peak_rss_bytes", "bytes")):
            if key not in entry or not old.get(key):
                continue
            ratio = entry[key] / old[key]
            level = "warning" if ratio > WARN_RATIO else "notice"
            lines.append(
                f"::{level} title={name} {key}::{fmt(entry[key], unit)} vs baseline "
                f"{fmt(old[key], unit)} ({ratio:.2f}x; baseline recorded "
                f"{where.get('date', '?')} on {where.get('platform', '?')})"
            )
    return lines


def run() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--criterion", type=Path, default=Path("target/criterion"))
    ap.add_argument("--rss", type=Path, default=Path("bench-rss.json"))
    ap.add_argument("--baseline", type=Path, default=Path("crates/core/benches/baseline.json"))
    ap.add_argument("--out", type=Path, default=Path("bench.json"))
    args = ap.parse_args()

    current = build(load_criterion(args.criterion), load_json(args.rss))
    args.out.write_text(json.dumps(current, indent=2) + "\n")
    lines = compare(current, load_json(args.baseline))
    for line in lines:
        print(line)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a") as f:
            f.write("## Benchmarks (report-only)\n\n")
            for line in lines:
                f.write(f"- {line.split('::', 2)[-1]}\n")


def main() -> int:
    # Report-only by contract: malformed inputs (a non-object JSON file, a
    # non-numeric baseline value) become an annotation, never a failure.
    try:
        run()
    except Exception as e:  # noqa: BLE001 - deliberately catch-all
        print(f"::warning title=bench report::report failed: {e!r}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
