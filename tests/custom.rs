//! SabiRuby's own tests: `tests/custom/<case>.rb`, compiled by the reference
//! `mrbc` into `.mrb` (`tools/custom.sh`), run here and compared with
//! `.expected`, which is decided by hand (the header of each `.rb` says from
//! what). `.rc.out` is the reference mruby's output, kept to show where the
//! expectation differs from it on purpose.
//!
//! A case whose header has `# pending: <feature>` is expected to FAIL until
//! that feature exists: the run passes while it fails and fails once it
//! passes, so the marker gets removed when the feature lands.

use std::path::Path;

struct Case { name: String, pending: Option<String> }

fn cases(dir: &Path) -> Vec<Case> {
    let mut v: Vec<Case> = std::fs::read_dir(dir).expect("tests/custom")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|x| x == "rb").unwrap_or(false))
        .map(|e| {
            let name = e.path().file_stem().unwrap().to_string_lossy().into_owned();
            let src = std::fs::read_to_string(e.path()).unwrap();
            let pending = src.lines().take_while(|l| l.starts_with('#'))
                .find_map(|l| l.strip_prefix("# pending:").map(|s| s.trim().to_string()));
            Case { name, pending }
        }).collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

fn run(dir: &Path, name: &str) -> Result<Vec<u8>, String> {
    let mrb = std::fs::read(dir.join(format!("{name}.mrb")))
        .map_err(|_| format!("{name}.mrb missing: run tools/custom.sh"))?;
    let mut vm = sabiruby::Vm::new();
    // the `eval` cases need a compiler (`Vm::set_host`)
    vm.set_host(Box::new(sabiruby_compiler::Compiler::new()));
    vm.set_gc_stress(std::env::var("SABIRUBY_GC_STRESS").map(|v| !v.is_empty() && v != "0").unwrap_or(false));
    vm.load_mrblib().expect("mrblib loads");
    let result = vm.load_and_run(&mrb);
    let mut out = vm.take_output();
    if let Err(e) = result {
        let msg = vm.describe_error(&e);
        out.extend_from_slice(format!("<error: {msg}>\n").as_bytes());
    }
    Ok(out)
}

#[test]
fn custom_cases() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/custom");
    let mut problems = vec![];
    let mut passed = 0;
    let mut pending = 0;
    for c in cases(&dir) {
        let expected = std::fs::read(dir.join(format!("{}.expected", c.name)))
            .unwrap_or_else(|_| panic!("{}.expected missing", c.name));
        let ok = match run(&dir, &c.name) {
            Ok(out) if out == expected => true,
            Ok(out) => {
                if c.pending.is_none() {
                    problems.push(format!("{}: output differs\n--- expected\n{}--- got\n{}", c.name,
                        String::from_utf8_lossy(&expected), String::from_utf8_lossy(&out)));
                }
                false
            }
            Err(e) => { problems.push(e); false }
        };
        match (ok, &c.pending) {
            (true, None) => passed += 1,
            (false, Some(_)) => pending += 1,
            (true, Some(feat)) => problems.push(format!("{}: passes now; remove `# pending: {feat}` from the .rb", c.name)),
            (false, None) => {}
        }
    }
    eprintln!("custom: {passed} passed, {pending} pending (expected to fail), {} problems", problems.len());
    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
}
