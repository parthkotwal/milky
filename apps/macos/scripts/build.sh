#!/bin/zsh
set -euo pipefail
package_dir="${0:A:h:h}"
rust_manifest="$package_dir/../../rust/Cargo.toml"
# Pin the archive location used by Package.swift even if CARGO_TARGET_DIR is set.
cargo build --manifest-path "$rust_manifest" --target-dir "$package_dir/../../rust/target" -p milky-ffi
archive="$package_dir/../../rust/target/debug/libmilky_ffi.a"
stamp="$package_dir/.build/milky-rust.sha256"
fingerprint="$(shasum -a 256 "$archive" | cut -d ' ' -f 1)"
if [[ ! -f "$stamp" || "$(cat "$stamp")" != "$fingerprint" ]]; then
    print 'Rust archive changed; clearing Swift products to prevent stale static linkage.'
    swift package --package-path "$package_dir" clean
fi
case "${1:-build}" in
    build) swift build --package-path "$package_dir" ;;
    test) swift test --package-path "$package_dir" ;;
    *) print -u2 'Usage: scripts/build.sh [build|test]'; exit 2 ;;
esac
# Write only after a successful Swift build/test. A failed build cannot bless stale products.
print -r -- "$fingerprint" > "$stamp"
