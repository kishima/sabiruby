//! Builds the vendored reference compiler (see `vendor/VENDOR.md`) as C, the way the
//! reference `mrbc` sub-build does: standalone, without the mruby VM.

use std::path::Path;

fn c_files(dir: &str) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{dir}: {e}"))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "c"))
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

fn main() {
    println!("cargo:rerun-if-changed=vendor");
    println!("cargo:rerun-if-changed=csrc");
    let mut files = c_files("vendor/mruby-compiler/src");
    files.extend(c_files("vendor/prism/src"));
    files.extend(c_files("vendor/prism/src/util"));
    files.extend(c_files("vendor/prism/generated/src"));
    assert!(Path::new("vendor/prism/generated/include/prism/ast.h").exists(), "vendor/ is incomplete: run tools/vendor_compiler.sh");
    let wasi = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("wasi");
    let ast = std::env::var_os("CARGO_FEATURE_AST").is_some();
    let mut build = cc::Build::new();
    build
        .files(&files)
        .file("csrc/shim.c")
        .include("vendor/mruby-compiler/include")
        .include("vendor/prism/generated/include")
        .include("vendor/prism/include")
        .include("vendor") // mrbconf.h
        // as mrbgems/mruby-compiler/mrbgem.rake for an mrbc build without MRC_DEBUG;
        // no MRC_TARGET_MRUBY / MRC_TARGET_MRUBYC: the standalone path of mrc_common.h
        .define("PRISM_XALLOCATOR", None)
        .define("PRISM_DEPTH_MAXIMUM", "256")
        // as the reference build (`-std=gnu99`); strict c99 hides POSIX declarations such as
        // memccpy (used by compile.c) in wasi-libc
        .std("gnu99")
        .warnings(false);
    if ast {
        // PRISM_BUILD_MINIMAL without PRISM_EXCLUDE_PRETTYPRINT (prism/defines.h): the parser
        // is the same, pm_prettyprint() is compiled in
        for d in ["PRISM_EXCLUDE_SERIALIZATION", "PRISM_EXCLUDE_JSON", "PRISM_EXCLUDE_PACK", "PRISM_ENCODING_EXCLUDE_FULL"] { build.define(d, None); }
        build.define("SABIRUBY_SHIM_AST", None);
    } else {
        build.define("PRISM_BUILD_MINIMAL", None);
    }
    if wasi {
        // MRC_TRY/MRC_THROW (codegen errors) are setjmp/longjmp, which wasi-libc implements
        // with WebAssembly exception handling. The legacy encoding runs on every current
        // browser engine and on Node 22.
        build.flag("-mllvm").flag("-wasm-enable-sjlj").flag("-mllvm").flag("-wasm-use-legacy-eh=true");
    }
    build.compile("sabiruby_mrc");
    if wasi {
        // ... and libsetjmp.a of the same sysroot (wasi-sdk; rustc's own wasi sysroot has none).
        let mut cmd = build.get_compiler().to_command();
        let out = cmd.arg("--print-file-name=libsetjmp.a").output().expect("run the C compiler");
        let lib = std::path::PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
        assert!(lib.is_absolute() && lib.exists(),
                "libsetjmp.a not found for wasm32-wasip1: build with wasi-sdk's clang (CC_wasm32_wasip1=<wasi-sdk>/bin/clang)");
        // Copied into OUT_DIR under its own name: putting wasi-sdk's lib directory on the search
        // path would also make `-lc` pick wasi-sdk's libc instead of rustc's (a crt mismatch).
        let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
        std::fs::copy(&lib, out_dir.join("libsabiruby_setjmp.a")).expect("copy libsetjmp.a");
        println!("cargo:rustc-link-search=native={}", out_dir.display());
        println!("cargo:rustc-link-lib=static=sabiruby_setjmp");
    }
}
