# reference/

Code Claude wrote that Parth did not write himself. Kept for comparison, not as
the working tree.

## first-pass/

A complete Swift <-> Rust boundary written by Claude in one go on 2026-09-10:
`milky-core` engine, `milky-ffi` C ABI shim, `milky-cli`, the C header, and
`MilkyKit`. It built and passed its tests.

It was moved here because writing it that way defeated the point of the project.
The FFI boundary, ownership across it, and the engine API are exactly the things
worth learning by hand.

How to use it:

- Do not copy from it.
- Do not read a file here before attempting the same file yourself.
- After writing your own version, diff against it and ask why the choices
  differ. Some differences will be Claude being wrong.

Delete a file here once the real version exists and is understood.
