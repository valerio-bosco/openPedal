//! Compiles cpp/probe.cpp with the `cc` crate.
//!
//! Eigen headers: `EIGEN_INCLUDE_DIR` (the dir that contains `Eigen/`), else /usr/include/eigen3.
//! iOS: set `CXX_aarch64_apple_ios` to a clang that can target Apple, `SDKROOT` to the iPhoneOS
//! SDK (the xtool one); libc++ headers then come from `$SDKROOT/usr/include/c++/v1`.
//! See docs/spikes/ios-cxx.md and apps/ios/scripts/build-core.sh.
use std::env;

fn main() {
    println!("cargo:rerun-if-changed=cpp/probe.cpp");
    println!("cargo:rerun-if-env-changed=EIGEN_INCLUDE_DIR");
    let mut b = cc::Build::new();
    b.cpp(true).file("cpp/probe.cpp").std("c++17").warnings(true);
    let eigen = env::var("EIGEN_INCLUDE_DIR").unwrap_or_else(|_| "/usr/include/eigen3".into());
    b.include(&eigen);

    let target = env::var("TARGET").unwrap_or_default();
    if target.contains("apple-ios") {
        let sdk = env::var("SDKROOT").expect("SDKROOT must point at the iPhoneOS SDK for iOS builds");
        b.cpp_set_stdlib(None).flag("-stdlib=libc++");
        // Headers of the SDK's libc++ (the nixpkgs clang is unwrapped: no implicit paths).
        b.flag("-isysroot").flag(&sdk);
        b.flag("-isystem").flag(format!("{sdk}/usr/include/c++/v1"));
        // Exceptions and RTTI stay on (default): Eigen/std need them only on error paths.
        println!("cargo:rustc-link-lib=c++");
    } else {
        b.cpp_link_stdlib(None); // use the platform default libstdc++
        println!("cargo:rustc-link-lib=stdc++");
    }
    b.compile("openpedal_cxx_probe");
}
