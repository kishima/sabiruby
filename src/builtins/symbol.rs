//! Symbol.

use crate::argc;
use crate::value::Value;
use crate::vm::Vm;

/// `:sym` or `:"weird sym"`.
pub fn sym_inspect(name: &[u8]) -> Vec<u8> {
    let s = String::from_utf8_lossy(name);
    let simple = {
        let ops = ["[]", "[]=", "!", "!=", "!~", "+", "-", "*", "/", "%", "**", "==", "===", "=~", "<=>", "<", "<=", ">", ">=", "<<", ">>", "&", "|", "^", "~", "+@", "-@", "call", "`"];
        let ident = |t: &str| { let mut cs = t.chars(); matches!(cs.next(), Some(c) if c == '_' || c.is_alphabetic() || !c.is_ascii()) && cs.all(|c| c == '_' || c.is_alphanumeric() || !c.is_ascii()) };
        if ops.contains(&s.as_ref()) { true }
        else if let Some(t) = s.strip_prefix("@@").or_else(|| s.strip_prefix('@')).or_else(|| s.strip_prefix('$')) { ident(t) }
        else if let Some(t) = s.strip_suffix('=').or_else(|| s.strip_suffix('?')).or_else(|| s.strip_suffix('!')) { ident(t) }
        else { ident(&s) }
    };
    if simple { let mut v = b":".to_vec(); v.extend_from_slice(name); v }
    else { let mut v = b":".to_vec(); v.extend(super::string::str_inspect(name)); v }
}

pub fn init(vm: &mut Vm) {
    let c = vm.core;
    vm.define_methods(c.symbol, &[
        ("to_s", |vm, s, _a, _b| { let n = match s { Value::Sym(x) => vm.syms.name(x).to_vec(), _ => vec![] }; Ok(vm.str_new(&n)) }),
        ("id2name", |vm, s, _a, _b| { let n = match s { Value::Sym(x) => vm.syms.name(x).to_vec(), _ => vec![] }; Ok(vm.str_new(&n)) }),
        ("name", |vm, s, _a, _b| { let n = match s { Value::Sym(x) => vm.syms.name(x).to_vec(), _ => vec![] }; Ok(vm.str_new(&n)) }),
        ("to_sym", |_vm, s, _a, _b| Ok(s)),
        ("inspect", |vm, s, _a, _b| { let n = match s { Value::Sym(x) => vm.syms.name(x).to_vec(), _ => vec![] }; let i = sym_inspect(&n); Ok(vm.str_new(&i)) }),
        ("==", |_vm, s, a, _b| Ok(Value::bool(a.first().map(|x| *x == s).unwrap_or(false)))),
        ("===", |_vm, s, a, _b| Ok(Value::bool(a.first().map(|x| *x == s).unwrap_or(false)))),
        ("<=>", |vm, s, a, _b| { argc!(vm, a, 1); match (s, a[0]) { (Value::Sym(x), Value::Sym(y)) => Ok(Value::Int(vm.syms.name(x).cmp(vm.syms.name(y)) as i64)), _ => Ok(Value::Nil) } }),
        ("length", |vm, s, _a, _b| Ok(Value::Int(match s { Value::Sym(x) => vm.syms.name(x).len() as i64, _ => 0 }))),
        ("size", |vm, s, _a, _b| Ok(Value::Int(match s { Value::Sym(x) => vm.syms.name(x).len() as i64, _ => 0 }))),
        ("hash", |vm, s, _a, _b| Ok(Value::Int(vm.value_hash(s)))),
        ("empty?", |vm, s, _a, _b| Ok(Value::bool(match s { Value::Sym(x) => vm.syms.name(x).is_empty(), _ => true }))),
    ]);
}
