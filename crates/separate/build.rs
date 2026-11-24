//! Compiles the vendored demucs.cpp (third_party/demucs-cpp) and our shim with the `cc` crate.
//!
//! Desktop: C++17, -O3, plain Eigen (the vendored copy; no -march=native, no OpenMP).
//! iOS: the recipe of docs/spikes/ios-cxx.md (CXX_<target> clang wrapper, SDKROOT, llvm-ar from
//! apps/ios/scripts/build-core.sh), plus -DEIGEN_USE_BLAS and `-framework Accelerate`.
use std::env;
use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../third_party/demucs-cpp");
    let src = root.join("src");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&src)
        .expect("third_party/demucs-cpp/src is missing")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "cpp"))
        .collect();
    files.sort();
    files.push("cpp/shim.cpp".into());
    for f in &files {
        println!("cargo:rerun-if-changed={}", f.display());
    }
    println!("cargo:rerun-if-changed=cpp/shim.h");
    println!("cargo:rerun-if-changed=build.rs");

    let mut b = cc::Build::new();
    b.cpp(true).files(&files).std("c++17").warnings(false).opt_level(3);
    b.include(&src).include(root.join("eigen")).include(root.join("eigen/unsupported"));
    b.define("NDEBUG", None);
    b.flag_if_supported("-fno-math-errno");

    let target = env::var("TARGET").unwrap_or_default();
    if target.contains("apple-ios") {
        let sdk = env::var("SDKROOT").expect("SDKROOT must point at the iPhoneOS SDK for iOS builds");
        b.cpp_set_stdlib(None).flag("-stdlib=libc++");
        b.flag("-isysroot").flag(&sdk);
        b.flag("-isystem").flag(format!("{sdk}/usr/include/c++/v1"));
        b.define("EIGEN_USE_BLAS", None);
        println!("cargo:rustc-link-lib=c++");
        println!("cargo:rustc-link-lib=framework=Accelerate");
    } else {
        b.cpp_link_stdlib(None);
        b.flag_if_supported("-pthread");
        println!("cargo:rustc-link-lib=stdc++");
    }
    b.compile("openpedal_separate");
}
