#!/bin/zsh
set -euo pipefail
package_dir="${0:A:h:h}"
"$package_dir/scripts/build.sh"
exec swift run --skip-build --package-path "$package_dir" milky-snapshot "$@"
