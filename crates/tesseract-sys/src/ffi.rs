// Raw declarations of the C++ shim (csrc/shim.cpp). Nothing else in the
// crate's native half is bound: Tesseract's own C API is deliberately not
// used (spec D-5). Constants mirror the #defines at the top of the shim;
// tests/shim_symbols.rs checks that every exported pxt_* symbol is declared
// here and reached from a test.

use std::ffi::{c_char, c_int, c_void};

/// Opaque handle owned by an `Instance`.
#[repr(C)]
pub struct PxtHandle {
    _private: [u8; 0],
}

pub const PXT_OK: c_int = 0;
pub const PXT_ERR_INIT: c_int = 1;
pub const PXT_ERR_EXCEPTION: c_int = 2;
pub const PXT_ERR_RECOGNIZE: c_int = 3;
pub const PXT_ERR_TIMEOUT: c_int = 4;
pub const PXT_ERR_ARG: c_int = 5;
pub const PXT_ERR_CALLBACK: c_int = 6;

pub const PXT_BLOCK_START: c_int = 1;
pub const PXT_LINE_START: c_int = 2;

/// Size of the error buffer every fallible call writes into.
pub const ERRBUF_LEN: usize = 512;

pub type WordCallback = unsafe extern "C" fn(
    user: *mut c_void,
    flags: c_int,
    text: *const c_char,
    len: usize,
    left: c_int,
    top: c_int,
    right: c_int,
    bottom: c_int,
    confidence: f32,
) -> c_int;

unsafe extern "C" {
    pub fn pxt_version() -> *const c_char;
    pub fn pxt_global_init(
        debug_null_path: *const c_char,
        err: *mut c_char,
        err_len: usize,
    ) -> c_int;
    /// 1 when `name` is a process-global parameter, 0 when it is not,
    /// otherwise an error code (never `PXT_OK`'s 0 for a failure).
    pub fn pxt_is_global_param(name: *const c_char, err: *mut c_char, err_len: usize) -> c_int;
    pub fn pxt_create() -> *mut PxtHandle;
    #[allow(clippy::too_many_arguments)]
    pub fn pxt_init(
        h: *mut PxtHandle,
        datapath: *const c_char,
        langs: *const c_char,
        oem: c_int,
        var_names: *const *const c_char,
        var_values: *const *const c_char,
        n_vars: usize,
        err: *mut c_char,
        err_len: usize,
    ) -> c_int;
    #[allow(clippy::too_many_arguments)]
    pub fn pxt_set_image(
        h: *mut PxtHandle,
        data: *const u8,
        width: c_int,
        height: c_int,
        bytes_per_pixel: c_int,
        bytes_per_line: c_int,
        err: *mut c_char,
        err_len: usize,
    ) -> c_int;
    pub fn pxt_set_resolution(
        h: *mut PxtHandle,
        ppi: c_int,
        err: *mut c_char,
        err_len: usize,
    ) -> c_int;
    pub fn pxt_set_psm(h: *mut PxtHandle, psm: c_int, err: *mut c_char, err_len: usize) -> c_int;
    pub fn pxt_recognize(
        h: *mut PxtHandle,
        deadline_ms: c_int,
        err: *mut c_char,
        err_len: usize,
    ) -> c_int;
    pub fn pxt_collect(
        h: *mut PxtHandle,
        cb: WordCallback,
        user: *mut c_void,
        err: *mut c_char,
        err_len: usize,
    ) -> c_int;
    pub fn pxt_clear(h: *mut PxtHandle);
    pub fn pxt_end(h: *mut PxtHandle);
    pub fn pxt_delete(h: *mut PxtHandle);
    #[cfg(debug_assertions)]
    pub fn pxt_test_throw(err: *mut c_char, err_len: usize) -> c_int;
}
