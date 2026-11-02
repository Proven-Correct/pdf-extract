---
id: 0086
title: Image-codec panics are caught at the codec boundary; the panic hook belongs to the application
date: 2026-10-19
status: Accepted
summary: The vendored hayro-syntax runs each image-codec call in catch_unwind (H18) and skips the image with an image decode failed warning, compiled out under cfg(fuzzing); the library never installs a panic hook for Rust callers. Partially supersedes ADR 0035.
---

# 0086. Image-codec panics are caught at the codec boundary; the panic hook belongs to the application

## Context

ADR 0035 says the Rust API catches no panics and only the Python boundary turns them into `InternalError`. The vendored image codecs (zune-jpeg, hayro-ccitt, hayro-jbig2, hayro-jpeg2000) still contain panic sites that input can reach (internal #76; ADR 0079's 2026-10-12 addendum). Under ADR 0035 one crafted image fails the whole page: Rust callers get a panic and Python callers get `InternalError`. That breaks R1, which says a crafted image must not fail the page. The internal #63 fix pass patched one panic shape (H16) and concluded that patching shapes one at a time never ends. Fuzzing is what finds the reachable sites (change `harden-fuzzing-sanitizers-loom`). A boundary is what stops them failing a page in production.

This record partially supersedes ADR 0035. It replaces 0035's "the Rust API does not catch panics" for image-codec decodes only. Everything else in 0035 stands. The index tooling has no partial-supersession status, so 0035 stays Accepted with an addendum pointing here.

## Decision

- **Where the catch is.** Vendor patch 0028 (H18) adds `guard_codec` to hayro-syntax `filter/mod.rs`. It runs a closure in `catch_unwind(AssertUnwindSafe(..))` and maps a panic to `DecodeFailure::ImageDecode`.
- **What it wraps.** Only the calls into the codec crates:
  - DCT: the zune-jpeg header and pixel decode, the frame check between them, and the YCCK fix-up of the output.
  - CCITT: each `hayro_ccitt::decode` call.
  - JBIG2: `Image::new_embedded` and `decode_with`.
  - JPEG 2000: each `Image::new` and the `decode` call.
- **What stays outside.** The `JBIG2Globals` resolution (xref and Flate), the image dictionary, `Filter::apply` and the rest of the parser are not wrapped, so a parser bug still propagates under ADR 0035.
- **What happens downstream.** A caught panic follows the existing undecodable-image path, with nothing changed: `InterpreterWarning::ImageDecodeFailure`, then `WarningKind::ImageDecodeFailure` (`image: decode failed`). The image draws nothing and the rest of the page is processed.
- **Fuzzing and `no_std`.** The catch is `cfg(all(feature = "std", not(fuzzing)))`. `cargo fuzz` passes `--cfg fuzzing` to every crate, so a fuzzing build sees every codec panic as a crash. `no_std` builds have no unwinding to catch. The vendored manifest declares `check-cfg` for `cfg(fuzzing)`. `hayro_syntax::CODEC_PANICS_CAUGHT` reports which build is running.
- **The test hook.** hayro-syntax gains a test-only feature, `codec-panic-hook`, with two per-thread, fire-once triggers:
  - `arm_codec_panic_for_test` panics inside the guard on the next codec call.
  - `arm_unguarded_image_panic_for_test` panics in the codec module after the image's parameters (and JBIG2's globals) are resolved, just before the guard, to prove the guard's scope: a guard widened to the module body or to `Filter::apply` would catch it.

  Core forwards the feature as `test-hooks` and re-exports the first trigger. Core's and render's dev-dependencies enable it. The Python binding enables it unconditionally and exposes `_native._arm_codec_panic_for_test()`, like the existing `_panic_for_test`. The production cost is one thread-local read per guarded codec call, a handful per image decode.
- **Who owns the panic hook.** The library never calls `panic::set_hook` for Rust callers. When a codec panic is caught, the application's hook runs: the default hook prints, and that is the application's choice. The Python binding keeps its silent module-init hook (ADR 0035, T48). That hook records every panic's message in the thread-local `LAST_PANIC`, including a codec panic that no `guarded` frame unwinds. So `guarded` now clears the slot on entry. It also uses a recorded message only when the message starts with the unwinding panic's payload text. That second check covers a panic re-raised with `resume_unwind`, which skips the hook: core's `run_parallel` does this with a worker thread's panic, while the calling thread's slot may hold a codec panic caught earlier in the same call. Together, the two checks mean an `InternalError` never carries a caught codec panic's message.

## Alternatives considered

- **Wrap `Filter::apply`.** One site instead of four, but it would also swallow panics in xref resolution, lazy repair and the nested Flate decode of `JBIG2Globals`. Those are parser bugs, which must keep propagating.
- **Catch at core's page boundary.** The whole page would still fail, which is exactly what internal #76 removes.
- **Audit and patch each panic site.** Rejected in the internal #63 fix pass as a chain that never ends. Fuzzing finds the sites, and each fix still goes through the ordinary issue workflow.
- **A new `ImageDecoderPanic` warning kind.** The user chose to reuse `ImageDecodeFailure`. A codec panic is, to the caller, an image that could not be decoded.
- **A runtime switch instead of `cfg(fuzzing)`.** It would do nothing useful in production and is easy to leave on by mistake.
- **A library-installed quiet hook, chained and scoped to the thread.** It seizes process-global state for the library's convenience.
- **Catch only when called from Python.** That would give one Rust API two behaviours.
- **A committed fixture that panics a codec today, instead of a hook.** Fuzzing would make it stale as soon as the bug behind it is fixed.

## Consequences

- **Caught codec panics are hidden.** Production Rust callers no longer see codec bugs as panics. They see a warning, plus whatever their own panic hook does. Fuzzing still sees every one (`cfg(fuzzing)`), and the warning still records the skipped image.
- **`AssertUnwindSafe` rests on locality.** The wrapped closures borrow only the encoded data, decoder state local to the decode, and copies of settings. Our own code that runs inside the guard touches only that local state: the CCITT and JBIG2 pixel sinks the codecs call back into, and the JPEG frame check. Inside the guard, the thread-local counters are bumped only after a codec call has returned, immediately before the error is returned (the JPEG frame check, the memory-budget mappings). A new call site must keep to the same rule.
- **Output on fd 2 from Rust is the application's choice.** For Rust callers, "the library writes nothing to fd 1 or 2" now means the library itself writes nothing, and the application's panic hook decides about panic output. From Python nothing is written (`tests/python/test_no_output.py`).
- **Other panics are unchanged.** A panic anywhere else still propagates to Rust callers and becomes `InternalError` in Python (ADR 0035).
- **Codec bugs still need fixing.** A codec that panics is a bug that fuzzing will report, and the boundary is not a reason to leave it unfixed.
- **The tests** are `crates/core/tests/h18_codec_guard.rs` (the hook, the per-thread arming, the guard's scope, and the cfg assertion); `crates/render/tests/codec_panic.rs` (all four codecs, end to end); `crates/render/tests/codec_panic_hook.rs` (the application's hook runs, in its own binary); and the Python tests in `test_errors.py` and `test_no_output.py`.
