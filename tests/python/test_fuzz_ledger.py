"""The local fuzzing ledger and session script (design D8; spec "Clean fuzzing
hours are counted per target in a committed ledger", "Long fuzzing runs
locally with a persistent corpus").

``tools/fuzz/ledger.py`` is tested directly. ``tools/fuzz/run.sh`` is run
end to end against a stub ``cargo`` on ``PATH`` that plays cargo-fuzz's part
(a clean run, a crash, or a run that is interrupted), so the session logic
that decides what the ledger is told is tested without a nightly toolchain.
"""

from __future__ import annotations

import json
import os
import shutil
import signal
import stat
import subprocess
import sys
import time
from pathlib import Path

import pytest

from pdfs import REPO

sys.path.insert(0, str(REPO / "tools" / "fuzz"))
import ledger

RUN_SH = REPO / "tools" / "fuzz" / "run.sh"
HOUR = 3600


def test_committed_ledger_names_every_target() -> None:
    stored = json.loads((REPO / "fuzz" / "ledger.json").read_text())
    assert sorted(stored) == sorted(ledger.TARGETS)
    assert all(set(e) == set(ledger.empty_entry()) for e in stored.values())


def test_clean_session_adds_the_time_fuzzed() -> None:
    book = ledger.initial()
    ledger.record(book, "text", 3600, clean=True, commit="abc")
    ledger.record(book, "text", 1800, clean=True, commit="def")
    entry = book["text"]
    assert entry["clean_seconds"] == 5400
    assert entry["sessions"] == 2
    assert entry["commit"] == "def"
    assert entry["last_session"] is not None
    assert entry["last_reset"] is None
    assert book["open"] == ledger.empty_entry()


def test_finding_resets_to_zero() -> None:
    book = ledger.initial()
    ledger.record(book, "render", 10 * HOUR, clean=True, commit="abc")
    ledger.record(book, "render", 600, clean=False, commit="abc")
    entry = book["render"]
    assert entry["clean_seconds"] == 0
    assert entry["sessions"] == 2
    assert entry["last_reset"] == entry["last_session"]
    ledger.record(book, "render", 60, clean=True, commit="abc")
    assert book["render"]["clean_seconds"] == 60


def test_manual_reset_keeps_the_session_count() -> None:
    book = ledger.initial()
    ledger.record(book, "cmap", 100, clean=True, commit=None)
    ledger.reset(book, "cmap")
    assert book["cmap"]["clean_seconds"] == 0
    assert book["cmap"]["sessions"] == 1
    assert book["cmap"]["last_reset"] is not None


def test_depth_goal_is_met_at_72_hours_for_every_target() -> None:
    book = ledger.initial()
    for target in ledger.TARGETS:
        ledger.record(book, target, 72 * HOUR - 1, clean=True, commit=None)
    assert not ledger.goal_met(book)
    assert ledger.status_lines(book)[-1].startswith("Fuzzing depth goal: not met")
    for target in ledger.TARGETS[:-1]:
        ledger.record(book, target, 1, clean=True, commit=None)
    assert not ledger.goal_met(book), "one target short of 72 h"
    ledger.record(book, ledger.TARGETS[-1], 1 + HOUR, clean=True, commit=None)
    assert ledger.goal_met(book)
    assert ledger.status_lines(book)[-1].startswith("Fuzzing depth goal: met")


def test_invalid_records_are_refused() -> None:
    book = ledger.initial()
    with pytest.raises(ValueError, match="unknown target"):
        ledger.record(book, "ocr", 1, clean=True, commit=None)
    with pytest.raises(ValueError, match="negative"):
        ledger.record(book, "open", -1, clean=True, commit=None)
    with pytest.raises(ValueError, match="unknown target"):
        ledger.reset(book, "ocr")


def test_cli_round_trip(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    path = tmp_path / "ledger.json"
    assert ledger.main(["--ledger", str(path), "status"]) == 0
    assert not path.exists(), "status does not write"
    out = capsys.readouterr().out
    assert "Fuzzing depth goal: not met" in out
    assert "not a release gate" in out
    args = ["--ledger", str(path), "record", "unicode", "--seconds", "90"]
    assert ledger.main([*args, "--clean", "--commit", "c0ffee"]) == 0
    assert "unicode      0.03 h clean" in capsys.readouterr().out
    stored = json.loads(path.read_text())
    assert stored["unicode"]["clean_seconds"] == 90
    assert stored["unicode"]["commit"] == "c0ffee"
    assert ledger.main([*args, "--clean"]) == 0
    assert json.loads(path.read_text())["unicode"]["commit"] == ledger.head_commit()
    assert ledger.main([*args, "--finding"]) == 0
    assert json.loads(path.read_text())["unicode"]["clean_seconds"] == 0
    assert ledger.main(["--ledger", str(path), "reset", "unicode"]) == 0
    assert ledger.main([*args[:-1], "-5", "--clean"]) == 2
    with pytest.raises(SystemExit):
        ledger.main([*args])  # neither --clean nor --finding


def test_load_rejects_an_unknown_target(tmp_path: Path) -> None:
    path = tmp_path / "ledger.json"
    path.write_text(json.dumps({"ocr": ledger.empty_entry()}))
    with pytest.raises(ValueError, match="unknown target"):
        ledger.load(path)
    assert ledger.main(["--ledger", str(path), "status"]) == 2


# --- run.sh against a stub cargo -------------------------------------------

STUB_CARGO = r"""#!/usr/bin/env bash
# Stub cargo for test_fuzz_ledger.py: plays cargo-fuzz per $STUB_RUN.
shift  # +toolchain
arg() { local p=$1; shift; for a; do case "$a" in "$p"*) echo "${a#"$p"}";; esac; done; }
if [[ "$1" == build ]]; then
  mkdir -p "$CARGO_TARGET_DIR/release"
  for bin in replay_small_stack synthetic_cmaps; do
    printf '#!/bin/sh\necho replay "$@" >>"%s/replays"\nexit %s\n' \
      "$STUB_DIR" "${STUB_REPLAY_STATUS:-0}" >"$CARGO_TARGET_DIR/release/$bin"
    chmod +x "$CARGO_TARGET_DIR/release/$bin"
  done
  exit 0
fi
[[ "$1" == fuzz ]] || exit 99
echo "$2" >>"$STUB_DIR/calls"
case "$2" in
  build | cmin) exit 0 ;;
  tmin) touch "$(arg -artifact_prefix= "$@")minimized-from-0001"; exit 0 ;;
  run)
    prefix=$(arg -artifact_prefix= "$@")
    case "$STUB_RUN" in
      clean) sleep 1; exit 0 ;;
      crash) sleep 1; echo boom >"${prefix}crash-0001"; exit 77 ;;
      broken) exit 1 ;;
      hang) touch "$STUB_DIR/fuzzing"; exec sleep 60 ;;
    esac
    ;;
esac
exit 98
"""


def run_sh_env(tmp_path: Path, mode: str) -> dict[str, str]:
    stub = tmp_path / "bin"
    stub.mkdir()
    cargo = stub / "cargo"
    cargo.write_text(STUB_CARGO)
    cargo.chmod(cargo.stat().st_mode | stat.S_IEXEC)
    led = tmp_path / "ledger.json"
    book = ledger.initial()
    for target in ledger.TARGETS:
        ledger.record(book, target, 1000, clean=True, commit="before")
    ledger.save(led, book)
    return {
        **os.environ,
        "PATH": f"{stub}{os.pathsep}{os.environ['PATH']}",
        "STUB_DIR": str(tmp_path),
        "STUB_RUN": mode,
        "FUZZ_STATE_DIR": str(tmp_path / "state"),
        "FUZZ_LEDGER": str(led),
        "PYTHON": sys.executable,
    }


def stored(tmp_path: Path, target: str) -> ledger.Entry:
    return ledger.load(tmp_path / "ledger.json")[target]


needs_bash = pytest.mark.skipif(shutil.which("bash") is None, reason="run.sh needs bash")


@needs_bash
def test_run_sh_credits_a_clean_session_and_replays(tmp_path: Path) -> None:
    env = run_sh_env(tmp_path, "clean")
    out = subprocess.run([str(RUN_SH), "--minutes", "5", "open"], env=env, check=False)
    assert out.returncode == 0
    entry = stored(tmp_path, "open")
    # Wall time of the stub's one-second run, not the five minutes asked for.
    assert 1000 + 1 <= entry["clean_seconds"] <= 1000 + 30
    assert entry["sessions"] == 2
    assert (tmp_path / "calls").read_text().split() == ["build", "run", "cmin"]
    assert "open" in (tmp_path / "replays").read_text()
    corpus = list((tmp_path / "state" / "corpus" / "open").iterdir())
    assert len(corpus) >= 300, "corpus B and C merged in"


@needs_bash
def test_run_sh_failed_replay_is_a_finding(tmp_path: Path) -> None:
    env = {**run_sh_env(tmp_path, "clean"), "STUB_REPLAY_STATUS": "1"}
    out = subprocess.run([str(RUN_SH), "--minutes", "1", "render"], env=env, check=False)
    assert out.returncode == 1
    assert stored(tmp_path, "render")["clean_seconds"] == 0
    logs = list((tmp_path / "state" / "artifacts" / "render").glob("replay-*.log"))
    assert len(logs) == 1


@needs_bash
def test_run_sh_replay_that_cannot_run_is_not_a_finding(tmp_path: Path) -> None:
    # Status 2 is the replay's usage/unreadable-input exit, not a panic.
    env = {**run_sh_env(tmp_path, "clean"), "STUB_REPLAY_STATUS": "2"}
    out = subprocess.run([str(RUN_SH), "--minutes", "1", "text"], env=env, check=False)
    assert out.returncode == 2
    assert stored(tmp_path, "text")["clean_seconds"] == 1000
    assert stored(tmp_path, "text")["sessions"] == 1


@needs_bash
def test_run_sh_finding_resets_and_stops(tmp_path: Path) -> None:
    env = run_sh_env(tmp_path, "crash")
    out = subprocess.run([str(RUN_SH), "unicode", "cmap"], env=env, check=False)
    assert out.returncode == 1
    assert stored(tmp_path, "unicode")["clean_seconds"] == 0
    assert stored(tmp_path, "unicode")["last_reset"] is not None
    assert stored(tmp_path, "cmap")["clean_seconds"] == 1000, "the session stopped"
    artifacts = tmp_path / "state" / "artifacts" / "unicode"
    assert (artifacts / "crash-0001").exists()
    assert (artifacts / "minimized-from-0001").exists()


@needs_bash
def test_run_sh_tool_failure_records_nothing(tmp_path: Path) -> None:
    env = run_sh_env(tmp_path, "broken")
    out = subprocess.run([str(RUN_SH), "unicode"], env=env, check=False)
    assert out.returncode == 2
    assert stored(tmp_path, "unicode")["sessions"] == 1


@needs_bash
def test_interrupted_session_credits_only_elapsed_time(tmp_path: Path) -> None:
    env = run_sh_env(tmp_path, "hang")
    # Its own process group, so the interrupt reaches the script and the
    # stub fuzzer together, as a terminal's Ctrl-C does.
    proc = subprocess.Popen(
        [str(RUN_SH), "--minutes", "60", "unicode"], env=env, start_new_session=True
    )
    deadline = time.monotonic() + 60
    while not (tmp_path / "fuzzing").exists():
        assert time.monotonic() < deadline, "the stub fuzzer never started"
        assert proc.poll() is None, "run.sh exited before fuzzing"
        time.sleep(0.1)
    time.sleep(2)
    os.killpg(proc.pid, signal.SIGINT)
    assert proc.wait(timeout=60) == 0
    entry = stored(tmp_path, "unicode")
    assert 1000 + 1 <= entry["clean_seconds"] <= 1000 + 30, entry
    assert entry["sessions"] == 2
    assert (tmp_path / "state" / "corpus" / "unicode").is_dir(), "corpus kept"
    assert "cmin" not in (tmp_path / "calls").read_text().split()


@needs_bash
def test_run_sh_rejects_bad_arguments(tmp_path: Path) -> None:
    env = run_sh_env(tmp_path, "clean")
    for args in (["ocr"], ["--minutes", "0"], ["--minutes"]):
        out = subprocess.run([str(RUN_SH), *args], env=env, check=False, capture_output=True)
        assert out.returncode == 2, args
