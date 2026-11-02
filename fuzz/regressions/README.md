# Fuzz regression set

Minimised reproducers of fixed fuzz findings, one directory per target:
`open/`, `text/`, `render/`, `cmap/`, `unicode/`. Every file here is replayed
by the ordinary test suite on stable, through the same driver and limits as
its fuzz target, on a 512 KiB thread stack:

- `crates/core/tests/fuzz_regressions.rs` replays `open`, `text`, `cmap` and
  `unicode`;
- `crates/render/tests/fuzz_regressions.rs` replays `render`.

The hardening workflow's fuzz smoke run also starts each target from its
directory here.

## Adding a reproducer

Commit it with the fix, named by the first 16 hex digits of its SHA-256:

```sh
f=fuzz/artifacts/text/minimized-from-...   # from `cargo fuzz tmin`
mkdir -p fuzz/regressions/text
cp "$f" "fuzz/regressions/text/$(shasum -a 256 "$f" | cut -c1-16)"
cargo test -p pdf-extract-core --test fuzz_regressions
```

Files whose names start with `.` or `README` are not replayed.
