/*
 * milky.h — the C surface of Milky's Rust core.
 *
 * Hand-written to match crates/milky-ffi/src/lib.rs. Nothing enforces that
 * they agree, so both must change together.
 */

#ifndef MILKY_H
#define MILKY_H

#include <stdint.h>

/* ABI version compiled into the linked library. */
uint32_t milky_abi_version(void);

#endif /* MILKY_H */