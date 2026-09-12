//! mruby-string-ext (`mrbgems/mruby-string-ext/src/string.c`) for a build
//! without `MRB_UTF8_STRING` (the reference configuration): strings are bytes,
//! one character per byte. The gem's Ruby part is `src/mrblib_string-ext.mrb`.

use alloc::{format, vec, vec::Vec};

use crate::argc;
use crate::error::VmResult;
use crate::object::ObjKind;
use crate::value::Value;
use crate::vm::Vm;

fn bytes(vm: &Vm, v: Value) -> Vec<u8> { vm.str_bytes(v).map(|b| b.to_vec()).unwrap_or_default() }
fn is_str(vm: &Vm, v: Value) -> bool { vm.str_bytes(v).is_some() }
fn frozen(vm: &Vm, v: Value) -> bool { v.obj().map(|o| vm.heap.get(o).frozen).unwrap_or(false) }
fn check_frozen(vm: &mut Vm, v: Value) -> VmResult<()> { if frozen(vm, v) { return Err(vm.frozen_error(v)); } Ok(()) }
/// `mrb_str_modify` + write.
fn set(vm: &mut Vm, v: Value, b: Vec<u8>) -> VmResult<()> {
    check_frozen(vm, v)?;
    match v.obj().map(|o| &mut vm.heap.get_mut(o).kind) { Some(ObjKind::String(s)) => { *s = b; Ok(()) } _ => Err(vm.raise_type("not a string")) }
}
/// `mrb_get_args("S")`: a String, or TypeError.
fn str_arg(vm: &mut Vm, v: Value) -> VmResult<Vec<u8>> {
    match vm.str_bytes(v) { Some(b) => Ok(b.to_vec()), None => { let d = vm.describe_for_type_error(v); Err(vm.raise_type(&format!("{d} cannot be converted to String"))) } }
}
/// `mrb_ensure_string_type`: String as is, `to_str` when it answers, else TypeError.
fn ensure_str(vm: &mut Vm, v: Value) -> VmResult<Vec<u8>> {
    if let Some(b) = vm.str_bytes(v) { return Ok(b.to_vec()); }
    let to_str = vm.intern("to_str");
    if vm.respond_to(v, to_str) { let r = vm.funcall(v, to_str, &[], Value::Nil)?; if let Some(b) = vm.str_bytes(r) { return Ok(b.to_vec()); } }
    let d = vm.describe_for_type_error(v);
    Err(vm.raise_type(&format!("{d} cannot be converted to String")))
}
fn dup(vm: &mut Vm, v: Value) -> Value { let b = bytes(vm, v); vm.str_new(&b) }

// ---- tr patterns (`tr_parse_pattern` and friends, byte for byte)

enum Pat { InOrder { start: usize, n: i64 }, Range { c0: i8, c1: i8, n: i64 } }
struct Pattern { pats: Vec<Pat>, reverse: bool }

fn parse_pattern(vm: &mut Vm, p: &[u8], reverse_enable: bool) -> VmResult<Pattern> {
    let mut pats = vec![];
    let mut i = 0;
    let mut reverse = false;
    if reverse_enable && p.len() >= 2 && p[0] == b'^' { reverse = true; i += 1; }
    while i < p.len() {
        if i + 2 < p.len() && p[i] != b'\\' && p[i + 1] == b'-' {
            let (c0, c1) = (p[i] as i8, p[i + 2] as i8);
            pats.push(Pat::Range { c0, c1, n: (c1 as i64 - c0 as i64 + 1) as u16 as i64 });
            i += 3;
        } else {
            let start = i;
            i += 1;
            while i < p.len() {
                if i + 2 < p.len() && p[i] != b'\\' && p[i + 1] == b'-' { break; }
                i += 1;
            }
            let n = i - start;
            if n > u16::MAX as usize { return Err(vm.raise_arg("tr pattern too long (max 65535)")); }
            pats.push(Pat::InOrder { start, n: n as i64 });
        }
    }
    Ok(Pattern { pats, reverse })
}

fn pat_n(p: &Pat) -> i64 { match p { Pat::InOrder { n, .. } => *n, Pat::Range { n, .. } => *n } }

/// `tr_find_character`: position of `ch` in the pattern (last match wins), -1 if absent.
fn find_char(pat: &Pattern, pstr: &[u8], ch: i8) -> i64 {
    let mut ret = -1i64;
    let mut n_sum = 0i64;
    for p in &pat.pats {
        match p {
            Pat::InOrder { start, n } => { for i in 0..*n { if pstr[*start + i as usize] as i8 == ch { ret = n_sum + i; } } }
            Pat::Range { c0, c1, .. } => { if *c0 <= ch && ch <= *c1 { ret = n_sum + (ch as i64 - *c0 as i64); } }
        }
        n_sum += pat_n(p);
    }
    if pat.reverse { return if ret < 0 { i64::MAX } else { -1 }; }
    ret
}

/// `tr_get_character`: the n-th character of the replacement pattern (its last one past the end).
fn get_char(pat: &Pattern, pstr: &[u8], nth: i64) -> i64 {
    let mut n_sum = 0i64;
    let mut it = pat.pats.iter().peekable();
    while let Some(p) = it.next() {
        if nth < n_sum + pat_n(p) {
            let i = nth - n_sum;
            return match p { Pat::InOrder { start, .. } => pstr[*start + i as usize] as i8 as i64, Pat::Range { c0, .. } => *c0 as i64 + i };
        }
        if it.peek().is_none() {
            return match p { Pat::InOrder { start, n } => pstr[*start + (*n - 1) as usize] as i8 as i64, Pat::Range { c1, .. } => *c1 as i64 };
        }
        n_sum += pat_n(p);
    }
    -1
}

/// `tr_compile_pattern`: the byte set of a pattern (note the exclusive range end, as in the reference).
fn compile_pattern(pat: &Pattern, pstr: &[u8]) -> [bool; 256] {
    let mut map = [false; 256];
    for p in &pat.pats {
        match p {
            Pat::InOrder { start, n } => { for i in 0..*n { map[pstr[*start + i as usize] as usize] = true; } }
            Pat::Range { c0, c1, .. } => { let mut i = *c0 as i64; while i < *c1 as i64 { map[(i as u8) as usize] = true; i += 1; } }
        }
    }
    if pat.reverse { for m in map.iter_mut() { *m = !*m; } }
    map
}

/// `str_tr`: returns whether anything changed.
fn tr(vm: &mut Vm, s: Value, p1: &[u8], p2: &[u8], squeeze: bool) -> VmResult<bool> {
    check_frozen(vm, s)?;
    let pat = parse_pattern(vm, p1, true)?;
    let rep = parse_pattern(vm, p2, false)?;
    let src = bytes(vm, s);
    let mut out = Vec::with_capacity(src.len());
    let mut changed = false;
    let mut lastch = -1i64;
    for &c in &src {
        let n = find_char(&pat, p1, c as i8);
        if n >= 0 {
            changed = true;
            let r = get_char(&rep, p2, n);
            if r < 0 || (squeeze && r == lastch) { continue; }
            if r > 0x80 { return Err(vm.raise_arg(&format!("character ({r}) out of range"))); }
            lastch = r;
            out.push(r as u8);
        } else {
            out.push(c);
        }
    }
    if changed { set(vm, s, out)?; }
    Ok(changed)
}

fn squeeze(vm: &mut Vm, s: Value, pat: Option<&[u8]>) -> VmResult<bool> {
    check_frozen(vm, s)?;
    let map = match pat { Some(p) => { let pp = parse_pattern(vm, p, true)?; Some(compile_pattern(&pp, p)) } None => None };
    let src = bytes(vm, s);
    let mut out = Vec::with_capacity(src.len());
    let mut changed = false;
    let mut lastch = -1i64;
    for &c in &src {
        let ci = c as i8 as i64;
        let dup = match &map { Some(m) => m[c as usize] && ci == lastch, None => ci >= 0 && ci == lastch };
        if dup { changed = true; } else { out.push(c); }
        lastch = ci;
    }
    if changed { set(vm, s, out)?; }
    Ok(changed)
}

fn delete(vm: &mut Vm, s: Value, p: &[u8]) -> VmResult<bool> {
    check_frozen(vm, s)?;
    let pp = parse_pattern(vm, p, true)?;
    let map = compile_pattern(&pp, p);
    let src = bytes(vm, s);
    let out: Vec<u8> = src.iter().copied().filter(|c| !map[*c as usize]).collect();
    let changed = out.len() != src.len();
    if changed { set(vm, s, out)?; }
    Ok(changed)
}

fn is_ws(c: u8) -> bool { matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0c | 0x0b | 0) }

fn pad_string(vm: &mut Vm, pad: &[u8], count: usize) -> VmResult<Vec<u8>> {
    if pad.is_empty() { return Err(vm.raise_arg("zero width padding")); }
    let mut out = Vec::with_capacity(count.min(1 << 20));
    while out.len() < count { let take = (count - out.len()).min(pad.len()); out.extend_from_slice(&pad[..take]); }
    Ok(out)
}

fn just(vm: &mut Vm, s: Value, a: &[Value], mode: u8) -> VmResult<Value> {
    argc!(vm, a, 1, 2);
    let width = vm.expect_int(a[0], "width")?;
    let pad = if a.len() == 2 { str_arg(vm, a[1])? } else { b" ".to_vec() };
    if pad.is_empty() { return Err(vm.raise_arg("zero width padding")); }
    let b = bytes(vm, s);
    if width <= b.len() as i64 { return Ok(vm.str_new(&b)); }
    let total = width as usize - b.len();
    let out = match mode {
        b'l' => { let mut o = b.clone(); o.extend(pad_string(vm, &pad, total)?); o }
        b'r' => { let mut o = pad_string(vm, &pad, total)?; o.extend_from_slice(&b); o }
        _ => { let left = total / 2; let right = total - left; let mut o = pad_string(vm, &pad, left)?; o.extend_from_slice(&b); o.extend(pad_string(vm, &pad, right)?); o }
    };
    Ok(vm.str_new(&out))
}

/// `int_chr_binary`.
fn int_chr_binary(vm: &mut Vm, num: Value) -> VmResult<Value> {
    let cp = vm.expect_int(num, "codepoint")?;
    if !(0..=0xff).contains(&cp) { let d = vm.inspect_str(num)?; return Err(vm.raise(vm.core.range_error, &format!("{d} out of char range"))); }
    Ok(vm.str_new(&[cp as u8]))
}

fn concat(vm: &mut Vm, s: Value, v: Value) -> VmResult<()> {
    let add = match v { Value::Int(_) | Value::Float(_) => { let c = int_chr_binary(vm, v)?; bytes(vm, c) } _ => ensure_str(vm, v)? };
    check_frozen(vm, s)?;
    let mut b = bytes(vm, s);
    b.extend_from_slice(&add);
    set(vm, s, b)
}

fn casecmp(vm: &Vm, s: Value, o: Value) -> Option<i64> {
    let (p1, p2) = (bytes(vm, s), bytes(vm, o));
    let len = p1.len().min(p2.len());
    for i in 0..len {
        let (c1, c2) = (p1[i].to_ascii_lowercase(), p2[i].to_ascii_lowercase());
        if c1 > c2 { return Some(1); }
        if c1 < c2 { return Some(-1); }
    }
    Some(p1.len().cmp(&p2.len()) as i64)
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.len() > hay.len() { return None; }
    (0..=hay.len() - needle.len()).find(|&i| &hay[i..i + needle.len()] == needle)
}
fn rfind(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.len() > hay.len() { return None; }
    (0..=hay.len() - needle.len()).rev().find(|&i| &hay[i..i + needle.len()] == needle)
}

pub fn init(vm: &mut Vm) {
    let string = vm.core.string;
    vm.define_methods(string, &[
        ("dump", |vm, s, _a, _b| { let i = super::string::str_inspect(&bytes(vm, s)); Ok(vm.str_new(&i)) }),
        ("swapcase!", |vm, s, _a, _b| { check_frozen(vm, s)?; let mut b = bytes(vm, s); let mut modify = false; for c in b.iter_mut() { if c.is_ascii_uppercase() { *c = c.to_ascii_lowercase(); modify = true; } else if c.is_ascii_lowercase() { *c = c.to_ascii_uppercase(); modify = true; } } if !modify { return Ok(Value::Nil); } set(vm, s, b)?; Ok(s) }),
        ("swapcase", |vm, s, _a, _b| { let b: Vec<u8> = bytes(vm, s).iter().map(|c| if c.is_ascii_uppercase() { c.to_ascii_lowercase() } else if c.is_ascii_lowercase() { c.to_ascii_uppercase() } else { *c }).collect(); Ok(vm.str_new(&b)) }),
        ("slice!", |vm, s, a, _b| {
            check_frozen(vm, s)?;
            argc!(vm, a, 1, 2);
            let b = bytes(vm, s);
            let str_len = b.len() as i64;
            let (mut beg, mut len);
            if a.len() == 1 {
                if is_str(vm, a[0]) {
                    let needle = bytes(vm, a[0]);
                    match find(&b, &needle) { Some(pos) => { beg = pos as i64; len = needle.len() as i64; } None => return Ok(Value::Nil) }
                } else if matches!(a[0].obj().map(|o| &vm.heap.get(o).kind), Some(ObjKind::Range { .. })) {
                    match super::ext_array::range_beg_len(vm, a[0], str_len, true)? { Some((bg, l)) => { beg = bg; len = l; } None => return Ok(Value::Nil) }
                } else {
                    beg = vm.expect_int(a[0], "index")?; if beg < 0 { beg += str_len; } if beg < 0 || beg >= str_len { return Ok(Value::Nil); } len = 1;
                }
            } else {
                beg = vm.expect_int(a[0], "index")?; len = vm.expect_int(a[1], "length")?; if beg < 0 { beg += str_len; } if len < 0 { return Ok(Value::Nil); } if beg < 0 || beg > str_len { return Ok(Value::Nil); }
            }
            if beg > str_len { return Ok(Value::Nil); }
            match beg.checked_add(len) { Some(e) if e <= str_len => {} _ => { len = str_len - beg; } }
            if len < 0 { len = 0; }
            let (bu, lu) = (beg as usize, len as usize);
            let piece = b[bu..bu + lu].to_vec();
            let mut rest = b; rest.drain(bu..bu + lu);
            set(vm, s, rest)?;
            Ok(vm.str_new(&piece))
        }),
        ("clear", |vm, s, _a, _b| { set(vm, s, vec![])?; Ok(s) }),
        ("<<", |vm, s, a, _b| { if a.len() != 1 { return Err(vm.argnum_error(a.len(), "1")); } concat(vm, s, a[0])?; Ok(s) }),
        ("concat", |vm, s, a, _b| { if a.len() == 1 { concat(vm, s, a[0])?; return Ok(s); } for v in a { concat(vm, s, *v)?; } if a.is_empty() { check_frozen(vm, s)?; } Ok(s) }),
        ("append_as_bytes", |vm, s, a, _b| { if a.len() == 1 { concat(vm, s, a[0])?; return Ok(s); } for v in a { concat(vm, s, *v)?; } if a.is_empty() { check_frozen(vm, s)?; } Ok(s) }),
        ("count", |vm, s, a, _b| { argc!(vm, a, 1); let p = str_arg(vm, a[0])?; let pp = parse_pattern(vm, &p, true)?; let map = compile_pattern(&pp, &p); let n = bytes(vm, s).iter().filter(|c| map[**c as usize]).count(); Ok(Value::Int(n as i64)) }),
        ("tr", |vm, s, a, _b| { argc!(vm, a, 2); let (p1, p2) = (str_arg(vm, a[0])?, str_arg(vm, a[1])?); let d = dup(vm, s); tr(vm, d, &p1, &p2, false)?; Ok(d) }),
        ("tr!", |vm, s, a, _b| { argc!(vm, a, 2); let (p1, p2) = (str_arg(vm, a[0])?, str_arg(vm, a[1])?); Ok(if tr(vm, s, &p1, &p2, false)? { s } else { Value::Nil }) }),
        ("tr_s", |vm, s, a, _b| { argc!(vm, a, 2); let (p1, p2) = (str_arg(vm, a[0])?, str_arg(vm, a[1])?); let d = dup(vm, s); tr(vm, d, &p1, &p2, true)?; Ok(d) }),
        ("tr_s!", |vm, s, a, _b| { argc!(vm, a, 2); let (p1, p2) = (str_arg(vm, a[0])?, str_arg(vm, a[1])?); Ok(if tr(vm, s, &p1, &p2, true)? { s } else { Value::Nil }) }),
        ("squeeze", |vm, s, a, _b| { argc!(vm, a, 0, 1); let p = if a.is_empty() { None } else { Some(str_arg(vm, a[0])?) }; let d = dup(vm, s); squeeze(vm, d, p.as_deref())?; Ok(d) }),
        ("squeeze!", |vm, s, a, _b| { argc!(vm, a, 0, 1); let p = if a.is_empty() { None } else { Some(str_arg(vm, a[0])?) }; Ok(if squeeze(vm, s, p.as_deref())? { s } else { Value::Nil }) }),
        ("delete", |vm, s, a, _b| { argc!(vm, a, 1); let p = str_arg(vm, a[0])?; let d = dup(vm, s); delete(vm, d, &p)?; Ok(d) }),
        ("delete!", |vm, s, a, _b| { argc!(vm, a, 1); let p = str_arg(vm, a[0])?; Ok(if delete(vm, s, &p)? { s } else { Value::Nil }) }),
        ("start_with?", |vm, s, a, _b| { let b = bytes(vm, s); for v in a { let sub = ensure_str(vm, *v)?; if b.len() >= sub.len() && b[..sub.len()] == sub[..] { return Ok(Value::True); } } Ok(Value::False) }),
        ("end_with?", |vm, s, a, _b| { let b = bytes(vm, s); for v in a { let sub = ensure_str(vm, *v)?; if b.len() >= sub.len() && b[b.len() - sub.len()..] == sub[..] { return Ok(Value::True); } } Ok(Value::False) }),
        ("hex", |vm, s, _a, _b| { let b = bytes(vm, s); Ok(super::string::to_i(vm, &b, 16)) }),
        ("oct", |vm, s, _a, _b| { let b = bytes(vm, s); Ok(super::string::to_i(vm, &b, 8)) }),
        ("chr", |vm, s, _a, _b| { let b = bytes(vm, s); Ok(vm.str_new(&b[..b.len().min(1)])) }),
        ("succ", |vm, s, _a, _b| { let b = super::string::str_succ(&bytes(vm, s)); Ok(vm.str_new(&b)) }),
        ("next", |vm, s, _a, _b| { let b = super::string::str_succ(&bytes(vm, s)); Ok(vm.str_new(&b)) }),
        ("succ!", |vm, s, _a, _b| { check_frozen(vm, s)?; let b = super::string::str_succ(&bytes(vm, s)); set(vm, s, b)?; Ok(s) }),
        ("next!", |vm, s, _a, _b| { check_frozen(vm, s)?; let b = super::string::str_succ(&bytes(vm, s)); set(vm, s, b)?; Ok(s) }),
        ("ord", |vm, s, _a, _b| { let b = bytes(vm, s); match b.first() { Some(c) => Ok(Value::Int(*c as i64)), None => Err(vm.raise_arg("empty string")) } }),
        ("delete_prefix!", |vm, s, a, _b| { argc!(vm, a, 1); let p = str_arg(vm, a[0])?; let b = bytes(vm, s); if p.len() > b.len() || b[..p.len()] != p[..] { return Ok(Value::Nil); } set(vm, s, b[p.len()..].to_vec())?; Ok(s) }),
        ("delete_prefix", |vm, s, a, _b| { argc!(vm, a, 1); let p = str_arg(vm, a[0])?; let b = bytes(vm, s); if p.len() > b.len() || b[..p.len()] != p[..] { return Ok(vm.str_new(&b)); } Ok(vm.str_new(&b[p.len()..])) }),
        ("delete_suffix!", |vm, s, a, _b| { argc!(vm, a, 1); let p = str_arg(vm, a[0])?; check_frozen(vm, s)?; let b = bytes(vm, s); if p.len() > b.len() || b[b.len() - p.len()..] != p[..] { return Ok(Value::Nil); } let n = b.len() - p.len(); set(vm, s, b[..n].to_vec())?; Ok(s) }),
        ("delete_suffix", |vm, s, a, _b| { argc!(vm, a, 1); let p = str_arg(vm, a[0])?; let b = bytes(vm, s); if p.len() > b.len() || b[b.len() - p.len()..] != p[..] { return Ok(vm.str_new(&b)); } let n = b.len() - p.len(); Ok(vm.str_new(&b[..n])) }),
        ("casecmp", |vm, s, a, _b| { argc!(vm, a, 1); if !is_str(vm, a[0]) { return Ok(Value::Nil); } Ok(Value::Int(casecmp(vm, s, a[0]).unwrap_or(0))) }),
        ("casecmp?", |vm, s, a, _b| { argc!(vm, a, 1); if !is_str(vm, a[0]) { return Ok(Value::Nil); } Ok(Value::bool(casecmp(vm, s, a[0]) == Some(0))) }),
        ("+@", |vm, s, _a, _b| { if frozen(vm, s) { Ok(dup(vm, s)) } else { Ok(s) } }),
        ("-@", |vm, s, _a, _b| { if frozen(vm, s) { return Ok(s); } let d = dup(vm, s); if let Some(o) = d.obj() { vm.heap.get_mut(o).frozen = true; } Ok(d) }),
        ("ascii_only?", |vm, s, _a, _b| Ok(Value::bool(bytes(vm, s).iter().all(|c| *c < 0x80)))),
        ("b", |vm, s, _a, _b| Ok(dup(vm, s))),
        ("__lines", |vm, s, _a, _b| { let b = bytes(vm, s); let mut out = vec![]; let mut start = 0; for (i, &c) in b.iter().enumerate() { if c == b'\n' { let l = vm.str_new(&b[start..=i]); out.push(l); start = i + 1; } } if start < b.len() { let l = vm.str_new(&b[start..]); out.push(l); } Ok(vm.ary_new(out)) }),
        ("__codepoints", |vm, s, _a, _b| { let v: Vec<Value> = bytes(vm, s).iter().map(|c| Value::Int(*c as i64)).collect(); Ok(vm.ary_new(v)) }),
        ("__scrub", |vm, s, a, _b| { argc!(vm, a, 0, 1); if let Some(r) = a.first() { if !r.is_nil() && !is_str(vm, *r) { let d = vm.describe_for_type_error(*r); return Err(vm.raise_type(&format!("{d} cannot be converted to String"))); } } Ok(dup(vm, s)) }),
        ("__scrub_chunks", |vm, s, _a, _b| { let d = dup(vm, s); Ok(vm.ary_new(vec![d])) }),
        ("lstrip", |vm, s, _a, _b| { let b = bytes(vm, s); let start = b.iter().position(|c| !is_ws(*c)).unwrap_or(b.len()); Ok(vm.str_new(&b[start..])) }),
        ("rstrip", |vm, s, _a, _b| { let b = bytes(vm, s); let end = b.iter().rposition(|c| !is_ws(*c)).map(|i| i + 1).unwrap_or(0); Ok(vm.str_new(&b[..end])) }),
        ("strip", |vm, s, _a, _b| { let b = bytes(vm, s); let start = b.iter().position(|c| !is_ws(*c)).unwrap_or(b.len()); let end = b.iter().rposition(|c| !is_ws(*c)).map(|i| i + 1).unwrap_or(0); if start >= end { return Ok(vm.str_new(b"")); } Ok(vm.str_new(&b[start..end])) }),
        ("lstrip!", |vm, s, _a, _b| { check_frozen(vm, s)?; let b = bytes(vm, s); let start = b.iter().position(|c| !is_ws(*c)).unwrap_or(b.len()); if start == 0 { return Ok(Value::Nil); } set(vm, s, b[start..].to_vec())?; Ok(s) }),
        ("rstrip!", |vm, s, _a, _b| { check_frozen(vm, s)?; let b = bytes(vm, s); let end = b.iter().rposition(|c| !is_ws(*c)).map(|i| i + 1).unwrap_or(0); if end == b.len() { return Ok(Value::Nil); } set(vm, s, b[..end].to_vec())?; Ok(s) }),
        ("strip!", |vm, s, _a, _b| { check_frozen(vm, s)?; let b = bytes(vm, s); let start = b.iter().position(|c| !is_ws(*c)).unwrap_or(b.len()); let end = b.iter().rposition(|c| !is_ws(*c)).map(|i| i + 1).unwrap_or(0); if start == 0 && end == b.len() { return Ok(Value::Nil); } let out = if start >= end { vec![] } else { b[start..end].to_vec() }; set(vm, s, out)?; Ok(s) }),
        ("__chars", |vm, s, _a, _b| { let b = bytes(vm, s); let mut out = Vec::with_capacity(b.len()); for c in b { let p = vm.str_new(&[c]); out.push(p); } Ok(vm.ary_new(out)) }),
        ("ljust", |vm, s, a, _b| just(vm, s, a, b'l')),
        ("rjust", |vm, s, a, _b| just(vm, s, a, b'r')),
        ("center", |vm, s, a, _b| just(vm, s, a, b'c')),
        ("partition", |vm, s, a, _b| {
            argc!(vm, a, 1);
            let sep = str_arg(vm, a[0])?;
            let b = bytes(vm, s);
            let (x, y, z) = if sep.is_empty() { (vec![], sep.clone(), b.clone()) } else { match find(&b, &sep) { Some(i) => (b[..i].to_vec(), sep.clone(), b[i + sep.len()..].to_vec()), None => (b.clone(), vec![], vec![]) } };
            let (x, y, z) = (vm.str_new(&x), vm.str_new(&y), vm.str_new(&z));
            Ok(vm.ary_new(vec![x, y, z]))
        }),
        ("rpartition", |vm, s, a, _b| {
            argc!(vm, a, 1);
            let sep = str_arg(vm, a[0])?;
            let b = bytes(vm, s);
            let (x, y, z) = if sep.is_empty() { (b.clone(), sep.clone(), vec![]) } else { match rfind(&b, &sep) { Some(i) => (b[..i].to_vec(), sep.clone(), b[i + sep.len()..].to_vec()), None => (vec![], vec![], b.clone()) } };
            let (x, y, z) = (vm.str_new(&x), vm.str_new(&y), vm.str_new(&z));
            Ok(vm.ary_new(vec![x, y, z]))
        }),
        ("insert", |vm, s, a, _b| {
            argc!(vm, a, 2);
            let idx = vm.expect_int(a[0], "index")?;
            let ins = str_arg(vm, a[1])?;
            check_frozen(vm, s)?;
            let mut b = bytes(vm, s);
            let len = b.len() as i64;
            let idx = if idx < 0 { len + idx + 1 } else { idx };
            if idx < 0 || idx > len { return Err(vm.raise(vm.core.index_error, &format!("index {idx} out of string"))); }
            let tail = b.split_off(idx as usize);
            b.extend_from_slice(&ins);
            b.extend_from_slice(&tail);
            set(vm, s, b)?;
            Ok(s)
        }),
        ("prepend", |vm, s, a, _b| {
            check_frozen(vm, s)?;
            if a.is_empty() { return Ok(s); }
            let mut pre = vec![];
            for v in a { let part = if *v == s { bytes(vm, s) } else { ensure_str(vm, *v)? }; pre.extend_from_slice(&part); }
            if pre.is_empty() { return Ok(s); }
            pre.extend_from_slice(&bytes(vm, s));
            set(vm, s, pre)?;
            Ok(s)
        }),
    ]);
    // Integer#chr(encoding = "ASCII-8BIT")
    let integer = vm.core.integer;
    vm.define_method(integer, "chr", |vm, s, a, _b| {
        argc!(vm, a, 0, 1);
        if let Some(enc) = a.first() {
            let e = str_arg(vm, *enc)?;
            let name = alloc::string::String::from_utf8_lossy(&e).to_ascii_uppercase();
            if name != "ASCII-8BIT" && name != "BINARY" {
                let shown = alloc::string::String::from_utf8_lossy(&e).into_owned();
                return Err(vm.raise_arg(&format!("unknown encoding name - {shown}")));
            }
        }
        int_chr_binary(vm, s)
    });
}
