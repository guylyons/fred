#!/bin/sh
# Build Fred.app: the Rust core (fred --gui helper), then the Swift front end.
set -e
here="$(cd "$(dirname "$0")" && pwd)"
root="$(dirname "$here")"
(cd "$root" && PATH="/opt/homebrew/opt/zig@0.15/bin:$PATH" cargo build --release --features ghostty)
swift build -c release --package-path "$here"
app="$here/Fred.app/Contents"
mkdir -p "$app/MacOS"
cp "$here/.build/release/Fred" "$app/MacOS/Fred"
cp "$root/target/release/fred" "$app/MacOS/fred-core"
cat > "$app/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>Fred</string>
<key>CFBundleIdentifier</key><string>dev.fred.editor</string>
<key>CFBundleExecutable</key><string>Fred</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>LSMinimumSystemVersion</key><string>26.0</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
echo "built $here/Fred.app"
