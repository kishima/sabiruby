//! mruby-kernel-ext (`mrbgems/mruby-kernel-ext/src/kernel.c`): `fail`, `caller`, `__method__`,
//! `__callee__`, `Integer()`, `Float()`, `String()`, `Array()`, `Hash()`. Each is a module
//! function of Kernel: a private instance method and a public method of `Kernel` itself. The
//! gem has no Ruby part.

use alloc::{format, vec, vec::Vec};

use crate::argc;
use crate::error::{VmError, VmResult};
use crate::value::Value;
use crate::vm::Vm;

use super::numeric;

fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

fn conv_digit(c: u8) -> i64 {
    match c {
        b'0'..=b'9' => (c - b'0') as i64,
        b'a'..=b'z' => (c - b'a') as i64 + 10,
        b'A'..=b'Z' => (c - b'A') as i64 + 10,
        _ => -1,
    }
}

/// `trailingbad`: nothing read, a trailing `_`, or anything but blanks after the number.
fn trailing_bad(s: &[u8], p: usize) -> bool {
    if p == 0 { return true; }
    if s[p - 1] == b'_' { return true; }
    let mut q = p;
    while q < s.len() && is_space(s[q]) { q += 1; }
    q < s.len()
}

fn invalid_number(vm: &mut Vm, s: &[u8]) -> VmError {
    let sv = vm.str_new(s);
    let d = vm.inspect_str(sv).unwrap_or_default();
    vm.raise_arg(&format!("invalid string for number({d})"))
}

/// `mrb_str_len_to_integer`: the reference's scanner, byte for byte. `base <= 0` reads a radix
/// prefix (`0x`, `0b`, `0o`, `0d`, a bare leading `0` is octal); a negative base is its absolute
/// value. With `badcheck` (`Integer()`), anything but a well-formed number is an ArgumentError.
pub(crate) fn str_to_integer(vm: &mut Vm, s: &[u8], mut base: i64, badcheck: bool) -> VmResult<Value> {
    let n = s.len();
    // C reads `p[0]`/`p[1]` past the text: a NUL terminator stands there
    let at = |i: usize| -> u8 { if i < n { s[i] } else { 0 } };
    let mut p = 0;
    let mut sign = true;
    while p < n && is_space(s[p]) { p += 1; }
    if at(p) == b'+' { p += 1; } else if at(p) == b'-' { p += 1; sign = false; }
    if base <= 0 {
        if at(p) == b'0' {
            base = match at(p + 1) { b'x' | b'X' => 16, b'b' | b'B' => 2, b'o' | b'O' => 8, b'd' | b'D' => 10, _ => 8 };
        } else if base < -1 {
            if base < -i64::MAX { return Err(vm.raise_arg(&format!("illegal radix {base}"))); }
            base = -base;
        } else {
            base = 10;
        }
    }
    match base {
        2 => if at(p) == b'0' && matches!(at(p + 1), b'b' | b'B') { p += 2; },
        8 => if at(p) == b'0' && matches!(at(p + 1), b'o' | b'O') { p += 2; },
        10 => if at(p) == b'0' && matches!(at(p + 1), b'd' | b'D') { p += 2; },
        16 => if at(p) == b'0' && matches!(at(p + 1), b'x' | b'X') { p += 2; },
        3..=36 => {}
        _ => return Err(vm.raise_arg(&format!("illegal radix {base}"))),
    }
    if p >= n {
        if badcheck { return Err(invalid_number(vm, s)); }
        return Ok(Value::Int(0));
    }
    if s[p] == b'0' {
        // squeeze preceding 0s
        p += 1;
        while p < n {
            let c = s[p];
            p += 1;
            if c == b'_' {
                if p < n && s[p] == b'_' {
                    if badcheck { return Err(invalid_number(vm, s)); }
                    break;
                }
                continue;
            }
            if c != b'0' { p -= 1; break; }
        }
        if s[p - 1] == b'0' { p -= 1; }
    }
    if p == n || s[p] == b'_' {
        if badcheck { return Err(invalid_number(vm, s)); }
        return Ok(Value::Int(0));
    }
    // the digits start here; a value too wide for `i64` is read from them again as a BigInt
    let p2 = p;
    let mut acc: i64 = 0;
    while p < n {
        if s[p] == b'_' {
            p += 1;
            if p == n {
                if badcheck { return Err(invalid_number(vm, s)); }
                continue;
            }
            if s[p] == b'_' {
                if badcheck { return Err(invalid_number(vm, s)); }
                break;
            }
        }
        if badcheck && s[p] == 0 { return Err(vm.raise_arg("string contains null byte")); }
        let c = conv_digit(s[p]);
        if c < 0 || c >= base { break; }
        acc = match acc.checked_mul(base) { Some(m) => m, None => return too_wide(vm, s, p2, p, base, sign, badcheck) };
        if i64::MAX - c < acc {
            if !sign && i64::MAX - acc == c - 1 {
                // MRB_INT_MIN: the reference breaks out here without consuming the digit, so
                // with badcheck the digit counts as trailing garbage (`Integer("-9223372036854775808")`
                // is an ArgumentError on the reference as well)
                acc = i64::MIN;
                sign = true;
                break;
            }
            return too_wide(vm, s, p2, p, base, sign, badcheck);
        }
        acc += c;
        p += 1;
    }
    if badcheck && trailing_bad(s, p) { return Err(invalid_number(vm, s)); }
    Ok(Value::Int(if sign { acc } else { acc.wrapping_neg() }))
}

/// The overflow exit of `mrb_str_len_to_integer`: the digits are read again as a wide
/// integer. `badcheck` still looks at where the accumulation stopped, not at the end of the
/// digits, which is why `Integer("18446744073709551616")` is an ArgumentError on the
/// reference while `"18446744073709551616".to_i` is the number.
fn too_wide(vm: &mut Vm, s: &[u8], p2: usize, stop: usize, base: i64, sign: bool, badcheck: bool) -> VmResult<Value> {
    let mut p3 = p2;
    while p3 < s.len() {
        let c = s[p3];
        if c != b'_' {
            let d = conv_digit(c);
            if d < 0 || d >= base { break; }
        }
        p3 += 1;
    }
    if badcheck && trailing_bad(s, stop) { return Err(invalid_number(vm, s)); }
    match crate::bigint::BigInt::from_str(&s[p2..p3], base as u32) {
        Some(b) => Ok(vm.bint_value(if sign { b } else { b.neg() })),
        None => Ok(Value::Int(0)),
    }
}

/// `mrb_read_float`: the span of a decimal number at the start of `s` (leading blanks, sign,
/// digits, fraction, exponent). Answers the value and the index after the number; None
/// without a digit. The value is parsed by `core` from the exact span (correct rounding).
pub(crate) fn read_float(s: &[u8]) -> Option<(f64, usize)> {
    let n = s.len();
    let at = |i: usize| -> u8 { if i < n { s[i] } else { 0 } };
    let mut p = 0;
    while p < n && is_space(s[p]) { p += 1; }
    let start = p;
    if at(p) == b'-' || at(p) == b'+' { p += 1; }
    let mut any = false;
    while at(p) == b'0' { p += 1; any = true; }
    while at(p).is_ascii_digit() { p += 1; any = true; }
    let mut a = p;
    if at(p) == b'.' {
        p += 1;
        while at(p).is_ascii_digit() { p += 1; any = true; }
        a = p;
    }
    if !any { return None; }
    if at(p) | 32 == b'e' {
        let mut q = p + 1;
        if at(q) == b'-' || at(q) == b'+' { q += 1; }
        if at(q).is_ascii_digit() {
            while at(q).is_ascii_digit() { q += 1; }
            a = q;
        }
    }
    let text = core::str::from_utf8(&s[start..a]).ok()?;
    let v: f64 = text.parse().ok()?;
    Some((v, a))
}

fn invalid_float(vm: &mut Vm, s: &[u8]) -> VmError {
    let sv = vm.str_new(s);
    let d = vm.inspect_str(sv).unwrap_or_default();
    vm.raise_arg(&format!("invalid string for float({d})"))
}

/// `mrb_str_len_to_dbl`: blanks, an optional `0x` integer, underscores only between digits;
/// with `badcheck` (`Float()`) a NUL byte or trailing garbage is an ArgumentError.
pub(crate) fn str_to_dbl(vm: &mut Vm, s: &[u8], badcheck: bool) -> VmResult<f64> {
    const BUF_MAX: usize = 15 * 4 + 20 - 1; // DBL_DIG * 4 + 20, less the terminator
    let n = s.len();
    let mut p = 0;
    while p < n && is_space(s[p]) { p += 1; }
    let p2 = p;
    if n - p > 2 && s[p] == b'0' && (s[p + 1] == b'x' || s[p + 1] == b'X') {
        if !badcheck { return Ok(0.0); }
        let v = str_to_integer(vm, &s[p..], 0, badcheck)?;
        return Ok(numeric::num_f64(vm, v).unwrap_or(0.0));
    }
    let mut pend = n;
    let mut nocopy = false;
    let mut q = p;
    while q < pend {
        if s[q] == 0 {
            if badcheck { return Err(vm.raise_arg("string for Float contains null byte")); }
            pend = q;
            nocopy = true;
            break;
        }
        if !badcheck && s[q] == b' ' { pend = q; nocopy = true; break; }
        if s[q] == b'_' { break; }
        q += 1;
    }
    let buf: Vec<u8>;
    let (bytes, from): (&[u8], usize) = if nocopy {
        (&s[..pend], p2)
    } else {
        p = p2;
        let mut out = Vec::new();
        let mut prev = 0u8;
        let mut dot = false;
        while p < pend {
            let c = s[p];
            p += 1;
            if c == b'.' { dot = true; }
            if c == b'_' {
                // an underscore is allowed between digits only
                if out.is_empty() || !prev.is_ascii_digit() || p == pend {
                    if badcheck { return Err(invalid_float(vm, s)); }
                    break;
                }
            } else if badcheck && prev == b'_' && !c.is_ascii_digit() {
                return Err(invalid_float(vm, s));
            } else {
                if out.len() == BUF_MAX {
                    if dot { break; } // cut off remaining fractions
                    return Ok(f64::INFINITY);
                }
                out.push(c);
            }
            prev = c;
        }
        buf = out;
        (&buf[..], 0)
    };
    let (d, end) = match read_float(&bytes[from..]) {
        Some((d, e)) => (d, from + e),
        None => { if badcheck { return Err(invalid_float(vm, s)); } return Ok(0.0); }
    };
    if badcheck {
        if end == from { return Err(invalid_float(vm, s)); }
        let mut e = end;
        while e < bytes.len() && is_space(bytes[e]) { e += 1; }
        if e < bytes.len() { return Err(invalid_float(vm, s)); }
    }
    Ok(d)
}

/// `Integer(arg, base = 0)`
fn kernel_integer(vm: &mut Vm, _s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    argc!(vm, a, 1, 2);
    let base = if a.len() == 2 { vm.expect_int(a[1], "base")? } else { 0 };
    let base_error = |vm: &mut Vm| vm.raise_arg("base specified for non string value");
    match a[0] {
        Value::Nil => {
            if base != 0 { return Err(base_error(vm)); }
            Err(vm.raise_type("can't convert nil into Integer"))
        }
        Value::Float(f) => {
            if base != 0 { return Err(base_error(vm)); }
            if !f.is_finite() { return Err(vm.raise(vm.core.float_domain_error, &numeric::float_to_s(f))); }
            Ok(Value::Int(libm::trunc(f) as i64))
        }
        Value::Int(_) => {
            if base != 0 { return Err(base_error(vm)); }
            Ok(a[0])
        }
        v => {
            if let Some(b) = vm.str_bytes(v).map(|b| b.to_vec()) {
                return str_to_integer(vm, &b, base, true);
            }
            if base != 0 {
                // `mrb_obj_as_string`: a base needs text
                let to_s = vm.intern("to_s");
                let t = vm.funcall(v, to_s, &[], Value::Nil)?;
                if let Some(b) = vm.str_bytes(t).map(|b| b.to_vec()) {
                    return str_to_integer(vm, &b, base, true);
                }
                return Err(base_error(vm));
            }
            // `mrb_ensure_integer_type`
            let d = vm.describe_for_type_error(v);
            Err(vm.raise_type(&format!("can't convert {d} into Integer")))
        }
    }
}

/// `Float(arg)`
fn kernel_float(vm: &mut Vm, _s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    argc!(vm, a, 1);
    if let Some(b) = vm.str_bytes(a[0]).map(|b| b.to_vec()) {
        return Ok(Value::Float(str_to_dbl(vm, &b, true)?));
    }
    // `mrb_ensure_float_type`
    match a[0] {
        Value::Int(i) => Ok(Value::Float(i as f64)),
        Value::Float(_) => Ok(a[0]),
        v => { let d = vm.describe_for_type_error(v); Err(vm.raise_type(&format!("can't convert {d} into Float"))) }
    }
}

/// `String(arg)`: `mrb_type_convert(arg, String, :to_s)`
fn kernel_string(vm: &mut Vm, _s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    argc!(vm, a, 1);
    if vm.str_bytes(a[0]).is_some() { return Ok(a[0]); }
    let to_s = vm.intern("to_s");
    if !vm.respond_to(a[0], to_s) {
        let d = vm.describe_for_type_error(a[0]);
        return Err(vm.raise_type(&format!("can't convert {d} into String")));
    }
    let r = vm.funcall(a[0], to_s, &[], Value::Nil)?;
    if vm.str_bytes(r).is_some() { return Ok(r); }
    let d = vm.describe_for_type_error(a[0]);
    let rd = vm.describe_for_type_error(r);
    Err(vm.raise_type(&format!("can't convert {d} to String ({d}#to_s gives {rd})")))
}

/// `Array(arg)`: `mrb_type_convert_check(arg, Array, :to_a)`, else `[arg]`
fn kernel_array(vm: &mut Vm, _s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    argc!(vm, a, 1);
    if matches!(a[0], Value::Obj(o) if matches!(vm.heap.get(o).kind, crate::object::ObjKind::Array(_))) { return Ok(a[0]); }
    let to_a = vm.intern("to_a");
    if vm.respond_to(a[0], to_a) {
        let r = vm.funcall(a[0], to_a, &[], Value::Nil)?;
        if matches!(r, Value::Obj(o) if matches!(vm.heap.get(o).kind, crate::object::ObjKind::Array(_))) { return Ok(r); }
        if !r.is_nil() {
            let d = vm.describe_for_type_error(a[0]);
            let rd = vm.describe_for_type_error(r);
            return Err(vm.raise_type(&format!("can't convert {d} to Array ({d}#to_a gives {rd})")));
        }
    }
    Ok(vm.ary_new(vec![a[0]]))
}

/// `Hash(arg)`: `{}` for nil and `[]`, a Hash as is, anything else a TypeError
fn kernel_hash(vm: &mut Vm, _s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    argc!(vm, a, 1);
    let kind = a[0].obj().map(|o| &vm.heap.get(o).kind);
    match kind {
        Some(crate::object::ObjKind::Hash(_)) => Ok(a[0]),
        Some(crate::object::ObjKind::Array(v)) if v.is_empty() => Ok(vm.hash_new()),
        _ if a[0].is_nil() => Ok(vm.hash_new()),
        _ => { let d = vm.describe_for_type_error(a[0]); Err(vm.raise_type(&format!("{d} cannot be converted to Hash"))) }
    }
}

/// `caller(start = 1)`, `caller(range)`, `caller(start, length)`: the reference's arithmetic
/// over `mrb_get_backtrace` (`Vm::backtrace`, whose first entry is `caller` itself).
fn caller(vm: &mut Vm, _s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    argc!(vm, a, 0, 2);
    let me = vm.intern("caller");
    let bt: Vec<Value> = vm.backtrace(Some(me)).into_iter().map(|s| vm.str_from(s)).collect();
    let bt_len = bt.len() as i64;
    let negative_level = |vm: &mut Vm, v: Value| -> VmResult<VmError> { let d = vm.inspect_str(v)?; Ok(vm.raise_arg(&format!("negative level ({d})"))) };
    let (lev, mut n) = match a.len() {
        0 => (1, bt_len - 1),
        1 => {
            if matches!(a[0], Value::Obj(o) if matches!(vm.heap.get(o).kind, crate::object::ObjKind::Range { .. })) {
                match super::ext_array::range_beg_len(vm, a[0], bt_len, true)? {
                    Some((beg, len)) => (beg, len),
                    None => return Ok(Value::Nil),
                }
            } else {
                let lev = vm.expect_int(a[0], "level")?;
                if lev < 0 { return Err(negative_level(vm, a[0])?); }
                (lev, bt_len - lev)
            }
        }
        _ => (vm.expect_int(a[0], "level")?, vm.expect_int(a[1], "length")?),
    };
    if lev >= bt_len { return Ok(Value::Nil); }
    if lev < 0 { return Err(negative_level(vm, a[0])?); }
    if n < 0 { return Err(vm.raise_arg(&format!("negative size ({n})"))); }
    if n == 0 { return Ok(vm.ary_new(vec![])); }
    if bt_len <= n + lev { n = bt_len - lev - 1; }
    let from = (lev + 1) as usize;
    let to = (from + n as usize).min(bt.len());
    Ok(vm.ary_new(bt[from..to].to_vec()))
}

/// `__method__`/`__callee__`: the name of the method the calling frame runs, or, for a
/// block, the name recorded in its environment (the method the block was written in); nil
/// at the top level. The frame of an aliased method carries the original name
/// (`Vm::alias_method`), as the reference's `MRB_PROC_ALIAS` procs do, so both answer `:m1`
/// inside `alias m3 m1`.
fn current_method(vm: &mut Vm, _s: Value, _a: &[Value], _b: Value) -> VmResult<Value> {
    let Some(f) = vm.ci.last() else { return Ok(Value::Nil) };
    if let Some(m) = f.mid { return Ok(Value::Sym(m)); }
    let env = vm.heap.proc_data(f.proc_).env;
    Ok(env.and_then(|e| vm.heap.env(e).mid).map(Value::Sym).unwrap_or(Value::Nil))
}

pub fn init(vm: &mut Vm) {
    let k = vm.core.kernel;
    let names = ["fail", "caller", "__method__", "__callee__", "Integer", "Float", "String", "Array", "Hash"];
    vm.define_methods(k, &[
        ("fail", super::kernel::raise),
        ("caller", caller),
        ("__method__", current_method),
        ("__callee__", current_method),
        ("Integer", kernel_integer),
        ("Float", kernel_float),
        ("String", kernel_string),
        ("Array", kernel_array),
        ("Hash", kernel_hash),
    ]);
    // module functions: public on `Kernel` itself, private as instance methods
    let ksc = vm.singleton_class(Value::Obj(k)).expect("Kernel singleton");
    for name in names {
        let n = vm.intern(name);
        if let Some((m, _)) = vm.find_method(k, n) {
            vm.def_method_raw(ksc, n, m);
            let _ = vm.set_visibility(k, n, crate::object::Vis::Private);
        }
    }
}
