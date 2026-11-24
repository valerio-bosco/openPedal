// swift-tools-version: 6.0
import Foundation
import PackageDescription

// The Rust core is a static library built by scripts/build-core.sh into OpenPedalCore/lib.
let coreLib = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
    .appendingPathComponent("OpenPedalCore/lib").path

// build-core.sh drops this marker next to the library when built with OPENPEDAL_CXX_PROBE=1
// (spike docs/spikes/ios-cxx.md): the core then contains C++ (libc++) and the probe symbol.
let cxxProbe = FileManager.default.fileExists(atPath: coreLib + "/.cxx-probe")

let package = Package(
    name: "OpenPedal",
    platforms: [.iOS("26.0")],
    products: [
        .library(name: "OpenPedal", targets: ["OpenPedalApp"]),
    ],
    targets: [
        // Header of the C ABI (crates/ffi/src/lib.rs) + the link to libopenpedal.a.
        .target(
            name: "OpenPedalCore",
            path: "OpenPedalCore",
            exclude: ["lib"],
            sources: ["stub.c"],
            publicHeadersPath: "include",
            cSettings: cxxProbe ? [.define("OPENPEDAL_CXX_PROBE")] : [],
            linkerSettings: [
                .unsafeFlags(["-L", coreLib]),
                .linkedLibrary("openpedal"),
                // What `rustc --print native-static-libs` asks for on aarch64-apple-ios.
                .linkedLibrary("objc"),
                .linkedLibrary("iconv"),
                .linkedFramework("Foundation"),
            ] + (cxxProbe ? [.linkedLibrary("c++")] : [])
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
