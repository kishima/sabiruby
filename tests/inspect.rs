//! `Vm::snapshot` and `Vm::set_trace` (src/inspect.rs): what a debugger or visualiser reads.

use std::path::{Path, PathBuf};

use sabiruby::inspect::{DetachReason, SwitchKind, TraceEvent, UnwindBy};
use sabiruby::{Step, Vm};

fn dir() -> PathBuf { Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf() }
fn read(rel: &str) -> Vec<u8> { std::fs::read(dir().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}")) }

/// Runs a program with recording on and returns everything it recorded.
fn trace_of(rel: &str) -> Vec<TraceEvent> {
    let mut vm = Vm::with_mrblib().expect("mrblib");
    vm.set_trace(true);
    vm.load_and_run(&read(rel)).ok();
    vm.take_trace()
}

#[test]
fn records_environments_created_and_detached() {
    let t = trace_of("tests/fixtures/closure.mrb");
    let created: Vec<_> = t.iter().filter(|e| matches!(e, TraceEvent::EnvCreate { .. })).collect();
    assert!(!created.is_empty(), "no EnvCreate in {t:?}");
    // a frame that made an environment and returned: the values moved into the object
    assert!(t.iter().any(|e| matches!(e, TraceEvent::EnvDetach { reason: DetachReason::FrameReturn, .. })),
            "no EnvDetach(FrameReturn)");
    // every detached environment was created first
    for e in &t {
        if let TraceEvent::EnvDetach { env, .. } = e {
            assert!(t.iter().any(|c| matches!(c, TraceEvent::EnvCreate { env: c, .. } if c == env)), "detached {env:?} was never created");
        }
    }
}

#[test]
fn records_a_raise_and_the_catch_table_lookups() {
    let t = trace_of("tests/fixtures/exception.mrb");
    let first = t.iter().position(|e| matches!(e, TraceEvent::Raise { .. })).expect("no Raise");
    let TraceEvent::Raise { class, .. } = &t[first] else { unreachable!() };
    assert!(!class.is_empty());
    // the frames are consulted from the innermost outwards until one matches
    let looks: Vec<_> = t[first + 1..].iter().take_while(|e| matches!(e, TraceEvent::CatchLook { .. } | TraceEvent::FrameUnwound { .. })).collect();
    assert!(!looks.is_empty(), "no CatchLook after the Raise: {:?}", &t[first..t.len().min(first + 5)]);
    assert!(t.iter().any(|e| matches!(e, TraceEvent::CatchLook { matched: Some(_), .. })), "nothing ever matched");
    assert!(t.iter().any(|e| matches!(e, TraceEvent::FrameUnwound { by: UnwindBy::Raise, .. })), "no frame unwound by a raise");
}

#[test]
fn records_fiber_switches() {
    // the enumerator fixture runs Enumerator, which is built on fibers
    let t = trace_of("tests/fixtures/enumerator.mrb");
    let switches: Vec<_> = t.iter().filter_map(|e| match e { TraceEvent::FiberSwitch { from, to, kind } => Some((*from, *to, *kind)), _ => None }).collect();
    assert!(switches.iter().any(|(_, _, k)| *k == SwitchKind::Resume), "no resume: {switches:?}");
    assert!(switches.iter().any(|(_, _, k)| *k == SwitchKind::Yield), "no yield: {switches:?}");
    assert!(switches.iter().any(|(from, to, _)| from != to), "a switch must change the context");
}

#[test]
fn records_collections() {
    let mut vm = Vm::with_mrblib().expect("mrblib");
    vm.set_gc_stress(true);
    vm.set_trace(true);
    vm.load_and_run(&read("tests/fixtures/hello.mrb")).expect("run");
    let t = vm.take_trace();
    let collects: Vec<_> = t.iter().filter_map(|e| match e { TraceEvent::GcCollect { before_live, after_live, swept, .. } => Some((*before_live, *after_live, *swept)), _ => None }).collect();
    assert!(!collects.is_empty(), "no GcCollect under stress");
    for (before, after, swept) in collects {
        assert_eq!(before, after + swept, "live before = live after + swept");
    }
}

#[test]
fn trace_is_off_by_default_and_take_clears_it() {
    let mut vm = Vm::with_mrblib().expect("mrblib");
    assert!(!vm.tracing());
    vm.load_and_run(&read("tests/fixtures/closure.mrb")).expect("run");
    assert!(vm.take_trace().is_empty());
    vm.set_trace(true);
    vm.load_and_run(&read("tests/fixtures/closure.mrb")).expect("run");
    assert!(!vm.take_trace().is_empty());
    assert!(vm.take_trace().is_empty(), "take_trace leaves the buffer empty");
    vm.set_trace(false);
    assert!(!vm.tracing());
}

#[test]
fn snapshot_describes_frames_registers_and_the_heap() {
    // compiled with -g (tools/custom.sh), so the registers carry their local variable names
    let bin = read("tests/custom/eval_outer_scope.mrb");
    let mut vm = Vm::with_mrblib().expect("mrblib");
    let irep = vm.load(&bin).expect("load");
    vm.start(irep);
    vm.step(3).expect("step");

    let s = vm.snapshot(8);
    assert_eq!(s.cur, 0, "the root context runs");
    let ctx = &s.contexts[s.cur];
    assert!(ctx.is_current);
    // the outermost frame is the program `start` pushed; after three instructions the
    // innermost one may already be a method of the program
    let frame = &ctx.frames[0];
    assert_eq!(frame.index, 0);
    assert_eq!(frame.irep, irep);
    assert!(frame.line.is_some(), "line numbers from the DBG section");
    assert_eq!(frame.target_class, "Object");
    assert_eq!(frame.nregs, frame.regs.len(), "R0..nregs");
    assert_eq!(frame.regs[0].value.text, "main", "R0 is self");
    assert_eq!(frame.regs[1].name.as_deref(), Some("x"), "R1 is the local x: {:?}", frame.regs);
    assert_eq!(frame.regs[1].value.text, "1", "x = 1 ran");
    assert_eq!(frame.proc_.irep, irep);
    assert!(s.heap.live > 0 && s.heap.len >= s.heap.live);
    assert_eq!(s.heap.free, s.heap.len - s.heap.live);
    assert!(s.instructions >= 3);
    assert!(s.pending_exc.is_none());
}

#[test]
fn snapshot_renders_values_without_running_ruby() {
    let mut vm = Vm::with_mrblib().expect("mrblib");
    let v = vm.ary_new(vec![sabiruby::Value::Int(1), sabiruby::Value::Float(2.5)]);
    let s = vm.str_new(b"hi\n");
    let sym = sabiruby::Value::Sym(vm.intern("go"));
    let before = vm.instructions;
    assert_eq!(vm.render(v, 2).text, "[1, 2.5]");
    assert_eq!(vm.render(v, 2).class, "Array");
    assert_eq!(vm.render(s, 2).text, "\"hi\\n\"");
    assert_eq!(vm.render(sym, 2).text, ":go");
    assert_eq!(vm.render(sabiruby::Value::Nil, 2).text, "nil");
    assert_eq!(vm.render(sabiruby::Value::Int(-7), 2).text, "-7");
    assert_eq!(vm.instructions, before, "rendering must not run any Ruby");
    // big containers are cut, not printed whole
    let big = vm.ary_new((0..20).map(sabiruby::Value::Int).collect());
    let text = vm.render(big, 2).text;
    assert!(text.starts_with("[0, 1, 2, 3, 4, 5, 6, 7, ...(+12)]"), "{text}");
}

#[test]
fn op_histogram_names_the_executed_opcodes() {
    let mut vm = Vm::with_mrblib().expect("mrblib");
    vm.load_and_run(&read("tests/fixtures/hello.mrb")).expect("run");
    let h = vm.op_histogram();
    assert!(h.iter().any(|(op, n)| *op == "SSEND" && *n > 0), "{h:?}");
    assert!(h.iter().all(|(_, n)| *n > 0), "opcodes never executed must be left out");
}

#[test]
fn snapshot_of_a_fiber_program_lists_both_contexts() {
    let bin = read("tests/fixtures/enumerator.mrb");
    let mut vm = Vm::with_mrblib().expect("mrblib");
    let irep = vm.load(&bin).expect("load");
    vm.start(irep);
    // run until a second context exists (the enumerator's fiber) or the program ends
    for _ in 0..2000 {
        if vm.contexts.len() > 1 { break; }
        if let Ok(Step::Finished(_)) = vm.step(50) { break; }
    }
    let s = vm.snapshot(4);
    assert!(s.contexts.len() > 1, "the enumerator should have made a fiber");
    assert_eq!(s.contexts.iter().filter(|c| c.is_current).count(), 1);
    for c in &s.contexts {
        assert_eq!(c.index, s.contexts[c.index].index);
        if c.is_current { assert!(!c.frames.is_empty()); }
    }
}
