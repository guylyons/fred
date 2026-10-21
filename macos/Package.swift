// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "Fred",
    platforms: [.macOS("26.0")],
    targets: [
        .target(name: "FredKit"),
        .executableTarget(name: "Fred", dependencies: ["FredKit"]),
        .executableTarget(name: "FredSelfTest", dependencies: ["FredKit"]),
    ]
)
