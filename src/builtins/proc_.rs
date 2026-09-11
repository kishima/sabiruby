//! Proc.

use crate::error::VmResult;
use crate::object::ObjKind;
use crate::value::Value;
use crate::vm::Vm;

fn proc_call(vm: &mut Vm, s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    vm.call_block(s, a)
}

pub fn init(vm: &mut Vm) {
    let c = vm.core;
    // `call` & co. are Ruby-level methods whose body is `OP_CALL` (mruby `call_proc`),
    // so a block call is a frame replacement, not a VM re-entry. `proc_call`
    // (native, re-entrant) stays available for `Vm::call_block` from Rust.
    let cp = vm.call_proc;
    for name in ["call", "()", "[]", "yield", "==="] {
        let n = vm.intern(name);
        vm.heap.class_mut(c.proc_).methods.insert(n, crate::object::Method::Ruby(cp));
    }
    let _ = proc_call;
    vm.define_methods(c.proc_, &[
        ("to_proc", |_vm, s, _a, _b| Ok(s)),
        ("lambda?", |vm, s, _a, _b| Ok(Value::bool(s.obj().map(|o| vm.heap.proc_data(o).strict).unwrap_or(false)))),
        ("arity", |vm, s, _a, _b| { let o = s.obj().unwrap(); let pd = vm.heap.proc_data(o); let irep = &vm.ireps[pd.irep]; Ok(Value::Int(arity_of(irep, pd.strict))) }),
        ("parameters", |vm, _s, _a, _b| Ok(vm.ary_new(vec![]))),
        ("inspect", |vm, s, _a, _b| { let t = super::object::any_to_s(vm, s); Ok(vm.str_from(t)) }),
        ("to_s", |vm, s, _a, _b| { let t = super::object::any_to_s(vm, s); Ok(vm.str_from(t)) }),
    ]);
    let sc = vm.singleton_class(Value::Obj(c.proc_)).unwrap();
    vm.define_method(sc, "new", |vm, _s, _a, b| match b { Value::Obj(o) if matches!(vm.heap.get(o).kind, ObjKind::Proc(_)) => Ok(b), _ => Err(vm.raise_arg("tried to create Proc object without a block")) });
}

/// Decodes the leading `OP_ENTER` to compute `Proc#arity` (mruby `mrb_proc_arity`).
fn arity_of(irep: &crate::vm::VmIrep, lambda: bool) -> i64 {
    if irep.iseq.first() != Some(&(crate::opcode::Op::Enter as u8)) { return 0; }
    let a = ((irep.iseq[1] as u32) << 16) | ((irep.iseq[2] as u32) << 8) | irep.iseq[3] as u32;
    let m1 = ((a >> 18) & 0x1f) as i64;
    let o = ((a >> 13) & 0x1f) as i64;
    let r = ((a >> 12) & 1) as i64;
    let m2 = ((a >> 7) & 0x1f) as i64;
    let k = ((a >> 2) & 0x1f) as i64;
    let kd = ((a >> 1) & 1) as i64;
    let req = m1 + m2;
    if r == 1 || o > 0 || (k > 0 && kd == 0) || (!lambda && (o > 0 || r == 1)) { if lambda || r == 1 || o > 0 { -(req + 1) } else { req } } else { req }
}
