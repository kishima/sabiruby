//! mruby-array-ext (`mrbgems/mruby-array-ext/src/array.c`) and the Array part
//! of mruby-enum-ext (`mruby-enum-ext/src/enum.c`): the natives the two gems'
//! Ruby parts (embedded as `src/mrblib/array-ext.mrb`, `src/mrblib/enum-ext.mrb`)
//! build on. Set operations use `eql?`/`hash` like the reference's khash set;
//! here that is a linear walk with `eql?`.

use alloc::{format, vec, vec::Vec};

use crate::argc;
use crate::error::VmResult;
use crate::object::ObjKind;
use crate::value::{slots_of, Value};
use crate::vm::Vm;

/// `ARY_MAX_SIZE`: the reference's is `MRB_INT_MAX / sizeof(mrb_value)`; this one is what a
/// host can allocate before the reference would have run out of memory anyway.
const ARY_MAX_SIZE: u64 = 1 << 28;

pub(crate) fn items(vm: &Vm, v: Value) -> Vec<Value> { vm.ary_vals(v).unwrap_or_default() }
/// How many elements, without copying them out (`items(vm, v).len()` allocates).
pub(crate) fn ary_len(vm: &Vm, v: Value) -> usize { vm.ary(v).map(|l| l.len()).unwrap_or(0) }

pub(crate) fn check_frozen(vm: &mut Vm, v: Value) -> VmResult<()> {
    if v.obj().map(|o| vm.heap.get(o).frozen).unwrap_or(false) { return Err(vm.frozen_error(v)); }
    Ok(())
}

/// Replaces the elements (`mrb_ary_modify` + write).
pub(crate) fn set_items(vm: &mut Vm, v: Value, list: Vec<Value>) -> VmResult<()> {
    check_frozen(vm, v)?;
    match v.obj().map(|o| &mut vm.heap.get_mut(o).kind) {
        Some(ObjKind::Array(a)) => { *a = slots_of(&list).into(); Ok(()) }
        _ => Err(vm.raise_type("not an array")),
    }
}

/// `mrb_check_array_type`: an Array as is, `to_ary` when it answers, else None.
pub(crate) fn check_array(vm: &mut Vm, v: Value) -> VmResult<Option<Vec<Value>>> {
    if let Some(a) = vm.ary_vals(v) { return Ok(Some(a)); }
    let to_ary = vm.intern("to_ary");
    if !vm.respond_to(v, to_ary) { return Ok(None); }
    let r = vm.funcall(v, to_ary, &[], Value::Nil)?;
    if r.is_nil() { return Ok(None); }
    match vm.ary_vals(r) { Some(a) => Ok(Some(a)), None => { let d = vm.describe_for_type_error(v); Err(vm.raise_type(&format!("{d} cannot be converted to Array"))) } }
}

/// `mrb_get_args("A")`.
fn ary_arg(vm: &mut Vm, v: Value) -> VmResult<Vec<Value>> {
    match check_array(vm, v)? { Some(a) => Ok(a), None => { let d = vm.describe_for_type_error(v); Err(vm.raise_type(&format!("{d} cannot be converted to Array"))) } }
}

/// `ary_get_array_args`: every argument converted, or TypeError.
fn array_args(vm: &mut Vm, a: &[Value]) -> VmResult<Vec<Vec<Value>>> {
    let mut out = Vec::with_capacity(a.len());
    for v in a {
        match check_array(vm, *v)? { Some(x) => out.push(x), None => return Err(vm.raise_type("can't convert passed argument to Array")) }
    }
    Ok(out)
}

/// `ary_memb_has`: is `v` `eql?` to an element of any of the arrays.
fn memb_has(vm: &mut Vm, arys: &[Vec<Value>], v: Value) -> VmResult<bool> {
    for a in arys { for x in a { if vm.key_eql(v, *x)? { return Ok(true); } } }
    Ok(false)
}
fn memb_in(vm: &mut Vm, list: &[Value], v: Value) -> VmResult<bool> {
    for x in list { if vm.key_eql(v, *x)? { return Ok(true); } }
    Ok(false)
}

fn type_name(vm: &mut Vm, v: Value) -> alloc::string::String { let c = vm.real_class_of(v); vm.class_name(c) }

/// `mrb_cmp` + the ArgumentError of `ary_cmp_ordered`: Integer, Float and
/// String compare natively, everything else through `<=>`.
pub(crate) fn cmp_ordered(vm: &mut Vm, a: Value, b: Value) -> VmResult<i64> {
    let r: Option<i64> = match (a, b) {
        (Value::Int(x), Value::Int(y)) => Some(x.cmp(&y) as i64),
        (Value::Int(x), Value::Float(y)) => (x as f64).partial_cmp(&y).map(|o| o as i64),
        (Value::Float(x), Value::Int(y)) => x.partial_cmp(&(y as f64)).map(|o| o as i64),
        (Value::Float(x), Value::Float(y)) => x.partial_cmp(&y).map(|o| o as i64),
        _ => match (vm.str_bytes(a), vm.str_bytes(b)) {
            (Some(p), Some(q)) => Some(p.cmp(q) as i64),
            _ => { let cmp = vm.intern("<=>"); match vm.funcall(a, cmp, &[b], Value::Nil)? { Value::Int(i) => Some(i.signum()), _ => None } }
        },
    };
    match r {
        Some(c) => Ok(c),
        None => { let x = type_name(vm, a); let y = type_name(vm, b); Err(vm.raise_arg(&format!("comparison of {x} with {y} failed"))) }
    }
}

/// `mrb_range_beg_len`: (start, length) of a Range against `len`; `trunc`
/// clips the length to the receiver. None when the range is out of it.
pub(crate) fn range_beg_len(vm: &mut Vm, r: Value, len: i64, trunc: bool) -> VmResult<Option<(i64, i64)>> {
    let (b, e, x) = match r.obj().map(|o| &vm.heap.get(o).kind) { Some(ObjKind::Range { begin, end, excl }) => (begin.get(), end.get(), *excl), _ => return Ok(None) };
    let mut beg = match b { Value::Nil => 0, v => vm.expect_int(v, "begin")? };
    let mut end = match e { Value::Nil => len, v => vm.expect_int(v, "end")? };
    let excl = if e.is_nil() { true } else { x };
    if beg < 0 { beg += len; if beg < 0 { return Ok(None); } }
    if end < 0 { end += len; }
    if !excl { end += 1; }
    if trunc {
        if beg > len { return Ok(None); }
        if end > len { end = len; }
    }
    let l = if end > beg { end - beg } else { 0 };
    Ok(Some((beg, l)))
}

fn is_range(vm: &Vm, v: Value) -> bool { matches!(v.obj().map(|o| &vm.heap.get(o).kind), Some(ObjKind::Range { .. })) }

fn union_add(vm: &mut Vm, src: &[Value], result: &mut Vec<Value>) -> VmResult<()> {
    for v in src { if !memb_in(vm, result, *v)? { result.push(*v); } }
    Ok(())
}

fn uniq_bang(vm: &mut Vm, s: Value) -> VmResult<Value> {
    let list = items(vm, s);
    if list.len() <= 1 { check_frozen(vm, s)?; return Ok(Value::Nil); }
    check_frozen(vm, s)?;
    let mut kept: Vec<Value> = vec![];
    for v in &list { if !memb_in(vm, &kept, *v)? { kept.push(*v); } }
    if kept.len() == list.len() { return Ok(Value::Nil); }
    set_items(vm, s, kept)?;
    Ok(s)
}

fn flatten_internal(vm: &Vm, s: Value, level: i64) -> (Vec<Value>, bool) {
    let mut result = vec![];
    let mut modified = false;
    // (array, index, depth), the reference's explicit stack
    let mut stack: Vec<(Vec<Value>, usize, i64)> = vec![(items(vm, s), 0, 1)];
    while let Some((ary, mut idx, depth)) = stack.pop() {
        while idx < ary.len() {
            let e = ary[idx];
            idx += 1;
            if vm.ary(e).is_some() && (level < 0 || depth <= level) {
                modified = true;
                let inner = items(vm, e);
                stack.push((ary, idx, depth));
                stack.push((inner, 0, depth + 1));
                break;
            } else {
                result.push(e);
            }
        }
    }
    (result, modified)
}

fn product_fetch(vm: &mut Vm, s: Value, arys: &[Value], mut n: i64) -> VmResult<Value> {
    let mut group = vec![Value::Nil; arys.len() + 1];
    let mut j = arys.len();
    while j > 0 {
        j -= 1;
        let a = match vm.ary_vals(arys[j]) { Some(a) => a, None => return Err(vm.raise_type("wrong argument type (expected Array)")) };
        let b = a.len() as i64;
        if b <= 0 { return Err(vm.raise_arg("cannot compute product with an empty array")); }
        group[j + 1] = a[(n % b) as usize];
        n /= b;
    }
    let me = items(vm, s);
    if n >= me.len() as i64 { return Err(vm.raise(vm.core.index_error, "index out of range")); }
    group[0] = me[n as usize];
    Ok(vm.ary_new(group))
}

const COMB_FINISHED: i64 = 0;
const COMB_REPEATED_PERMUTATION: i64 = 1;
const COMB_REPEATED_COMBINATION: i64 = 2;
const COMB_PERMUTATION: i64 = 3;
const COMB_COMBINATION: i64 = 4;

fn state_ints(vm: &Vm, st: Value) -> Vec<i64> { items(vm, st).iter().map(|v| match v { Value::Int(i) => *i, _ => 0 }).collect() }
fn state_store(vm: &mut Vm, st: Value, v: Vec<i64>) -> VmResult<()> { let list: Vec<Value> = v.into_iter().map(Value::Int).collect(); set_items(vm, st, list) }

fn adjust_next_permutation_index(ind: &mut [i64], i: usize) {
    let mut j = i as i64 - 1;
    while j >= 0 {
        if ind[i] == ind[j as usize] { ind[i] += 1; j = i as i64; }
        j -= 1;
    }
}

pub fn init(vm: &mut Vm) {
    let ary = vm.core.array;
    vm.define_methods(ary, &[
        ("assoc", |vm, s, a, _b| { argc!(vm, a, 1); for v in items(vm, s) { if let Some(x) = check_array(vm, v)? { if !x.is_empty() && vm.equal(x[0], a[0])? { return Ok(v); } } } Ok(Value::Nil) }),
        ("rassoc", |vm, s, a, _b| { argc!(vm, a, 1); for v in items(vm, s) { if let Some(x) = vm.ary_vals(v) { if x.len() > 1 && vm.equal(x[1], a[0])? { return Ok(v); } } } Ok(Value::Nil) }),
        ("at", |vm, s, a, _b| { argc!(vm, a, 1); let i = vm.expect_int(a[0], "index")?; let list = items(vm, s); let i = if i < 0 { i + list.len() as i64 } else { i }; Ok(if i < 0 { Value::Nil } else { list.get(i as usize).copied().unwrap_or(Value::Nil) }) }),
        ("values_at", |vm, s, a, _b| {
            // mrb_get_values_at: Integers and Ranges (a Range past the end pads with nil)
            let list = items(vm, s);
            let olen = list.len() as i64;
            let mut out = vec![];
            for v in a {
                if is_range(vm, *v) {
                    if let Some((beg, len)) = range_beg_len(vm, *v, olen, false)? {
                        let mut j = beg;
                        while j < beg + len { out.push(if j < olen { list[j as usize] } else { Value::Nil }); j += 1; }
                    }
                } else {
                    let i = vm.expect_int(*v, "index")?;
                    let i = if i < 0 { i + olen } else { i };
                    out.push(if i < 0 { Value::Nil } else { list.get(i as usize).copied().unwrap_or(Value::Nil) });
                }
            }
            Ok(vm.ary_new(out))
        }),
        ("slice!", |vm, s, a, _b| {
            argc!(vm, a, 1, 2);
            check_frozen(vm, s)?;
            let list = items(vm, s);
            let alen = list.len() as i64;
            let (mut i, mut len);
            if a.len() == 1 {
                if is_range(vm, a[0]) {
                    match range_beg_len(vm, a[0], alen, true)? { Some((b, l)) => { i = b; len = l; } None => return Ok(Value::Nil) }
                } else {
                    // mrb_ary_delete_at
                    let idx = vm.expect_int(a[0], "index")?;
                    let idx = if idx < 0 { idx + alen } else { idx };
                    if idx < 0 || idx >= alen { return Ok(Value::Nil); }
                    let mut l = list; let v = l.remove(idx as usize); set_items(vm, s, l)?; return Ok(v);
                }
            } else {
                i = vm.expect_int(a[0], "start")?; len = vm.expect_int(a[1], "length")?;
            }
            if i < 0 { i += alen; }
            if i < 0 || alen < i { return Ok(Value::Nil); }
            if len < 0 { return Ok(Value::Nil); }
            if alen == i { return Ok(vm.ary_new(vec![])); }
            if len > alen - i { len = alen - i; }
            let mut l = list;
            let cut: Vec<Value> = l.drain(i as usize..(i + len) as usize).collect();
            set_items(vm, s, l)?;
            Ok(vm.ary_new(cut))
        }),
        ("compact!", |vm, s, _a, _b| { check_frozen(vm, s)?; let list = items(vm, s); let n = list.len(); let kept: Vec<Value> = list.into_iter().filter(|v| !v.is_nil()).collect(); if kept.len() == n { return Ok(Value::Nil); } set_items(vm, s, kept)?; Ok(s) }),
        ("compact", |vm, s, _a, _b| { let kept: Vec<Value> = items(vm, s).into_iter().filter(|v| !v.is_nil()).collect(); Ok(vm.ary_new(kept)) }),
        ("rotate", |vm, s, a, _b| { argc!(vm, a, 0, 1); let count = if a.is_empty() { 1 } else { vm.expect_int(a[0], "count")? }; let list = items(vm, s); let len = list.len() as i64; if len <= 0 { return Ok(vm.ary_new(vec![])); } let mut idx = if count < 0 { len - ((!count) % len) - 1 } else { count % len }; let mut out = Vec::with_capacity(list.len()); for _ in 0..len { out.push(list[idx as usize]); idx += 1; if idx == len { idx = 0; } } Ok(vm.ary_new(out)) }),
        ("rotate!", |vm, s, a, _b| { argc!(vm, a, 0, 1); let count = if a.is_empty() { 1 } else { vm.expect_int(a[0], "count")? }; check_frozen(vm, s)?; let list = items(vm, s); let len = list.len() as i64; if len == 0 || count == 0 { return Ok(s); } let idx = if count < 0 { len - ((!count) % len) - 1 } else { count % len }; let mut v = list; v.rotate_left(idx as usize); set_items(vm, s, v)?; Ok(s) }),
        ("-", |vm, s, a, _b| { argc!(vm, a, 1); let other = ary_arg(vm, a[0])?; let mut out = vec![]; for v in items(vm, s) { if !memb_in(vm, &other, v)? { out.push(v); } } Ok(vm.ary_new(out)) }),
        ("difference", |vm, s, a, _b| { if a.is_empty() { let l = items(vm, s); return Ok(vm.ary_new(l)); } let arys = array_args(vm, a)?; let mut out = vec![]; for v in items(vm, s) { if !memb_has(vm, &arys, v)? { out.push(v); } } Ok(vm.ary_new(out)) }),
        ("|", |vm, s, a, _b| { argc!(vm, a, 1); let other = ary_arg(vm, a[0])?; let mut out = vec![]; let me = items(vm, s); union_add(vm, &me, &mut out)?; union_add(vm, &other, &mut out)?; Ok(vm.ary_new(out)) }),
        ("union", |vm, s, a, _b| { let arys = array_args(vm, a)?; let mut out = vec![]; let me = items(vm, s); union_add(vm, &me, &mut out)?; for x in &arys { union_add(vm, x, &mut out)?; } Ok(vm.ary_new(out)) }),
        ("&", |vm, s, a, _b| { argc!(vm, a, 1); let other = ary_arg(vm, a[0])?; let out = intersection(vm, s, &[other])?; Ok(vm.ary_new(out)) }),
        ("intersection", |vm, s, a, _b| { if a.is_empty() { let l = items(vm, s); return Ok(vm.ary_new(l)); } let arys = array_args(vm, a)?; let out = intersection(vm, s, &arys)?; Ok(vm.ary_new(out)) }),
        ("intersect?", |vm, s, a, _b| { argc!(vm, a, 1); let other = ary_arg(vm, a[0])?; let me = items(vm, s); if me.is_empty() || other.is_empty() { return Ok(Value::False); } let (shorter, longer) = if me.len() > other.len() { (other, me) } else { (me, other) }; for v in &longer { if memb_in(vm, &shorter, *v)? { return Ok(Value::True); } } Ok(Value::False) }),
        ("__fill_parse_arg", |vm, s, a, b| {
            argc!(vm, a, 0, 3);
            let ary_len = ary_len(vm, s) as i64;
            let arg = |i: usize| a.get(i).copied().unwrap_or(Value::Nil);
            let argc = a.len();
            let (mut start, mut length) = (0i64, 0i64);
            if !b.is_nil() {
                if argc == 0 || arg(0).is_nil() { start = 0; length = ary_len; }
                else if is_range(vm, arg(0)) { if let Some((bg, l)) = range_beg_len(vm, arg(0), ary_len, true)? { start = bg; length = l; } }
                else { start = vm.expect_int(arg(0), "start")?; if start < 0 { start += ary_len; } if start < 0 { start = 0; } if argc == 1 || arg(1).is_nil() { length = ary_len - start; } else { length = vm.expect_int(arg(1), "length")?; if length < 0 { length = 0; } } }
            } else if argc >= 1 && !arg(0).is_nil() {
                if argc == 1 || (arg(1).is_nil() && arg(2).is_nil()) { start = 0; length = ary_len; }
                else if is_range(vm, arg(1)) { if let Some((bg, l)) = range_beg_len(vm, arg(1), ary_len, true)? { start = bg; length = l; } }
                else if !arg(1).is_nil() { start = vm.expect_int(arg(1), "start")?; if start < 0 { start += ary_len; } if start < 0 { start = 0; } if argc == 2 || arg(2).is_nil() { length = ary_len - start; } else { length = vm.expect_int(arg(2), "length")?; if length < 0 { length = 0; } } }
            }
            Ok(vm.ary_new(vec![Value::Int(start), Value::Int(length)]))
        }),
        ("__fill_exec", |vm, s, a, _b| {
            argc!(vm, a, 3);
            let start = vm.expect_int(a[0], "start")?; let length = vm.expect_int(a[1], "length")?; let obj = a[2];
            if start < 0 { return Err(vm.raise_arg("negative start index")); }
            if length < 0 { return Err(vm.raise_arg("negative length")); }
            let mut list = items(vm, s);
            let end = match start.checked_add(length) { Some(e) if (e as u64) <= ARY_MAX_SIZE => e, _ => return Err(vm.raise_arg("array size too big")) };
            if end as usize > list.len() { check_frozen(vm, s)?; list.resize(end as usize, Value::Nil); }
            if start as usize >= list.len() || length <= 0 { check_frozen(vm, s)?; if end as usize > ary_len(vm, s) { set_items(vm, s, list)?; } return Ok(s); }
            for i in start..end { list[i as usize] = obj; }
            set_items(vm, s, list)?;
            Ok(s)
        }),
        ("__uniq!", |vm, s, _a, _b| uniq_bang(vm, s)),
        ("__uniq", |vm, s, _a, _b| { let l = items(vm, s); let d = vm.ary_new(l); uniq_bang(vm, d)?; Ok(d) }),
        ("flatten", |vm, s, a, _b| { argc!(vm, a, 0, 1); let level = if a.is_empty() { -1 } else { vm.expect_int(a[0], "level")? }; let (r, _) = flatten_internal(vm, s, level); Ok(vm.ary_new(r)) }),
        ("flatten!", |vm, s, a, _b| { argc!(vm, a, 0, 1); let level = if a.is_empty() { -1 } else { vm.expect_int(a[0], "level")? }; check_frozen(vm, s)?; let (r, modified) = flatten_internal(vm, s, level); if !modified { return Ok(Value::Nil); } set_items(vm, s, r)?; Ok(s) }),
        ("__normalize_index", |vm, s, a, _b| { argc!(vm, a, 1); let i = vm.expect_int(a[0], "index")?; let len = ary_len(vm, s) as i64; let i = if i < 0 { i + len } else { i }; Ok(if i >= 0 && i < len { Value::Int(i) } else { Value::Nil }) }),
        ("__fetch", |vm, s, a, _b| { argc!(vm, a, 3); let orig = vm.expect_int(a[0], "index")?; let list = items(vm, s); let len = list.len() as i64; let i = if orig < 0 { orig + len } else { orig }; if i < 0 || i >= len { if a[1] == a[2] { return Err(vm.raise(vm.core.index_error, &format!("index {orig} outside of array bounds: {}...{len}", -len))); } return Ok(a[1]); } Ok(list[i as usize]) }),
        ("insert", |vm, s, a, _b| {
            if a.is_empty() { return Err(vm.argnum_error(0, "1+")); }
            let idx = vm.expect_int(a[0], "index")?;
            let vals = &a[1..];
            if vals.is_empty() { check_frozen(vm, s)?; return Ok(s); }
            let mut list = items(vm, s);
            let len = list.len() as i64;
            let idx = if idx < 0 { let j = idx + len + 1; if j < 0 { return Err(vm.raise(vm.core.index_error, &format!("index {} outside of array bounds", j - (len + 1)))); } j } else { idx };
            check_frozen(vm, s)?;
            if idx.max(len) as u64 + vals.len() as u64 > ARY_MAX_SIZE { return Err(vm.raise_arg("array size too big")); }
            if idx as usize > list.len() { list.resize(idx as usize, Value::Nil); }
            for (k, v) in vals.iter().enumerate() { list.insert(idx as usize + k, *v); }
            set_items(vm, s, list)?;
            Ok(s)
        }),
        ("deconstruct", |_vm, s, _a, _b| Ok(s)),
        ("__product_generate", |vm, s, a, b| {
            argc!(vm, a, 1);
            let arys = ary_arg(vm, a[0])?;
            let mut total = ary_len(vm, s) as i64;
            for x in &arys { let n = match vm.ary_vals(*x) { Some(v) => v.len() as i64, None => return Err(vm.raise_type("wrong argument type (expected Array)")) }; if n == 0 { total = 0; break; } total = match total.checked_mul(n) { Some(t) => t, None => return Err(vm.raise_arg("result too big")) }; }
            if b.is_nil() {
                let mut result = Vec::new();
                for i in 0..total { let g = product_fetch(vm, s, &arys, i)?; result.push(g); }
                return Ok(vm.ary_new(result));
            }
            if total > 0 { Ok(vm.ary_new(vec![Value::Int(total), Value::Int(0)])) } else { Ok(Value::Nil) }
        }),
        ("__product_next", |vm, s, a, _b| { argc!(vm, a, 2); let arys = ary_arg(vm, a[0])?; let st = state_ints(vm, a[1]); if st.len() != 2 { return Err(vm.raise_type("wrong argument type (expected ary_product_generator)")); } let (total, cursor) = (st[0], st[1]); if cursor >= total { return Ok(Value::Nil); } state_store(vm, a[1], vec![total, cursor + 1])?; product_fetch(vm, s, &arys, cursor) }),
        ("__combination_init", |vm, s, a, _b| {
            argc!(vm, a, 2);
            let mode_sym = match a[0] { Value::Sym(m) => vm.sym_name(m), _ => return Err(vm.raise_type("not a symbol")) };
            let k = vm.expect_int(a[1], "k")?;
            let n = ary_len(vm, s) as i64;
            if k < 1 || n < 1 { return Ok(Value::Nil); }
            let mode = match mode_sym.as_str() {
                "repeated_permutation" => COMB_REPEATED_PERMUTATION,
                "repeated_combination" => COMB_REPEATED_COMBINATION,
                "permutation" => { if k > n { return Ok(Value::Nil); } COMB_PERMUTATION }
                "combination" => { if k > n { return Ok(Value::Nil); } COMB_COMBINATION }
                _ => return Err(vm.raise_arg("wrong mode")),
            };
            let mut st = vec![mode, n, k];
            for i in 0..k { st.push(if mode == COMB_PERMUTATION || mode == COMB_COMBINATION { i } else { 0 }); }
            let list: Vec<Value> = st.into_iter().map(Value::Int).collect();
            Ok(vm.ary_new(list))
        }),
        ("__combination_next", |vm, s, a, _b| {
            argc!(vm, a, 1);
            let st = state_ints(vm, a[0]);
            if st.len() < 3 { return Err(vm.raise_type("wrong argument type (expected CombinationState)")); }
            let (mode, n, k) = (st[0], st[1], st[2] as usize);
            if mode == COMB_FINISHED { return Ok(Value::Nil); }
            let list = items(vm, s);
            if list.len() as i64 != n { return Err(vm.raise(vm.core.runtime_error, "array modified during iteration")); }
            let mut ind: Vec<i64> = st[3..].to_vec();
            for i in 0..k { if ind[i] >= n { state_store(vm, a[0], vec![COMB_FINISHED, n, k as i64])?; return Ok(Value::Nil); } }
            let cur: Vec<Value> = ind.iter().map(|&i| list[i as usize]).collect();
            let result = vm.ary_new(cur);
            let mut advanced = false;
            match mode {
                COMB_REPEATED_PERMUTATION | COMB_REPEATED_COMBINATION => {
                    let mut i = k as i64 - 1;
                    while i >= 0 {
                        let iu = i as usize;
                        ind[iu] += 1;
                        if ind[iu] < n { let reset = if mode == COMB_REPEATED_PERMUTATION { 0 } else { ind[iu] }; for j in iu + 1..k { ind[j] = reset; } advanced = true; break; }
                        i -= 1;
                    }
                }
                COMB_PERMUTATION => {
                    let mut i = k as i64 - 1;
                    while i >= 0 {
                        let iu = i as usize;
                        ind[iu] += 1;
                        adjust_next_permutation_index(&mut ind, iu);
                        if ind[iu] < n { for j in iu + 1..k { ind[j] = 0; adjust_next_permutation_index(&mut ind, j); } advanced = true; break; }
                        i -= 1;
                    }
                }
                COMB_COMBINATION => {
                    let mut i = k as i64 - 1;
                    while i >= 0 {
                        let iu = i as usize;
                        ind[iu] += 1;
                        if ind[iu] <= n - k as i64 + i { for j in iu + 1..k { ind[j] = ind[j - 1] + 1; } advanced = true; break; }
                        i -= 1;
                    }
                }
                _ => {}
            }
            if advanced { let mut ns = vec![mode, n, k as i64]; ns.extend(ind); state_store(vm, a[0], ns)?; } else { state_store(vm, a[0], vec![COMB_FINISHED, n, k as i64])?; }
            Ok(result)
        }),
        ("__max", |vm, s, _a, _b| { let list = items(vm, s); if list.is_empty() { return Ok(Value::Nil); } let mut r = list[0]; for v in &list[1..] { if cmp_ordered(vm, *v, r)? == 1 { r = *v; } } Ok(r) }),
        ("__min", |vm, s, _a, _b| { let list = items(vm, s); if list.is_empty() { return Ok(Value::Nil); } let mut r = list[0]; for v in &list[1..] { if cmp_ordered(vm, *v, r)? == -1 { r = *v; } } Ok(r) }),
        ("include?", |vm, s, a, _b| { argc!(vm, a, 1); let mut i = 0; loop { let list = items(vm, s); if i >= list.len() { break; } if vm.equal(list[i], a[0])? { return Ok(Value::True); } i += 1; } Ok(Value::False) }),
        ("member?", |vm, s, a, _b| { argc!(vm, a, 1); let mut i = 0; loop { let list = items(vm, s); if i >= list.len() { break; } if vm.equal(list[i], a[0])? { return Ok(Value::True); } i += 1; } Ok(Value::False) }),
        // mruby-enum-ext
        ("__minmax", |vm, s, _a, _b| { let list = items(vm, s); if list.is_empty() { return Ok(vm.ary_new(vec![Value::Nil, Value::Nil])); } let (mut min, mut max) = (list[0], list[0]); for v in &list[1..] { if cmp_ordered(vm, *v, max)? > 0 { max = *v; } if cmp_ordered(vm, *v, min)? < 0 { min = *v; } } Ok(vm.ary_new(vec![min, max])) }),
        ("__count", |vm, s, a, _b| { argc!(vm, a, 1); let mut n = 0; let mut i = 0; loop { let list = items(vm, s); if i >= list.len() { break; } if vm.equal(list[i], a[0])? { n += 1; } i += 1; } Ok(Value::Int(n)) }),
    ]);
}

/// `ary_intersection_body`: narrow the receiver by each argument in turn, keeping one of each element.
fn intersection(vm: &mut Vm, s: Value, arys: &[Vec<Value>]) -> VmResult<Vec<Value>> {
    let mut result: Vec<Value> = vec![];
    for (j, arg) in arys.iter().enumerate() {
        let src = if j == 0 { items(vm, s) } else { result.clone() };
        let mut taken: Vec<Value> = vec![];
        let mut next: Vec<Value> = vec![];
        for v in src {
            // take(): the element must be in the argument and not taken before
            if !memb_in(vm, arg, v)? { continue; }
            if j == 0 && memb_in(vm, &taken, v)? { continue; }
            taken.push(v);
            next.push(v);
        }
        result = next;
        if result.is_empty() { break; }
    }
    Ok(result)
}
