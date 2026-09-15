//! Integer and Float.
//!
//! An Integer is a `Value::Int` while it fits in an `i64` and a heap
//! [`BigInt`](crate::bigint::BigInt) when it does not; every operation that can leave the
//! `i64` range promotes instead of raising, as mruby's `#ifdef MRB_USE_BIGINT` branches in
//! `src/numeric.c` do. A result that fits is always normalized back to `Value::Int`
//! (`Vm::bint_value`), so the two representations never hold the same number.

use alloc::{format, string::String, string::ToString, vec};

use crate::argc;
use crate::bigint::BigInt;
use crate::error::{VmError, VmResult};
use crate::value::{Slot, Value};
use crate::vm::Vm;

/// Ruby's floor division.
pub fn div_floor(p: i64, q: i64) -> i64 {
    let d = p.wrapping_div(q);
    if (p.wrapping_rem(q) != 0) && ((p < 0) != (q < 0)) { d - 1 } else { d }
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

/// `mrb_as_int`'s message, raised by the operations a wide integer takes part in.
fn not_int(vm: &mut Vm, other: Value) -> VmError {
    let d = vm.describe_for_type_error(other);
    vm.raise_type(&format!("{d} cannot be converted to Integer"))
}

fn zero_div(vm: &mut Vm) -> VmError {
    vm.raise(vm.core.zero_division_error, "divided by 0")
}

// ------------------------------------------------------------------ wide integers

/// The receiver of an Integer method as a [`BigInt`] (it is an Integer either way).
fn big(vm: &Vm, v: Value) -> BigInt {
    vm.as_bigint(v).unwrap_or_else(BigInt::zero)
}

/// `mrb_as_float`: an Integer of any width, or a Float.
pub(crate) fn num_f64(vm: &Vm, v: Value) -> Option<f64> {
    match v {
        Value::Int(i) => Some(i as f64),
        Value::Float(f) => Some(f),
        _ if super::ext_rational::is_rational(vm, v) => Some(super::ext_rational::to_f(vm, v)),
        Value::Obj(o) => vm.heap.bigint(o).map(|b| b.to_f64()),
        _ => None,
    }
}

// --------------------------------------------- Rational and Complex (the rest of the tower)

/// The arms `numeric.c` has for a Rational or a Complex operand: the pair is handed to the
/// gem that owns the wider type. `None` when the operand is neither.
fn zero_like(x: Value) -> Value {
    // `mrb_complex_new(a, 0)` builds the two-Float form, so a Float receiver keeps Float parts
    if matches!(x, Value::Float(_)) { Value::Float(0.0) } else { Value::Int(0) }
}

fn tower(vm: &mut Vm, x: Value, y: Value, op: char) -> VmResult<Option<Value>> {
    use super::{ext_complex as cpx, ext_rational as rat};
    if !matches!(y, Value::Obj(_)) { return Ok(None); }
    if rat::is_rational(vm, y) && !matches!(x, Value::Float(_)) {
        // a Float receiver keeps the Float arms: `1.0 + Rational(1, 2)` is 1.5
        return Ok(Some(match op {
            '+' => rat::add(vm, y, x)?,
            '*' => rat::mul(vm, y, x)?,
            '-' => { let r = rat::as_rational(vm, x)?; rat::sub(vm, r, y)? }
            _ => { let r = rat::as_rational(vm, x)?; rat::div(vm, r, y)? }
        }));
    }
    if cpx::is_complex(vm, y) {
        return Ok(Some(match op {
            '+' => cpx::arith(vm, y, x, '+')?,
            '*' => cpx::arith(vm, y, x, '*')?,
            '-' => { let c = cpx::new_complex(vm, x, zero_like(x))?; cpx::arith(vm, c, y, '-')? }
            _ => { let c = cpx::new_complex(vm, x, zero_like(x))?; cpx::div(vm, c, y)? }
        }));
    }
    Ok(None)
}

/// `mrb_bint_cmp` against a Float: the float is split at its integer part, which is exact,
/// so the fraction alone decides once the integer parts are equal.
fn bint_float_cmp(b: &BigInt, f: f64) -> Option<core::cmp::Ordering> {
    use core::cmp::Ordering::*;
    if f.is_nan() { return None; }
    if f.is_infinite() { return Some(if f < 0.0 { Greater } else { Less }); }
    let fi = libm::trunc(f);
    let c = b.cmp(&BigInt::from_f64(fi));
    if c != Equal { return Some(c); }
    Some(if f > fi { Less } else if f < fi { Greater } else { Equal })
}

/// The three operations that promote on overflow (`mrb_int_add`/`sub`/`mul`).
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum IntOp { Add, Sub, Mul }

fn int_arith(vm: &mut Vm, s: Value, a: &[Value], op: IntOp) -> VmResult<Value> {
    let (x, y) = num_args(vm, s, a)?;
    if let (Value::Int(p), Value::Int(q)) = (x, y) {
        let r = match op { IntOp::Add => p.checked_add(q), IntOp::Sub => p.checked_sub(q), IntOp::Mul => p.checked_mul(q) };
        if let Some(r) = r { return Ok(Value::Int(r)); }
        return Ok(bint_arith(vm, &BigInt::from_i64(p), &BigInt::from_i64(q), op));
    }
    if let Some(v) = tower(vm, x, y, match op { IntOp::Add => '+', IntOp::Sub => '-', IntOp::Mul => '*' })? { return Ok(v); }
    if let Value::Float(q) = y {
        let p = num_f64(vm, x).unwrap_or(0.0);
        return Ok(Value::Float(match op { IntOp::Add => p + q, IntOp::Sub => p - q, IntOp::Mul => p * q }));
    }
    match vm.as_bigint(y) {
        Some(q) => { let p = big(vm, x); Ok(bint_arith(vm, &p, &q, op)) }
        None if vm.is_bigint(x) => Err(not_int(vm, y)),
        None => Err(coerce_fail(vm, y, "+")),
    }
}

/// The promoted `+`/`-`/`*`, normalized back to a `Value::Int` when it fits.
pub(crate) fn bint_arith(vm: &mut Vm, p: &BigInt, q: &BigInt, op: IntOp) -> Value {
    let r = match op { IntOp::Add => p.add(q), IntOp::Sub => p.sub(q), IntOp::Mul => p.mul(q) };
    vm.bint_value(r)
}

/// `OP_ADD`/`OP_SUB`/`OP_MUL`'s overflow exit (`L_INT_OVERFLOW`): the VM's fast path keeps
/// its `checked_*` and lands here only when the result left the `i64` range.
pub fn int_overflow_op(vm: &mut Vm, p: i64, q: i64, mid: crate::symbol::Sym) -> Value {
    let s = vm.s;
    let op = if mid == s.minus { IntOp::Sub } else if mid == s.mul { IntOp::Mul } else { IntOp::Add };
    bint_arith(vm, &BigInt::from_i64(p), &BigInt::from_i64(q), op)
}

/// `Integer#/`: floor division, promoting `MRB_INT_MIN / -1`.
fn int_div(vm: &mut Vm, s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    let (x, y) = num_args(vm, s, a)?;
    if let (Value::Int(p), Value::Int(q)) = (x, y) {
        if q == 0 { return Err(zero_div(vm)); }
        if p == i64::MIN && q == -1 { return Ok(vm.bint_value(BigInt::from_i64(p).neg())); }
        return Ok(Value::Int(div_floor(p, q)));
    }
    if let Some(v) = tower(vm, x, y, '/')? { return Ok(v); }
    if let Value::Float(q) = y {
        let p = num_f64(vm, x).unwrap_or(0.0);
        return Ok(Value::Float(p / q));
    }
    bint_div_floor(vm, x, y)
}

fn bint_div_floor(vm: &mut Vm, x: Value, y: Value) -> VmResult<Value> {
    match vm.as_bigint(y) {
        Some(q) => {
            if q.is_zero() { return Err(zero_div(vm)); }
            let p = big(vm, x);
            Ok(vm.bint_value(p.div_floor(&q)))
        }
        None if vm.is_bigint(x) => Err(not_int(vm, y)),
        None => Err(coerce_fail(vm, y, "/")),
    }
}

fn int_idiv(vm: &mut Vm, s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    let (x, y) = num_args(vm, s, a)?;
    match (x, y) {
        (Value::Int(p), Value::Int(q)) => {
            if q == 0 { return Err(zero_div(vm)); }
            if p == i64::MIN && q == -1 { return Ok(vm.bint_value(BigInt::from_i64(p).neg())); }
            Ok(Value::Int(div_floor(p, q)))
        }
        // the reference's `mrb_bint_div` multiplies by a Float instead of dividing; the
        // floor division is taken here (`docs/design/gems.md`, `tests/custom/bigint_div_float`)
        _ if float_arm(vm, y) => {
            let (p, q) = (num_f64(vm, x).unwrap_or(0.0), num_f64(vm, y).unwrap_or(0.0));
            if q == 0.0 { return Err(zero_div(vm)); }
            let d = libm::floor(p / q);
            Ok(if (-9223372036854775808.0..9223372036854775808.0).contains(&d) { Value::Int(d as i64) } else { vm.bint_value(BigInt::from_f64(d)) })
        }
        _ => bint_div_floor(vm, x, y),
    }
}

/// True where `mrb_as_float` would take over: a Float, or a Rational, which converts
/// (`Integer#%`, `div` and `divmod` have no exact arm for one).
fn float_arm(vm: &Vm, y: Value) -> bool {
    matches!(y, Value::Float(_)) || super::ext_rational::is_rational(vm, y)
}

fn int_mod(vm: &mut Vm, s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    let (x, y) = num_args(vm, s, a)?;
    match (x, y) {
        (Value::Int(p), Value::Int(q)) => {
            if q == 0 { return Err(zero_div(vm)); }
            if p == i64::MIN && q == -1 { return Ok(Value::Int(0)); }
            Ok(Value::Int(mod_floor(p, q)))
        }
        // as for `div`, the floor remainder rather than the reference's `fmod`
        _ if float_arm(vm, y) => {
            let (p, q) = (num_f64(vm, x).unwrap_or(0.0), num_f64(vm, y).unwrap_or(0.0));
            let m = p % q;
            Ok(Value::Float(if m != 0.0 && ((m < 0.0) != (q < 0.0)) { m + q } else { m }))
        }
        _ => match vm.as_bigint(y) {
            Some(q) => {
                if q.is_zero() { return Err(zero_div(vm)); }
                let p = big(vm, x);
                Ok(vm.bint_value(p.mod_floor(&q)))
            }
            None if vm.is_bigint(x) => Err(not_int(vm, y)),
            None => Err(coerce_fail(vm, y, "%")),
        },
    }
}

fn int_divmod(vm: &mut Vm, s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    let (x, y) = num_args(vm, s, a)?;
    match (x, y) {
        (Value::Int(p), Value::Int(q)) => {
            if q == 0 { return Err(zero_div(vm)); }
            if p == i64::MIN && q == -1 { let d = vm.bint_value(BigInt::from_i64(p).neg()); return Ok(vm.ary_new(vec![d, Value::Int(0)])); }
            Ok(vm.ary_new(vec![Value::Int(div_floor(p, q)), Value::Int(mod_floor(p, q))]))
        }
        _ if float_arm(vm, y) => {
            let (p, q) = (num_f64(vm, x).unwrap_or(0.0), num_f64(vm, y).unwrap_or(0.0));
            let (d, m) = flodivmod(vm, p, q)?;
            let dv = if d.is_finite() { float_to_int(vm, d) } else { Value::Float(d) };
            Ok(vm.ary_new(vec![dv, Value::Float(m)]))
        }
        _ => match vm.as_bigint(y) {
            Some(q) => {
                if q.is_zero() { return Err(zero_div(vm)); }
                let p = big(vm, x);
                let (d, m) = p.divmod_floor(&q);
                let (d, m) = (vm.bint_value(d), vm.bint_value(m));
                Ok(vm.ary_new(vec![d, m]))
            }
            None if vm.is_bigint(x) => Err(not_int(vm, y)),
            None => Err(coerce_fail(vm, y, "divmod")),
        },
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
    let (x, y) = (s, a[0]);
    if matches!(x, Value::Obj(_)) && !vm.is_bigint(x) {
        // Numeric's `<`/`<=`/`>`/`>=` are Comparable's in the reference, which asks `<=>`;
        // a Rational or a Complex receiver reaches them through this
        let cmp = vm.intern("<=>");
        let r = vm.funcall(x, cmp, &[y], Value::Nil)?;
        return Ok(match r { Value::Int(i) => Ok(Some(i.cmp(&0))), _ => Err(()) });
    }
    if super::ext_rational::is_rational(vm, y) {
        // `cmpnum`: a Rational is compared as a Float
        let (p, q) = (num_f64(vm, x), num_f64(vm, y));
        return Ok(match (p, q) { (Some(p), Some(q)) => Ok(p.partial_cmp(&q)), _ => Err(()) });
    }
    if super::ext_complex::is_complex(vm, y) {
        // the default arm: ask the other operand and reverse the answer
        let cmp = vm.intern("<=>");
        let r = vm.funcall(y, cmp, &[x], Value::Nil)?;
        return Ok(match r { Value::Int(i) => Ok(Some(match i.cmp(&0) { core::cmp::Ordering::Less => core::cmp::Ordering::Greater, core::cmp::Ordering::Equal => core::cmp::Ordering::Equal, core::cmp::Ordering::Greater => core::cmp::Ordering::Less })), _ => Err(()) });
    }
    if vm.is_bigint(x) || vm.is_bigint(y) {
        // the reference takes the wide value first and reverses the answer for `Float <=> big`
        if let Value::Float(f) = x {
            let q = match vm.as_bigint(y) { Some(q) => q, None => return Ok(Err(())) };
            return Ok(Ok(bint_float_cmp(&q, f).map(|o| o.reverse())));
        }
        let p = match vm.as_bigint(x) { Some(p) => p, None => return Ok(Err(())) };
        return Ok(match y {
            Value::Float(f) => Ok(bint_float_cmp(&p, f)),
            _ => match vm.as_bigint(y) { Some(q) => Ok(Some(p.cmp(&q))), None => Err(()) },
        });
    }
    Ok(match (x, y) {
        (Value::Int(p), Value::Int(q)) => Ok(Some(p.cmp(&q))),
        (Value::Int(p), Value::Float(q)) => Ok(int_float_cmp(p, q)),
        (Value::Float(p), Value::Int(q)) => Ok(int_float_cmp(q, p).map(|o| o.reverse())),
        (Value::Float(p), Value::Float(q)) => Ok(p.partial_cmp(&q)),
        _ => Err(()),
    })
}
/// For `<` and friends: unordered numbers answer false, non-numbers raise.
/// `int_equal` / `flo_eq`: a Rational or a Complex on the right answers the comparison
/// itself (a Float and a Rational compare as Floats).
fn num_eq(vm: &mut Vm, s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    argc!(vm, a, 1);
    let y = a[0];
    if matches!(y, Value::Obj(_)) {
        let rational = super::ext_rational::is_rational(vm, y);
        if rational && matches!(s, Value::Float(_)) {
            return Ok(Value::bool(num_f64(vm, s) == num_f64(vm, y)));
        }
        if rational || super::ext_complex::is_complex(vm, y) {
            let eq = vm.s.eq;
            return vm.funcall(y, eq, &[s], Value::Nil);
        }
    }
    Ok(Value::bool(cmp(vm, s, a)? == Ok(Some(core::cmp::Ordering::Equal))))
}

fn cmp_or_fail(vm: &mut Vm, s: Value, a: &[Value]) -> VmResult<Option<core::cmp::Ordering>> {
    match cmp(vm, s, a)? {
        Ok(o) => Ok(o),
        Err(()) => { let x = vm.describe_for_error(s); let y = vm.describe_for_error(a[0]); Err(vm.raise_arg(&format!("comparison of {x} with {y} failed"))) }
    }
}
fn ord_test(o: Option<core::cmp::Ordering>, f: fn(core::cmp::Ordering) -> bool) -> Value { Value::bool(o.map(f).unwrap_or(false)) }
fn unordered(vm: &mut Vm, s: Value, other: Value) -> crate::error::VmError { let x = vm.describe_for_error(s); let y = vm.describe_for_error(other); vm.raise_arg(&format!("comparison of {x} with {y} failed")) }

/// The exponent of `**`: an `i64`, or a RangeError for one that is itself wide.
fn pow_exp(vm: &mut Vm, y: Value) -> VmResult<i64> {
    match y {
        Value::Int(e) => Ok(e),
        _ if vm.is_bigint(y) => Err(vm.raise(vm.core.range_error, "integer out of range")),
        // `mrb_as_int`: a Rational truncates, anything else is a TypeError
        Value::Obj(_) => vm.expect_int(y, "exponent"),
        _ => { let d = vm.describe_for_type_error(y); Err(vm.raise_type(&format!("{d} cannot be converted to Integer"))) }
    }
}

/// The reference's cap on `mrb_bint_pow` (`MRB_BIGINT_POW_MAX_BITS`).
const POW_MAX_BITS: u64 = 1000000;

/// `mrb_bint_pow`: a wide base, with the reference's guard against a huge result.
fn bint_pow(vm: &mut Vm, base: BigInt, e: i64) -> VmResult<Value> {
    if e < 0 { return Err(vm.raise_arg("negative exponent")); }
    let bits = base.bit_length().max(1);
    if e > 0 && e as u64 > POW_MAX_BITS / bits { return Err(vm.raise(vm.core.range_error, "exponent too large")); }
    Ok(vm.bint_value(base.pow(e as u64)))
}

pub(crate) fn int_pow(vm: &mut Vm, s: Value, a: &[Value]) -> VmResult<Value> {
    let (x, y) = num_args(vm, s, a)?;
    if vm.is_bigint(x) {
        if let Value::Float(q) = y { return Ok(Value::Float(libm::pow(num_f64(vm, x).unwrap_or(0.0), q))); }
        let e = pow_exp(vm, y)?;
        let b = big(vm, x);
        return bint_pow(vm, b, e);
    }
    let p = match x { Value::Int(p) => p, _ => 0 };
    if let Value::Float(q) = y { return Ok(Value::Float(libm::pow(p as f64, q))); }
    let e = pow_exp(vm, y)?;
    if e < 0 { return Ok(Value::Float(libm::pow(p as f64, e as f64))); }
    match (|| { let mut r: i64 = 1; let mut b = p; let mut e = e; loop { if e & 1 == 1 { r = r.checked_mul(b)?; } e >>= 1; if e == 0 { break; } b = b.checked_mul(b)?; } Some(r) })() {
        Some(r) => Ok(Value::Int(r)),
        None => bint_pow(vm, BigInt::from_i64(p), e),
    }
}

/// `Integer#to_s(base)`, for both widths.
fn int_to_s(vm: &mut Vm, s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    argc!(vm, a, 0, 1);
    let base = if a.len() == 1 { vm.expect_int(a[0], "base")? } else { 10 };
    if !(2..=36).contains(&base) { return Err(vm.raise_arg(&format!("invalid radix {base}"))); }
    if let Value::Int(i) = s {
        let out = if base == 10 { i.to_string() } else { to_radix(i, base as u32) };
        return Ok(vm.str_from(out));
    }
    let out = big(vm, s).to_string_radix(base as u32);
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

/// `Float#to_i` and friends: a value beyond `i64` becomes a wide integer.
fn float_to_int(vm: &mut Vm, f: f64) -> Value {
    if (-9223372036854775808.0..9223372036854775808.0).contains(&f) { Value::Int(f as i64) } else { vm.bint_value(BigInt::from_f64(f)) }
}

fn bit(vm: &mut Vm, s: Value, a: &[Value], f: fn(i64, i64) -> i64, fb: fn(&BigInt, &BigInt) -> BigInt) -> VmResult<Value> {
    let (x, y) = num_args(vm, s, a)?;
    if let (Value::Int(p), Value::Int(q)) = (x, y) { return Ok(Value::Int(f(p, q))); }
    match vm.as_bigint(y) {
        Some(q) if vm.as_bigint(x).is_some() => { let p = big(vm, x); Ok(vm.bint_value(fb(&p, &q))) }
        _ => Err(coerce_fail(vm, y, "bitop")),
    }
}

fn shift(vm: &mut Vm, s: Value, a: &[Value], left: bool) -> VmResult<Value> {
    let (x, y) = num_args(vm, s, a)?;
    let w = match y {
        Value::Int(q) => q,
        _ if vm.is_bigint(y) => return Err(vm.raise(vm.core.range_error, "integer out of range")),
        _ => return Err(coerce_fail(vm, y, "shift")),
    };
    let (left, n) = if w < 0 { (!left, w.unsigned_abs()) } else { (left, w as u64) };
    if let Value::Int(p) = x {
        if !left { return Ok(Value::Int(if n >= 64 { if p < 0 { -1 } else { 0 } } else { p >> n })); }
        if p == 0 { return Ok(Value::Int(0)); }
        if n < 64 {
            if let Some(r) = p.checked_shl(n as u32) { if (r >> n) == p { return Ok(Value::Int(r)); } }
        }
    } else if !vm.is_bigint(x) {
        return Err(coerce_fail(vm, y, "shift"));
    }
    let p = big(vm, x);
    // the reference lets the allocation fail; a width that cannot be held is refused here
    if left && n > MAX_SHIFT { return Err(vm.raise(vm.core.range_error, "shift width too big")); }
    Ok(vm.bint_value(if left { p.shl(n) } else { p.shr(n) }))
}

/// Widest left shift accepted, in bits (`docs/design/gems.md`: the reference has no limit and
/// fails in `mrb_realloc` instead).
const MAX_SHIFT: u64 = 1 << 26;

pub fn init(vm: &mut Vm) {
    let c = vm.core;
    // EPSILON, MAX, MIN, DIG, ... are mruby-numeric-ext (`ext_numeric.rs`)
    for (name, v) in [("INFINITY", f64::INFINITY), ("NAN", f64::NAN)] {
        let n = vm.intern(name);
        vm.heap.class_mut(c.float).consts.insert(n, Slot::from(Value::Float(v)));
    }
    let isc = vm.singleton_class(Value::Obj(c.integer)).unwrap();
    vm.define_method(isc, "__ensure", |vm, _s, a, _b| { argc!(vm, a, 1); match a[0] { Value::Int(_) => Ok(a[0]), Value::Float(f) if f.is_finite() => Ok(Value::Int(libm::trunc(f) as i64)), v if vm.is_bigint(v) => Ok(v), v => { let d = vm.describe_for_type_error(v); Err(vm.raise_type(&format!("can't convert {d} into Integer"))) } } });
    vm.define_methods(c.numeric, &[
        ("+@", |_vm, s, _a, _b| Ok(s)),
        ("<=>", |vm, s, a, _b| Ok(match cmp(vm, s, a)? { Ok(Some(o)) => Value::Int(o as i64), _ => Value::Nil })),
        ("<", |vm, s, a, _b| { let o = cmp_or_fail(vm, s, a)?; Ok(ord_test(o, |o| o.is_lt())) }),
        ("<=", |vm, s, a, _b| { let o = cmp_or_fail(vm, s, a)?; Ok(ord_test(o, |o| o.is_le())) }),
        (">", |vm, s, a, _b| { let o = cmp_or_fail(vm, s, a)?; Ok(ord_test(o, |o| o.is_gt())) }),
        (">=", |vm, s, a, _b| { let o = cmp_or_fail(vm, s, a)?; Ok(ord_test(o, |o| o.is_ge())) }),
        ("==", num_eq),
        ("between?", |vm, s, a, _b| { argc!(vm, a, 2); match cmp_or_fail(vm, s, &a[..1])? { Some(l) if l.is_lt() => return Ok(Value::False), Some(_) => {} None => return Err(unordered(vm, s, a[0])) } match cmp_or_fail(vm, s, &a[1..])? { Some(h) => Ok(Value::bool(h.is_le())), None => Err(unordered(vm, s, a[1])) } }),
        ("abs", |vm, s, _a, _b| Ok(match s {
            Value::Int(i) => match i.checked_abs() { Some(r) => Value::Int(r), None => vm.bint_value(BigInt::from_i64(i).abs()) },
            Value::Float(f) => Value::Float(f.abs()),
            v if vm.is_bigint(v) => { let b = big(vm, v).abs(); vm.bint_value(b) }
            v => v,
        })),
        ("to_int", |vm, s, _a, _b| Ok(match s { Value::Float(f) => float_to_int(vm, libm::trunc(f)), v => v })),
        ("nan?", |_vm, s, _a, _b| Ok(Value::bool(matches!(s, Value::Float(f) if f.is_nan())))),
        ("infinite?", |_vm, s, _a, _b| Ok(match s { Value::Float(f) if f.is_infinite() => Value::Int(if f > 0.0 { 1 } else { -1 }), _ => Value::Nil })),
        ("finite?", |_vm, s, _a, _b| Ok(Value::bool(!matches!(s, Value::Float(f) if !f.is_finite())))),
        ("step", |vm, _s, _a, _b| Err(vm.raise(vm.core.not_implemented_error, "Numeric#step is provided by mrblib"))),
    ]);
    vm.define_methods(c.integer, &[
        ("+", |vm, s, a, _b| int_arith(vm, s, a, IntOp::Add)),
        ("-", |vm, s, a, _b| int_arith(vm, s, a, IntOp::Sub)),
        ("*", |vm, s, a, _b| int_arith(vm, s, a, IntOp::Mul)),
        ("/", int_div),
        ("div", int_idiv),
        ("%", int_mod),
        ("**", |vm, s, a, _b| int_pow(vm, s, a)),
        ("pow", |vm, s, a, _b| int_pow(vm, s, a)),
        ("divmod", int_divmod),
        ("-@", |vm, s, _a, _b| Ok(match s {
            Value::Int(i) => match i.checked_neg() { Some(r) => Value::Int(r), None => vm.bint_value(BigInt::from_i64(i).neg()) },
            v if vm.is_bigint(v) => { let b = big(vm, v).neg(); vm.bint_value(b) }
            v => v,
        })),
        ("~", |vm, s, _a, _b| Ok(match s {
            Value::Int(i) => Value::Int(!i),
            v if vm.is_bigint(v) => { let b = big(vm, v).not(); vm.bint_value(b) }
            v => v,
        })),
        ("&", |vm, s, a, _b| bit(vm, s, a, |p, q| p & q, BigInt::and)),
        ("|", |vm, s, a, _b| bit(vm, s, a, |p, q| p | q, BigInt::or)),
        ("^", |vm, s, a, _b| bit(vm, s, a, |p, q| p ^ q, BigInt::xor)),
        ("<<", |vm, s, a, _b| shift(vm, s, a, true)),
        (">>", |vm, s, a, _b| shift(vm, s, a, false)),
        ("==", num_eq),
        // `eql?` compares the class as well: a Float is never `eql?` to an Integer
        ("eql?", |vm, s, a, _b| Ok(Value::bool(match a.first() {
            Some(&y) if matches!(y, Value::Int(_)) || vm.is_bigint(y) => cmp(vm, s, &[y])? == Ok(Some(core::cmp::Ordering::Equal)),
            _ => false,
        }))),
        ("hash", |vm, s, _a, _b| Ok(Value::Int(vm.value_hash(s)))),
        ("__coerce_step_counter", |vm, s, a, _b| { argc!(vm, a, 1); Ok(match a[0] { Value::Float(_) => Value::Float(num_f64(vm, s).unwrap_or(0.0)), _ => s }) }),
        ("to_s", int_to_s),
        ("inspect", int_to_s),
        ("to_i", |_vm, s, _a, _b| Ok(s)),
        ("to_int", |_vm, s, _a, _b| Ok(s)),
        ("to_f", |vm, s, _a, _b| Ok(Value::Float(num_f64(vm, s).unwrap_or(0.0)))),
        ("succ", |vm, s, _a, _b| int_arith(vm, s, &[Value::Int(1)], IntOp::Add)),
        ("pred", |vm, s, _a, _b| int_arith(vm, s, &[Value::Int(1)], IntOp::Sub)),
        ("chr", |vm, s, _a, _b| {
            let i = match s { Value::Int(i) => i, _ => return Err(vm.raise(vm.core.range_error, "integer out of range")) };
            if !(0..=255).contains(&i) { return Err(vm.raise(vm.core.range_error, &format!("{i} out of char range"))); }
            Ok(vm.str_new(&[i as u8]))
        }),
        ("floor", |vm, s, a, _b| int_rounding(vm, s, a, Rounding::Floor)),
        ("ceil", |vm, s, a, _b| int_rounding(vm, s, a, Rounding::Ceil)),
        ("round", |vm, s, a, _b| int_rounding(vm, s, a, Rounding::Round)),
        ("truncate", |vm, s, a, _b| int_rounding(vm, s, a, Rounding::Truncate)),
        // `int_quo`: with mruby-rational loaded the quotient of two Integers is exact
        ("quo", |vm, s, a, _b| {
            argc!(vm, a, 1);
            let y = a[0];
            if let (Some(n), Some(d)) = (vm.as_bigint(s), vm.as_bigint(y)) { return super::ext_rational::new_rat(vm, &n, &d); }
            if super::ext_rational::is_rational(vm, y) {
                let r = super::ext_rational::as_rational(vm, s)?;
                return super::ext_rational::div(vm, r, y);
            }
            let p = num_f64(vm, s).unwrap_or(0.0);
            match num_f64(vm, y) { Some(q) => Ok(Value::Float(p / q)), None => Err(coerce_fail(vm, y, "quo")) }
        }),
        ("fdiv", |vm, s, a, _b| { argc!(vm, a, 1); let p = num_f64(vm, s).unwrap_or(0.0); match num_f64(vm, a[0]) { Some(q) => Ok(Value::Float(p / q)), None => Err(coerce_fail(vm, a[0], "fdiv")) } }),
        ("__num_to_a", |_vm, _s, _a, _b| Ok(Value::Nil)),
    ]);
    vm.define_methods(c.float, &[
        // `flo_idiv`: floor division to an Integer
        ("div", |vm, s, a, _b| { argc!(vm, a, 1); let x = match s { Value::Float(x) => x, _ => 0.0 }; if !x.is_finite() { return Err(vm.raise(vm.core.float_domain_error, &float_to_s(x))); } let y = vm.expect_int(a[0], "divisor")?; if !(-9223372036854775808.0..9223372036854775808.0).contains(&x) { return Err(vm.raise(vm.core.range_error, "integer overflow in div")); } if y == 0 { return Err(vm.raise(vm.core.zero_division_error, "divided by 0")); } Ok(Value::Int(div_floor(x as i64, y))) }),
        ("+", |vm, s, a, _b| { argc!(vm, a, 1); if let Some(v) = tower(vm, s, a[0], '+')? { return Ok(v); } float_binop(vm, s, a, |p, q| p + q) }),
        ("-", |vm, s, a, _b| { argc!(vm, a, 1); if let Some(v) = tower(vm, s, a[0], '-')? { return Ok(v); } float_binop(vm, s, a, |p, q| p - q) }),
        ("*", |vm, s, a, _b| { argc!(vm, a, 1); if let Some(v) = tower(vm, s, a[0], '*')? { return Ok(v); } float_binop(vm, s, a, |p, q| p * q) }),
        ("/", |vm, s, a, _b| { argc!(vm, a, 1); if let Some(v) = tower(vm, s, a[0], '/')? { return Ok(v); } float_binop(vm, s, a, |p, q| p / q) }),
        ("quo", |vm, s, a, _b| float_binop(vm, s, a, |p, q| p / q)),
        ("fdiv", |vm, s, a, _b| float_binop(vm, s, a, |p, q| p / q)),
        ("%", |vm, s, a, _b| float_binop(vm, s, a, |p, q| { let m = p % q; if m != 0.0 && ((m < 0.0) != (q < 0.0)) { m + q } else { m } })),
        ("**", |vm, s, a, _b| float_binop(vm, s, a, libm::pow)),
        ("-@", |_vm, s, _a, _b| Ok(match s { Value::Float(f) => Value::Float(-f), v => v })),
        ("==", num_eq),
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
        // `flo_divmod`: the quotient comes back as an Integer when it fits
        ("divmod", |vm, s, a, _b| { argc!(vm, a, 1); let p = match s { Value::Float(f) => f, _ => 0.0 }; let q = match num_f64(vm, a[0]) { Some(q) => q, None => return Err(coerce_fail(vm, a[0], "divmod")) }; let (d, m) = flodivmod(vm, p, q)?; let dv = if d.is_finite() { float_to_int(vm, d) } else { Value::Float(d) }; Ok(vm.ary_new(vec![dv, Value::Float(m)])) }),
    ]);
}

/// numeric.c `flodivmod`: the remainder comes from `fmod`, which is exact, and the quotient
/// from `(x - mod) / y` rounded, so a large quotient keeps its last digits.
fn flodivmod(vm: &mut Vm, x: f64, y: f64) -> VmResult<(f64, f64)> {
    if y.is_nan() { return Ok((y, y)); }
    if y == 0.0 { return Err(zero_div(vm)); }
    let mut m = if y.is_infinite() && !x.is_infinite() { x } else { libm::fmod(x, y) };
    let mut d = if x.is_infinite() && !y.is_infinite() { x } else { libm::round((x - m) / y) };
    if d == 0.0 { d = 0.0; }
    if m == 0.0 { m = 0.0; }
    if y * m < 0.0 { m += y; d -= 1.0; }
    Ok((d, m))
}

fn float_binop(vm: &mut Vm, s: Value, a: &[Value], ff: fn(f64, f64) -> f64) -> VmResult<Value> {
    let (x, y) = num_args(vm, s, a)?;
    let p = match x { Value::Float(p) => p, _ => 0.0 };
    match num_f64(vm, y) {
        Some(q) => Ok(Value::Float(ff(p, q))),
        None => { let d = vm.describe_for_error(y); Err(vm.raise_type(&format!("{d} can't be coerced into Float"))) }
    }
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

/// `Integer#floor/ceil/round/truncate(ndigits)` for negative `ndigits`
/// (numeric.c `prepare_int_rounding` and the wide arms of `int_floor` and friends; the
/// wide arms answer the immediate case too, which they agree with and which can overflow).
fn int_rounding(vm: &mut Vm, s: Value, a: &[Value], mode: Rounding) -> VmResult<Value> {
    argc!(vm, a, 0, 1);
    let nd = if a.is_empty() { 0 } else { vm.expect_int(a[0], "ndigits")? };
    if nd >= 0 { return Ok(s); }
    let x = match vm.as_bigint(s) { Some(x) => x, None => return Ok(s) };
    // more trailing zeros than the value has digits: the answer is 0
    let bytes = if x.mag.len() > 2 { x.byte_size() as f64 + 0.125 } else { 8.0 - 0.125 };
    if -0.415241 * nd as f64 > bytes { return Ok(Value::Int(0)); }
    let f = BigInt::from_i64(10).pow((-nd) as u64);
    let m = x.mod_floor(&f);
    let n = x.sub(&m);
    let r = match mode {
        Rounding::Floor => n,
        Rounding::Ceil => if m.is_zero() { x } else { n.add(&f) },
        Rounding::Truncate => if n.sign() < 0 { n.add(&f) } else { n },
        Rounding::Round => {
            let h = f.shr(1);
            let c = m.cmp(&h);
            if c.is_gt() || (c.is_eq() && x.sign() > 0) { n.add(&f) } else { n }
        }
    };
    Ok(vm.bint_value(r))
}

fn float_to_i(vm: &mut Vm, s: Value, _a: &[Value], _b: Value) -> VmResult<Value> {
    match s {
        Value::Float(f) if f.is_finite() => Ok(float_to_int(vm, libm::trunc(f))),
        Value::Float(f) => Err(vm.raise(vm.core.float_domain_error, &float_to_s(f))),
        v => Ok(v),
    }
}
fn float_round(vm: &mut Vm, s: Value, f: fn(f64) -> f64) -> VmResult<Value> {
    match s {
        Value::Float(x) if x.is_finite() => Ok(float_to_int(vm, f(x))),
        Value::Float(x) => Err(vm.raise(vm.core.float_domain_error, &float_to_s(x))),
        v => Ok(v),
    }
}
