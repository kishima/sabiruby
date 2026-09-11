//! Kernel: I/O, raise, block_given?, conversions.

use alloc::{format, string::String, vec, vec::Vec};

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
        ("__id__", |vm, s, a, _b| { argc!(vm, a, 0); Ok(super::object::object_id(s)) }),
        ("object_id", |vm, s, a, _b| { argc!(vm, a, 0); Ok(super::object::object_id(s)) }),
        ("iterator?", block_given),
        ("global_variables", |vm, _s, _a, _b| { let l: Vec<Value> = vm.globals.keys().map(|k| Value::Sym(*k)).collect(); Ok(vm.ary_new(l)) }),
        ("local_variables", |vm, _s, _a, _b| Ok(vm.ary_new(vec![]))),
        ("instance_variable_names", |vm, s, _a, _b| { let names: Vec<Value> = match s { Value::Obj(o) => vm.heap.get(o).ivars.iter().map(|(k, _)| Value::Sym(*k)).collect(), _ => vec![] }; Ok(vm.ary_new(names)) }),
        ("__ENCODING__", |vm, _s, _a, _b| Ok(vm.str_new(b"ASCII-8BIT"))),
        // `case`/`when` with a splat: `when *list` compiles to `__case_eqq`
        ("__case_eqq", |vm, s, a, _b| {
            argc!(vm, a, 1);
            let eqq = vm.s.eqq;
            if s.is_nil() { return Ok(Value::False); }
            let to_a = vm.intern("to_a");
            let list = if vm.ary(s).is_some() { s } else if !vm.respond_to(s, to_a) { return Ok(Value::bool(vm.funcall(s, eqq, &[a[0]], Value::Nil)?.truthy())); } else {
                let r = vm.funcall(s, to_a, &[], Value::Nil)?;
                if r.is_nil() { return vm.funcall(s, eqq, &[a[0]], Value::Nil); }
                r
            };
            for it in vm.ary(list).cloned().unwrap_or_default() {
                if vm.funcall(it, eqq, &[a[0]], Value::Nil)?.truthy() { return Ok(Value::True); }
            }
            Ok(Value::False)
        }),
        // `defined?` is compiled to these (kernel.c); they answer the description string or nil
        ("__defined_ivar?", |vm, s, a, _b| { argc!(vm, a, 1); let n = super::object::sym_arg(vm, a[0])?; let ok = match s { Value::Obj(o) => vm.heap.get(o).ivars.iter().any(|(k, _)| *k == n), _ => false }; Ok(defined_str(vm, ok, "instance-variable")) }),
        ("__defined_gvar?", |vm, _s, a, _b| { argc!(vm, a, 1); let n = super::object::sym_arg(vm, a[0])?; let ok = vm.globals.contains_key(&n); Ok(defined_str(vm, ok, "global-variable")) }),
        ("__defined_cvar?", |vm, _s, a, _b| { argc!(vm, a, 1); let n = super::object::sym_arg(vm, a[0])?; let ci = *vm.ci.last().unwrap(); let cls = vm.cvar_class_of(ci.proc_); let ok = vm.cvar_lookup(cls, n).is_some(); Ok(defined_str(vm, ok, "class variable")) }),
        ("__defined_const?", |vm, _s, a, _b| { argc!(vm, a, 1); let n = super::object::sym_arg(vm, a[0])?; let ci = *vm.ci.last().unwrap(); let ok = vm.const_lookup_noraise(&ci, n).is_some(); Ok(defined_str(vm, ok, "constant")) }),
        ("__defined_const_path?", |vm, _s, a, _b| { argc!(vm, a, 2); let (p, c) = (super::object::sym_arg(vm, a[0])?, super::object::sym_arg(vm, a[1])?); let ci = *vm.ci.last().unwrap(); let ok = match vm.const_lookup_noraise(&ci, p) { Some(Value::Obj(o)) if vm.heap.is_class(o) => vm.const_get(o, c).is_some(), _ => false }; Ok(defined_str(vm, ok, "constant")) }),
        ("__defined_method?", |vm, s, a, _b| { argc!(vm, a, 1); let n = super::object::sym_arg(vm, a[0])?; let ok = vm.respond_to(s, n); Ok(defined_str(vm, ok, "method")) }),
        ("__defined_yield?", |vm, s, a, b| { let r = block_given(vm, s, a, b)?; Ok(defined_str(vm, r.truthy(), "yield")) }),
        ("__defined_super?", |vm, _s, _a, _b| { let ci = *vm.ci.last().unwrap(); let ok = match ci.mid { Some(m) => match vm.heap.class(ci.target_class).superclass { Some(sup) => vm.find_method(sup, m).is_some(), None => false }, None => false }; Ok(defined_str(vm, ok, "super")) }),
        ("__printstr__", |vm, _s, a, _b| { for v in a { let b = vm.as_string(*v)?; vm.write_out(&b); } Ok(Value::Nil) }),
        ("!~", |vm, s, a, _b| { argc!(vm, a, 1); let m = vm.intern("=~"); let r = vm.funcall(s, m, &[a[0]], Value::Nil)?; Ok(Value::bool(!r.truthy())) }),
    ]);
    // module functions: callable as Kernel.raise, private as instance methods (kernel.c MRB_MT_PRIVATE)
    let ksc = vm.singleton_class(Value::Obj(k)).unwrap();
    for name in ["raise", "block_given?", "iterator?", "p", "print", "puts", "lambda", "proc", "Integer", "Float", "String", "Array", "__printstr__"] {
        let n = vm.intern(name);
        if let Some((m, _)) = vm.find_method(k, n) {
            vm.def_method_raw(ksc, n, m);
            let _ = vm.set_visibility(k, n, crate::object::Vis::Private);
        }
    }
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

/// `mrb_f_block_given_p_m`: the block of the enclosing *method* frame, found by
/// walking the caller's proc chain up to the first scope proc.
fn block_given(vm: &mut Vm, _s: Value, _a: &[Value], _b: Value) -> VmResult<Value> {
    let ci = match vm.ci.last() { Some(c) => *c, None => return Ok(Value::False) };
    let mut p = Some(ci.proc_);
    let mut e = None;
    while let Some(pid) = p {
        let pd = vm.heap.proc_data(pid);
        if pd.scope { break; }
        e = pd.env;
        p = pd.upper;
    }
    let p = match p { Some(p) => p, None => return Ok(Value::False) };
    let blk = if let Some(e) = e {
        if vm.heap.env(e).mid.is_none() { return Ok(Value::False); } // top level / class body
        let bidx = vm.heap.env(e).bidx;
        if bidx >= vm.heap.env(e).len && !vm.heap.env(e).attached { return Ok(Value::False); }
        vm.env_value(e, bidx)
    } else {
        // the frame running `p` itself (a top-level or class-body frame has no block)
        match vm.ci.iter().rev().find(|c| c.proc_ == p) {
            Some(c) if c.mid.is_some() => { let b = Vm::frame_bidx(c); vm.stack.get(c.base + b).copied().unwrap_or(Value::Nil) }
            _ => return Ok(Value::False),
        }
    };
    Ok(Value::bool(!blk.is_nil()))
}

fn defined_str(vm: &mut Vm, ok: bool, s: &str) -> Value { if ok { vm.str_new(s.as_bytes()) } else { Value::Nil } }

/// `raise`, `raise "msg"`, `raise Class`, `raise Class, "msg"`, `raise exc`.
fn raise(vm: &mut Vm, _s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    argc!(vm, a, 0, 3);
    let exc = match a.len() {
        // mruby 4.1.0-rc: a bare `raise` is RuntimeError with an empty message (verified, not a re-raise)
        0 => vm.exc_new(vm.core.runtime_error, ""),
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
            Ok(Value::Int(libm::trunc(f) as i64))
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
