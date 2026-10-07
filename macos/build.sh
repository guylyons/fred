#!/bin/sh
# Build Fred.app: the Rust library, then the Swift front end around it.
set -e
cd "$(dirname "$0")"
cargo rustc --manifest-path ../Cargo.toml --release --lib --crate-type staticlib
swift build -c release
app=Fred.app/Contents
mkdir -p "$app/MacOS"
cp .build/release/Fred "$app/MacOS/Fred"
cat > "$app/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>Fred</string>
<key>CFBundleIdentifier</key><string>dev.fred.editor</string>
<key>CFBundleExecutable</key><string>Fred</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
echo "built $(pwd)/Fred.app"
