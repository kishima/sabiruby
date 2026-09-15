//! Regression floor for mruby's test suite: every file must pass at least as many assertions as
//! recorded in `tests/mrbtest/baseline.txt` (the default build, strings as characters),
//! `tests/mrbtest/baseline-bytes.txt` (without the feature `utf8`) or
//! `tests/mrbtest/baseline-noregexp.txt` (without the feature `regexp`, which also leaves the
//! gem's own test files out of the list). Refresh them with `tools/mrbtest.sh --update`,
//! `tools/mrbtest.sh --bytes --update` and `tools/mrbtest.sh --no-regexp --update`.

use std::path::Path;

#[test]
fn mrbtest_no_regression() {
    let t = std::thread::Builder::new().stack_size(256 << 20).spawn(run_all).expect("spawn");
    t.join().expect("mrbtest thread");
}

fn run_all() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/mrbtest");
    // each build has its own floor: the UTF-8 one runs the assertions guarded by
    // `UTF8STRING`, which the byte-string build leaves out (`tools/mrbtest.sh --bytes`), and
    // a build without mruby-regexp has no `Regexp` for the gem's files to name at all
    let floor = match (cfg!(feature = "utf8"), cfg!(feature = "regexp")) {
        (true, true) => "baseline.txt",
        (false, true) => "baseline-bytes.txt",
        (true, false) => "baseline-noregexp.txt",
        // no baseline is kept for bytes-without-regexp: the two features are read separately
        // and nothing builds that pair
        (false, false) => { eprintln!("no baseline for a build without utf8 and without regexp"); return; }
    };
    let baseline = match std::fs::read_to_string(dir.join(floor)) {
        Ok(s) => s,
        Err(_) => { eprintln!("no baseline; run tools/mrbtest.sh --update"); return; }
    };
    let assert_mrb = std::fs::read(dir.join("assert.mrb")).expect("assert.mrb");
    // the helpers the gem test files share, which the reference's driver gets by linking every
    // file into one program (`tools/mrbtest.sh`)
    let prelude = std::fs::read(dir.join("prelude.mrb")).unwrap_or_default();
    // SABIRUBY_GC_STRESS=1: collect after every allocation (finds missing GC roots)
    let stress = std::env::var("SABIRUBY_GC_STRESS").map(|v| !v.is_empty() && v != "0").unwrap_or(false);
    let mut failures = vec![];
    for line in baseline.lines() {
        let mut it = line.split_whitespace();
        let (Some(name), Some(min)) = (it.next(), it.next()) else { continue };
        let min: u64 = min.parse().unwrap();
        let bin = std::fs::read(dir.join(format!("{name}.mrb"))).expect("test .mrb");
        let host = Box::new(sabiruby_compiler::Compiler::new());
        let sum = sabiruby::mrbtest::run_file_prelude(&assert_mrb, &prelude, &bin, 300_000_000, false, stress, Some(host)).expect("runner");
        if sum.ok < min {
            failures.push(format!("{name}: ok {} < baseline {} ({})", sum.ok, min, sum.aborted.clone().unwrap_or_default()));
        }
    }
    assert!(failures.is_empty(), "regressions:\n{}", failures.join("\n"));
}
