//! Runs every `tests/fixtures/*.mrb` (compiled by the reference mruby 4.1.0-rc
//! `mrbc`, see `tools/fixtures.sh`) and compares stdout with the `.out` file
//! recorded from the reference `mruby` binary.

use std::path::Path;

fn run_fixture(name: &str) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mrb = std::fs::read(dir.join(format!("{name}.mrb"))).expect("fixture .mrb");
    let expected = std::fs::read(dir.join(format!("{name}.out"))).expect("fixture .out");
    let mut vm = sabiruby::Vm::with_mrblib().expect("mrblib loads");
    let result = vm.load_and_run(&mrb);
    let mut out = vm.take_output();
    if let Err(e) = result {
        let msg = vm.describe_error(&e);
        out.extend_from_slice(format!("<error: {msg}>\n").as_bytes());
    }
    assert_eq!(String::from_utf8_lossy(&out), String::from_utf8_lossy(&expected), "fixture {name}");
}

macro_rules! fixture { ($($n:ident),*) => { $( #[test] fn $n() { run_fixture(stringify!($n)); } )* } }

fixture!(hello, arith, method, control, block, collections, klass, exception, closure, strings, objects, errors, errors2, args);

#[test]
fn kwargs() { run_fixture("kwargs"); }

/// mruby-enumerator is ported; `each_slice`/`each_cons` still need mruby-enum-ext.
#[test]
#[ignore = "needs mruby-enum-ext (each_slice, each_cons)"]
fn enumerator() { run_fixture("enumerator"); }
