/*
 * milky.h — the C surface of Milky's Rust core. ABI version 2.
 *
 * Contract: .agents/DECISIONS.md, 2026-09-13. Hand-written to match
 * rust/crates/milky-ffi/src/lib.rs; change both together and bump the version.
 *
 * Ownership: strings returned here are owned by the caller and must be released
 * with milky_string_free, never free(). An engine is released exactly once with
 * milky_engine_free, never while a request is in flight.
 */

#ifndef MILKY_H
#define MILKY_H

#include <stdint.h>

/* Must equal milky_abi_version() in the linked library. */
#define MILKY_ABI_VERSION 2

/* Opaque handle. */
typedef struct MilkyEngine MilkyEngine;

uint32_t     milky_abi_version(void);

/* Returns NULL on failure. */
MilkyEngine *milky_engine_new(void);

/* NULL is a no-op. */
void         milky_engine_free(MilkyEngine *engine);

/* Owned JSON response, never NULL. Safe to call concurrently on one engine. */
char        *milky_engine_request(const MilkyEngine *engine, const char *request_json);

/* NULL is a no-op. */
void         milky_string_free(char *s);

#endif /* MILKY_H */