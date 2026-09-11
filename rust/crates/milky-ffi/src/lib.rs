//! C ABI shim around Milky's engine.
//!
//! Everything in this crate is called from Swift, so every function here uses
//! the C calling convention and an unmangled name.

/// Version of the C ABI this library implements.
///
/// Bump on any change to a signature or to who owns what.
#[unsafe(no_mangle)]
pub extern "C" fn milky_abi_version() -> u32 {
    1
}