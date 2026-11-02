"""Helpers for the Python tests: fixture paths, small PDF assembly, threads.

``Fixtures`` gives named access to the generated fixture directory that
``conftest.py`` fills from the Rust test kit. ``assemble_pdf`` and
``set_xref_offset`` build the few contract inputs that the Rust tests also
assemble by hand (``crates/core/tests/common::assemble_pdf``) and edit xref
tables in place; they mirror the Rust helpers byte for byte so the two suites
test the same files. ``run_threads`` and ``on_small_stack`` are the thread
harnesses of the concurrency and stack tests (run spec §4.12).
"""

from __future__ import annotations

import ctypes
import queue
import sys
import threading
from collections.abc import Callable, Sequence
from pathlib import Path
from typing import cast

REPO = Path(__file__).resolve().parents[2]
CORPUS_C = REPO / "tests" / "fixtures" / "corpus-c"
#: The builder call script both test kits replay (golden test, spec §4.12).
GOLDEN_SCRIPT = REPO / "crates" / "testkit" / "tests" / "data" / "builder_golden.tsv"
#: The thread stack size the spec promises extraction fits in (§2.3, §4.12).
SMALL_STACK = 512 * 1024
#: How far above the requested size a measured stack may be and still be
#: the one asked for: macOS reports 512 KiB as 524,288 + 12,288 bytes, Linux
#: exactly; a reused or default stack is megabytes larger.
STACK_SLACK = 64 * 1024
#: Bound on the oversized threads one ``on_small_stack`` call parks. glibc's
#: stack cache holds 40 MiB by default, so at most 20 of Rust's 2 MiB
#: blocks; needing more means something other than the cache is at work.
MAX_PARKED = 64


class Fixtures:
    """Named access to the generated fixture directory."""

    def __init__(self, root: Path) -> None:
        self.root = root

    def path(self, name: str) -> Path:
        """Path of ``<name>.pdf``; ``name`` may include a subdirectory, as in
        ``samples/builder_type3``."""
        path = self.root / f"{name}.pdf"
        if not path.is_file():
            raise FileNotFoundError(f"no generated fixture {name!r} in {self.root}")
        return path

    def bytes(self, name: str) -> bytes:
        return self.path(name).read_bytes()

    def index(self) -> list[tuple[Path, str]]:
        """Every generated file with its group (``contract``, ``hostile``,
        ``sample-builder``, ...)."""
        rows = []
        for line in (self.root / "index.tsv").read_text().splitlines():
            rel, group = line.split("\t")
            rows.append((self.root / rel, group))
        return rows


def corpus_c(name: str) -> Path:
    """A committed corpus-C fixture."""
    return CORPUS_C / name


def assemble_pdf(bodies: Sequence[str | bytes]) -> bytes:
    """A classic-xref PDF whose objects 1..n are ``bodies``; object 1 is the
    catalog (``/Root 1 0 R``)."""
    out = bytearray(b"%PDF-1.7\n")
    offsets = []
    for i, body in enumerate(bodies, start=1):
        offsets.append(len(out))
        out += f"{i} 0 obj\n".encode()
        out += body.encode() if isinstance(body, str) else body
        out += b"\nendobj\n"
    xref_at = len(out)
    tail = f"xref\n0 {len(bodies) + 1}\n0000000000 65535 f \n"
    tail += "".join(f"{offset:010} 00000 n \n" for offset in offsets)
    tail += f"trailer\n<< /Size {len(bodies) + 1} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF\n"
    out += tail.encode()
    return bytes(out)


#: An 8 x 8 all-black bitmap in T.6 (Group 4) coding, the CCITT fixture of
#: ``crates/render/tests/common/codecs.rs``.
MMR_BLACK_8X8 = bytes([0x26, 0xA2, 0xFF, 0xFE])


def ccitt_image_pdf() -> bytes:
    """A 200 x 200 pt page drawing one 8 x 8 CCITT image twice, side by side
    (the codec-panic tests: an armed codec panic skips only the first)."""
    image = (
        b"<< /Type /XObject /Subtype /Image /Width 8 /Height 8 /ColorSpace /DeviceGray "
        b"/BitsPerComponent 1 /Filter /CCITTFaxDecode "
        b"/DecodeParms << /K -1 /Columns 8 /Rows 8 >> /Length 4 >>\nstream\n"
        + MMR_BLACK_8X8
        + b"\nendstream"
    )
    content = b"q 100 0 0 100 0 100 cm /Im0 Do Q q 100 0 0 100 100 100 cm /Im0 Do Q"
    return assemble_pdf(
        [
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] "
            "/Resources << /XObject << /Im0 5 0 R >> >> /Contents 4 0 R >>",
            b"<< /Length %d >>\nstream\n" % len(content) + content + b"\nendstream",
            image,
        ]
    )


def xref_entries_start(data: bytes) -> int:
    """Offset of the first 20-byte entry of the last classic xref section."""
    section = data.rindex(b"\nxref\n") + len(b"\nxref\n")
    return data.index(b"\n", section) + 1


def set_xref_offset(data: bytes, obj: int, offset: int) -> bytes:
    """Points the xref entry of object ``obj`` at byte ``offset``, in place
    (same length, so nothing else moves)."""
    at = xref_entries_start(data) + 20 * obj
    out = bytearray(data)
    out[at : at + 10] = f"{offset:010}".encode()
    return bytes(out)


def run_threads[T](count: int, work: Callable[[int], T]) -> list[T]:
    """Runs ``work(i)`` for ``i`` in ``range(count)``, each on its own thread,
    and returns the results in that order.

    All threads wait on one barrier before calling ``work``, so they start
    together and contend as much as the machine allows. An exception in any
    thread is re-raised here (the first by thread number) after every thread
    has finished; a thread that raises never leaves the others stuck, because
    the barrier is passed before ``work`` runs.
    """
    barrier = threading.Barrier(count)
    results: list[T | None] = [None] * count
    errors: list[BaseException | None] = [None] * count

    def run(i: int) -> None:
        try:
            barrier.wait()
            results[i] = work(i)
        except BaseException as e:  # re-raised on the calling thread
            errors[i] = e

    threads = [threading.Thread(target=run, args=(i,)) for i in range(count)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    for e in errors:
        if e is not None:
            raise e
    # Every slot was filled by `work` (no error was recorded).
    return cast("list[T]", results)


def thread_stack_size() -> int | None:
    """The calling thread's actual stack size in bytes, or ``None`` where it
    cannot be asked (neither macOS nor Linux).

    macOS answers with ``pthread_get_stacksize_np``; Linux (glibc or musl)
    with ``pthread_getattr_np``, which for a thread other than the main one
    reports the stack block it was given, guard page excluded.
    """
    # A plain str, so the type checker does not prune a branch as
    # unreachable on the platform it runs on.
    platform: str = sys.platform
    libc = ctypes.CDLL(None)
    libc.pthread_self.restype = ctypes.c_void_p
    if platform == "darwin":
        libc.pthread_get_stacksize_np.argtypes = [ctypes.c_void_p]
        libc.pthread_get_stacksize_np.restype = ctypes.c_size_t
        return int(libc.pthread_get_stacksize_np(libc.pthread_self()))
    if not platform.startswith("linux"):
        return None
    libc.pthread_getattr_np.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
    libc.pthread_attr_getstacksize.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_size_t)]
    libc.pthread_attr_destroy.argtypes = [ctypes.c_void_p]
    # Opaque pthread_attr_t: 56 bytes on x86-64 glibc, 64 on aarch64; 256
    # leaves room for any ABI.
    attr = ctypes.create_string_buffer(256)
    if libc.pthread_getattr_np(libc.pthread_self(), attr) != 0:
        raise OSError("pthread_getattr_np failed")
    try:
        size = ctypes.c_size_t()
        if libc.pthread_attr_getstacksize(attr, ctypes.byref(size)) != 0:
            raise OSError("pthread_attr_getstacksize failed")
        return size.value
    finally:
        libc.pthread_attr_destroy(attr)


def stack_fits(measured: int | None, stack_size: int = SMALL_STACK) -> bool:
    """Whether a thread whose stack measured ``measured`` bytes has the
    ``stack_size`` stack it asked for (``None``, unmeasurable, is taken on
    trust)."""
    return measured is None or stack_size <= measured <= stack_size + STACK_SLACK


def on_small_stack[T](work: Callable[[], T], stack_size: int = SMALL_STACK) -> T:
    """Runs ``work()`` on a new thread whose stack is ``stack_size`` bytes and
    returns its result (re-raising its exception).

    ``threading.stack_size`` is process-wide and applies to threads started
    after the call, so it is restored as soon as each thread has started;
    the Rust code called from the thread runs on the same small stack.

    The setting alone does not guarantee the size on Linux (internal issue #70):
    glibc serves a new thread from its cache of exited threads' stacks,
    taking the smallest free block of at least the requested size, even one
    up to four times larger (``get_cached_stack`` in nptl/allocatestack.c).
    Rust's worker threads (``extract(workers=...)``) run on 2 MiB stacks,
    which is within that bound of 512 KiB plus the guard page, so once they
    have exited a 512 KiB request can be handed one of their 2 MiB blocks.
    So each thread measures its own stack first; one that is too big is
    parked, holding its block so the next thread cannot be given it, and
    another is started, until one has the requested size. Parked threads
    exit when ``work`` is done; the exact-size block that thread leaves in
    the cache is an exact match for the next call, so later calls rarely
    need to park any.
    """
    outcome: list[T] = []
    error: list[BaseException] = []
    sizes: queue.SimpleQueue[int | None] = queue.SimpleQueue()
    release = threading.Event()

    def run() -> None:
        try:
            measured = thread_stack_size()
        except BaseException as e:  # unblocks the caller, which re-raises it
            error.append(e)
            sizes.put(None)
            return
        sizes.put(measured)
        if not stack_fits(measured, stack_size):
            release.wait()
            return
        try:
            outcome.append(work())
        except BaseException as e:  # re-raised on the calling thread
            error.append(e)

    parked: list[threading.Thread] = []
    try:
        while True:
            previous = threading.stack_size(stack_size)
            try:
                thread = threading.Thread(target=run)
                thread.start()
            finally:
                threading.stack_size(previous)
            measured = sizes.get()
            if stack_fits(measured, stack_size):
                break
            parked.append(thread)
            if len(parked) > MAX_PARKED:
                raise RuntimeError(
                    f"{len(parked)} threads asked for a {stack_size}-byte stack "
                    f"and were given more (last: {measured} bytes)"
                )
        thread.join()
    finally:
        release.set()
        for t in parked:
            t.join()
    if error:
        raise error[0]
    return outcome[0]
