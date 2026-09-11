//! Exception hierarchy natives (the classes themselves are created in `Vm::new`).

use alloc::{format, string::String};

use crate::argc;
use crate::error::VmResult;
use crate::value::Value;
use crate::vm::Vm;

fn exc_to_s(vm: &mut Vm, s: Value, _a: &[Value], _b: Value) -> VmResult<Value> {
    let o = s.obj().unwrap();
    let m = vm.heap.ivar_get(o, vm.s.mesg);
    if m.is_nil() { let c = vm.real_class_of(s); let n = vm.class_name(c); return Ok(vm.str_from(n)); }
    if vm.str_bytes(m).is_some() { return Ok(m); }
    let b = vm.as_string(m)?; // non-String message (e.g. a Symbol) is converted
    Ok(vm.str_new(&b))
}

pub fn init(vm: &mut Vm) {
    let c = vm.core;
    vm.define_methods(c.exception, &[
        ("initialize", |vm, s, a, _b| { argc!(vm, a, 0, 1); if let Some(m) = a.first() { let mesg = vm.s.mesg; vm.heap.ivar_set(s.obj().unwrap(), mesg, *m); } Ok(s) }),
        ("exception", |vm, s, a, _b| { argc!(vm, a, 0, 1); if a.is_empty() { return Ok(s); } let d = super::object_dup(vm, s)?; let mesg = vm.s.mesg; vm.heap.ivar_set(d.obj().unwrap(), mesg, a[0]); Ok(d) }),
        ("to_s", exc_to_s),
        ("message", |vm, s, _a, _b| { let to_s = vm.s.to_s; vm.funcall(s, to_s, &[], Value::Nil) }),
        ("inspect", |vm, s, _a, _b| { let c = vm.real_class_of(s); let cn = vm.class_name(c); let m = vm.heap.ivar_get(s.obj().unwrap(), vm.s.mesg); let mb = if m.is_nil() { Vec::new() } else { vm.as_string(m)? }; if mb.is_empty() { return Ok(vm.str_from(cn)); } let ms = String::from_utf8_lossy(&mb).into_owned(); Ok(vm.str_from(format!("#<{cn}: {ms}>"))) }),
        ("backtrace", |_vm, _s, _a, _b| Ok(Value::Nil)),
        ("set_backtrace", |_vm, _s, a, _b| Ok(a.first().copied().unwrap_or(Value::Nil))),
        ("full_message", |vm, s, _a, _b| { let insp = vm.inspect_str(s)?; Ok(vm.str_from(insp)) }),
        ("==", |vm, s, a, _b| { argc!(vm, a, 1); if s == a[0] { return Ok(Value::True); } if vm.real_class_of(s) != vm.real_class_of(a[0]) { return Ok(Value::False); } let (x, y) = (exc_to_s(vm, s, &[], Value::Nil)?, exc_to_s(vm, a[0], &[], Value::Nil)?); vm.equal(x, y).map(Value::bool) }),
    ]);
    // Exception.exception(msg) == Exception.new(msg)
    let sc = vm.singleton_class(Value::Obj(c.exception)).unwrap();
    vm.define_method(sc, "exception", |vm, s, a, b| vm.class_new_instance(s.obj().unwrap(), a, b));
}
