//! The 4cam C ABI: every function takes JSON text (and, where a part mesh is involved, its
//! STL/3MF bytes) and returns JSON text, the same documents as the WebAssembly and Python
//! APIs (`docs/api.md`). Errors come back as `{"error": "..."}`, never as a crash: a panic
//! inside the engine is caught and reported the same way.
//!
//! Memory: every `char *` a function returns is allocated here and must be released with
//! [`fourcam_free`] (passing NULL is fine). Strings passed in are borrowed for the call
//! only; NULL counts as empty. All functions are safe to call from several threads.

use std::ffi::{c_char, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};

use fourcam_wasm::api;
use serde_json::{json, Value};

/// Borrow a C string as UTF-8 text (NULL or invalid UTF-8: empty).
///
/// # Safety
/// `p` is NULL or points to a NUL-terminated string valid for the call.
unsafe fn text<'a>(p: *const c_char) -> std::borrow::Cow<'a, str> {
    if p.is_null() {
        return "".into();
    }
    // SAFETY: the caller passes a NUL-terminated string that outlives the call.
    unsafe { CStr::from_ptr(p) }.to_string_lossy()
}

/// Borrow `len` bytes (NULL or 0: empty).
///
/// # Safety
/// `p` is NULL or points to `len` readable bytes valid for the call.
unsafe fn bytes<'a>(p: *const u8, len: usize) -> &'a [u8] {
    if p.is_null() || len == 0 {
        return &[];
    }
    // SAFETY: the caller passes `len` readable bytes that outlive the call.
    unsafe { std::slice::from_raw_parts(p, len) }
}

/// JSON value to a C string the caller frees with `fourcam_free`.
fn out(v: Value) -> *mut c_char {
    let s = v.to_string();
    // JSON text never contains NUL (it is escaped as \u0000)
    CString::new(s)
        .unwrap_or_else(|_| CString::new(r#"{"error":"NUL in output"}"#).expect("no NUL"))
        .into_raw()
}

/// Run `f`, turning a panic into `{"error": ...}`.
fn guarded(f: impl FnOnce() -> Value) -> *mut c_char {
    out(catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|p| {
        let msg = p
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| p.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown".into());
        json!({ "error": format!("4cam internal error: {msg}") })
    }))
}

/// Release a string returned by any `fourcam_*` function. NULL is ignored.
///
/// # Safety
/// `s` is NULL or a pointer returned by this library, not yet freed.
#[no_mangle]
pub unsafe extern "C" fn fourcam_free(s: *mut c_char) {
    if !s.is_null() {
        // SAFETY: `s` came from CString::into_raw in `out` and is freed once.
        drop(unsafe { CString::from_raw(s) });
    }
}

/// The library's name, version and project schema, as JSON:
/// `{"name": "4cam", "version": "x.y.z", "schema_version": "0.1"}`.
#[no_mangle]
pub extern "C" fn fourcam_version() -> *mut c_char {
    guarded(api::version)
}

/// The C ABI's own version (bumped when a function's signature changes): 1.
#[no_mangle]
pub extern "C" fn fourcam_abi_version() -> u32 {
    1
}

/// Plan, post, verify and check a project (or 0.1 job): G-code per program with issues,
/// limit and gouge checks, holder/shank collisions, time estimates and a toolpath polyline.
/// `machine`: a built-in machine id or a machine profile's JSON (NULL/empty: the project's,
/// else the Makera Z1). `mesh`: the part mesh's bytes when the project has one.
/// `opts`: JSON such as `{"setup": "id", "max_points": 200000}` (NULL: defaults).
///
/// # Safety
/// String arguments are NULL or NUL-terminated; `mesh` is NULL or `mesh_len` bytes.
#[no_mangle]
pub unsafe extern "C" fn fourcam_plan_and_post(
    project: *const c_char,
    machine: *const c_char,
    mesh: *const u8,
    mesh_len: usize,
    opts: *const c_char,
) -> *mut c_char {
    // SAFETY: forwarded from the caller's contract above.
    let (p, m, b, o) = unsafe {
        (
            text(project),
            text(machine),
            bytes(mesh, mesh_len),
            text(opts),
        )
    };
    guarded(|| api::plan_and_post(&p, &m, b, &o))
}

/// Simulate material removal of every program in order. `opts`: `{"res": "normal"}` or
/// `{"dx": mm, "na": cells}` (rotary), `{"cell": mm}` (3-axis), `{"setup": "id"}`.
///
/// # Safety
/// As [`fourcam_plan_and_post`].
#[no_mangle]
pub unsafe extern "C" fn fourcam_simulate(
    project: *const c_char,
    opts: *const c_char,
    mesh: *const u8,
    mesh_len: usize,
) -> *mut c_char {
    // SAFETY: forwarded from the caller's contract above.
    let (p, o, b) = unsafe { (text(project), text(opts), bytes(mesh, mesh_len)) };
    guarded(|| api::simulate(&p, &o, b))
}

/// Machining time of G-code text on a machine (built-in id or profile JSON; NULL/empty:
/// the Makera Z1): total, cutting, rapid and other seconds, distances and notes.
///
/// # Safety
/// String arguments are NULL or NUL-terminated.
#[no_mangle]
pub unsafe extern "C" fn fourcam_estimate_gcode(
    gcode: *const c_char,
    machine: *const c_char,
) -> *mut c_char {
    // SAFETY: forwarded from the caller's contract above.
    let (g, m) = unsafe { (text(gcode), text(machine)) };
    guarded(|| api::estimate_gcode(&g, &m))
}

/// The operation types and their parameters (the registry every UI draws its forms from).
#[no_mangle]
pub extern "C" fn fourcam_op_types() -> *mut c_char {
    guarded(|| api::call("op_types", &[], &[], &[]))
}

/// A new operation with every field filled by the engine. `request`:
/// `{"type": "rotary_finish", "setup": "id", "tool": 3, "geometry": [...], "machine": "id"}`.
///
/// # Safety
/// As [`fourcam_plan_and_post`].
#[no_mangle]
pub unsafe extern "C" fn fourcam_op_defaults(
    project: *const c_char,
    request: *const c_char,
    mesh: *const u8,
    mesh_len: usize,
) -> *mut c_char {
    // SAFETY: forwarded from the caller's contract above.
    let (p, r, b) = unsafe { (text(project), text(request), bytes(mesh, mesh_len)) };
    guarded(|| api::op_defaults(&p, &r, b))
}

/// Upgrade a 0.1 job to a 0.2 project and check its operations:
/// `{"project": ..., "upgraded": bool, "problems": [...]}`.
///
/// # Safety
/// `project` is NULL or NUL-terminated.
#[no_mangle]
pub unsafe extern "C" fn fourcam_check_project(project: *const c_char) -> *mut c_char {
    // SAFETY: forwarded from the caller's contract above.
    let p = unsafe { text(project) };
    guarded(|| api::call("check_project", p.as_bytes(), &[], &[]))
}

/// Any engine function by name, as the WebAssembly module exposes them (`docs/api.md`):
/// three byte arguments whose meaning depends on the function (JSON text, or mesh bytes).
/// Unknown names return an error.
///
/// # Safety
/// `name` is NULL or NUL-terminated; each `a`/`b`/`c` is NULL or that many bytes.
#[no_mangle]
pub unsafe extern "C" fn fourcam_call(
    name: *const c_char,
    a: *const u8,
    a_len: usize,
    b: *const u8,
    b_len: usize,
    c: *const u8,
    c_len: usize,
) -> *mut c_char {
    // SAFETY: forwarded from the caller's contract above.
    let (n, a, b, c) = unsafe {
        (
            text(name),
            bytes(a, a_len),
            bytes(b, b_len),
            bytes(c, c_len),
        )
    };
    guarded(|| api::call(&n, a, b, c))
}
