// Source checks that stand in for coverage of the C++ shim (spec 0002
// §4.15, D-13): cargo-llvm-cov instruments Rust only, so instead
//
// 1. every function csrc/shim.cpp exports is declared in src/ffi.rs, and is
//    reached from a test: some Rust function in src/lib.rs calls it
//    (directly or through other functions of the crate), and that function
//    is named in a test under tests/;
// 2. every `unsafe` block and `unsafe impl` in src/ carries a `// SAFETY:`
//    comment directly above it.
//
// Both are syntactic and approximate by design (a name in a test is taken
// as a call), which is enough to catch a shim function added without a test
// or an unsafe block added without its argument.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use rust_idents::idents;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Names of the functions the shim defines at file scope (`pxt_*`).
fn shim_exports() -> BTreeSet<String> {
    read(&root().join("csrc/shim.cpp"))
        .lines()
        .filter(|l| !l.starts_with(' ') && !l.starts_with('/') && !l.trim_end().ends_with(';'))
        .filter_map(|l| {
            let head = l.split('(').next()?;
            let name = head.rsplit([' ', '*']).next()?;
            (name.starts_with("pxt_") && l.contains('(')).then(|| name.to_owned())
        })
        .collect()
}

/// Identifier sets, via syn, so comments and strings never count.
mod rust_idents {
    use std::collections::{BTreeMap, BTreeSet};
    use syn::visit::Visit;

    struct Idents(BTreeSet<String>);
    impl<'ast> Visit<'ast> for Idents {
        fn visit_ident(&mut self, i: &'ast syn::Ident) {
            self.0.insert(i.to_string());
        }
        fn visit_macro(&mut self, m: &'ast syn::Macro) {
            // Macro bodies (assert!, format!) are token streams to syn.
            collect_tokens(m.tokens.clone(), &mut self.0);
            syn::visit::visit_macro(self, m);
        }
    }

    fn collect_tokens(ts: proc_macro2::TokenStream, out: &mut BTreeSet<String>) {
        for tt in ts {
            match tt {
                proc_macro2::TokenTree::Ident(i) => {
                    out.insert(i.to_string());
                }
                proc_macro2::TokenTree::Group(g) => collect_tokens(g.stream(), out),
                _ => {}
            }
        }
    }

    /// Every identifier in a whole file.
    pub fn idents(src: &str) -> BTreeSet<String> {
        let file = syn::parse_file(src).expect("parse");
        let mut v = Idents(BTreeSet::new());
        v.visit_file(&file);
        v.0
    }

    /// For each function (free, or in an impl, `drop` included): the
    /// identifiers its body mentions.
    pub fn function_bodies(src: &str) -> BTreeMap<String, BTreeSet<String>> {
        struct Fns(BTreeMap<String, BTreeSet<String>>);
        impl<'ast> Visit<'ast> for Fns {
            fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
                let mut v = Idents(BTreeSet::new());
                v.visit_block(&f.block);
                self.0
                    .entry(f.sig.ident.to_string())
                    .or_default()
                    .extend(v.0);
            }
            fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
                let mut v = Idents(BTreeSet::new());
                v.visit_block(&f.block);
                self.0
                    .entry(f.sig.ident.to_string())
                    .or_default()
                    .extend(v.0);
            }
        }
        let file = syn::parse_file(src).expect("parse");
        let mut v = Fns(BTreeMap::new());
        v.visit_file(&file);
        v.0
    }
}

#[test]
fn every_shim_export_is_declared_and_tested() {
    let exports = shim_exports();
    // A sanity floor: the list in spec §3.4 plus the test hook.
    assert!(exports.len() >= 13, "parsed only {exports:?}");

    let ffi = idents(&read(&root().join("src/ffi.rs")));
    let bodies = rust_idents::function_bodies(&read(&root().join("src/lib.rs")));
    let mut tested = BTreeSet::new();
    for entry in std::fs::read_dir(root().join("tests")).expect("tests dir") {
        let path = entry.expect("entry").path();
        if path.extension().is_some_and(|e| e == "rs")
            && path.file_name() != Some("shim_symbols.rs".as_ref())
        {
            tested.extend(idents(&read(&path)));
        }
    }

    let mut missing = BTreeMap::new();
    for sym in &exports {
        if !ffi.contains(sym) {
            missing.insert(sym.clone(), "not declared in src/ffi.rs".to_owned());
            continue;
        }
        // Functions of lib.rs that reach `sym`, to a fixed point.
        let mut callers: BTreeSet<String> = BTreeSet::from([sym.clone()]);
        loop {
            let before = callers.len();
            for (name, body) in &bodies {
                if body.iter().any(|i| callers.contains(i)) {
                    callers.insert(name.clone());
                }
            }
            if callers.len() == before {
                break;
            }
        }
        callers.remove(sym);
        if !callers.iter().any(|c| tested.contains(c)) {
            missing.insert(
                sym.clone(),
                format!("callers {callers:?} not named in any test"),
            );
        }
    }
    assert!(missing.is_empty(), "untested shim exports: {missing:#?}");
}

#[test]
fn every_unsafe_block_has_a_safety_comment() {
    let mut offenders = Vec::new();
    for entry in std::fs::read_dir(root().join("src")).expect("src dir") {
        let path = entry.expect("entry").path();
        let src = read(&path);
        let lines: Vec<&str> = src.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            if !(code.contains("unsafe {") || code.contains("unsafe impl")) {
                continue;
            }
            let has_safety = lines[..i]
                .iter()
                .rev()
                .take_while(|l| l.trim_start().starts_with("//"))
                .any(|l| l.contains("SAFETY:"));
            if !has_safety {
                offenders.push(format!("{}:{}: {}", path.display(), i + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "unsafe without SAFETY comment:\n{}",
        offenders.join("\n")
    );
}
