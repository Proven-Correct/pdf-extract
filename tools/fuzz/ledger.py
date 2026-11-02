#!/usr/bin/env python3
"""The clean-hours ledger of local fuzzing (design D8; spec "Clean fuzzing
hours are counted per target in a committed ledger").

``fuzz/ledger.json`` holds, per fuzz target, the clean fuzzing time
accumulated since its last finding. ``tools/fuzz/run.sh`` records each
session; the developer commits the updated file. The depth goal is met when
every target has at least 72 clean hours. It is a goal, not a Phase 3 exit or
a release gate: ADR 0093 waived it, and callers contain crashes instead.

Policy: a finding resets its target to zero; unrelated commits do not. When a
fix changes a target's code path, a reviewer may reset that target by hand
(``reset``). Each session stores the ``HEAD`` commit it fuzzed, so a reviewer
can see which code the hours were earned on. The ledger is a progress log,
not an attestation: it records what the script measured.

Usage:
    ledger.py [--ledger PATH] record TARGET --seconds N (--clean | --finding)
                                     [--commit SHA]
    ledger.py [--ledger PATH] reset TARGET
    ledger.py [--ledger PATH] status
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
from datetime import datetime, timezone
from pathlib import Path
from typing import TypedDict

REPO = Path(__file__).resolve().parents[2]
DEFAULT_LEDGER = REPO / "fuzz" / "ledger.json"
TARGETS = ("open", "text", "render", "cmap", "unicode")
GOAL_HOURS = 72
GOAL_SECONDS = GOAL_HOURS * 3600


class Entry(TypedDict):
    clean_seconds: int
    sessions: int
    last_reset: str | None
    last_session: str | None
    commit: str | None


Ledger = dict[str, Entry]


def empty_entry() -> Entry:
    return {
        "clean_seconds": 0,
        "sessions": 0,
        "last_reset": None,
        "last_session": None,
        "commit": None,
    }


def initial() -> Ledger:
    return {t: empty_entry() for t in TARGETS}


def load(path: Path) -> Ledger:
    """The ledger at ``path``; a missing file or target is all zeros."""
    ledger = initial()
    if path.exists():
        stored = json.loads(path.read_text())
        for target, entry in stored.items():
            if target not in TARGETS:
                raise ValueError(f"{path}: unknown target {target!r}")
            ledger[target] = {**empty_entry(), **entry}
    return ledger


def save(path: Path, ledger: Ledger) -> None:
    """Writes ``ledger`` atomically: an interrupted write leaves the old file."""
    text = json.dumps({t: ledger[t] for t in TARGETS}, indent=2) + "\n"
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=path.parent, prefix=".ledger-")
    with os.fdopen(fd, "w") as f:
        f.write(text)
    os.replace(tmp, path)


def now() -> str:
    # timezone.utc, not 3.11's datetime.UTC: run.sh calls this with the
    # system python3, which is 3.9 on macOS.
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat()  # noqa: UP017


def record(
    ledger: Ledger,
    target: str,
    seconds: int,
    *,
    clean: bool,
    commit: str | None,
) -> None:
    """One session of ``target``: ``seconds`` of clean fuzzing are credited,
    or a finding resets the total. Only the time actually fuzzed is passed in
    (``run.sh`` measures wall time), so an interrupted session credits what
    it ran, not what was requested."""
    check_target(target)
    if seconds < 0:
        raise ValueError(f"negative session length {seconds}")
    entry = ledger[target]
    at = now()
    entry["sessions"] += 1
    entry["last_session"] = at
    entry["commit"] = commit
    if clean:
        entry["clean_seconds"] += seconds
    else:
        entry["clean_seconds"] = 0
        entry["last_reset"] = at


def reset(ledger: Ledger, target: str) -> None:
    """A reviewer's reset after a fix changed ``target``'s code path."""
    check_target(target)
    ledger[target]["clean_seconds"] = 0
    ledger[target]["last_reset"] = now()


def goal_met(ledger: Ledger) -> bool:
    return all(ledger[t]["clean_seconds"] >= GOAL_SECONDS for t in TARGETS)


def status_lines(ledger: Ledger) -> list[str]:
    lines = []
    for t in TARGETS:
        e = ledger[t]
        commit = (e["commit"] or "-")[:12]
        lines.append(
            f"{t:<8} {e['clean_seconds'] / 3600:8.2f} h clean"
            f"  sessions {e['sessions']:<4} last {e['last_session'] or '-'}  commit {commit}"
        )
    verdict = "met" if goal_met(ledger) else "not met"
    lines.append(
        f"Fuzzing depth goal: {verdict} ({GOAL_HOURS} h clean per target;"
        " not a release gate, ADR 0093)"
    )
    return lines


def check_target(target: str) -> None:
    if target not in TARGETS:
        raise ValueError(f"unknown target {target!r}; expected one of {', '.join(TARGETS)}")


def head_commit() -> str | None:
    try:
        out = subprocess.run(
            ["git", "-C", str(REPO), "rev-parse", "HEAD"],
            check=True,
            capture_output=True,
            text=True,
        )
    except (OSError, subprocess.CalledProcessError):
        return None
    return out.stdout.strip() or None


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--ledger", type=Path, default=DEFAULT_LEDGER)
    sub = parser.add_subparsers(dest="command", required=True)
    rec = sub.add_parser("record", help="record one session of a target")
    rec.add_argument("target", choices=TARGETS)
    rec.add_argument("--seconds", type=int, required=True, help="time actually fuzzed")
    outcome = rec.add_mutually_exclusive_group(required=True)
    outcome.add_argument("--clean", action="store_true")
    outcome.add_argument("--finding", action="store_true")
    rec.add_argument("--commit", help="commit fuzzed (default: HEAD)")
    rst = sub.add_parser("reset", help="reset a target's clean total by hand")
    rst.add_argument("target", choices=TARGETS)
    sub.add_parser("status", help="print every target's total and the depth-goal verdict")
    args = parser.parse_args(argv)

    try:
        ledger = load(args.ledger)
        if args.command == "record":
            commit = args.commit or head_commit()
            record(ledger, args.target, args.seconds, clean=args.clean, commit=commit)
            save(args.ledger, ledger)
        elif args.command == "reset":
            reset(ledger, args.target)
            save(args.ledger, ledger)
    except ValueError as e:
        print(f"ledger: {e}", file=sys.stderr)
        return 2
    print("\n".join(status_lines(ledger)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
