#!/bin/zsh
set -euo pipefail
package_dir="${0:A:h:h}"
swift build --package-path "$package_dir" --product milky-launcher
binary_dir="$(swift build --package-path "$package_dir" --show-bin-path)"
bundle_dir="$package_dir/.build/Milky Fixtures.app"
mkdir -p "$bundle_dir/Contents/MacOS"
cp "$binary_dir/milky-launcher" "$bundle_dir/Contents/MacOS/milky-launcher"
cat > "$bundle_dir/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>milky-launcher</string>
<key>CFBundleIdentifier</key><string>dev.milky.fixtures</string>
<key>CFBundleName</key><string>Milky Fixtures</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>LSMinimumSystemVersion</key><string>14.0</string>
<key>LSUIElement</key><true/>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
open "$bundle_dir" --args --fixtures "$@"
