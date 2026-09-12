//! mruby-symbol-ext (`mrbgems/mruby-symbol-ext/src/symbol.c`): `Symbol#length`/`size`,
//! `slice`/`[]`. Its Ruby part (`include Comparable`, `capitalize`, `downcase`, `upcase`,
//! `casecmp`, `casecmp?`, `empty?`, `intern`) is `src/mrblib_symbol-ext.mrb`.
//! `Symbol.all_symbols` is `MRB_USE_ALL_SYMBOLS` only and is not defined here.

use crate::error::VmResult;
use crate::value::Value;
use crate::vm::Vm;

fn sym_len(vm: &mut Vm, s: Value, _a: &[Value], _b: Value) -> VmResult<Value> {
    // byte length (`MRB_UTF8_STRING` is off in this build)
    Ok(Value::Int(match s { Value::Sym(x) => vm.syms.name(x).len() as i64, _ => 0 }))
}

/// `mrb_sym_slice`: the name (read from the symbol, not through `to_s`) is handed to
/// `String#slice`, which does the argument checking and answers a fresh String.
fn sym_slice(vm: &mut Vm, s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    let name = match s { Value::Sym(x) => vm.syms.name(x).to_vec(), _ => alloc::vec::Vec::new() };
    let str = vm.str_new(&name);
    let slice = vm.intern("slice");
    vm.funcall(str, slice, a, Value::Nil)
}

pub fn init(vm: &mut Vm) {
    let c = vm.core;
    vm.define_methods(c.symbol, &[
        ("length", sym_len),
        ("size", sym_len),
        ("slice", sym_slice),
        ("[]", sym_slice),
    ]);
}
