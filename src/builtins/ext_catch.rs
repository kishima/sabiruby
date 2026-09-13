//! mruby-catch (`mrbgems/mruby-catch/src/catch.c`): `Kernel#catch`/`throw`. Its Ruby part
//! (`UncaughtThrowError`) is `src/mrblib/catch.mrb`.
//!
//! The reference's `catch` is a bytecode method (`r2.call(r1)`) that `throw` recognises on the
//! call stack by its proc; `throw` then raises an `RBreak` aimed at that frame, so `ensure`
//! bodies run and `rescue Exception` does not see it. Here `catch` is a native and records its
//! tag with the depth of the block's frame in `Vm::catch_tags`; `throw` finds the innermost
//! entry whose tag is the same object and returns a `Break` to that frame, the mechanism
//! `return` from a nested block already uses.

use crate::argc;
use crate::error::{VmError, VmResult};
use crate::object::BreakTag;
use crate::value::Value;
use crate::vm::Vm;

use super::object::same_object;

fn catch_m(vm: &mut Vm, _s: Value, a: &[Value], b: Value) -> VmResult<Value> {
    argc!(vm, a, 0, 1);
    let tag = match a.first() {
        Some(t) => *t,
        None => { let new = vm.intern("new"); vm.funcall(Value::Obj(vm.core.object), new, &[], Value::Nil)? }
    };
    if b.is_nil() {
        // `r2.call(r1)` with no block
        let call = vm.intern("call");
        return Err(vm.no_method_error(call, Value::Nil, "undefined method 'call' for nil"));
    }
    let depth = vm.ci.len(); // the block's frame will sit here
    vm.catch_tags.push((tag, vm.cur, depth));
    let r = vm.call_block(b, &[tag]);
    vm.catch_tags.pop();
    r
}

fn throw_m(vm: &mut Vm, _s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    argc!(vm, a, 1, 2);
    let tag = a[0];
    let obj = a.get(1).copied().unwrap_or(Value::Nil);
    let cur = vm.cur;
    let target = vm.catch_tags.iter().rev().find(|(t, ctx, _)| *ctx == cur && same_object(*t, tag)).map(|e| e.2);
    match target {
        Some(depth) => {
            let brk = vm.break_new(BreakTag::Break, depth, obj);
            Err(VmError::Break(brk))
        }
        None => {
            let name = vm.intern("UncaughtThrowError");
            let cls = match vm.const_get(vm.core.object, name) { Some(c) => c, None => return Err(vm.raise_arg("uncaught throw")) };
            let new = vm.intern("new");
            let exc = vm.funcall(cls, new, &[tag, obj], Value::Nil)?;
            Err(VmError::Raise(exc))
        }
    }
}

pub fn init(vm: &mut Vm) {
    let k = vm.core.kernel;
    vm.define_methods(k, &[("catch", catch_m), ("throw", throw_m)]);
    // module functions: public on `Kernel` itself, private as instance methods
    let ksc = vm.singleton_class(Value::Obj(k)).expect("Kernel singleton");
    for name in ["catch", "throw"] {
        let n = vm.intern(name);
        if let Some((m, _)) = vm.find_method(k, n) {
            vm.def_method_raw(ksc, n, m);
            let _ = vm.set_visibility(k, n, crate::object::Vis::Private);
        }
    }
}
