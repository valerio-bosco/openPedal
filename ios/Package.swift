// swift-tools-version: 6.0
import Foundation
import PackageDescription

// The Rust core is a static library built by scripts/build-core.sh into OpenPedalCore/lib.
let coreLib = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
    .appendingPathComponent("OpenPedalCore/lib").path

let package = Package(
    name: "OpenPedal",
    platforms: [.iOS("26.0")],
    products: [
        .library(name: "OpenPedal", targets: ["OpenPedalApp"]),
    ],
    targets: [
        // Header of the C ABI (src/ffi.rs) + the link to libopenpedal.a.
        .target(
            name: "OpenPedalCore",
            path: "OpenPedalCore",
            exclude: ["lib"],
            sources: ["stub.c"],
            publicHeadersPath: "include",
            linkerSettings: [
                .unsafeFlags(["-L", coreLib]),
                .linkedLibrary("openpedal"),
                // What `rustc --print native-static-libs` asks for on aarch64-apple-ios.
                .linkedLibrary("objc"),
                .linkedLibrary("iconv"),
                .linkedFramework("Foundation"),
            ]
        ),
        .target(
            name: "OpenPedalApp",
            dependencies: ["OpenPedalCore"],
            path: "Sources/OpenPedalApp",
            resources: [.copy("Resources/presets"), .copy("Resources/pedals")]
        ),
    ],
    swiftLanguageModes: [.v5] // the audio callbacks capture raw pointers
)
