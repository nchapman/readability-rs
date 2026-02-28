// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "ReadabilityTests",
    platforms: [.macOS(.v14)],
    targets: [
        .systemLibrary(
            name: "readability_uniffiFFI",
            path: "Sources/readability_uniffiFFI"
        ),
        .target(
            name: "Readability",
            dependencies: ["readability_uniffiFFI"],
            path: "Sources/Readability"
        ),
        .testTarget(
            name: "ReadabilityTests",
            dependencies: ["Readability"],
            path: "Tests/ReadabilityTests"
        ),
    ]
)
