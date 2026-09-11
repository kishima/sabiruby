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
    cc::Build::new()
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
        .define("PRISM_BUILD_MINIMAL", None)
        .std("c99")
        .warnings(false)
        .compile("sabiruby_mrc");
}
