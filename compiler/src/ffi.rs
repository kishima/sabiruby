//! The only `unsafe` of the crate: the three functions of `csrc/shim.c`.

use std::ffi::{c_char, c_int, c_uint, c_void, CStr, CString};

pub const DEBUG_INFO: c_uint = 1;
pub const REMOVE_LV: c_uint = 2;
pub const NO_EXT_OPS: c_uint = 4;
pub const NO_OPTIMIZE: c_uint = 8;

pub const OK: c_int = 0;
pub const COMPILE_ERROR: c_int = 1;
pub const DUMP_ERROR: c_int = 2;
pub const NO_MEMORY: c_int = 3;

unsafe extern "C" {
    fn sabiruby_mrc_compile(src: *const u8, len: usize, filename: *const c_char, flags: c_uint,
                            out: *mut *mut u8, out_len: *mut usize, diag: *mut *mut c_char) -> c_int;
    fn sabiruby_mrc_free(p: *mut c_void);
    fn sabiruby_mrc_version() -> *const c_char;
}

/// One compilation: the shim's result code, the RITE binary (empty unless `OK`) and the
/// raw diagnostics text (records `\x1e`, fields `\x1f`).
pub fn compile(src: &[u8], filename: &CString, flags: c_uint) -> (c_int, Vec<u8>, String) {
    let mut out: *mut u8 = std::ptr::null_mut();
    let mut out_len: usize = 0;
    let mut diag: *mut c_char = std::ptr::null_mut();
    // SAFETY: `src` and `filename` outlive the call; the shim copies what it keeps. On
    // return `out` is null or a malloc'ed block of `out_len` bytes and `diag` is null or
    // a malloc'ed NUL-terminated string; both are copied, then released with
    // `sabiruby_mrc_free` (the shim's `free`).
    unsafe {
        let code = sabiruby_mrc_compile(src.as_ptr(), src.len(), filename.as_ptr(), flags, &mut out, &mut out_len, &mut diag);
        let bin = if out.is_null() { Vec::new() } else { std::slice::from_raw_parts(out, out_len).to_vec() };
        let text = if diag.is_null() { String::new() } else { CStr::from_ptr(diag).to_string_lossy().into_owned() };
        sabiruby_mrc_free(out as *mut c_void);
        sabiruby_mrc_free(diag as *mut c_void);
        (code, bin, text)
    }
}

pub fn version() -> &'static str {
    // SAFETY: the shim returns a pointer to a static NUL-terminated ASCII literal.
    unsafe { CStr::from_ptr(sabiruby_mrc_version()).to_str().unwrap_or("mruby 4.1.0-rc") }
}
