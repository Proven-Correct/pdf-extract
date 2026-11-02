//! Source scan for spec T45 / R1: every exported Python-facing function body
//! must run inside the panic guard (`guarded`), so a Rust panic surfaces as
//! `InternalError` and never as `pyo3.PanicException`.
//!
//! Why a scan: the only runtime test of the guard goes through the injected
//! `_panic_for_test` helper, so deleting `guarded` from a real method such as
//! `Page.text` failed no test (internal issue #22). The extension crate has
//! `test = false` (it cannot link a test binary against libpython), so, like
//! `license.rs` and `wordlist.rs`, the check lives here and reads the source.
//!
//! **This is an approximate, syntactic check.** Rules, applied to every
//! `crates/python/src/*.rs`:
//! - each fn in an `impl` carrying `#[pymethods]`, and each fn carrying
//!   `#[pyfunction]`, must have a body that is exactly one tail expression: a
//!   call whose callee path ends in `guarded` (`guarded(..)` or
//!   `guard::guarded(..)`). Any other statement, before or after, fails, since
//!   code outside the closure would run unguarded;
//! - a `#[pyo3(get)]` / `#[pyo3(set)]` field, or `get_all` / `set_all` on
//!   `#[pyclass]`, fails: PyO3 generates those accessors without any body of
//!   ours, so they cannot be guarded;
//! - a macro invocation among the items of a `#[pymethods]` impl fails: its
//!   expansion cannot be inspected.
//!
//! Deliberately out of scope: PyO3 argument extraction (runs before the body,
//! and reports failures as ordinary exceptions); the `#[pymodule]` init
//! (documented as unguarded, spec 2.3); macro-generated slots (none today);
//! whether the callee named `guarded` is really `guard::guarded` (a local
//! shadowing function would fool the scan; code review catches that).

use std::path::{Path, PathBuf};

use syn::{Attribute, Expr, ImplItem, Item, ItemFn, Stmt};

/// Does any attribute's path end in `name` (so `pyo3::pymethods` matches)?
fn has_attr(attrs: &[Attribute], name: &str) -> bool {
    attrs
        .iter()
        .any(|a| a.path().segments.last().is_some_and(|s| s.ident == name))
}

/// Words inside the parenthesised arguments of every attribute called `name`.
/// Token-stream text split on non-identifier characters is crude but enough to
/// spot `get`/`set` among `#[pyo3(get, name = "x")]` style arguments.
fn attr_words(attrs: &[Attribute], name: &str) -> Vec<String> {
    attrs
        .iter()
        .filter(|a| a.path().is_ident(name))
        .filter_map(|a| a.meta.require_list().ok())
        .flat_map(|l| {
            l.tokens
                .to_string()
                .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .filter(|w| !w.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Is the body exactly one tail expression calling `...::guarded(..)`?
fn body_is_guarded(block: &syn::Block) -> bool {
    match block.stmts.as_slice() {
        [Stmt::Expr(Expr::Call(call), None)] => match &*call.func {
            Expr::Path(p) => p.path.segments.last().is_some_and(|s| s.ident == "guarded"),
            _ => false,
        },
        _ => false,
    }
}

#[derive(Default)]
struct Scan {
    violations: Vec<String>,
    /// Guard-required fns seen; the real scan asserts this is non-zero so a
    /// parser or path mistake cannot make the test pass vacuously.
    checked_fns: usize,
}

impl Scan {
    fn check_fn(&mut self, file: &str, owner: &str, f: &ItemFn) {
        self.checked_fns += 1;
        if !body_is_guarded(&f.block) {
            self.violations.push(format!(
                "{file}: {owner}{} is not a single `guarded(..)` tail expression",
                f.sig.ident
            ));
        }
    }

    fn items(&mut self, file: &str, items: &[Item]) {
        for item in items {
            match item {
                Item::Fn(f) if has_attr(&f.attrs, "pyfunction") => self.check_fn(file, "", f),
                Item::Impl(imp) if has_attr(&imp.attrs, "pymethods") => {
                    for it in &imp.items {
                        match it {
                            ImplItem::Fn(m) => {
                                // Reuse the free-fn check via a transient ItemFn.
                                let f = ItemFn {
                                    attrs: m.attrs.clone(),
                                    vis: m.vis.clone(),
                                    sig: m.sig.clone(),
                                    block: Box::new(m.block.clone()),
                                };
                                self.check_fn(file, "method ", &f);
                            }
                            ImplItem::Macro(_) => self.violations.push(format!(
                                "{file}: macro invocation inside #[pymethods] cannot be inspected"
                            )),
                            _ => {}
                        }
                    }
                }
                Item::Struct(s) => {
                    for w in attr_words(&s.attrs, "pyclass") {
                        if w == "get_all" || w == "set_all" {
                            self.violations
                                .push(format!("{file}: struct {} uses {w}", s.ident));
                        }
                    }
                    for field in &s.fields {
                        let words = attr_words(&field.attrs, "pyo3");
                        if words.iter().any(|w| w == "get" || w == "set") {
                            let name = field.ident.as_ref().map_or("?".into(), ToString::to_string);
                            self.violations.push(format!(
                                "{file}: field {}.{name} has a #[pyo3(get/set)] accessor",
                                s.ident
                            ));
                        }
                    }
                }
                Item::Mod(m) => {
                    if let Some((_, inner)) = &m.content {
                        self.items(file, inner);
                    }
                }
                _ => {}
            }
        }
    }
}

/// Scans one source text; `file` only labels violations.
fn scan_source(file: &str, src: &str) -> Scan {
    let parsed = syn::parse_file(src).unwrap_or_else(|e| panic!("{file}: {e}"));
    let mut scan = Scan::default();
    scan.items(file, &parsed.items);
    scan
}

fn python_sources() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../python/src");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .collect();
    files.sort();
    files
}

#[test]
fn every_python_entry_point_runs_inside_guarded() {
    let mut total = Scan::default();
    for path in python_sources() {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let src = std::fs::read_to_string(&path).unwrap();
        let s = scan_source(&name, &src);
        total.checked_fns += s.checked_fns;
        total.violations.extend(s.violations);
    }
    // The exact count after run 0003's `OcrEngine.noise_precheck` getter
    // (81 entry points): a floor, so adding entry points never breaks it,
    // while a scanner that silently stops seeing a file does.
    assert!(
        total.checked_fns >= 81,
        "scan found only {} guard-required fns; the scanner is likely broken",
        total.checked_fns
    );
    assert!(
        total.violations.is_empty(),
        "T45: unguarded Python entry points:\n{}",
        total.violations.join("\n")
    );
}

// ---- self-tests on in-memory mutants: the scanner must be able to fail ----

#[test]
fn scanner_accepts_guarded_method_and_function() {
    let src = r#"
        #[pymethods]
        impl Page {
            fn a(&self) -> PyResult<u8> { guarded(|| Ok(1)) }
            fn b(&self) -> PyResult<u8> { crate::guard::guarded(|| { Ok(2) }) }
        }
        #[pyfunction]
        fn f() -> PyResult<u8> { guard::guarded(|| Ok(3)) }
        impl Page { fn helper(&self) -> u8 { 4 } }
    "#;
    let s = scan_source("ok.rs", src);
    assert_eq!(s.checked_fns, 3, "plain impls are not scanned");
    assert!(s.violations.is_empty(), "{:?}", s.violations);
}

#[test]
fn scanner_rejects_unguarded_method() {
    let src = "#[pymethods] impl P { fn text(&self) -> PyResult<u8> { Ok(1) } }";
    assert_eq!(scan_source("m.rs", src).violations.len(), 1);
}

#[test]
fn scanner_rejects_unguarded_pyfunction() {
    let src = "#[pyfunction] fn f() -> PyResult<u8> { Ok(1) }";
    assert_eq!(scan_source("m.rs", src).violations.len(), 1);
}

#[test]
fn scanner_rejects_extra_statement_around_guarded() {
    let before =
        "#[pymethods] impl P { fn t(&self) -> PyResult<u8> { let x = 1; guarded(|| Ok(x)) } }";
    let after = "#[pymethods] impl P { fn t(&self) -> PyResult<u8> { guarded(|| Ok(1)); Ok(2) } }";
    let semi = "#[pymethods] impl P { fn t(&self) -> PyResult<u8> { guarded(|| Ok(1)); } }";
    for src in [before, after, semi] {
        assert_eq!(scan_source("m.rs", src).violations.len(), 1, "{src}");
    }
}

#[test]
fn scanner_rejects_non_guarded_call() {
    let src = "#[pymethods] impl P { fn t(&self) -> PyResult<u8> { other(|| Ok(1)) } }";
    assert_eq!(scan_source("m.rs", src).violations.len(), 1);
}

#[test]
fn scanner_rejects_get_set_fields_and_all_variants() {
    let get = "#[pyclass] struct P { #[pyo3(get)] x: u8 }";
    let set = "#[pyclass] struct P { #[pyo3(get, set)] x: u8 }";
    let all = "#[pyclass(get_all)] struct P { x: u8 }";
    let set_all = "#[pyclass(frozen, set_all)] struct P { x: u8 }";
    for src in [get, set, all, set_all] {
        assert_eq!(scan_source("m.rs", src).violations.len(), 1, "{src}");
    }
    let plain = "#[pyclass(frozen)] struct P { #[allow(dead_code)] x: u8 }";
    assert!(scan_source("m.rs", plain).violations.is_empty());
}

#[test]
fn scanner_rejects_macro_inside_pymethods() {
    let src = "#[pymethods] impl P { make_slots!(); }";
    assert_eq!(scan_source("m.rs", src).violations.len(), 1);
}
