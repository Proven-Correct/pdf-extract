"""Pretty-prints the parity reports (spec §4.13, §4.14).

``crates/core/tests/parity.rs`` compares our extraction with the checked-in
oracle expectations and writes ``target/parity-report.json`` (gate C, per
fixture) and ``target/parity-report-b.json`` (gate B, corpus metrics); the
private corpus-A run writes a report of the second kind outside the
repository. This script only formats those files. It never runs the oracle
and never imports the library, so it works in any Python 3.11+ (no PyMuPDF
needed).

Usage (from the repository root or ``tools/oracle``)::

    cargo test -p pdf-extract-core --test parity    # writes both reports
    python3 tools/oracle/score.py                   # failures + summaries
    python3 tools/oracle/score.py --all             # every fixture
    python3 tools/oracle/score.py --strict          # exit 1 on any failure
    python3 tools/oracle/score.py --gate-report R   # one corpus report only
"""

from __future__ import annotations

import argparse
import json
import sys
from collections import defaultdict
from pathlib import Path
from typing import Any

REPO = Path(__file__).resolve().parents[2]
DEFAULT_REPORT = REPO / "target" / "parity-report.json"
DEFAULT_GATE_B_REPORT = REPO / "target" / "parity-report-b.json"
# Pages listed under "least similar" in a corpus report.
WORST_SHOWN = 10


def similarity_floor(fixture: dict[str, Any]) -> float | None:
    """The lowest per-page similarity of a fixture (None if no page pairs)."""
    sims = [p["similarity"] for p in fixture["pages"] if p["similarity"] is not None]
    return min(sims) if sims else None


def fmt_sim(value: float | None) -> str:
    return "  -   " if value is None else f"{value:6.4f}"


def render(report: dict[str, Any], show_all: bool) -> list[str]:
    fixtures: list[dict[str, Any]] = report["fixtures"]
    lines = []
    width = max((len(f["path"]) for f in fixtures), default=10)
    rows = fixtures if show_all else [f for f in fixtures if not f["pass"]]
    if rows:
        lines.append(f"{'fixture':{width}}  {'gate':10}  {'pass':4}  {'agrees':6}  min-sim  pages")
        for f in rows:
            pages = f"{f['ours_pages']}/{f['oracle_pages']}"
            lines.append(
                f"{f['path']:{width}}  {f['gate']:10}  {'yes' if f['pass'] else 'NO':4}  "
                f"{'yes' if f['agrees'] else 'no':6}  {fmt_sim(similarity_floor(f))}  {pages}"
            )
            if not f["pass"] and f["problem"]:
                lines.extend(f"    {line}" for line in f["problem"].splitlines())
        lines.append("")

    by_point: dict[str, list[int]] = defaultdict(lambda: [0, 0])
    for f in fixtures:
        for point in f["compat_points"] or ["(none)"]:
            by_point[point][0] += 1
            by_point[point][1] += 0 if f["pass"] else 1
    lines.append("by compat point (fixtures, failed):")
    for point in sorted(by_point, key=lambda p: (len(p), p)):
        total, failed = by_point[point]
        lines.append(f"  {point:7} {total:4} {failed:4}")
    lines.append("")
    lines.append("by gate (fixtures, failed):")
    for gate, counts in sorted(report["summary"].items()):
        lines.append(f"  {gate:10} {counts['fixtures']:4} {counts['failed']:4}")
    return lines


def render_gate(report: dict[str, Any]) -> list[str]:
    """A corpus-gate report (gate B, or corpus A): metrics against thresholds,
    failures, and the least similar pages. Reports carry no page text."""
    m, th = report["metrics"], report["thresholds"]
    low = th["similarity_low_percentile"]
    verdict = "pass" if report["pass"] else "FAIL"
    lines = [
        f"gate {report['gate']}: {verdict}",
        f"  files {m['files']}, oracle opened {m['oracle_opened']}, "
        f"open exceptions {len(m['open_exceptions'])}",
        f"  pages {m['pages']} (unscored {m['pages_unscored']})",
        f"  routing agreement {m['routing_rate']:.4f}  (min {th['routing_min_rate']})",
        f"  similarity median {m['similarity_median']:.4f}  (min {th['similarity_min_median']})",
        f"  similarity p{low:g}     {m['similarity_low']:.4f}  (min {th['similarity_min_low']})",
        f"  similarity min {m['similarity_min']:.4f}, mean {m['similarity_mean']:.4f}",
        f"  exact after strip {m['exact_rate']:.4f}  (reported)",
        f"  line-count delta: mean |d| {m['line_delta_mean_abs']:.3f}, "
        f"max |d| {m['line_delta_max_abs']}, pages with |d| > 2: "
        f"{m['line_delta_pages_over_2']}  (reported)",
    ]
    lines.extend(f"  exception: {e}" for e in m["open_exceptions"])
    lines.extend(f"  failure: {f}" for f in report["failures"])
    worst = sorted((p for p in report["pages"] if not p["exact"]), key=lambda p: p["similarity"])
    if worst:
        lines.append("  least similar pages:")
        for p in worst[:WORST_SHOWN]:
            lines.append(
                f"    {p['similarity']:.4f}  {p['path']} page {p['index'] + 1} "
                f"(ours {p['ours_len']} chars, oracle {p['oracle_len']}, "
                f"line delta {p['line_delta']:+d})"
            )
    return lines


def load(path: Path) -> dict[str, Any] | None:
    try:
        report: dict[str, Any] = json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        return None
    return report


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--report", type=Path, default=DEFAULT_REPORT, help="report path")
    parser.add_argument("--all", action="store_true", help="list every fixture")
    parser.add_argument("--strict", action="store_true", help="exit 1 if any fixture failed")
    parser.add_argument(
        "--gate-report", type=Path, help="render only this corpus report (gate B or corpus A)"
    )
    args = parser.parse_args(argv)
    if args.gate_report is not None:
        gate = load(args.gate_report)
        if gate is None:
            print(f"score.py: {args.gate_report} not found", file=sys.stderr)
            return 2
        print("\n".join(render_gate(gate)))
        return 1 if args.strict and not gate["pass"] else 0
    report = load(args.report)
    if report is None:
        print(f"score.py: {args.report} not found; run the parity test first", file=sys.stderr)
        return 2
    print("\n".join(render(report, args.all)))
    failed = sum(1 for f in report["fixtures"] if not f["pass"])
    gate_b = load(DEFAULT_GATE_B_REPORT) if args.report == DEFAULT_REPORT else None
    if gate_b is not None:
        print()
        print("\n".join(render_gate(gate_b)))
        failed += 0 if gate_b["pass"] else 1
    return 1 if args.strict and failed else 0


if __name__ == "__main__":
    sys.exit(main())
