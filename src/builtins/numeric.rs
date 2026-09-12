//! Integer and Float.

use alloc::{format, string::String, string::ToString, vec};

use crate::argc;
use crate::error::VmResult;
use crate::value::{Slot, Value};
use crate::vm::Vm;

/// Ruby's floor division.
pub fn div_floor(p: i64, q: i64) -> i64 {
    let d = p.wrapping_div(q);
    if (p % q != 0) && ((p < 0) != (q < 0)) { d - 1 } else { d }
}
pub fn mod_floor(p: i64, q: i64) -> i64 {
    let m = p.wrapping_rem(q);
    if m != 0 && ((m < 0) != (q < 0)) { m + q } else { m }
}

/// Float formatting close to mruby's `flo_to_s` (`%.16g` with a forced `.0`).
pub fn float_to_s(f: f64) -> String {
    if f.is_nan() { return "NaN".into(); }
    if f.is_infinite() { return if f > 0.0 { "Infinity".into() } else { "-Infinity".into() }; }
    if f == 0.0 { return if f.is_sign_negative() { "-0.0".into() } else { "0.0".into() }; }
    let abs = f.abs();
    if !(1e-4..1e15).contains(&abs) {
        // exponent form: d.ddde+XX
        let s = format!("{:e}", f);
        let (mant, exp) = s.split_once('e').unwrap();
        let mant = if mant.contains('.') { mant.to_string() } else { format!("{mant}.0") };
        let exp: i32 = exp.parse().unwrap();
        return format!("{mant}e{}{:02}", if exp < 0 { '-' } else { '+' }, exp.abs());
    }
    let s = format!("{}", f);
    if s.contains('.') { s } else { format!("{s}.0") }
}

pub(crate) fn num_args(vm: &mut Vm, s: Value, a: &[Value]) -> VmResult<(Value, Value)> {
    argc!(vm, a, 1);
    Ok((s, a[0]))
}

/// mruby coerces a non-numeric operand with `mrb_ensure_float_type`, hence the message.
pub(crate) fn coerce_fail(vm: &mut Vm, other: Value, _op: &str) -> crate::error::VmError {
    let d = vm.describe_for_type_error(other);
    vm.raise_type(&format!("can't convert {d} into Float"))
}

fn int_binop(vm: &mut Vm, s: Value, a: &[Value], name: &str, fi: fn(i64, i64) -> Option<i64>, ff: fn(f64, f64) -> f64) -> VmResult<Value> {
    let (x, y) = num_args(vm, s, a)?;
    match (x, y) {
        (Value::Int(p), Value::Int(q)) => match fi(p, q) { Some(r) => Ok(Value::Int(r)), None => Err(vm.raise(vm.core.range_error, "integer overflow")) },
        (Value::Int(p), Value::Float(q)) => Ok(Value::Float(ff(p as f64, q))),
        _ => { let e = coerce_fail(vm, y, name); Err(e) }
    }
}
fn float_binop(vm: &mut Vm, s: Value, a: &[Value], ff: fn(f64, f64) -> f64) -> VmResult<Value> {
    let (x, y) = num_args(vm, s, a)?;
    let p = match x { Value::Float(p) => p, _ => 0.0 };
    match y {
        Value::Int(q) => Ok(Value::Float(ff(p, q as f64))),
        Value::Float(q) => Ok(Value::Float(ff(p, q))),
        _ => { let d = vm.describe_for_error(y); Err(vm.raise_type(&format!("{d} can't be coerced into Float"))) }
    }
}

pub(crate) fn as_f64(v: Value) -> Option<f64> {
    match v { Value::Int(i) => Some(i as f64), Value::Float(f) => Some(f), _ => None }
}

/// Exact Integer/Float comparison (numeric.c `mrb_int_float_cmp`): `None` for a NaN.
pub fn int_float_cmp(x: i64, y: f64) -> Option<core::cmp::Ordering> {
    use core::cmp::Ordering::*;
    if y.is_nan() { return None; }
    if y < -9223372036854775808.0 { return Some(Greater); }
    if y >= 9223372036854775808.0 { return Some(Less); }
    let yi = y as i64;
    if x > yi { return Some(Greater); }
    if x < yi { return Some(Less); }
    if y > yi as f64 { return Some(Less); }
    if y < yi as f64 { return Some(Greater); }
    Some(Equal)
}

/// `Ok(Some)` ordered, `Ok(None)` numeric but unordered (NaN), `Err` not comparable.
fn cmp(vm: &mut Vm, s: Value, a: &[Value]) -> VmResult<Result<Option<core::cmp::Ordering>, ()>> {
    argc!(vm, a, 1);
    Ok(match (s, a[0]) {
        (Value::Int(p), Value::Int(q)) => Ok(Some(p.cmp(&q))),
        (Value::Int(p), Value::Float(q)) => Ok(int_float_cmp(p, q)),
        (Value::Float(p), Value::Int(q)) => Ok(int_float_cmp(q, p).map(|o| o.reverse())),
        (Value::Float(p), Value::Float(q)) => Ok(p.partial_cmp(&q)),
        _ => Err(()),
    })
}
/// For `<` and friends: unordered numbers answer false, non-numbers raise.
fn cmp_or_fail(vm: &mut Vm, s: Value, a: &[Value]) -> VmResult<Option<core::cmp::Ordering>> {
    match cmp(vm, s, a)? {
        Ok(o) => Ok(o),
        Err(()) => { let x = vm.describe_for_error(s); let y = vm.describe_for_error(a[0]); Err(vm.raise_arg(&format!("comparison of {x} with {y} failed"))) }
    }
}
fn ord_test(o: Option<core::cmp::Ordering>, f: fn(core::cmp::Ordering) -> bool) -> Value { Value::bool(o.map(f).unwrap_or(false)) }
fn unordered(vm: &mut Vm, s: Value, other: Value) -> crate::error::VmError { let x = vm.describe_for_error(s); let y = vm.describe_for_error(other); vm.raise_arg(&format!("comparison of {x} with {y} failed")) }

pub(crate) fn int_pow(vm: &mut Vm, s: Value, a: &[Value]) -> VmResult<Value> {
    let (x, y) = num_args(vm, s, a)?;
    match (x, y) {
        (Value::Int(p), Value::Int(q)) => {
            if q < 0 { return Ok(Value::Float(libm::pow(p as f64, q as f64))); }
            match p.checked_pow(q as u32) { Some(r) => Ok(Value::Int(r)), None => Err(vm.raise(vm.core.range_error, "integer overflow")) }
        }
        (Value::Int(p), Value::Float(q)) => Ok(Value::Float(libm::pow(p as f64, q))),
        _ => Err(coerce_fail(vm, y, "**")),
    }
}

fn int_to_s(vm: &mut Vm, s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    argc!(vm, a, 0, 1);
    let i = match s { Value::Int(i) => i, _ => 0 };
    let base = if a.len() == 1 { vm.expect_int(a[0], "base")? } else { 10 };
    if !(2..=36).contains(&base) { return Err(vm.raise_arg(&format!("invalid radix {base}"))); }
    let out = if base == 10 { i.to_string() } else { to_radix(i, base as u32) };
    Ok(vm.str_from(out))
}
fn to_radix(i: i64, base: u32) -> String {
    if i == 0 { return "0".into(); }
    let neg = i < 0;
    let mut n = (i as i128).unsigned_abs();
    let mut digits = vec![];
    while n > 0 { digits.push(core::char::from_digit((n % base as u128) as u32, base).unwrap()); n /= base as u128; }
    if neg { digits.push('-'); }
    digits.iter().rev().collect()
}

pub fn init(vm: &mut Vm) {
    let c = vm.core;
    // EPSILON, MAX, MIN, DIG, ... are mruby-numeric-ext (`ext_numeric.rs`)
    for (name, v) in [("INFINITY", f64::INFINITY), ("NAN", f64::NAN)] {
        let n = vm.intern(name);
        vm.heap.class_mut(c.float).consts.insert(n, Slot::from(Value::Float(v)));
    }
    let isc = vm.singleton_class(Value::Obj(c.integer)).unwrap();
    vm.define_method(isc, "__ensure", |vm, _s, a, _b| { argc!(vm, a, 1); match a[0] { Value::Int(_) => Ok(a[0]), Value::Float(f) if f.is_finite() => Ok(Value::Int(libm::trunc(f) as i64)), v => { let d = vm.describe_for_type_error(v); Err(vm.raise_type(&format!("can't convert {d} into Integer"))) } } });
    vm.define_methods(c.numeric, &[
        ("+@", |_vm, s, _a, _b| Ok(s)),
        ("<=>", |vm, s, a, _b| Ok(match cmp(vm, s, a)? { Ok(Some(o)) => Value::Int(o as i64), _ => Value::Nil })),
        ("<", |vm, s, a, _b| { let o = cmp_or_fail(vm, s, a)?; Ok(ord_test(o, |o| o.is_lt())) }),
        ("<=", |vm, s, a, _b| { let o = cmp_or_fail(vm, s, a)?; Ok(ord_test(o, |o| o.is_le())) }),
        (">", |vm, s, a, _b| { let o = cmp_or_fail(vm, s, a)?; Ok(ord_test(o, |o| o.is_gt())) }),
        (">=", |vm, s, a, _b| { let o = cmp_or_fail(vm, s, a)?; Ok(ord_test(o, |o| o.is_ge())) }),
        ("==", |vm, s, a, _b| Ok(Value::bool(cmp(vm, s, a)? == Ok(Some(core::cmp::Ordering::Equal))))),
        ("between?", |vm, s, a, _b| { argc!(vm, a, 2); match cmp_or_fail(vm, s, &a[..1])? { Some(l) if l.is_lt() => return Ok(Value::False), Some(_) => {} None => return Err(unordered(vm, s, a[0])) } match cmp_or_fail(vm, s, &a[1..])? { Some(h) => Ok(Value::bool(h.is_le())), None => Err(unordered(vm, s, a[1])) } }),
        ("abs", |_vm, s, _a, _b| Ok(match s { Value::Int(i) => Value::Int(i.wrapping_abs()), Value::Float(f) => Value::Float(f.abs()), v => v })),
        ("to_int", |_vm, s, _a, _b| Ok(match s { Value::Float(f) => Value::Int(f as i64), v => v })),
        ("nan?", |_vm, s, _a, _b| Ok(Value::bool(matches!(s, Value::Float(f) if f.is_nan())))),
        ("infinite?", |_vm, s, _a, _b| Ok(match s { Value::Float(f) if f.is_infinite() => Value::Int(if f > 0.0 { 1 } else { -1 }), _ => Value::Nil })),
        ("finite?", |_vm, s, _a, _b| Ok(Value::bool(!matches!(s, Value::Float(f) if !f.is_finite())))),
        ("step", |vm, _s, _a, _b| Err(vm.raise(vm.core.not_implemented_error, "Numeric#step is provided by mrblib"))),
    ]);
    vm.define_methods(c.integer, &[
        ("+", |vm, s, a, _b| int_binop(vm, s, a, "+", i64::checked_add, |p, q| p + q)),
        ("-", |vm, s, a, _b| int_binop(vm, s, a, "-", i64::checked_sub, |p, q| p - q)),
        ("*", |vm, s, a, _b| int_binop(vm, s, a, "*", i64::checked_mul, |p, q| p * q)),
        ("/", |vm, s, a, _b| { let (x, y) = num_args(vm, s, a)?; if let (Value::Int(p), Value::Int(q)) = (x, y) { if q == 0 { return Err(vm.raise(vm.core.zero_division_error, "divided by 0")); } return Ok(Value::Int(div_floor(p, q))); } int_binop(vm, s, a, "/", |_, _| None, |p, q| p / q) }),
        ("div", |vm, s, a, _b| { let (x, y) = num_args(vm, s, a)?; match (x, y) { (Value::Int(p), Value::Int(q)) => { if q == 0 { return Err(vm.raise(vm.core.zero_division_error, "divided by 0")); } Ok(Value::Int(div_floor(p, q))) } (Value::Int(p), Value::Float(q)) => Ok(Value::Int(libm::floor(p as f64 / q) as i64)), _ => Err(coerce_fail(vm, y, "div")) } }),
        ("%", int_mod),
        ("**", |vm, s, a, _b| int_pow(vm, s, a)),
        ("pow", |vm, s, a, _b| int_pow(vm, s, a)),
        ("divmod", |vm, s, a, _b| { let (x, y) = num_args(vm, s, a)?; match (x, y) { (Value::Int(p), Value::Int(q)) => { if q == 0 { return Err(vm.raise(vm.core.zero_division_error, "divided by 0")); } Ok(vm.ary_new(vec![Value::Int(div_floor(p, q)), Value::Int(mod_floor(p, q))])) } (Value::Int(p), Value::Float(q)) => { let d = libm::floor(p as f64 / q); Ok(vm.ary_new(vec![Value::Float(d), Value::Float(p as f64 - d * q)])) } _ => Err(coerce_fail(vm, y, "divmod")) } }),
        ("-@", |vm, s, _a, _b| match s { Value::Int(i) => i.checked_neg().map(Value::Int).ok_or_else(|| vm.raise(vm.core.range_error, "integer overflow")), v => Ok(v) }),
        ("~", |_vm, s, _a, _b| Ok(match s { Value::Int(i) => Value::Int(!i), v => v })),
        ("&", |vm, s, a, _b| bit(vm, s, a, |p, q| p & q)),
        ("|", |vm, s, a, _b| bit(vm, s, a, |p, q| p | q)),
        ("^", |vm, s, a, _b| bit(vm, s, a, |p, q| p ^ q)),
        ("<<", |vm, s, a, _b| shift(vm, s, a, true)),
        (">>", |vm, s, a, _b| shift(vm, s, a, false)),
        ("==", |vm, s, a, _b| Ok(Value::bool(cmp(vm, s, a)? == Ok(Some(core::cmp::Ordering::Equal))))),
        ("eql?", |_vm, s, a, _b| Ok(Value::bool(a.first().map(|x| *x == s).unwrap_or(false)))),
        ("hash", |_vm, s, _a, _b| Ok(s)),
        ("__coerce_step_counter", |vm, s, a, _b| { argc!(vm, a, 1); Ok(match a[0] { Value::Float(_) => Value::Float(match s { Value::Int(i) => i as f64, _ => 0.0 }), _ => s }) }),
        ("to_s", int_to_s),
        ("inspect", int_to_s),
        ("to_i", |_vm, s, _a, _b| Ok(s)),
        ("to_int", |_vm, s, _a, _b| Ok(s)),
        ("to_f", |_vm, s, _a, _b| Ok(match s { Value::Int(i) => Value::Float(i as f64), v => v })),
        ("succ", |vm, s, _a, _b| int_binop(vm, s, &[Value::Int(1)], "+", i64::checked_add, |p, q| p + q)),
        ("pred", |vm, s, _a, _b| int_binop(vm, s, &[Value::Int(1)], "-", i64::checked_sub, |p, q| p - q)),
        ("chr", |vm, s, _a, _b| { let i = match s { Value::Int(i) => i, _ => 0 }; if !(0..=255).contains(&i) { return Err(vm.raise(vm.core.range_error, &format!("{i} out of char range"))); } Ok(vm.str_new(&[i as u8])) }),
        ("floor", |vm, s, a, _b| int_rounding(vm, s, a, Rounding::Floor)),
        ("ceil", |vm, s, a, _b| int_rounding(vm, s, a, Rounding::Ceil)),
        ("round", |vm, s, a, _b| int_rounding(vm, s, a, Rounding::Round)),
        ("truncate", |vm, s, a, _b| int_rounding(vm, s, a, Rounding::Truncate)),
        ("quo", |vm, s, a, _b| { argc!(vm, a, 1); let p = match s { Value::Int(i) => i as f64, _ => 0.0 }; match as_f64(a[0]) { Some(q) => Ok(Value::Float(p / q)), None => Err(coerce_fail(vm, a[0], "quo")) } }),
        ("fdiv", |vm, s, a, _b| { argc!(vm, a, 1); let p = match s { Value::Int(i) => i as f64, _ => 0.0 }; match as_f64(a[0]) { Some(q) => Ok(Value::Float(p / q)), None => Err(coerce_fail(vm, a[0], "fdiv")) } }),
        ("__num_to_a", |_vm, _s, _a, _b| Ok(Value::Nil)),
    ]);
    vm.define_methods(c.float, &[
        // `flo_idiv`: floor division to an Integer
        ("div", |vm, s, a, _b| { argc!(vm, a, 1); let x = match s { Value::Float(x) => x, _ => 0.0 }; if !x.is_finite() { return Err(vm.raise(vm.core.float_domain_error, &float_to_s(x))); } let y = vm.expect_int(a[0], "divisor")?; if !(-9223372036854775808.0..9223372036854775808.0).contains(&x) { return Err(vm.raise(vm.core.range_error, "integer overflow in div")); } if y == 0 { return Err(vm.raise(vm.core.zero_division_error, "divided by 0")); } Ok(Value::Int(div_floor(x as i64, y))) }),
        ("+", |vm, s, a, _b| float_binop(vm, s, a, |p, q| p + q)),
        ("-", |vm, s, a, _b| float_binop(vm, s, a, |p, q| p - q)),
        ("*", |vm, s, a, _b| float_binop(vm, s, a, |p, q| p * q)),
        ("/", |vm, s, a, _b| float_binop(vm, s, a, |p, q| p / q)),
        ("quo", |vm, s, a, _b| float_binop(vm, s, a, |p, q| p / q)),
        ("fdiv", |vm, s, a, _b| float_binop(vm, s, a, |p, q| p / q)),
        ("%", |vm, s, a, _b| float_binop(vm, s, a, |p, q| { let m = p % q; if m != 0.0 && ((m < 0.0) != (q < 0.0)) { m + q } else { m } })),
        ("**", |vm, s, a, _b| float_binop(vm, s, a, libm::pow)),
        ("-@", |_vm, s, _a, _b| Ok(match s { Value::Float(f) => Value::Float(-f), v => v })),
        ("==", |vm, s, a, _b| Ok(Value::bool(cmp(vm, s, a)? == Ok(Some(core::cmp::Ordering::Equal))))),
        ("eql?", |_vm, s, a, _b| Ok(Value::bool(a.first().map(|x| *x == s).unwrap_or(false)))),
        ("hash", |_vm, s, _a, _b| Ok(Value::Int(match s { Value::Float(f) => f.to_bits() as i64, _ => 0 }))),
        ("__coerce_step_counter", |_vm, s, _a, _b| Ok(s)),
        ("to_s", |vm, s, _a, _b| { let f = match s { Value::Float(f) => f, _ => 0.0 }; Ok(vm.str_from(float_to_s(f))) }),
        ("inspect", |vm, s, _a, _b| { let f = match s { Value::Float(f) => f, _ => 0.0 }; Ok(vm.str_from(float_to_s(f))) }),
        ("to_f", |_vm, s, _a, _b| Ok(s)),
        ("to_i", float_to_i),
        ("to_int", float_to_i),
        ("truncate", float_to_i),
        ("floor", |vm, s, _a, _b| float_round(vm, s, libm::floor)),
        ("ceil", |vm, s, _a, _b| float_round(vm, s, libm::ceil)),
        ("round", |vm, s, a, _b| { argc!(vm, a, 0, 1); let nd = if a.is_empty() { 0 } else { vm.expect_int(a[0], "digits")? }; flo_round(vm, s, nd) }),
        ("abs", |_vm, s, _a, _b| Ok(match s { Value::Float(f) => Value::Float(f.abs()), v => v })),
        ("divmod", |vm, s, a, _b| { argc!(vm, a, 1); let p = match s { Value::Float(f) => f, _ => 0.0 }; let q = match as_f64(a[0]) { Some(q) => q, None => return Err(coerce_fail(vm, a[0], "divmod")) }; let d = libm::floor(p / q); Ok(vm.ary_new(vec![Value::Float(d), Value::Float(p - d * q)])) }),
    ]);
}

/// numeric.c `flo_round`.
fn flo_round(vm: &mut Vm, s: Value, nd: i64) -> VmResult<Value> {
    let number = match s { Value::Float(f) => f, _ => return Ok(s) };
    if nd > 0 && !number.is_finite() { return Ok(s); }
    if !number.is_finite() { return Err(vm.raise(vm.core.float_domain_error, &float_to_s(number))); }
    const DBL_DIG: i64 = 15;
    if nd < -DBL_DIG - 2 { return Ok(Value::Int(0)); }
    if nd > DBL_DIG + 2 { return Ok(s); }
    let mut f = 1.0f64;
    for _ in 0..nd.abs() { f *= 10.0; }
    let mut x = number;
    if f.is_infinite() { if nd < 0 { x = 0.0; } }
    else {
        if nd < 0 { x /= f; } else { x *= f; }
        if x > 0.0 { let d = libm::floor(x); x = d + if x - d >= 0.5 { 1.0 } else { 0.0 }; }
        else if x < 0.0 { let d = libm::ceil(x); x = d - if d - x >= 0.5 { 1.0 } else { 0.0 }; }
        if nd < 0 { x *= f; } else { x /= f; }
    }
    if nd > 0 { return Ok(if x.is_finite() { Value::Float(x) } else { s }); }
    if x.abs() >= 4611686018427387904.0 { return Ok(Value::Float(x)); }
    Ok(Value::Int(x as i64))
}

#[derive(Clone, Copy, PartialEq)]
enum Rounding { Floor, Ceil, Round, Truncate }

/// `Integer#floor/ceil/round/truncate(ndigits)` for negative `ndigits` (numeric.c `prepare_int_rounding`).
fn int_rounding(vm: &mut Vm, s: Value, a: &[Value], mode: Rounding) -> VmResult<Value> {
    argc!(vm, a, 0, 1);
    let x = match s { Value::Int(i) => i, _ => return Ok(s) };
    let nd = if a.is_empty() { 0 } else { vm.expect_int(a[0], "ndigits")? };
    if nd >= 0 { return Ok(s); }
    if -0.415241 * nd as f64 > 8.0 - 0.125 { return Ok(Value::Int(0)); }
    let f = 10i64.pow((-nd) as u32);
    let c = x % f;
    if c == 0 { return Ok(s); }
    let base = x - c;
    let r = match mode {
        Rounding::Truncate => base,
        Rounding::Floor => if x < 0 { base - f } else { base },
        Rounding::Ceil => if x < 0 { base } else { base + f },
        Rounding::Round => {
            let half = f / 2;
            if c < 0 { if -c < half { base } else { base - f } } else if c < half { base } else { base + f }
        }
    };
    Ok(Value::Int(r))
}

fn float_to_i(vm: &mut Vm, s: Value, _a: &[Value], _b: Value) -> VmResult<Value> {
    match s {
        Value::Float(f) if f.is_finite() => Ok(Value::Int(libm::trunc(f) as i64)),
        Value::Float(f) => Err(vm.raise(vm.core.float_domain_error, &float_to_s(f))),
        v => Ok(v),
    }
}
fn float_round(vm: &mut Vm, s: Value, f: fn(f64) -> f64) -> VmResult<Value> {
    match s {
        Value::Float(x) if x.is_finite() => Ok(Value::Int(f(x) as i64)),
        Value::Float(x) => Err(vm.raise(vm.core.float_domain_error, &float_to_s(x))),
        v => Ok(v),
    }
}
fn int_mod(vm: &mut Vm, s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    let (x, y) = num_args(vm, s, a)?;
    match (x, y) {
        (Value::Int(p), Value::Int(q)) => { if q == 0 { return Err(vm.raise(vm.core.zero_division_error, "divided by 0")); } Ok(Value::Int(mod_floor(p, q))) }
        (Value::Int(p), Value::Float(q)) => { let p = p as f64; let m = p % q; Ok(Value::Float(if m != 0.0 && ((m < 0.0) != (q < 0.0)) { m + q } else { m })) }
        _ => Err(coerce_fail(vm, y, "%")),
    }
}
fn bit(vm: &mut Vm, s: Value, a: &[Value], f: fn(i64, i64) -> i64) -> VmResult<Value> {
    let (x, y) = num_args(vm, s, a)?;
    match (x, y) { (Value::Int(p), Value::Int(q)) => Ok(Value::Int(f(p, q))), _ => Err(coerce_fail(vm, y, "bitop")) }
}
fn shift(vm: &mut Vm, s: Value, a: &[Value], left: bool) -> VmResult<Value> {
    let (x, y) = num_args(vm, s, a)?;
    match (x, y) {
        (Value::Int(p), Value::Int(q)) => {
            let (p, left, q) = if q < 0 { (p, !left, -q) } else { (p, left, q) };
            if left {
                if q >= 64 { if p == 0 { return Ok(Value::Int(0)); } return Err(vm.raise(vm.core.range_error, "integer overflow")); }
                match p.checked_shl(q as u32) { Some(r) if (r >> q) == p => Ok(Value::Int(r)), _ => Err(vm.raise(vm.core.range_error, "integer overflow")) }
            } else {
                Ok(Value::Int(if q >= 64 { if p < 0 { -1 } else { 0 } } else { p >> q }))
            }
        }
        _ => Err(coerce_fail(vm, y, "shift")),
    }
}
