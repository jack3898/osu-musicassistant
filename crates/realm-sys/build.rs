use std::env;
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let core_dir = manifest_dir.join("../../vendor/realm-core").canonicalize().expect(
        "vendor/realm-core is missing; run `git submodule update --init --recursive vendor/realm-core`",
    );
    let header = core_dir.join("src/realm.h");

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", header.display());
    println!("cargo:rerun-if-changed={}", core_dir.join("dependencies.yml").display());

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();

    let mut cfg = cmake::Config::new(&core_dir);
    cfg.build_target("RealmFFIStatic")
        // Always build core optimised and against the release CRT that Rust links on MSVC.
        .profile("Release")
        .define("REALM_BUILD_LIB_ONLY", "ON")
        .define("REALM_NO_TESTS", "ON")
        // We only ever read lazer's unencrypted database, so skip OpenSSL and the S2 geometry library.
        .define("REALM_ENABLE_ENCRYPTION", "OFF")
        .define("REALM_ENABLE_GEOSPATIAL", "OFF");
    if target_env != "msvc" {
        // realm-core 20.1.2 relies on <cstdlib> arriving transitively, which newer libc++ no longer does.
        cfg.cxxflag("-include cstdlib");
    }
    if which("ninja") {
        cfg.generator("Ninja");
    }
    let build_dir = cfg.build().join("build");

    for sub in ["src/realm/object-store/c_api", "src/realm/object-store", "src/realm/parser", "src/realm"] {
        println!("cargo:rustc-link-search=native={}", build_dir.join(sub).display());
        // Multi-config generators (Visual Studio) put libraries in a per-configuration folder.
        println!("cargo:rustc-link-search=native={}", build_dir.join(sub).join("Release").display());
    }
    for lib in ["realm-ffi-static", "realm-object-store", "realm-parser", "realm"] {
        println!("cargo:rustc-link-lib=static={lib}");
    }
    match target_os.as_str() {
        "macos" | "ios" => {
            println!("cargo:rustc-link-lib=c++");
            println!("cargo:rustc-link-lib=framework=Foundation");
            println!("cargo:rustc-link-lib=framework=Security");
        }
        "windows" => {
            println!("cargo:rustc-link-lib=Version");
            println!("cargo:rustc-link-lib=psapi");
        }
        _ => {
            println!("cargo:rustc-link-lib=stdc++");
            println!("cargo:rustc-link-lib=pthread");
        }
    }

    let bindings = bindgen::Builder::default()
        .header(header.to_str().unwrap())
        .clang_arg(format!("-I{}", core_dir.join("src").display()))
        .clang_arg("-DRLM_NO_DLLIMPORT")
        .allowlist_function("realm_.*")
        .allowlist_type("realm_.*")
        .allowlist_var("RLM_.*")
        // Newtype enums accept any value C hands back, including OR-ed flags.
        .default_enum_style(bindgen::EnumVariation::NewType { is_bitfield: false, is_global: false })
        .bitfield_enum("realm_(class|property)_flags.*|realm_column_attr.*")
        .prepend_enum_name(false)
        .derive_default(true)
        .layout_tests(false)
        .generate()
        .expect("failed to generate realm-core bindings");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings.write_to_file(out.join("bindings.rs")).unwrap();
}

fn which(program: &str) -> bool {
    std::process::Command::new(program)
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}
