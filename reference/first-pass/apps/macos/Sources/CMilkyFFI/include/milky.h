/*
 * milky.h — the entire C surface of Milky's Rust core.
 *
 * Hand-written to match crates/milky-ffi/src/lib.rs. Nothing enforces that
 * agreement, so any change to one side must change the other in the same
 * commit, and must bump MILKY_ABI_VERSION. See .agents/ISSUES.md.
 *
 * Ownership rules:
 *   - char * returned by this library is owned by the caller and must be
 *     released with milky_string_free.
 *   - MilkyEngineHandle * must be released exactly once with milky_engine_free.
 */

#ifndef MILKY_H
#define MILKY_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Must match MILKY_ABI_VERSION in milky-ffi. */
#define MILKY_ABI_VERSION 1

/* Opaque handle to a live engine. */
typedef struct MilkyEngineHandle MilkyEngineHandle;

/* ABI version compiled into the linked library. */
uint32_t milky_abi_version(void);

/*
 * Create an engine. config_json may be NULL or empty for defaults.
 * Returns NULL on success, having written a handle to *out_engine.
 * Returns an owned error string on failure, leaving *out_engine untouched.
 */
char *milky_engine_new(const char *config_json, MilkyEngineHandle **out_engine);

/* Destroy an engine. NULL is a no-op. Never call twice on the same pointer. */
void milky_engine_free(MilkyEngineHandle *engine);

/*
 * Handle one JSON request and return an owned JSON response.
 * Never returns NULL: errors and caught panics come back as error responses.
 */
char *milky_engine_request(const MilkyEngineHandle *engine, const char *request_json);

/* Release a string returned by this library. NULL is a no-op. */
void milky_string_free(char *s);

#ifdef __cplusplus
}
#endif

#endif /* MILKY_H */
