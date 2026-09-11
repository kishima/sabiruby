//! Kernel: I/O, raise, block_given?, conversions.

use crate::argc;
use crate::error::{VmError, VmResult};
use crate::object::ObjKind;
use crate::value::Value;
use crate::vm::Vm;

pub fn init(vm: &mut Vm) {
    let k = vm.core.kernel;
    vm.define_methods(k, &[
        ("puts", puts),
        ("print", print),
        ("p", p),
        ("raise", raise),
        ("block_given?", block_given),
        ("Integer", kernel_integer),
        ("Float", kernel_float),
        ("String", |vm, _s, a, _b| { argc!(vm, a, 1); let b = vm.as_string(a[0])?; Ok(vm.str_new(&b)) }),
        ("Array", |vm, _s, a, _b| { argc!(vm, a, 1); let v = if a[0].is_nil() { vec![] } else { vm.to_array(a[0])? }; Ok(vm.ary_new(v)) }),
        ("lambda", |vm, _s, _a, b| make_proc(vm, b, true)),
        ("proc", |vm, _s, _a, b| make_proc(vm, b, false)),
        ("__method__", |vm, _s, _a, _b| { let m = vm.ci.iter().rev().find_map(|c| c.mid); Ok(m.map(Value::Sym).unwrap_or(Value::Nil)) }),
        ("iterator?", block_given),
        ("global_variables", |vm, _s, _a, _b| { let l: Vec<Value> = vm.globals.keys().map(|k| Value::Sym(*k)).collect(); Ok(vm.ary_new(l)) }),
        ("local_variables", |vm, _s, _a, _b| Ok(vm.ary_new(vec![]))),
        ("instance_variable_names", |vm, s, _a, _b| { let names: Vec<Value> = match s { Value::Obj(o) => vm.heap.get(o).ivars.iter().map(|(k, _)| Value::Sym(*k)).collect(), _ => vec![] }; Ok(vm.ary_new(names)) }),
        ("__printstr__", |vm, _s, a, _b| { for v in a { let b = vm.as_string(*v)?; vm.write_out(&b); } Ok(Value::Nil) }),
        ("!~", |vm, s, a, _b| { argc!(vm, a, 1); let m = vm.intern("=~"); let r = vm.funcall(s, m, &[a[0]], Value::Nil)?; Ok(Value::bool(!r.truthy())) }),
    ]);
}

fn puts_value(vm: &mut Vm, v: Value, depth: usize) -> VmResult<()> {
    if let Some(items) = vm.ary(v).cloned() {
        if items.is_empty() && depth == 0 { vm.write_out(b"\n"); }
        for it in items { puts_value(vm, it, depth + 1)?; }
        return Ok(());
    }
    let mut b = vm.as_string(v)?;
    if b.last() != Some(&b'\n') { b.push(b'\n'); }
    vm.write_out(&b);
    Ok(())
}

fn puts(vm: &mut Vm, _s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    if a.is_empty() { vm.write_out(b"\n"); }
    for v in a { puts_value(vm, *v, 0)?; }
    Ok(Value::Nil)
}

fn print(vm: &mut Vm, _s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    for v in a { let b = vm.as_string(*v)?; vm.write_out(&b); }
    Ok(Value::Nil)
}

fn p(vm: &mut Vm, _s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    for v in a {
        let mut b = vm.inspect(*v)?;
        b.push(b'\n');
        vm.write_out(&b);
    }
    match a.len() {
        0 => Ok(Value::Nil),
        1 => Ok(a[0]),
        _ => Ok(vm.ary_new(a.to_vec())),
    }
}

fn block_given(vm: &mut Vm, _s: Value, _a: &[Value], _b: Value) -> VmResult<Value> {
    // The block of the *calling* Ruby frame: it sits after its arguments.
    let ci = match vm.ci.last() { Some(c) => c.clone(), None => return Ok(Value::False) };
    let n = ci.n as usize;
    let bidx = if n == 15 { ci.base + 2 } else { ci.base + n + 1 };
    let blk = match ci.env {
        Some(e) if !vm.heap.env(e).attached => { let ed = vm.heap.env(e); ed.values.get(bidx - ci.base).copied().unwrap_or(Value::Nil) }
        _ => vm.stack.get(bidx).copied().unwrap_or(Value::Nil),
    };
    Ok(Value::bool(!blk.is_nil()))
}

/// `raise`, `raise "msg"`, `raise Class`, `raise Class, "msg"`, `raise exc`.
fn raise(vm: &mut Vm, _s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    argc!(vm, a, 0, 3);
    let exc = match a.len() {
        0 => vm.exc_new(vm.core.runtime_error, "unhandled exception"),
        _ => {
            if let Some(msg) = vm.str_bytes(a[0]).map(|b| b.to_vec()) {
                if a.len() > 1 { return Err(vm.raise_type("exception class/object expected")); }
                let s = String::from_utf8_lossy(&msg).into_owned();
                vm.exc_new(vm.core.runtime_error, &s)
            } else {
                let exception = vm.intern("exception");
                if !vm.respond_to(a[0], exception) { return Err(vm.raise_type("exception class/object expected")); }
                let rest = &a[1..a.len().min(2)];
                let e = vm.funcall(a[0], exception, rest, Value::Nil)?;
                if !vm.obj_is_kind_of(e, vm.core.exception) { return Err(vm.raise_type("exception object expected")); }
                e
            }
        }
    };
    Err(VmError::Raise(exc))
}

pub fn parse_int(s: &[u8], base: u32) -> Option<i64> {
    let t = String::from_utf8_lossy(s);
    let t = t.trim();
    let t: String = t.chars().filter(|c| *c != '_').collect();
    let (neg, body) = match t.strip_prefix('-') { Some(r) => (true, r), None => (false, t.strip_prefix('+').unwrap_or(&t)) };
    let (base, body) = if base == 10 || base == 0 {
        if let Some(r) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) { (16, r) }
        else if let Some(r) = body.strip_prefix("0b").or_else(|| body.strip_prefix("0B")) { (2, r) }
        else if let Some(r) = body.strip_prefix("0o").or_else(|| body.strip_prefix("0O")) { (8, r) }
        else if base == 0 && body.len() > 1 && body.starts_with('0') { (8, &body[1..]) }
        else { (10, body) }
    } else { (base, body) };
    if body.is_empty() { return None; }
    i64::from_str_radix(body, base).ok().map(|v| if neg { -v } else { v })
}

fn kernel_integer(vm: &mut Vm, _s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    argc!(vm, a, 1, 2);
    match a[0] {
        Value::Int(_) => Ok(a[0]),
        Value::Float(f) => {
            if !f.is_finite() { return Err(vm.raise(vm.core.float_domain_error, &numeric::float_to_s(f))); }
            Ok(Value::Int(f.trunc() as i64))
        }
        v => {
            let base = if a.len() == 2 { vm.expect_int(a[1], "base")? as u32 } else { 0 };
            match vm.str_bytes(v).map(|b| b.to_vec()) {
                Some(b) => match parse_int(&b, base) {
                    Some(i) => Ok(Value::Int(i)),
                    None => { let d = vm.inspect_str(v)?; Err(vm.raise_arg(&format!("invalid string for number({d})"))) }
                },
                None => { let to_i = vm.intern("to_i"); if vm.respond_to(v, to_i) { vm.funcall(v, to_i, &[], Value::Nil) } else { let d = vm.describe_for_error(v); Err(vm.raise_type(&format!("can't convert {d} into Integer"))) } }
            }
        }
    }
}

fn kernel_float(vm: &mut Vm, _s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    argc!(vm, a, 1);
    match a[0] {
        Value::Int(i) => Ok(Value::Float(i as f64)),
        Value::Float(_) => Ok(a[0]),
        v => match vm.str_bytes(v).map(|b| String::from_utf8_lossy(b).into_owned()) {
            Some(s) => match s.trim().replace('_', "").parse::<f64>() {
                Ok(f) => Ok(Value::Float(f)),
                Err(_) => { let d = vm.inspect_str(v)?; Err(vm.raise_arg(&format!("invalid value for Float(): {d}"))) }
            },
            None => { let to_f = vm.intern("to_f"); if vm.respond_to(v, to_f) { vm.funcall(v, to_f, &[], Value::Nil) } else { let d = vm.describe_for_error(v); Err(vm.raise_type(&format!("can't convert {d} into Float"))) } }
        },
    }
}

fn make_proc(vm: &mut Vm, b: Value, lambda: bool) -> VmResult<Value> {
    match b {
        Value::Obj(o) if matches!(vm.heap.get(o).kind, ObjKind::Proc(_)) => {
            if lambda { if let ObjKind::Proc(pd) = &mut vm.heap.get_mut(o).kind { pd.strict = true; } }
            Ok(b)
        }
        _ => Err(vm.raise_arg("tried to create Proc object without a block")),
    }
}

use super::numeric;
