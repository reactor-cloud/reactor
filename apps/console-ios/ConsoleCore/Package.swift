// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "ConsoleCore",
    platforms: [
        .macOS(.v14),
        .iOS(.v17),
    ],
    products: [
        .library(name: "ConsoleCore", targets: ["ConsoleCore"]),
    ],
    targets: [
        .target(name: "ConsoleCore"),
        .testTarget(name: "ConsoleCoreTests", dependencies: ["ConsoleCore"]),
    ]
)
