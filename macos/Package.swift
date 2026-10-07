// swift-tools-version:5.9
// Fred's native macOS front end. Build with ./build.sh (Rust first).
import PackageDescription

let package = Package(
    name: "Fred",
    platforms: [.macOS("27.0")],
    targets: [
        .systemLibrary(name: "CFred", path: "Sources/CFred"),
        .executableTarget(
            name: "Fred",
            dependencies: ["CFred"],
            linkerSettings: [.unsafeFlags(["-L../target/release"]), .linkedLibrary("iconv")]
        ),
    ]
)
