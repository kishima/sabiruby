//! The one thing the VM cannot know about itself: which commit it was built from.
//!
//! `MRUBY_REVISION` is the reference's commit hash (`src/version.c`, filled in by its build
//! system); here it is this repository's. A tree with no git — a build from the crates.io
//! package, for instance — keeps the reference's own default, the literal `HEAD`.

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // a commit changes the answer, so watch what git moves when one is made
    for p in [".git/HEAD", ".git/refs/heads/main"] {
        if std::path::Path::new(p).exists() {
            println!("cargo:rerun-if-changed={p}");
        }
    }
    let rev = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|r| r.len() == 40 && r.bytes().all(|b| b.is_ascii_hexdigit()));
    if let Some(rev) = rev {
        println!("cargo:rustc-env=SABIRUBY_REVISION={rev}");
    }
}
