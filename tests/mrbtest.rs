//! Regression floor for mruby's test suite: every file must pass at least as
//! many assertions as recorded in `tests/mrbtest/baseline.txt`
//! (refresh it with `tools/mrbtest.sh --update`).

use std::path::Path;

#[test]
fn mrbtest_no_regression() {
    let t = std::thread::Builder::new().stack_size(256 << 20).spawn(run_all).expect("spawn");
    t.join().expect("mrbtest thread");
}

fn run_all() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/mrbtest");
    let baseline = match std::fs::read_to_string(dir.join("baseline.txt")) {
        Ok(s) => s,
        Err(_) => { eprintln!("no baseline; run tools/mrbtest.sh --update"); return; }
    };
    let assert_mrb = std::fs::read(dir.join("assert.mrb")).expect("assert.mrb");
    let mut failures = vec![];
    for line in baseline.lines() {
        let mut it = line.split_whitespace();
        let (Some(name), Some(min)) = (it.next(), it.next()) else { continue };
        let min: u64 = min.parse().unwrap();
        let bin = std::fs::read(dir.join(format!("{name}.mrb"))).expect("test .mrb");
        let sum = sabiruby::mrbtest::run_file(&assert_mrb, &bin, 300_000_000).expect("runner");
        if sum.ok < min {
            failures.push(format!("{name}: ok {} < baseline {} ({})", sum.ok, min, sum.aborted.clone().unwrap_or_default()));
        }
    }
    assert!(failures.is_empty(), "regressions:\n{}", failures.join("\n"));
}
