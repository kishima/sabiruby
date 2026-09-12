//! mruby-numeric-ext (`mrbgems/mruby-numeric-ext/src/numeric_ext.c`): `Integer#remainder`,
//! `pow(b, m)`, `digits`, `size`, `bit_length`, `odd?`, `even?`, `gcd`, `lcm`, `modulo`,
//! `Integer.sqrt`; `Float#remainder`, `modulo` and the `Float::RADIX`.. constants. Its Ruby
//! part (`Numeric#zero?`, `nonzero?`, `positive?`, `negative?`, `integer?`, `Integer#allbits?`,
//! `anybits?`, `nobits?`, `ceildiv`) is `src/mrblib_numeric-ext.mrb`.

use alloc::{format, vec, vec::Vec};

use crate::argc;
use crate::error::VmResult;
use crate::value::{Slot, Value};
use crate::vm::Vm;

use crate::bigint::BigInt;

use super::numeric::{as_f64, coerce_fail, int_pow, num_args, num_f64};

fn zero_div(vm: &mut Vm) -> crate::error::VmError {
    vm.raise(vm.core.zero_division_error, "divided by 0")
}

fn int_overflow(vm: &mut Vm, what: &str) -> crate::error::VmError {
    vm.raise(vm.core.range_error, &format!("integer overflow in {what}"))
}

fn cant_convert(vm: &mut Vm, v: Value) -> crate::error::VmError {
    let d = vm.describe_for_type_error(v);
    vm.raise_type(&format!("can't convert {d} into Integer"))
}

fn expect_integer(vm: &mut Vm, v: Value) -> VmResult<i64> {
    match v {
        Value::Int(i) => Ok(i),
        _ => { let d = vm.describe_for_type_error(v); Err(vm.raise_type(&format!("can't convert {d} into Integer"))) }
    }
}

/// `x.remainder(y)` = `x - y * (x / y).truncate`
fn flo_remainder(vm: &mut Vm, a: f64, y: Value) -> VmResult<Value> {
    let b = match as_f64(y) { Some(b) => b, None => return Err(coerce_fail(vm, y, "remainder")) };
    if b == 0.0 { return Err(zero_div(vm)); }
    if b.is_infinite() { return Ok(Value::Float(a)); }
    Ok(Value::Float(a - b * libm::trunc(a / b)))
}

fn int_remainder(vm: &mut Vm, s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    let (x, y) = num_args(vm, s, a)?;
    if vm.is_bigint(x) || vm.is_bigint(y) {
        if let Some(q) = vm.as_bigint(y) {
            if q.is_zero() { return Err(zero_div(vm)); }
            let p = vm.as_bigint(x).unwrap_or_else(BigInt::zero);
            return Ok(vm.bint_value(p.rem_trunc(&q)));
        }
        let p = num_f64(vm, x).unwrap_or(0.0);
        return flo_remainder(vm, p, y);
    }
    let p = match x { Value::Int(p) => p, _ => 0 };
    match y {
        Value::Int(q) => {
            if q == 0 { return Err(zero_div(vm)); }
            if p == i64::MIN && q == -1 { return Ok(Value::Int(0)); }
            Ok(Value::Int(p % q))
        }
        _ => flo_remainder(vm, p as f64, y),
    }
}

/// `pow(b)` is `**`; `pow(b, m)` is modular exponentiation without the huge temporaries,
/// with the reference's overflow rule: a product that overflows is reduced modulo `m` once,
/// and a second overflow is a RangeError (no bigint here).
fn int_powm(vm: &mut Vm, s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    argc!(vm, a, 1, 2);
    if a.len() == 1 { return int_pow(vm, s, a); }
    if !matches!(a[0], Value::Int(_)) && !vm.is_bigint(a[0]) {
        return Err(vm.raise_type("int.pow(n,m): 2nd argument not allowed unless 1st argument is an integer"));
    }
    if vm.is_bigint(s) || vm.is_bigint(a[0]) || vm.is_bigint(a[1]) {
        return bint_powm(vm, s, a[0], a[1]);
    }
    let x = match s { Value::Int(x) => x, _ => 0 };
    let exp = match a[0] { Value::Int(e) => e, _ => 0 };
    if exp < 0 { return Err(vm.raise_arg("int.pow(n,m): n must be positive")); }
    let mut m = match a[1] { Value::Int(m) => m, _ => return Err(vm.raise_type("int.pow(n,m): m must be integer")) };
    if m == 0 { return Err(zero_div(vm)); }
    let neg_mod = m < 0;
    if neg_mod { m = m.wrapping_neg(); }
    if m == 1 { return Ok(Value::Int(0)); }
    let mut base = x;
    if base == 0 && exp > 0 { return Ok(Value::Int(0)); }
    let mut exp = exp;
    let mut result: i64 = 1;
    loop {
        if exp & 1 == 1 {
            let t = match result.checked_mul(base) {
                Some(t) => t,
                None => {
                    result %= m;
                    base %= m;
                    match result.checked_mul(base) { Some(t) => t, None => return bint_powm(vm, s, a[0], a[1]) }
                }
            };
            result = t % m;
        }
        exp >>= 1;
        if exp == 0 { break; }
        let t = match base.checked_mul(base) {
            Some(t) => t,
            None => {
                base %= m;
                match base.checked_mul(base) { Some(t) => t, None => return bint_powm(vm, s, a[0], a[1]) }
            }
        };
        base = t % m;
    }
    // a negative modulus: `result + m` for a non-zero result
    if neg_mod && result != 0 { result -= m; }
    Ok(Value::Int(result))
}

/// `mrb_bint_powm`: modular exponentiation where any of the three is wide.
fn bint_powm(vm: &mut Vm, x: Value, e: Value, m: Value) -> VmResult<Value> {
    let mut modulus = match vm.as_bigint(m) { Some(m) => m, None => return Err(vm.raise_type("int.pow(n,m): m must be integer")) };
    if modulus.is_zero() { return Err(zero_div(vm)); }
    let neg_mod = modulus.sign() < 0;
    if neg_mod { modulus = modulus.abs(); }
    let exp = match vm.as_bigint(e) { Some(e) => e, None => return Err(vm.raise_type("int.pow(n,m): 2nd argument not allowed unless 1st argument is an integer")) };
    if exp.sign() < 0 { return Err(vm.raise_arg("int.pow(n,m): n must be positive")); }
    let base = vm.as_bigint(x).unwrap_or_else(BigInt::zero);
    if base.is_zero() && exp.sign() > 0 { return Ok(Value::Int(0)); }
    let mut r = base.powm(&exp, &modulus);
    // a negative modulus: `result + m` for a non-zero result
    if neg_mod && !r.is_zero() { r = r.sub(&modulus); }
    Ok(vm.bint_value(r))
}

fn int_digits(vm: &mut Vm, s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    argc!(vm, a, 0, 1);
    let base = if a.is_empty() { 10 } else { vm.expect_int(a[0], "base")? };
    if base < 0 { return Err(vm.raise_arg("negative radix")); }
    if base < 2 { return Err(vm.raise_arg(&format!("invalid radix {base}"))); }
    if vm.is_bigint(s) {
        let mut x = vm.as_bigint(s).unwrap_or_else(BigInt::zero);
        if x.sign() < 0 { return Err(vm.raise_arg("number should be positive")); }
        let bv = BigInt::from_i64(base);
        let mut digits: Vec<Value> = Vec::new();
        while x.sign() > 0 {
            let (q, r) = x.divmod_floor(&bv);
            let d = vm.bint_value(r);
            digits.push(d);
            x = q;
        }
        return Ok(vm.ary_new(digits));
    }
    let mut n = match s { Value::Int(n) => n, _ => 0 };
    if n < 0 { return Err(vm.raise_arg("number should be positive")); }
    let mut digits: Vec<Value> = Vec::new();
    if n == 0 { digits.push(Value::Int(0)); }
    while n > 0 {
        digits.push(Value::Int(n % base));
        n /= base;
    }
    Ok(vm.ary_new(digits))
}

/// `mrb_int_gcd` on magnitudes; `2^63` (the gcd of `MRB_INT_MIN` with itself or 0) does not
/// fit and comes back negative.
fn gcd_raw(x: i64, y: i64) -> i64 {
    let (mut ux, mut uy) = (x.unsigned_abs(), y.unsigned_abs());
    while uy != 0 {
        let t = uy;
        uy = ux % uy;
        ux = t;
    }
    ux as i64
}

fn int_gcd(vm: &mut Vm, s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    let (x, y) = num_args(vm, s, a)?;
    if vm.is_bigint(x) || vm.is_bigint(y) {
        let q = match vm.as_bigint(y) { Some(q) => q, None => return Err(cant_convert(vm, y)) };
        let p = vm.as_bigint(x).unwrap_or_else(BigInt::zero);
        return Ok(vm.bint_value(p.gcd(&q)));
    }
    let (p, q) = (match x { Value::Int(p) => p, _ => 0 }, expect_integer(vm, y)?);
    let g = gcd_raw(p, q);
    if g < 0 { return Err(int_overflow(vm, "gcd")); }
    Ok(Value::Int(g))
}

fn int_lcm(vm: &mut Vm, s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    let (x, y) = num_args(vm, s, a)?;
    if vm.is_bigint(x) || vm.is_bigint(y) {
        let q = match vm.as_bigint(y) { Some(q) => q, None => return Err(cant_convert(vm, y)) };
        let p = vm.as_bigint(x).unwrap_or_else(BigInt::zero);
        return Ok(vm.bint_value(p.lcm(&q)));
    }
    let (p, q) = (match x { Value::Int(p) => p, _ => 0 }, expect_integer(vm, y)?);
    if p == 0 || q == 0 { return Ok(Value::Int(0)); }
    if p == i64::MIN || q == i64::MIN { return Err(int_overflow(vm, "lcm")); }
    let g = gcd_raw(p, q);
    match (p.abs() / g).checked_mul(q.abs()) {
        Some(l) => Ok(Value::Int(l)),
        None => Err(int_overflow(vm, "lcm")),
    }
}

/// Babylonian integer square root: the largest `x` with `x * x <= n`.
fn isqrt(n: i64) -> i64 {
    if n < 2 { return n; }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

pub fn init(vm: &mut Vm) {
    let c = vm.core;
    vm.define_methods(c.integer, &[
        ("remainder", int_remainder),
        ("pow", int_powm),
        ("digits", int_digits),
        // the reference answers the bytes the limbs take, so a wide integer's size grows
        ("size", |vm, s, _a, _b| Ok(Value::Int(match vm.as_bigint(s) { Some(b) if vm.is_bigint(s) => b.byte_size() as i64, _ => 8 }))),
        ("bit_length", |vm, s, _a, _b| Ok(Value::Int(match s {
            Value::Int(i) => (64 - if i < 0 { (!i).leading_zeros() } else { i.leading_zeros() }) as i64,
            v if vm.is_bigint(v) => { let b = vm.as_bigint(v).unwrap(); (if b.sign() < 0 { b.not() } else { b }).bit_length() as i64 }
            _ => 0,
        }))),
        ("odd?", |vm, s, _a, _b| Ok(Value::bool(match s { Value::Int(i) => i % 2 != 0, v if vm.is_bigint(v) => !vm.as_bigint(v).unwrap().is_even(), _ => false }))),
        ("even?", |vm, s, _a, _b| Ok(Value::bool(match s { Value::Int(i) => i % 2 == 0, v if vm.is_bigint(v) => vm.as_bigint(v).unwrap().is_even(), _ => false }))),
        ("gcd", int_gcd),
        ("lcm", int_lcm),
    ]);
    let (modulo, rem) = (vm.intern("modulo"), vm.intern("%"));
    vm.alias_method(c.integer, modulo, rem).expect("Integer#%");
    let isc = vm.singleton_class(Value::Obj(c.integer)).expect("Integer singleton");
    vm.define_method(isc, "sqrt", |vm, _s, a, _b| {
        argc!(vm, a, 1);
        match a[0] {
            Value::Int(n) => {
                if n < 0 { return Err(vm.raise_arg("non-negative integer required")); }
                Ok(Value::Int(isqrt(n)))
            }
            v if vm.is_bigint(v) => {
                let b = vm.as_bigint(v).unwrap();
                if b.sign() < 0 { return Err(vm.raise_arg("square root of negative number")); }
                Ok(vm.bint_value(b.sqrt()))
            }
            _ => Err(vm.raise_type("expected Integer")),
        }
    });
    vm.alias_method(c.float, modulo, rem).expect("Float#%");
    vm.define_method(c.float, "remainder", |vm, s, a, _b| {
        argc!(vm, a, 1);
        let p = match s { Value::Float(p) => p, _ => 0.0 };
        flo_remainder(vm, p, a[0])
    });
    for (name, v) in [("EPSILON", f64::EPSILON), ("MAX", f64::MAX), ("MIN", f64::MIN_POSITIVE)] {
        let n = vm.intern(name);
        vm.heap.class_mut(c.float).consts.insert(n, Slot::from(Value::Float(v)));
    }
    for (name, v) in [("RADIX", 2i64), ("MANT_DIG", 53), ("DIG", 15), ("MIN_EXP", -1021), ("MIN_10_EXP", -307), ("MAX_EXP", 1024), ("MAX_10_EXP", 308)] {
        let n = vm.intern(name);
        vm.heap.class_mut(c.float).consts.insert(n, Slot::from(Value::Int(v)));
    }
    let _ = vec![0u8];
}
