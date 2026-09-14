//! C ABI shim around Milky's engine (DECISIONS 2026-09-13).
//!
//! Five functions. All logic lives in `milky_core::engine::Engine::handle_json`;
//! this file only moves ownership across the boundary. Rules held throughout:
//! no Rust type crosses except behind an opaque pointer, no panic crosses, and
//! whatever Rust allocates, Rust frees.

use std::ffi::{CStr, CString, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};

use milky_core::api::{ErrorReason, error_json};
use milky_core::engine::Engine;

/// Opaque to C: the header declares this type with no fields.
pub struct MilkyEngine {
    engine: Engine,
}

/// Version of the C ABI. Bump on any signature or ownership change.
#[unsafe(no_mangle)]
pub extern "C" fn milky_abi_version() -> u32 {
    2
}

/// Create an engine, scanning for apps once. Returns null on failure.
#[unsafe(no_mangle)]
pub extern "C" fn milky_engine_new() -> *mut MilkyEngine {
    catch_unwind(|| {
        let handle = MilkyEngine { engine: Engine::new() };
        Box::into_raw(Box::new(handle))
    })
    .unwrap_or(std::ptr::null_mut())
}

/// Destroy an engine. Null is a no-op.
///
/// # Safety
///
/// `engine` must be null or a pointer from `milky_engine_new` that has not
/// been freed, with no request in flight on any thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn milky_engine_free(engine: *mut MilkyEngine) {
    if engine.is_null() {
        return;
    }
    let boxed = unsafe { Box::from_raw(engine) };
    let _ = catch_unwind(|| drop(boxed));
}

/// Handle one JSON request. Returns an owned JSON response, never null; release
/// it with `milky_string_free`. The response does not borrow from the engine.
///
/// # Safety
///
/// `engine` must be null or a live pointer from `milky_engine_new`.
/// `request_json` must be null or a valid NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn milky_engine_request(
    engine: *const MilkyEngine,
    request_json: *const c_char,
) -> *mut c_char {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let Some(handle) = (unsafe { engine.as_ref() }) else {
            return error_json(ErrorReason::BadRequest, "engine pointer was null");
        };
        if request_json.is_null() {
            return error_json(ErrorReason::BadRequest, "request pointer was null");
        }
        match unsafe { CStr::from_ptr(request_json) }.to_str() {
            Ok(text) => handle.engine.handle_json(text),
            Err(_) => error_json(ErrorReason::BadRequest, "request was not valid UTF-8"),
        }
    }));
    let response =
        result.unwrap_or_else(|_| error_json(ErrorReason::Panic, "panic while handling request"));
    into_c_string(response)
}

/// Release a string returned by this library. Null is a no-op.
///
/// # Safety
///
/// `s` must be null or a string returned by this library that has not been freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn milky_string_free(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    drop(unsafe {
        CString::from_raw(s)
    });
}

/// Move a response into a C string the caller owns.
fn into_c_string(text: String) -> *mut c_char {
    match CString::new(text) {
        Ok(cstring) => cstring.into_raw(),
        Err(_) => {
            let fallback: CString =
                c"{\"kind\":\"error\",\"reason\":\"internal\",\"message\":\"response contained a NUL byte\"}"
                    .into();
            fallback.into_raw()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    /// Read a response, free it, parse it. Fails if Rust ever returns null.
    unsafe fn take(ptr: *mut c_char) -> Value {
        assert!(!ptr.is_null(), "request must never return null");
        let text = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_owned();
        unsafe { milky_string_free(ptr) };
        serde_json::from_str(&text).expect("valid JSON")
    }

    #[test]
    fn abi_version_is_two() {
        assert_eq!(milky_abi_version(), 2);
    }

    #[test]
    fn create_search_and_free() {
        let engine = milky_engine_new();
        assert!(!engine.is_null());
        let v = unsafe {
            take(milky_engine_request(
                engine,
                c"{\"op\":\"search\",\"query\":\"term\",\"limit\":3}".as_ptr(),
            ))
        };
        assert_eq!(v["kind"], "search");
        assert!(v["results"].as_array().unwrap().iter().any(|r| r["name"] == "Terminal"));
        unsafe { milky_engine_free(engine) };
    }

    #[test]
    fn null_request_is_a_bad_request_response() {
        let engine = milky_engine_new();
        let v = unsafe { take(milky_engine_request(engine, std::ptr::null())) };
        assert_eq!(v["reason"], "bad_request");
        unsafe { milky_engine_free(engine) };
    }

    #[test]
    fn null_engine_is_a_bad_request_response() {
        let v = unsafe { take(milky_engine_request(std::ptr::null(), c"{}".as_ptr())) };
        assert_eq!(v["kind"], "error");
        assert_eq!(v["reason"], "bad_request");
    }

    #[test]
    fn invalid_utf8_is_a_bad_request_response() {
        let engine = milky_engine_new();
        let bytes: &[u8] = b"\xff\xfe\0";
        let v = unsafe { take(milky_engine_request(engine, bytes.as_ptr().cast())) };
        assert_eq!(v["reason"], "bad_request");
        unsafe { milky_engine_free(engine) };
    }

    #[test]
    fn freeing_null_is_a_no_op() {
        unsafe {
            milky_engine_free(std::ptr::null_mut());
            milky_string_free(std::ptr::null_mut());
        }
    }

    #[test]
    fn a_response_outlives_its_engine() {
        let engine = milky_engine_new();
        let raw = unsafe {
            milky_engine_request(
                engine,
                c"{\"op\":\"search\",\"query\":\"safari\",\"limit\":1}".as_ptr(),
            )
        };
        unsafe { milky_engine_free(engine) };
        let v = unsafe { take(raw) };
        assert_eq!(v["kind"], "search");
    }

    #[test]
    fn concurrent_requests_share_one_engine() {
        let engine = milky_engine_new();
        // Raw pointers are not Send; pass the address, as a foreign caller would.
        let address = engine as usize;
        let threads: Vec<_> = (0..8)
            .map(|_| {
                std::thread::spawn(move || {
                    let engine = address as *const MilkyEngine;
                    for _ in 0..200 {
                        let v = unsafe {
                            take(milky_engine_request(
                                engine,
                                c"{\"op\":\"search\",\"query\":\"co\",\"limit\":5}".as_ptr(),
                            ))
                        };
                        assert_eq!(v["kind"], "search");
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        unsafe { milky_engine_free(engine) };
    }
}