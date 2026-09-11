//! Runner for mruby's own test suite (`test/assert.rb` + `test/t/*.rb`).
//!
//! The reference suite is plain Ruby driven by `assert { ... }` blocks; the C
//! side (`mrbgems/mruby-test/driver.c`) only adds `t_print`, `_str_match?`
//! (a glob matcher used by `assert_match`) and the `Mrbtest` module. Those are
//! provided here so the suite can run on SabiRuby unchanged. Use
//! `sabiruby mrbtest` (CLI) or `tools/mrbtest.sh`.

use alloc::{format, string::String, vec, vec::Vec};

use crate::error::VmResult;
use crate::value::{Slot, Value};
use crate::vm::{Step, Vm};

/// Defines the native helpers the suite expects.
pub fn install(vm: &mut Vm) {
    let k = vm.core.kernel;
    vm.define_method(k, "t_print", |vm, _s, a, _b| {
        for v in a { let b = vm.as_string(*v)?; vm.write_out(&b); }
        Ok(Value::Nil)
    });
    vm.define_method(k, "_str_match?", |vm, _s, a, _b| {
        vm.check_argc(a, 2, 2)?;
        let pat = vm.expect_str(a[0], "pattern")?;
        let s = vm.expect_str(a[1], "string")?;
        Ok(Value::bool(glob_match(&pat, &s, 0)))
    });
    let m = vm.define_module("Mrbtest");
    let tol = vm.intern("FLOAT_TOLERANCE");
    vm.heap.class_mut(m).consts.insert(tol, Slot::from(Value::Float(1e-10)));
    let sc = vm.singleton_class(Value::Obj(m)).expect("module singleton");
    vm.define_method(sc, "nofree_cstr?", |_vm, _s, _a, _b| Ok(Value::True));
    // notimplement.c: a method that raises through mrb_notimplement()
    let tni = vm.define_class("TestNotImplement", vm.core.object);
    fn gone(vm: &mut Vm, _s: Value, _a: &[Value], _b: Value) -> VmResult<Value> { Err(vm.raise(vm.core.not_implemented_error, "gone() function is unimplemented on this machine")) }
    vm.define_method(tni, "gone", gone);
    vm.notimpl_fns.push(gone);
    let tsc = vm.singleton_class(Value::Obj(tni)).expect("singleton");
    vm.define_method(tsc, "gone", gone);
    let nameless = vm.exc_new(vm.core.not_implemented_error, "function is unimplemented on this machine");
    for (k, v) in [("NAMELESS_RAISED", Value::True), ("NAMELESS_RESULT", nameless)] { let n = vm.intern(k); vm.heap.class_mut(tni).consts.insert(n, Slot::from(v)); }
    // mrbgems/mruby-fiber/test/fibertest.c: switching across native code
    let fiber = vm.core.fiber;
    let fsc = vm.singleton_class(Value::Obj(fiber)).expect("Fiber singleton");
    vm.define_method(fsc, "yield_by_c_func", |vm, _s, a, _b| { let v = a.first().copied().unwrap_or(Value::Nil); vm.fiber_yield(&[v]) });
    vm.define_method(fsc, "yield_by_c_method", |vm, s, a, _b| { let v = a.first().copied().unwrap_or(Value::Nil); let m = vm.intern("yield"); vm.funcall(s, m, &[v], Value::Nil) });
    vm.define_method(fiber, "resume_by_c_func", |vm, s, _a, _b| {
        let depth = vm.ci.len();
        let r = vm.fiber_resume(s, &[])?;
        if depth != vm.ci.len() { return Err(vm.raise(vm.core.exception, &format!("[BUG] INVALID CI POSITION (expected {depth}, but actual {}) [BUG]", vm.ci.len()))); }
        Ok(r)
    });
    vm.define_method(fiber, "resume_by_c_method", |vm, s, _a, _b| {
        let depth = vm.ci.len();
        let m = vm.intern("resume");
        let r = vm.funcall(s, m, &[], Value::Nil)?;
        if depth != vm.ci.len() { return Err(vm.raise(vm.core.exception, &format!("[BUG] INVALID CI POSITION (expected {depth}, but actual {}) [BUG]", vm.ci.len()))); }
        Ok(r)
    });
    vm.define_method(fiber, "transfer_by_c", |vm, s, _a, _b| { let m = vm.intern("transfer"); vm.funcall(s, m, &[], Value::Nil) });
    let psc = vm.singleton_class(Value::Obj(vm.core.proc_)).expect("Proc singleton");
    vm.define_method(psc, "c_tunnel", |vm, _s, _a, b| { if b.is_nil() { return Err(vm.raise_arg("no block given")); } vm.call_block(b, &[]) });
}

/// Port of `str_match_p` in `mrbgems/mruby-test/driver.c`: `*`, `?`, `[...]`,
/// `{a,b}` alternatives and backslash escapes.
pub fn glob_match(pat: &[u8], s: &[u8], depth: usize) -> bool {
    if depth > 100 { return false; }
    // find an outermost {...}
    let (mut lbrace, mut rbrace, mut nest) = (None, None, 0usize);
    let mut i = 0;
    while i < pat.len() {
        match pat[i] {
            b'{' => { if nest == 0 { lbrace = Some(i); } nest += 1; }
            b'}' if lbrace.is_some() => { nest -= 1; if nest == 0 { rbrace = Some(i); break; } }
            b'\\' => { i += 1; if i >= pat.len() { break; } }
            _ => {}
        }
        i += 1;
    }
    match (lbrace, rbrace) {
        (Some(l), Some(r)) => {
            let mut p = l;
            while p < r {
                let t = p + 1;
                let mut q = t;
                let mut n = 0;
                while q < r && !(pat[q] == b',' && n == 0) {
                    match pat[q] { b'{' => n += 1, b'}' => n -= 1, b'\\' => { q += 1; if q >= r { break; } } _ => {} }
                    q += 1;
                }
                let mut ex = Vec::with_capacity(pat.len());
                ex.extend_from_slice(&pat[..l]);
                ex.extend_from_slice(&pat[t..q.min(r)]);
                ex.extend_from_slice(&pat[r + 1..]);
                if glob_match(&ex, s, depth + 1) { return true; }
                p = q;
            }
            false
        }
        (None, None) => match_no_brace(pat, s),
        _ => false,
    }
}

fn unescape(pat: &[u8], p: usize) -> usize {
    if p < pat.len() && pat[p] == b'\\' { p + 1 } else { p }
}

fn match_bracket(pat: &[u8], mut p: usize, c: u8) -> Option<usize> {
    if p >= pat.len() { return None; }
    let mut negated = false;
    if pat[p] == b'!' || pat[p] == b'^' { negated = true; p += 1; }
    let mut ok = false;
    loop {
        if p >= pat.len() { return None; }
        if pat[p] == b']' { break; }
        let t1 = unescape(pat, p);
        if t1 >= pat.len() { return None; }
        p = t1 + 1;
        if p >= pat.len() { return None; }
        if pat[p] == b'-' && p + 1 < pat.len() && pat[p + 1] != b']' {
            let t2 = unescape(pat, p + 1);
            if t2 >= pat.len() { return None; }
            p = t2 + 1;
            if !ok && pat[t1] <= c && c <= pat[t2] { ok = true; }
        } else if !ok && pat[t1] == c {
            ok = true;
        }
    }
    if ok == negated { None } else { Some(p + 1) }
}

fn match_no_brace(pat: &[u8], s: &[u8]) -> bool {
    let (mut p, mut i) = (0usize, 0usize);
    let (mut p_tmp, mut s_tmp): (Option<usize>, Option<usize>) = (None, None);
    loop {
        if p == pat.len() { return i == s.len(); }
        let mut failed = false;
        match pat[p] {
            b'*' => {
                while p < pat.len() && pat[p] == b'*' { p += 1; }
                if unescape(pat, p) >= pat.len() { return true; }
                if i == s.len() { return false; }
                p_tmp = Some(p); s_tmp = Some(i);
                continue;
            }
            b'?' => { if i == s.len() { return false; } p += 1; i += 1; continue; }
            b'[' => {
                if i == s.len() { return false; }
                match match_bracket(pat, p + 1, s[i]) { Some(t) => { p = t; i += 1; continue; } None => failed = true }
            }
            _ => {}
        }
        if !failed {
            p = unescape(pat, p);
            if i == s.len() { return p == pat.len(); }
            if p < pat.len() && pat[p] == s[i] { p += 1; i += 1; continue; }
        }
        // failed: retry from the last '*'
        match (p_tmp, s_tmp) {
            (Some(pp), Some(ss)) => { p = pp; i = ss + 1; s_tmp = Some(ss + 1); }
            _ => return false,
        }
    }
}

/// Counts reported by `report` at the end of a test file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Summary {
    pub total: u64,
    pub ok: u64,
    pub ko: u64,
    pub crash: u64,
    pub warn: u64,
    pub skip: u64,
    /// The file did not finish within the instruction cap.
    pub timeout: bool,
    /// The file aborted with an error the harness could not rescue.
    pub aborted: Option<String>,
    /// Everything the file printed, including the `report` output.
    pub output: Vec<u8>,
    /// Executions per opcode during this file (see `Vm::op_counts`).
    pub op_counts: Vec<u64>,
}

/// Runs one test file on a fresh VM. `assert_mrb` is the compiled
/// `test/assert.rb`; `cap` bounds the instructions executed.
pub fn run_file(assert_mrb: &[u8], test_mrb: &[u8], cap: u64) -> VmResult<Summary> {
    run_file_opt(assert_mrb, test_mrb, cap, false)
}

/// `verbose` sets `$mrbtest_verbose`, so `assert` prints each test name before running it.
pub fn run_file_opt(assert_mrb: &[u8], test_mrb: &[u8], cap: u64, verbose: bool) -> VmResult<Summary> {
    let mut vm = Vm::with_mrblib()?;
    install(&mut vm);
    if verbose { let g = vm.intern("$mrbtest_verbose"); vm.globals.insert(g, Slot::from(Value::True)); }
    vm.load_and_run(assert_mrb)?;
    let mut sum = Summary::default();
    let irep = match vm.load(test_mrb) { Ok(i) => i, Err(e) => { sum.aborted = Some(format!("{e}")); return Ok(sum); } };
    vm.start(irep);
    let slice = 1_000_000u64;
    let mut used = 0u64;
    loop {
        match vm.step(slice) {
            Ok(Step::Finished(_)) => break,
            Ok(Step::Paused) => { used += slice; if used >= cap { sum.timeout = true; break; } }
            Err(e) => { sum.aborted = Some(vm.describe_error(&e)); break; }
        }
    }
    if sum.timeout {
        // leave the frames; report still works on the globals
        vm.reset_to_root();
    }
    let report = vm.intern("report");
    let top = Value::Obj(vm.top_self);
    if let Err(e) = vm.funcall(top, report, &[], Value::Nil) {
        let msg = vm.describe_error(&e);
        if sum.aborted.is_none() { sum.aborted = Some(format!("report failed: {msg}")); }
    }
    sum.output = vm.take_output();
    let text = String::from_utf8_lossy(&sum.output).into_owned();
    for line in text.lines() {
        let l = line.trim();
        let num = |prefix: &str| -> Option<u64> { l.strip_prefix(prefix).and_then(|r| r.trim().parse().ok()) };
        if let Some(n) = num("Total:") { sum.total = n; }
        else if let Some(n) = num("OK:") { sum.ok = n; }
        else if let Some(n) = num("KO:") { sum.ko = n; }
        else if let Some(n) = num("Crash:") { sum.crash = n; }
        else if let Some(n) = num("Warning:") { sum.warn = n; }
        else if let Some(n) = num("Skip:") { sum.skip = n; }
    }
    sum.op_counts = vm.op_counts.clone();
    Ok(sum)
}

/// The names of opcodes with a zero count.
pub fn unexecuted_opcodes(counts: &[u64]) -> Vec<&'static str> {
    let mut v = vec![];
    for (i, c) in counts.iter().enumerate() {
        if *c == 0 { if let Some(op) = crate::opcode::Op::from_u8(i as u8) { v.push(op.name()); } }
    }
    v
}
