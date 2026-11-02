//! Build script of the `_native` extension module.
//!
//! An extension module must not link libpython: the interpreter that imports
//! it supplies those symbols. maturin passes the right linker flags itself;
//! plain cargo builds of the workspace (CI's `rust` and `coverage` jobs) get
//! them from here. On macOS that is `-undefined dynamic_lookup`; elsewhere
//! it is nothing (PYO3_BUILD_EXTENSION_MODULE, set in .cargo/config.toml,
//! already stops pyo3-ffi from requesting libpython).
fn main() {
    pyo3_build_config::add_extension_module_link_args();
}
