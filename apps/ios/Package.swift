// swift-tools-version: 6.0
import Foundation
import PackageDescription

// The Rust core is a static library built by scripts/build-core.sh into OpenPedalCore/lib.
let coreLib = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
    .appendingPathComponent("OpenPedalCore/lib").path

// build-core.sh drops this marker next to the library when built with OPENPEDAL_CXX_PROBE=1
// (spike docs/spikes/ios-cxx.md): the core then contains C++ (libc++) and the probe symbol.
let cxxProbe = FileManager.default.fileExists(atPath: coreLib + "/.cxx-probe")

// Same for the separation build (default; OPENPEDAL_SEPARATE=0 opts out): the core then contains demucs.cpp (libc++, Accelerate BLAS).
let separate = FileManager.default.fileExists(atPath: coreLib + "/.separate")

// build-core.sh copies the Demucs weights into Resources/models only for separation builds.
let hasModels = FileManager.default.fileExists(atPath: URL(fileURLWithPath: #filePath).deletingLastPathComponent()
    .appendingPathComponent("Sources/OpenPedalApp/Resources/models").path)

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
            cSettings: (cxxProbe ? [.define("OPENPEDAL_CXX_PROBE")] : [])
                + (separate ? [.define("OPENPEDAL_SEPARATE")] : []),
            linkerSettings: [
                .unsafeFlags(["-L", coreLib]),
                .linkedLibrary("openpedal"),
                // What `rustc --print native-static-libs` asks for on aarch64-apple-ios.
                .linkedLibrary("objc"),
                .linkedLibrary("iconv"),
                .linkedFramework("Foundation"),
            ] + (cxxProbe || separate ? [.linkedLibrary("c++")] : [])
                + (separate ? [.linkedFramework("Accelerate")] : [])
        ),
        .target(
            name: "OpenPedalApp",
            dependencies: ["OpenPedalCore"],
            path: "Sources/OpenPedalApp",
            resources: [.copy("Resources/presets"), .copy("Resources/pedals")]
                + (hasModels ? [.copy("Resources/models")] : [])
        ),
    ],
    swiftLanguageModes: [.v5] // the audio callbacks capture raw pointers
)
