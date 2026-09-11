//! String (byte strings; no encoding support, like mruby without MRB_UTF8_STRING).

use alloc::{format, string::String, vec, vec::Vec};

use crate::argc;
use crate::error::VmResult;
use crate::object::ObjKind;
use crate::value::Value;
use crate::vm::Vm;

pub fn str_inspect(b: &[u8]) -> Vec<u8> {
    let mut out = vec![b'"'];
    for (i, &c) in b.iter().enumerate() {
        let iter_peek = &b[i + 1..];
        match c {
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'\t' => out.extend_from_slice(b"\\t"),
            0x0c => out.extend_from_slice(b"\\f"),
            0x0b => out.extend_from_slice(b"\\v"),
            0x08 => out.extend_from_slice(b"\\b"),
            0x07 => out.extend_from_slice(b"\\a"),
            0x1b => out.extend_from_slice(b"\\e"),
            0x20..=0x7e => { if c == b'#' { if let Some(&n) = iter_peek.get(0) { if n == b'{' || n == b'$' || n == b'@' { out.push(b'\\'); } } } out.push(c); }
            _ => out.extend_from_slice(format!("\\x{c:02X}").as_bytes()),
        }
    }
    out.push(b'"');
    out
}

fn bytes(vm: &Vm, v: Value) -> Vec<u8> {
    vm.str_bytes(v).map(|b| b.to_vec()).unwrap_or_default()
}
fn set(vm: &mut Vm, v: Value, b: Vec<u8>) -> VmResult<()> {
    match v.obj() {
        Some(o) => {
            if vm.heap.get(o).frozen { return Err(vm.raise(vm.core.frozen_error, "can't modify frozen String")); }
            if let ObjKind::String(s) = &mut vm.heap.get_mut(o).kind { *s = b; }
            Ok(())
        }
        None => Err(vm.raise_type("not a string")),
    }
}

/// Resolves `(index, len)` / `index` / `range` against a length; returns `None` when out of range.
pub fn index_args(vm: &mut Vm, len: usize, a: &[Value]) -> VmResult<Option<(usize, usize)>> {
    let norm = |i: i64| -> Option<usize> { let i = if i < 0 { i + len as i64 } else { i }; if i < 0 || i as usize > len { None } else { Some(i as usize) } };
    let a: Vec<Value> = a.iter().map(|v| match v { Value::Float(f) => Value::Int(*f as i64), v => *v }).collect();
    match &a[..] {
        [Value::Int(i)] => Ok(norm(*i).filter(|&i| i < len).map(|i| (i, 1))),
        [Value::Int(i), Value::Int(n)] => { if *n < 0 { return Ok(None); } Ok(norm(*i).map(|i| (i, (*n as usize).min(len - i)))) }
        [Value::Obj(o)] => {
            if let ObjKind::Range { begin, end, excl } = vm.heap.get(*o).kind {
                let b = match begin { Value::Int(b) => b, Value::Nil => 0, _ => return Err(vm.raise_type("no implicit conversion into Integer")) };
                let e = match end { Value::Int(e) => e, Value::Nil => len as i64, _ => return Err(vm.raise_type("no implicit conversion into Integer")) };
                let b = match norm(b) { Some(b) => b, None => return Ok(None) };
                let mut e = if e < 0 { e + len as i64 } else { e };
                if !excl || end.is_nil() { e += 1; }
                let e = (e.max(b as i64) as usize).min(len);
                Ok(Some((b, e - b)))
            } else { Err(vm.raise_type("no implicit conversion into Integer")) }
        }
        _ => Err(vm.raise_type("no implicit conversion into Integer")),
    }
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() { return if from <= hay.len() { Some(from) } else { None }; }
    if from >= hay.len() { return None; }
    hay[from..].windows(needle.len()).position(|w| w == needle).map(|p| p + from)
}

fn to_i(b: &[u8], base: u32) -> i64 {
    let s = String::from_utf8_lossy(b);
    let s = s.trim_start();
    let mut end = 0;
    let mut cs: Vec<char> = s.chars().collect();
    if end < cs.len() && (cs[end] == '-' || cs[end] == '+') { end += 1; }
    // optional radix prefix matching the base (`0x` for 16, `0b` for 2, `0o` for 8)
    let prefix = match base { 16 => Some('x'), 2 => Some('b'), 8 => Some('o'), _ => None };
    if let Some(p) = prefix {
        if cs.len() > end + 2 && cs[end] == '0' && cs[end + 1].to_ascii_lowercase() == p { cs.drain(end..end + 2); }
    }
    while end < cs.len() && (cs[end].is_digit(base) || (cs[end] == '_' && end > 0 && cs[end - 1] != '_')) { end += 1; }
    let t: String = cs[..end].iter().filter(|c| **c != '_').collect();
    i64::from_str_radix(t.trim_start_matches('+'), base).unwrap_or(0)
}

fn to_f(b: &[u8]) -> f64 {
    let s = String::from_utf8_lossy(b);
    let s = s.trim_start();
    let mut end = 0;
    let cs: Vec<char> = s.chars().collect();
    let mut seen_dot = false; let mut seen_e = false;
    while end < cs.len() {
        let c = cs[end];
        if c == '_' {
            // a single underscore between digits is skipped; anything else ends the number
            if end > 0 && cs[end - 1].is_ascii_digit() && end + 1 < cs.len() && cs[end + 1].is_ascii_digit() { end += 1; } else { break; }
        }
        else if c.is_ascii_digit() { end += 1; }
        else if c == '.' && !seen_dot && !seen_e && end + 1 < cs.len() && cs[end + 1].is_ascii_digit() { seen_dot = true; end += 1; }
        else if (c == 'e' || c == 'E') && !seen_e && end > 0 { seen_e = true; end += 1; if end < cs.len() && (cs[end] == '-' || cs[end] == '+') { end += 1; } }
        else if (c == '-' || c == '+') && end == 0 { end += 1; }
        else { break; }
    }
    let t: String = cs[..end].iter().filter(|c| **c != '_').collect();
    t.parse().unwrap_or(0.0)
}

pub fn init(vm: &mut Vm) {
    let c = vm.core;
    vm.define_methods(c.string, &[
        ("initialize", |vm, s, a, _b| { argc!(vm, a, 0, 1); if a.len() == 1 { let b = vm.expect_str(a[0], "argument")?; set(vm, s, b)?; } Ok(s) }),
        ("initialize_copy", |vm, s, a, _b| { argc!(vm, a, 1); let b = vm.expect_str(a[0], "argument")?; set(vm, s, b)?; Ok(s) }),
        ("to_s", |_vm, s, _a, _b| Ok(s)),
        ("to_str", |_vm, s, _a, _b| Ok(s)),
        ("inspect", |vm, s, _a, _b| { let i = str_inspect(&bytes(vm, s)); Ok(vm.str_new(&i)) }),
        ("to_sym", |vm, s, _a, _b| { let b = bytes(vm, s); Ok(Value::Sym(vm.syms.intern(&b))) }),
        ("intern", |vm, s, _a, _b| { let b = bytes(vm, s); Ok(Value::Sym(vm.syms.intern(&b))) }),
        ("to_i", |vm, s, a, _b| { argc!(vm, a, 0, 1); let base = if a.len() == 1 { vm.expect_int(a[0], "base")? as u32 } else { 10 }; if !(2..=36).contains(&base) { return Err(vm.raise_arg(&format!("invalid radix {base}"))); } Ok(Value::Int(to_i(&bytes(vm, s), base))) }),
        ("to_f", |vm, s, _a, _b| Ok(Value::Float(to_f(&bytes(vm, s))))),
        ("size", |vm, s, _a, _b| Ok(Value::Int(bytes(vm, s).len() as i64))),
        ("length", |vm, s, _a, _b| Ok(Value::Int(bytes(vm, s).len() as i64))),
        ("byteslice", |vm, s, a, _b| { argc!(vm, a, 1, 2); let b = bytes(vm, s); match index_args(vm, b.len(), a)? { Some((i, n)) => Ok(vm.str_new(&b[i..i + n])), None => Ok(Value::Nil) } }),
        ("byteindex", |vm, s, a, _b| { argc!(vm, a, 1, 2); let n = vm.expect_str(a[0], "argument")?; let b = bytes(vm, s); let from = if a.len() == 2 { let f = vm.expect_int(a[1], "offset")?; if f < 0 { let f = f + b.len() as i64; if f < 0 { return Ok(Value::Nil); } f as usize } else { f as usize } } else { 0 }; Ok(find(&b, &n, from).map(|i| Value::Int(i as i64)).unwrap_or(Value::Nil)) }),
        ("bytesize", |vm, s, _a, _b| Ok(Value::Int(bytes(vm, s).len() as i64))),
        ("empty?", |vm, s, _a, _b| Ok(Value::bool(bytes(vm, s).is_empty()))),
        ("==", str_eq),
        ("eql?", str_eq),
        ("===", str_eq),
        ("hash", |vm, s, _a, _b| Ok(Value::Int(vm.value_hash(s)))),
        ("<=>", |vm, s, a, _b| { argc!(vm, a, 1); match vm.str_bytes(a[0]).map(|b| b.to_vec()) { Some(o) => Ok(Value::Int(bytes(vm, s).cmp(&o) as i64)), None => Ok(Value::Nil) } }),
        ("+", |vm, s, a, _b| { argc!(vm, a, 1); let mut b = bytes(vm, s); match vm.str_bytes(a[0]) { Some(o) => { b.extend_from_slice(o); Ok(vm.str_new(&b)) } None => { let d = vm.describe_for_error(a[0]); Err(vm.raise_type(&format!("{d} cannot be converted to String"))) } } }),
        ("*", |vm, s, a, _b| { argc!(vm, a, 1); let n = vm.expect_int(a[0], "argument")?; if n < 0 { return Err(vm.raise_arg("negative argument")); } let b = bytes(vm, s); if (b.len() as i64).checked_mul(n).map(|t| t > i32::MAX as i64).unwrap_or(true) { return Err(vm.raise_arg("argument too big")); } let b = b.repeat(n as usize); Ok(vm.str_new(&b)) }),
        ("<<", str_concat),
        ("concat", str_concat),
        ("[]", str_aref),
        ("slice", str_aref),
        ("[]=", |vm, s, a, _b| {
            argc!(vm, a, 2, 3);
            let mut b = bytes(vm, s);
            let val = vm.expect_str(a[a.len() - 1], "value")?;
            if a.len() == 3 { let n = vm.expect_int(a[1], "length")?; if n < 0 { return Err(vm.raise(vm.core.index_error, &format!("negative length {n}"))); } }
            match index_args(vm, b.len(), &a[..a.len() - 1])? {
                Some((i, n)) => { b.splice(i..i + n, val.iter().copied()); set(vm, s, b)?; Ok(a[a.len() - 1]) }
                None => { let d = vm.inspect_str(a[0])?; Err(vm.raise(vm.core.index_error, &format!("index {d} out of string"))) }
            }
        }),
        ("upcase", |vm, s, _a, _b| { let b = bytes(vm, s).to_ascii_uppercase(); Ok(vm.str_new(&b)) }),
        ("downcase", |vm, s, _a, _b| { let b = bytes(vm, s).to_ascii_lowercase(); Ok(vm.str_new(&b)) }),
        ("upcase!", |vm, s, _a, _b| { let b = bytes(vm, s); let u = b.to_ascii_uppercase(); if u == b { Ok(Value::Nil) } else { set(vm, s, u)?; Ok(s) } }),
        ("downcase!", |vm, s, _a, _b| { let b = bytes(vm, s); let u = b.to_ascii_lowercase(); if u == b { Ok(Value::Nil) } else { set(vm, s, u)?; Ok(s) } }),
        ("capitalize", |vm, s, _a, _b| { let mut b = bytes(vm, s).to_ascii_lowercase(); if let Some(f) = b.first_mut() { *f = f.to_ascii_uppercase(); } Ok(vm.str_new(&b)) }),
        ("capitalize!", |vm, s, _a, _b| { let b = bytes(vm, s); let mut u = b.to_ascii_lowercase(); if let Some(f) = u.first_mut() { *f = f.to_ascii_uppercase(); } if u == b { Ok(Value::Nil) } else { set(vm, s, u)?; Ok(s) } }),
        ("swapcase!", |vm, s, _a, _b| { let b = bytes(vm, s); let u: Vec<u8> = b.iter().map(|c| if c.is_ascii_uppercase() { c.to_ascii_lowercase() } else { c.to_ascii_uppercase() }).collect(); if u == b { Ok(Value::Nil) } else { set(vm, s, u)?; Ok(s) } }),
        ("chomp!", |vm, s, a, _b| { argc!(vm, a, 0, 1); let b = bytes(vm, s); let rs = match a.first() { Some(Value::Nil) => return Ok(Value::Nil), Some(v) => Some(vm.expect_str(*v, "separator")?), None => None }; let n = chomp_bytes(&b, rs.as_deref()); if n == b.len() { Ok(Value::Nil) } else { set(vm, s, b[..n].to_vec())?; Ok(s) } }),
        ("chop!", |vm, s, _a, _b| { let mut b = bytes(vm, s); if b.is_empty() { return Ok(Value::Nil); } if b.ends_with(b"\r\n") { b.truncate(b.len() - 2); } else { b.pop(); } set(vm, s, b)?; Ok(s) }),
        ("setbyte", |vm, s, a, _b| { argc!(vm, a, 2); let i = vm.expect_int(a[0], "index")?; let v = vm.expect_int(a[1], "byte")?; let mut b = bytes(vm, s); let idx = if i < 0 { i + b.len() as i64 } else { i }; if idx < 0 || idx as usize >= b.len() { return Err(vm.raise(vm.core.index_error, &format!("index {i} out of string"))); } b[idx as usize] = v as u8; set(vm, s, b)?; Ok(a[1]) }),
        ("byterindex", |vm, s, a, _b| { argc!(vm, a, 1, 2); let n = vm.expect_str(a[0], "argument")?; let b = bytes(vm, s); let mut pos = if a.len() == 2 { vm.expect_int(a[1], "pos")? } else { b.len() as i64 }; if pos < 0 { pos += b.len() as i64; if pos < 0 { return Ok(Value::Nil); } } let pos = (pos as usize).min(b.len()); if n.len() > b.len() { return Ok(Value::Nil); } let start = pos.min(b.len() - n.len()); Ok((0..=start).rev().find(|&i| &b[i..i + n.len()] == &n[..]).map(|i| Value::Int(i as i64)).unwrap_or(Value::Nil)) }),
        ("bytesplice", |vm, s, a, _b| { argc!(vm, a, 2, 5); let b = bytes(vm, s); let (range_args, rest) = match a.len() { 2 => (&a[..1], &a[1..]), 3 => (&a[..2], &a[2..]), n => (&a[..2], &a[2..n]) }; let (i, n) = match index_args(vm, b.len(), range_args)? { Some(x) => x, None => return Err(vm.raise(vm.core.index_error, "index out of string")) }; let rep = vm.expect_str(rest[0], "replacement")?; let rep = if rest.len() >= 2 { let (ri, rn) = match index_args(vm, rep.len(), &rest[1..])? { Some(x) => x, None => return Err(vm.raise(vm.core.index_error, "index out of string")) }; rep[ri..ri + rn].to_vec() } else { rep }; let mut nb = b.clone(); nb.splice(i..i + n, rep); set(vm, s, nb)?; Ok(s) }),
        ("swapcase", |vm, s, _a, _b| { let b: Vec<u8> = bytes(vm, s).iter().map(|c| if c.is_ascii_uppercase() { c.to_ascii_lowercase() } else { c.to_ascii_uppercase() }).collect(); Ok(vm.str_new(&b)) }),
        ("reverse", |vm, s, _a, _b| { let mut b = bytes(vm, s); b.reverse(); Ok(vm.str_new(&b)) }),
        ("reverse!", |vm, s, _a, _b| { let mut b = bytes(vm, s); b.reverse(); set(vm, s, b)?; Ok(s) }),
        ("strip", |vm, s, _a, _b| { let b = bytes(vm, s); let t = String::from_utf8_lossy(&b).trim_matches(|c: char| c.is_ascii_whitespace() || c == '\0').as_bytes().to_vec(); Ok(vm.str_new(&t)) }),
        ("lstrip", |vm, s, _a, _b| { let b = bytes(vm, s); let t = String::from_utf8_lossy(&b).trim_start().as_bytes().to_vec(); Ok(vm.str_new(&t)) }),
        ("rstrip", |vm, s, _a, _b| { let b = bytes(vm, s); let t = String::from_utf8_lossy(&b).trim_end_matches(|c: char| c.is_ascii_whitespace() || c == '\0').as_bytes().to_vec(); Ok(vm.str_new(&t)) }),
        ("chomp", |vm, s, a, _b| { argc!(vm, a, 0, 1); let b = bytes(vm, s); let rs = match a.first() { Some(Value::Nil) => return Ok(vm.str_new(&b)), Some(v) => Some(vm.expect_str(*v, "separator")?), None => None }; let n = chomp_bytes(&b, rs.as_deref()); Ok(vm.str_new(&b[..n])) }),
        ("chop", |vm, s, _a, _b| { let mut b = bytes(vm, s); if b.ends_with(b"\r\n") { b.truncate(b.len() - 2); } else { b.pop(); } Ok(vm.str_new(&b)) }),
        ("chars", |vm, s, _a, _b| { let items: Vec<Value> = bytes(vm, s).iter().map(|c| vm.str_new(&[*c])).collect(); Ok(vm.ary_new(items)) }),
        ("bytes", |vm, s, _a, _b| { let items: Vec<Value> = bytes(vm, s).iter().map(|c| Value::Int(*c as i64)).collect(); Ok(vm.ary_new(items)) }),
        ("each_char", |vm, s, _a, b| { for c in bytes(vm, s) { let ch = vm.str_new(&[c]); vm.call_block(b, &[ch])?; } Ok(s) }),
        ("each_byte", |vm, s, _a, b| { for c in bytes(vm, s) { vm.call_block(b, &[Value::Int(c as i64)])?; } Ok(s) }),
        ("getbyte", |vm, s, a, _b| { argc!(vm, a, 1); let i = vm.expect_int(a[0], "index")?; let b = bytes(vm, s); let i = if i < 0 { i + b.len() as i64 } else { i }; Ok(b.get(i as usize).map(|c| Value::Int(*c as i64)).unwrap_or(Value::Nil)) }),
        ("ord", |vm, s, _a, _b| { let b = bytes(vm, s); match b.first() { Some(c) => Ok(Value::Int(*c as i64)), None => Err(vm.raise_arg("empty string")) } }),
        ("include?", |vm, s, a, _b| { argc!(vm, a, 1); let n = vm.expect_str(a[0], "argument")?; Ok(Value::bool(find(&bytes(vm, s), &n, 0).is_some())) }),
        ("index", |vm, s, a, _b| { argc!(vm, a, 1, 2); let n = vm.expect_str(a[0], "argument")?; let b = bytes(vm, s); let from = if a.len() == 2 { let f = vm.expect_int(a[1], "offset")?; if f < 0 { (f + b.len() as i64).max(0) as usize } else { f as usize } } else { 0 }; Ok(find(&b, &n, from).map(|i| Value::Int(i as i64)).unwrap_or(Value::Nil)) }),
        ("rindex", |vm, s, a, _b| { argc!(vm, a, 1, 2); let n = vm.expect_str(a[0], "argument")?; let b = bytes(vm, s); let mut pos = if a.len() == 2 { vm.expect_int(a[1], "pos")? } else { b.len() as i64 }; if pos < 0 { pos += b.len() as i64; if pos < 0 { return Ok(Value::Nil); } } let pos = (pos as usize).min(b.len()); if n.len() > b.len() { return Ok(Value::Nil); } let start = pos.min(b.len() - n.len()); Ok((0..=start).rev().find(|&i| &b[i..i + n.len()] == &n[..]).map(|i| Value::Int(i as i64)).unwrap_or(Value::Nil)) }),
        ("start_with?", |vm, s, a, _b| { let b = bytes(vm, s); for p in a { let p = vm.expect_str(*p, "argument")?; if b.starts_with(&p) { return Ok(Value::True); } } Ok(Value::False) }),
        ("end_with?", |vm, s, a, _b| { let b = bytes(vm, s); for p in a { let p = vm.expect_str(*p, "argument")?; if b.ends_with(&p) { return Ok(Value::True); } } Ok(Value::False) }),
        ("split", str_split),
        ("replace", |vm, s, a, _b| { argc!(vm, a, 1); let b = vm.expect_str(a[0], "argument")?; set(vm, s, b)?; Ok(s) }),
        ("clear", |vm, s, _a, _b| { set(vm, s, vec![])?; Ok(s) }),
        ("dup", |vm, s, _a, _b| { let b = bytes(vm, s); let c = vm.real_class_of(s); Ok(Value::Obj(vm.heap.alloc(c, ObjKind::String(b)))) }),
        ("freeze", |vm, s, _a, _b| { if let Some(o) = s.obj() { vm.heap.get_mut(o).frozen = true; } Ok(s) }),
        ("frozen?", |vm, s, _a, _b| Ok(Value::bool(s.obj().map(|o| vm.heap.get(o).frozen).unwrap_or(true)))),
        ("succ", |vm, s, _a, _b| { let b = str_succ(&bytes(vm, s)); Ok(vm.str_new(&b)) }),
        ("next", |vm, s, _a, _b| { let b = str_succ(&bytes(vm, s)); Ok(vm.str_new(&b)) }),
        ("center", |vm, s, a, _b| { argc!(vm, a, 1, 2); let w = vm.expect_int(a[0], "width")? as usize; let pad = if a.len() == 2 { vm.expect_str(a[1], "pad")? } else { b" ".to_vec() }; let b = bytes(vm, s); if w <= b.len() || pad.is_empty() { return Ok(vm.str_new(&b)); } let total = w - b.len(); let left = total / 2; let right = total - left; let mut out: Vec<u8> = pad.iter().cycle().take(left).copied().collect(); out.extend(&b); out.extend(pad.iter().cycle().take(right)); Ok(vm.str_new(&out)) }),
        ("ljust", |vm, s, a, _b| { argc!(vm, a, 1, 2); let w = vm.expect_int(a[0], "width")? as usize; let pad = if a.len() == 2 { vm.expect_str(a[1], "pad")? } else { b" ".to_vec() }; let mut b = bytes(vm, s); if w > b.len() && !pad.is_empty() { let n = w - b.len(); b.extend(pad.iter().cycle().take(n)); } Ok(vm.str_new(&b)) }),
        ("rjust", |vm, s, a, _b| { argc!(vm, a, 1, 2); let w = vm.expect_int(a[0], "width")? as usize; let pad = if a.len() == 2 { vm.expect_str(a[1], "pad")? } else { b" ".to_vec() }; let b = bytes(vm, s); if w <= b.len() || pad.is_empty() { return Ok(vm.str_new(&b)); } let mut out: Vec<u8> = pad.iter().cycle().take(w - b.len()).copied().collect(); out.extend(&b); Ok(vm.str_new(&out)) }),
        ("tr", |vm, s, a, _b| { argc!(vm, a, 2); let from = vm.expect_str(a[0], "from")?; let to = vm.expect_str(a[1], "to")?; let b: Vec<u8> = bytes(vm, s).iter().map(|c| match from.iter().position(|f| f == c) { Some(i) => *to.get(i).or(to.last()).unwrap_or(c), None => *c }).collect(); Ok(vm.str_new(&b)) }),
        ("delete", |vm, s, a, _b| { argc!(vm, a, 1); let del = vm.expect_str(a[0], "argument")?; let b: Vec<u8> = bytes(vm, s).into_iter().filter(|c| !del.contains(c)).collect(); Ok(vm.str_new(&b)) }),
        ("count", |vm, s, a, _b| { argc!(vm, a, 1); let set_ = vm.expect_str(a[0], "argument")?; Ok(Value::Int(bytes(vm, s).iter().filter(|c| set_.contains(c)).count() as i64)) }),
        ("__sub_replace", |vm, s, a, _b| {
            // Expands `\\`, `` \` ``, `\&`/`\0`, `\'` in a replacement string (no regexp groups).
            argc!(vm, a, 3);
            let (rep, pat) = (vm.expect_str(a[0], "replacement")?, vm.expect_str(a[1], "pattern")?);
            let found = vm.expect_int(a[2], "offset")? as usize;
            let me = bytes(vm, s);
            if found > me.len() { return Err(vm.raise(vm.core.runtime_error, "argument out of range")); }
            let mut out = vec![];
            let mut i = 0;
            while i < rep.len() {
                if rep[i] != b'\\' || i + 1 == rep.len() { out.push(rep[i]); i += 1; continue; }
                i += 1;
                match rep[i] {
                    b'\\' => out.push(b'\\'),
                    b'`' => out.extend_from_slice(&me[..found]),
                    b'&' | b'0' => out.extend_from_slice(&pat),
                    b'\'' => { let off = found + pat.len(); if me.len() > off { out.extend_from_slice(&me[off..]); } }
                    b'1'..=b'9' => {}
                    c => { out.push(b'\\'); out.push(c); }
                }
                i += 1;
            }
            Ok(vm.str_new(&out))
        }),
        ("sub", |vm, s, a, b| str_sub(vm, s, a, b, false)),
        ("gsub", |vm, s, a, b| str_sub(vm, s, a, b, true)),
        ("lines", |vm, s, _a, _b| { let b = bytes(vm, s); let mut items = vec![]; let mut start = 0; for (i, c) in b.iter().enumerate() { if *c == b'\n' { items.push(vm.str_new(&b[start..=i])); start = i + 1; } } if start < b.len() { items.push(vm.str_new(&b[start..])); } Ok(vm.ary_new(items)) }),
        ("%", |vm, _s, _a, _b| Err(vm.raise(vm.core.not_implemented_error, "String#% (mruby-sprintf) is not implemented"))),
        ("__upto_endless", |vm, _s, _a, _b| Err(vm.raise(vm.core.not_implemented_error, "endless string range"))),
        ("upto", |vm, s, a, b| { argc!(vm, a, 1, 2); let last = vm.expect_str(a[0], "argument")?; let excl = a.len() == 2 && a[1].truthy(); let mut cur = bytes(vm, s); let mut n = 0; loop { if cur.len() > last.len() { break; } if cur == last { if !excl { let v = vm.str_new(&cur); vm.call_block(b, &[v])?; } break; } let v = vm.str_new(&cur); vm.call_block(b, &[v])?; cur = str_succ(&cur); n += 1; if n > 1_000_000 { break; } } Ok(s) }),
    ]);
}

/// Length after `chomp(rs)`: no separator = one trailing newline (`\r\n`, `\n` or `\r`);
/// empty separator = all trailing newlines; otherwise the separator once.
fn chomp_bytes(b: &[u8], rs: Option<&[u8]>) -> usize {
    match rs {
        None => { if b.ends_with(b"\r\n") { b.len() - 2 } else if b.ends_with(b"\n") || b.ends_with(b"\r") { b.len() - 1 } else { b.len() } }
        Some(rs) if rs.is_empty() => { let mut n = b.len(); while n > 0 && b[n - 1] == b'\n' { n -= 1; if n > 0 && b[n - 1] == b'\r' { n -= 1; } } n }
        Some(rs) if rs == b"\n" => { if b.ends_with(b"\r\n") { b.len() - 2 } else if b.ends_with(b"\n") || b.ends_with(b"\r") { b.len() - 1 } else { b.len() } }
        Some(rs) => { if b.ends_with(rs) { b.len() - rs.len() } else { b.len() } }
    }
}

fn str_eq(vm: &mut Vm, s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    argc!(vm, a, 1);
    Ok(Value::bool(match vm.str_bytes(a[0]) { Some(o) => o == &bytes(vm, s)[..], None => false }))
}

fn str_concat(vm: &mut Vm, s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    argc!(vm, a, 1);
    let mut b = bytes(vm, s);
    match a[0] {
        Value::Int(i) => { if !(0..=255).contains(&i) { return Err(vm.raise(vm.core.range_error, &format!("{i} out of char range"))); } b.push(i as u8); }
        v => { let o = vm.expect_str(v, "argument")?; b.extend(o); }
    }
    set(vm, s, b)?;
    Ok(s)
}

fn str_aref(vm: &mut Vm, s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    argc!(vm, a, 1, 2);
    let b = bytes(vm, s);
    if a.len() == 1 {
        if let Some(n) = vm.str_bytes(a[0]).map(|n| n.to_vec()) {
            return Ok(if find(&b, &n, 0).is_some() { vm.str_new(&n) } else { Value::Nil });
        }
    }
    match index_args(vm, b.len(), a)? {
        Some((i, n)) => Ok(vm.str_new(&b[i..i + n])),
        None => Ok(Value::Nil),
    }
}

fn str_split(vm: &mut Vm, s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    argc!(vm, a, 0, 2);
    let b = bytes(vm, s);
    let limit = if a.len() == 2 { vm.expect_int(a[1], "limit")? } else { 0 };
    let sep: Option<Vec<u8>> = match a.first() { None | Some(Value::Nil) => None, Some(v) => Some(vm.expect_str(*v, "separator")?) };
    let mut parts: Vec<Vec<u8>> = vec![];
    match sep.as_deref() {
        None | Some(b" ") => {
            let mut cur: Vec<u8> = vec![];
            let mut i = 0;
            while i < b.len() {
                if b[i].is_ascii_whitespace() {
                    if !cur.is_empty() { parts.push(core::mem::take(&mut cur)); }
                    if limit > 0 && parts.len() as i64 == limit - 1 { let rest: Vec<u8> = b[i..].iter().skip_while(|c| c.is_ascii_whitespace()).copied().collect(); if !rest.is_empty() { parts.push(rest); } cur.clear(); break; }
                } else { cur.push(b[i]); }
                i += 1;
            }
            if !cur.is_empty() { parts.push(cur); }
        }
        Some(sep) if sep.is_empty() => { for c in &b { parts.push(vec![*c]); } }
        Some(sep) => {
            let mut start = 0;
            while let Some(p) = find(&b, sep, start) {
                if limit > 0 && parts.len() as i64 == limit - 1 { break; }
                parts.push(b[start..p].to_vec());
                start = p + sep.len();
            }
            parts.push(b[start..].to_vec());
            if limit == 0 { while parts.last().map(|p| p.is_empty()).unwrap_or(false) { parts.pop(); } }
        }
    }
    let items: Vec<Value> = parts.iter().map(|p| vm.str_new(p)).collect();
    Ok(vm.ary_new(items))
}

fn str_sub(vm: &mut Vm, s: Value, a: &[Value], blk: Value, global: bool) -> VmResult<Value> {
    argc!(vm, a, 1, 2);
    let pat = vm.expect_str(a[0], "pattern")?;
    let b = bytes(vm, s);
    let mut out = vec![];
    let mut start = 0;
    while let Some(p) = find(&b, &pat, start) {
        out.extend_from_slice(&b[start..p]);
        let rep = if a.len() == 2 { vm.expect_str(a[1], "replacement")? } else { let m = vm.str_new(&pat); let r = vm.call_block(blk, &[m])?; vm.as_string(r)? };
        out.extend(rep);
        start = p + pat.len();
        if pat.is_empty() { if start < b.len() { out.push(b[start]); } start += 1; if start > b.len() { break; } }
        if !global { break; }
    }
    if start <= b.len() { out.extend_from_slice(&b[start.min(b.len())..]); }
    Ok(vm.str_new(&out))
}

pub fn str_succ(b: &[u8]) -> Vec<u8> {
    if b.is_empty() { return vec![]; }
    let mut v = b.to_vec();
    let has_alnum = v.iter().any(|c| c.is_ascii_alphanumeric());
    let mut i = v.len();
    loop {
        if i == 0 {
            // carry out of the leftmost position
            let first = *b.iter().find(|c| !has_alnum || c.is_ascii_alphanumeric()).unwrap_or(&b[0]);
            let ins = if first.is_ascii_digit() { b'1' } else if first.is_ascii_lowercase() { b'a' } else if first.is_ascii_uppercase() { b'A' } else { 1 };
            let pos = if has_alnum { v.iter().position(|c| c.is_ascii_alphanumeric()).unwrap_or(0) } else { 0 };
            v.insert(pos, ins);
            return v;
        }
        i -= 1;
        let c = v[i];
        if has_alnum && !c.is_ascii_alphanumeric() { continue; }
        match c {
            b'z' => v[i] = b'a',
            b'Z' => v[i] = b'A',
            b'9' => v[i] = b'0',
            0xff => v[i] = 0,
            _ => { v[i] = c + 1; return v; }
        }
    }
}
