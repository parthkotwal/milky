//! C ABI shim around [`milky_core::Engine`].
//!
//! This is the entire surface Swift sees. It is deliberately tiny — five
//! functions — because every function here is hand-written `unsafe` glue that
//! must stay in sync with `apps/macos/Sources/CMilkyFFI/include/milky.h`.
//! Rich payloads cross as JSON strings instead of as C structs, so adding a new
//! operation never means touching this file or the header.
//!
//! Three rules hold everywhere below:
//!
//! 1. No Rust type crosses the boundary. Only pointers, `u32`, and C strings.
//! 2. No panic crosses the boundary. Unwinding into C is undefined behavior, so
//!    every entry point wraps its body in [`catch_unwind`].
//! 3. Ownership is explicit. Anything Rust allocates, Rust frees: strings via
//!    `milky_string_free`, engines via `milky_engine_free`.

use std::ffi::{c_char, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};

use milky_core::{Config, Engine};

/// Version of the C ABI in this file.
///
/// Bump on any signature or ownership change. `MilkyKit` compares this against
/// the value it was compiled against and refuses to run on a mismatch, which
/// turns "mystery corruption" into a clear startup error.
pub const MILKY_ABI_VERSION: u32 = 1;

/// Opaque engine handle. C and Swift only ever hold a pointer to this.
pub struct MilkyEngineHandle {
    engine: Engine,
}

/// Return the ABI version compiled into this library.
#[no_mangle]
pub extern "C" fn milky_abi_version() -> u32 {
    MILKY_ABI_VERSION
}

/// Create an engine.
///
/// `config_json` may be null or empty for defaults. On success writes a handle
/// to `out_engine` and returns null. On failure returns an owned error string
/// the caller must release with [`milky_string_free`], and leaves `out_engine`
/// untouched.
///
/// # Safety
///
/// `config_json` must be null or a valid NUL-terminated C string, and
/// `out_engine` must be a valid, writable pointer.
#[no_mangle]
pub unsafe extern "C" fn milky_engine_new(
    config_json: *const c_char,
    out_engine: *mut *mut MilkyEngineHandle,
) -> *mut c_char {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if out_engine.is_null() {
            return Err("out_engine must not be null".to_string());
        }

        let config = match cstr_to_str(config_json) {
            Ok(None) => Config::default(),
            Ok(Some(text)) if text.trim().is_empty() => Config::default(),
            Ok(Some(text)) => milky_core::engine::parse_config(text)?,
            Err(err) => return Err(err),
        };

        let engine = Engine::new(config).map_err(|err| err.to_string())?;
        let handle = Box::new(MilkyEngineHandle { engine });

        // Hand ownership of the allocation to the caller. Nothing on the Rust
        // side will free it until it comes back through milky_engine_free.
        unsafe { *out_engine = Box::into_raw(handle) };
        Ok(())
    }));

    match result {
        Ok(Ok(())) => std::ptr::null_mut(),
        Ok(Err(message)) => into_c_string(message),
        Err(_) => into_c_string("panic while creating engine".to_string()),
    }
}

/// Destroy an engine created by [`milky_engine_new`]. Null is a no-op.
///
/// # Safety
///
/// `engine` must be null, or a handle from [`milky_engine_new`] that has not
/// already been freed. Calling this twice on the same pointer is a double free.
#[no_mangle]
pub unsafe extern "C" fn milky_engine_free(engine: *mut MilkyEngineHandle) {
    if engine.is_null() {
        return;
    }
    // Reconstruct the Box so its destructor runs, then let it fall out of scope.
    let handle = unsafe { Box::from_raw(engine) };
    let _ = catch_unwind(AssertUnwindSafe(move || drop(handle)));
}

/// Handle one request.
///
/// Takes a JSON request, returns an owned JSON response that the caller must
/// release with [`milky_string_free`]. Never returns null: engine-level
/// failures, parse failures, and caught panics all come back as an error
/// response so the caller has exactly one thing to check.
///
/// # Safety
///
/// `engine` must be a live handle from [`milky_engine_new`], and
/// `request_json` a valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn milky_engine_request(
    engine: *const MilkyEngineHandle,
    request_json: *const c_char,
) -> *mut c_char {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let handle = match unsafe { engine.as_ref() } {
            Some(handle) => handle,
            None => return error_json("internal", "engine pointer was null"),
        };
        match cstr_to_str(request_json) {
            Ok(Some(text)) => handle.engine.handle_json(text),
            Ok(None) => error_json("bad_request", "request_json was null"),
            Err(err) => error_json("bad_request", &err),
        }
    }));

    let json = result.unwrap_or_else(|_| error_json("panic", "panic while handling request"));
    into_c_string(json)
}

/// Free a string returned by this library. Null is a no-op.
///
/// # Safety
///
/// `s` must be null or a string produced by this library, not yet freed, and
/// not allocated by the caller.
#[no_mangle]
pub unsafe extern "C" fn milky_string_free(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    drop(unsafe { CString::from_raw(s) });
}

/// Borrow a C string as `&str`. `Ok(None)` means the pointer was null.
///
/// # Safety
///
/// `ptr` must be null or a valid NUL-terminated C string that outlives the
/// returned reference.
unsafe fn cstr_to_str<'a>(ptr: *const c_char) -> Result<Option<&'a str>, String> {
    if ptr.is_null() {
        return Ok(None);
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .map(Some)
        .map_err(|_| "string was not valid UTF-8".to_string())
}

/// Move a Rust `String` into a C string the caller owns.
fn into_c_string(text: String) -> *mut c_char {
    // A String may contain interior NUL bytes, which CString rejects. Strip
    // them rather than failing: this is an error path and must not itself fail.
    match CString::new(text) {
        Ok(cstring) => cstring.into_raw(),
        Err(err) => {
            let mut bytes = err.into_vec();
            bytes.retain(|byte| *byte != 0);
            CString::new(bytes)
                .unwrap_or_else(|_| CString::new("unrepresentable string").unwrap())
                .into_raw()
        }
    }
}

/// Build an error response without going through the engine, for failures that
/// happen before we have one. Shape matches `milky_core::api::Response::Error`.
fn error_json(reason: &str, message: &str) -> String {
    let escaped = message.replace('\\', "\\\\").replace('"', "\\\"");
    format!(r#"{{"kind":"error","reason":"{reason}","message":"{escaped}"}}"#)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(text: &str) -> CString {
        CString::new(text).unwrap()
    }

    unsafe fn take_string(ptr: *mut c_char) -> String {
        let owned = CStr::from_ptr(ptr).to_string_lossy().into_owned();
        milky_string_free(ptr);
        owned
    }

    #[test]
    fn create_query_and_free() {
        unsafe {
            let mut engine: *mut MilkyEngineHandle = std::ptr::null_mut();
            let err = milky_engine_new(std::ptr::null(), &mut engine);
            assert!(err.is_null(), "default config should succeed");
            assert!(!engine.is_null());

            let request = c(r#"{"op":"search","query":"terminal"}"#);
            let response = take_string(milky_engine_request(engine, request.as_ptr()));
            assert!(response.contains(r#""kind":"search""#), "got {response}");

            milky_engine_free(engine);
        }
    }

    #[test]
    fn invalid_config_returns_error_string() {
        unsafe {
            let mut engine: *mut MilkyEngineHandle = std::ptr::null_mut();
            let config = c(r#"{"default_limit":0}"#);
            let err = milky_engine_new(config.as_ptr(), &mut engine);
            assert!(!err.is_null());
            let message = take_string(err);
            assert!(message.contains("default_limit"), "got {message}");
            assert!(engine.is_null(), "engine must be left untouched on failure");
        }
    }

    #[test]
    fn null_request_is_an_error_response_not_a_crash() {
        unsafe {
            let mut engine: *mut MilkyEngineHandle = std::ptr::null_mut();
            assert!(milky_engine_new(std::ptr::null(), &mut engine).is_null());
            let response = take_string(milky_engine_request(engine, std::ptr::null()));
            assert!(response.contains(r#""kind":"error""#), "got {response}");
            milky_engine_free(engine);
        }
    }

    #[test]
    fn freeing_null_is_a_no_op() {
        unsafe {
            milky_engine_free(std::ptr::null_mut());
            milky_string_free(std::ptr::null_mut());
        }
    }
}
