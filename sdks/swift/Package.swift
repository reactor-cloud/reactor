// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "Reactor",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [
        .library(name: "Reactor", targets: ["Reactor"]),
    ],
    targets: [
        .target(name: "Reactor"),
        .testTarget(name: "ReactorTests", dependencies: ["Reactor"]),
    ]
)
