//! mruby-string-ext (`mrbgems/mruby-string-ext/src/string.c`) for a build
//! without `MRB_UTF8_STRING` (the reference configuration): strings are bytes,
//! one character per byte. The gem's Ruby part is `src/mrblib/string-ext.mrb`.

use alloc::{format, vec, vec::Vec};

use crate::argc;
use crate::error::VmResult;
use crate::object::ObjKind;
use crate::value::Value;
use crate::vm::Vm;

use super::string::{char_len, char_mode, char_span, chars_of};

fn bytes(vm: &Vm, v: Value) -> Vec<u8> { vm.str_bytes(v).map(|b| b.to_vec()).unwrap_or_default() }
fn is_str(vm: &Vm, v: Value) -> bool { vm.str_bytes(v).is_some() }
fn frozen(vm: &Vm, v: Value) -> bool { v.obj().map(|o| vm.heap.get(o).frozen).unwrap_or(false) }
/// `mrb_str_modify`'s check, whose message names no value (unlike `mrb_check_frozen`).
fn check_frozen(vm: &mut Vm, v: Value) -> VmResult<()> { if frozen(vm, v) { return Err(vm.raise(vm.core.frozen_error, "can't modify frozen String")); } Ok(()) }
/// `mrb_str_modify` + write.
/// Appends to the string in place (`mrb_str_cat`). Reading the receiver out into a `Vec`,
/// extending the copy and putting it back made `<<` cost the whole length of the receiver at
/// every call, so a loop of appends was O(n^2); the argument is already a `Vec` of its own by
/// the time we get here (its conversion can run Ruby), so nothing borrows the heap twice.
fn append(vm: &mut Vm, v: Value, add: &[u8]) -> VmResult<()> {
    if let Some(o) = v.obj() {
        if let ObjKind::String(b) = &mut vm.heap.get_mut(o).kind {
            b.extend_from_slice(add);
            return Ok(());
        }
    }
    Err(vm.raise_type("not a string"))
}
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
fn dup(vm: &mut Vm, v: Value) -> Value { let b = bytes(vm, v); vm.str_new_like(&b, v) }

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

/// `count` characters of `pad`, repeating it (`mrb_str_justify`).
fn pad_string(vm: &mut Vm, pad: &[u8], count: usize, chars: bool) -> VmResult<Vec<u8>> {
    if pad.is_empty() { return Err(vm.raise_arg("zero width padding")); }
    let cs = chars_of(pad, chars);
    let mut out = Vec::with_capacity(count.min(1 << 20));
    for i in 0..count { out.extend_from_slice(cs[i % cs.len()]); }
    Ok(out)
}

fn just(vm: &mut Vm, s: Value, a: &[Value], mode: u8) -> VmResult<Value> {
    argc!(vm, a, 1, 2);
    let width = vm.expect_int(a[0], "width")?;
    let pad = if a.len() == 2 { str_arg(vm, a[1])? } else { b" ".to_vec() };
    if pad.is_empty() { return Err(vm.raise_arg("zero width padding")); }
    let b = bytes(vm, s);
    let chars = char_mode(vm, s);
    let clen = char_len(&b, chars);
    if width <= clen as i64 { return Ok(vm.str_new_like(&b, s)); }
    let total = width as usize - clen;
    let out = match mode {
        b'l' => { let mut o = b.clone(); o.extend(pad_string(vm, &pad, total, chars)?); o }
        b'r' => { let mut o = pad_string(vm, &pad, total, chars)?; o.extend_from_slice(&b); o }
        _ => { let left = total / 2; let right = total - left; let mut o = pad_string(vm, &pad, left, chars)?; o.extend_from_slice(&b); o.extend(pad_string(vm, &pad, right, chars)?); o }
    };
    Ok(vm.str_new_like(&out, s))
}

/// `int_chr_binary`.
fn int_chr_binary(vm: &mut Vm, num: Value) -> VmResult<Value> {
    let cp = vm.expect_int(num, "codepoint")?;
    if !(0..=0xff).contains(&cp) { let d = vm.inspect_str(num)?; return Err(vm.raise(vm.core.range_error, &format!("{d} out of char range"))); }
    Ok(vm.str_new(&[cp as u8]))
}

/// `int_chr_utf8`: a value outside the Unicode range spells no character, and neither does a
/// surrogate, which is where `Integer#chr` refuses one.
fn int_chr_utf8(vm: &mut Vm, num: Value) -> VmResult<Value> {
    let cp = vm.expect_int(num, "codepoint")?;
    if !(0..=0x10ffff).contains(&cp) || (0xd800..=0xdfff).contains(&cp) {
        let d = vm.inspect_str(num)?;
        return Err(vm.raise(vm.core.range_error, &format!("{d} out of char range")));
    }
    let b = super::string::from_code_point(cp as u32);
    Ok(vm.str_new(&b))
}

/// `str_concat`: an Integer is a code point, which a byte-read receiver takes as one byte and one
/// read as characters as the character it spells. `binary` is `append_as_bytes`, which takes only
/// the bytes of its argument whatever the receiver is.
fn concat_enc(vm: &mut Vm, s: Value, v: Value, binary: bool) -> VmResult<()> {
    let add = match v {
        Value::Int(_) | Value::Float(_) => {
            let c = if binary { int_chr_binary(vm, v)? } else { int_chr_utf8(vm, v)? };
            bytes(vm, c)
        }
        _ => ensure_str(vm, v)?,
    };
    check_frozen(vm, s)?;
    append(vm, s, &add)?;
    // bytes that spell no character carry their reading over: a string read as characters that is
    // handed a byte-read string holding one becomes byte-read itself (`str_cat_enc_check`)
    if !binary && !vm.str_binary(s) && vm.str_binary(v) && !add.iter().all(|c| *c < 0x80) {
        vm.str_set_binary(s, true);
    }
    Ok(())
}

/// `<<` and `concat`: the receiver's own reading says how an Integer is taken.
fn concat(vm: &mut Vm, s: Value, v: Value) -> VmResult<()> {
    let binary = !cfg!(feature = "utf8") || vm.str_binary(s);
    concat_enc(vm, s, v, binary)
}

/// True when every byte is part of a character (`mrb_str_valid_encoding_p`); always true for a
/// byte-read string, which claims no encoding.
fn valid_utf8(b: &[u8], chars: bool) -> bool { super::string::valid_chars(b, chars) }

/// The byte length of the maximal subpart at `i`, the longest prefix that could still have grown
/// into a well-formed sequence. Unicode 3.9 gives one replacement to each of those rather than one
/// to a whole run of bad bytes, so `"\xE0\x80\xAF"` is three replacements and a truncated
/// `"\xE3\x81"` one. Only asked where the sequence at `i` spells no character
/// (`str_scrub_subpart_len`).
fn subpart_len(b: &[u8], i: usize) -> usize {
    let c = b[i];
    // a byte that leads nothing stands alone, continuation bytes included
    if !(0xc2..=0xf4).contains(&c) { return 1; }
    let want = if c < 0xe0 { 2 } else if c < 0xf0 { 3 } else { 4 };
    // the second byte is the one whose range depends on the lead
    let (lo, hi) = match c { 0xe0 => (0xa0, 0xbf), 0xed => (0x80, 0x9f), 0xf0 => (0x90, 0xbf), 0xf4 => (0x80, 0x8f), _ => (0x80, 0xbf) };
    let mut n = 1;
    while n < want && i + n < b.len() {
        let x = b[i + n];
        let ok = if n == 1 { (lo..=hi).contains(&x) } else { (0x80..=0xbf).contains(&x) };
        if !ok { break; }
        n += 1;
    }
    n
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
        ("dump", |vm, s, _a, _b| { let i = super::string::str_dump(&bytes(vm, s)); Ok(vm.str_new(&i)) }),
        ("inspect", |vm, s, _a, _b| { let chars = char_mode(vm, s); let i = super::string::str_inspect(&bytes(vm, s), chars); Ok(vm.str_new(&i)) }),
        ("slice!", |vm, s, a, _b| {
            check_frozen(vm, s)?;
            argc!(vm, a, 1, 2);
            let b = bytes(vm, s);
            // positions and lengths are characters here (bytes without the feature `utf8`);
            // `chars` below turns them into the byte range to cut
            let chars = char_mode(vm, s);
            let str_len = char_len(&b, chars) as i64;
            let (mut beg, mut len);
            if a.len() == 1 {
                if is_str(vm, a[0]) {
                    let needle = bytes(vm, a[0]);
                    // a needle whose bytes spell no character is found nowhere (`str_index_str`)
                    if !super::string::valid_chars(&needle, char_mode(vm, a[0])) { return Ok(Value::Nil); }
                    match find(&b, &needle) { Some(pos) => { beg = super::string::byte_to_char(&b, pos, chars) as i64; len = char_len(&needle, chars) as i64; } None => return Ok(Value::Nil) }
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
            let (bu, lu) = char_span(&b, beg as usize, len as usize, chars);
            let piece = b[bu..bu + lu].to_vec();
            let mut rest = b; rest.drain(bu..bu + lu);
            set(vm, s, rest)?;
            Ok(vm.str_new_like(&piece, s))
        }),
        ("clear", |vm, s, _a, _b| { set(vm, s, vec![])?; Ok(s) }),
        ("<<", |vm, s, a, _b| { if a.len() != 1 { return Err(vm.argnum_error(a.len(), "1")); } concat(vm, s, a[0])?; Ok(s) }),
        ("concat", |vm, s, a, _b| { if a.len() == 1 { concat(vm, s, a[0])?; return Ok(s); } for v in a { concat(vm, s, *v)?; } if a.is_empty() { check_frozen(vm, s)?; } Ok(s) }),
        ("append_as_bytes", |vm, s, a, _b| { for v in a { concat_enc(vm, s, *v, true)?; } if a.is_empty() { check_frozen(vm, s)?; } Ok(s) }),
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
        // a suffix whose bytes are not the encoding it is taken to be in ends nothing, the way one
        // is found nowhere by `index` (`mrb_str_valid_encoding_p`)
        ("end_with?", |vm, s, a, _b| { let b = bytes(vm, s); for v in a { let sub = ensure_str(vm, *v)?; if !super::string::valid_chars(&sub, char_mode(vm, *v)) { continue; } if b.len() >= sub.len() && b[b.len() - sub.len()..] == sub[..] { return Ok(Value::True); } } Ok(Value::False) }),
        ("hex", |vm, s, _a, _b| { let b = bytes(vm, s); Ok(super::string::to_i(vm, &b, 16)) }),
        ("oct", |vm, s, _a, _b| { let b = bytes(vm, s); Ok(super::string::to_i(vm, &b, 8)) }),
        ("chr", |vm, s, _a, _b| { let chars = char_mode(vm, s); let b = bytes(vm, s); let n = if b.is_empty() { 0 } else { super::string::utf8len(&b, 0, chars) }; let head = b[..n].to_vec(); Ok(vm.str_new_like(&head, s)) }),
        ("succ", |vm, s, _a, _b| { let chars = char_mode(vm, s); let b = super::string::str_succ_mode(&bytes(vm, s), chars); Ok(vm.str_new_like(&b, s)) }),
        ("next", |vm, s, _a, _b| { let chars = char_mode(vm, s); let b = super::string::str_succ_mode(&bytes(vm, s), chars); Ok(vm.str_new_like(&b, s)) }),
        ("succ!", |vm, s, _a, _b| { check_frozen(vm, s)?; let chars = char_mode(vm, s); let b = super::string::str_succ_mode(&bytes(vm, s), chars); set(vm, s, b)?; Ok(s) }),
        ("next!", |vm, s, _a, _b| { check_frozen(vm, s)?; let chars = char_mode(vm, s); let b = super::string::str_succ_mode(&bytes(vm, s), chars); set(vm, s, b)?; Ok(s) }),
        ("ord", |vm, s, _a, _b| { let chars = char_mode(vm, s); let b = bytes(vm, s); if b.is_empty() { return Err(vm.raise_arg("empty string")); } Ok(Value::Int(super::string::char_code(vm, &b, 0, chars)?)) }),
        ("delete_prefix!", |vm, s, a, _b| { argc!(vm, a, 1); let p = str_arg(vm, a[0])?; let b = bytes(vm, s); if !super::string::valid_chars(&p, char_mode(vm, a[0])) { return Ok(Value::Nil); } if p.len() > b.len() || b[..p.len()] != p[..] { return Ok(Value::Nil); } set(vm, s, b[p.len()..].to_vec())?; Ok(s) }),
        ("delete_prefix", |vm, s, a, _b| { argc!(vm, a, 1); let p = str_arg(vm, a[0])?; let b = bytes(vm, s); if !super::string::valid_chars(&p, char_mode(vm, a[0])) { return Ok(vm.str_new_like(&b, s)); } if p.len() > b.len() || b[..p.len()] != p[..] { return Ok(vm.str_new_like(&b, s)); } let t = b[p.len()..].to_vec(); Ok(vm.str_new_like(&t, s)) }),
        ("delete_suffix!", |vm, s, a, _b| { argc!(vm, a, 1); let p = str_arg(vm, a[0])?; check_frozen(vm, s)?; let b = bytes(vm, s); if !super::string::valid_chars(&p, char_mode(vm, a[0])) { return Ok(Value::Nil); } if p.len() > b.len() || b[b.len() - p.len()..] != p[..] { return Ok(Value::Nil); } let n = b.len() - p.len(); set(vm, s, b[..n].to_vec())?; Ok(s) }),
        ("delete_suffix", |vm, s, a, _b| { argc!(vm, a, 1); let p = str_arg(vm, a[0])?; let b = bytes(vm, s); if !super::string::valid_chars(&p, char_mode(vm, a[0])) { return Ok(vm.str_new_like(&b, s)); } if p.len() > b.len() || b[b.len() - p.len()..] != p[..] { return Ok(vm.str_new_like(&b, s)); } let n = b.len() - p.len(); let t = b[..n].to_vec(); Ok(vm.str_new_like(&t, s)) }),
        ("casecmp", |vm, s, a, _b| { argc!(vm, a, 1); if !is_str(vm, a[0]) { return Ok(Value::Nil); } Ok(Value::Int(casecmp(vm, s, a[0]).unwrap_or(0))) }),
        // `casecmp` compares ASCII-folded bytes; `casecmp?` folds the case of every
        // character (`"Ä".casecmp("ä")` is -1 while `casecmp?` is true)
        ("casecmp?", |vm, s, a, _b| {
            argc!(vm, a, 1);
            if !is_str(vm, a[0]) { return Ok(Value::Nil); }
            let (cs, co) = (char_mode(vm, s), char_mode(vm, a[0]));
            let (x, y) = (bytes(vm, s), bytes(vm, a[0]));
            // only one side has to hold a character above ASCII for both to be folded; with
            // nothing above ASCII on either the two order by their bytes as they always have
            if (cs && !x.iter().all(|c| *c < 0x80)) || (co && !y.iter().all(|c| *c < 0x80)) {
                let fx = super::string::fold_case(vm, &x, cs)?;
                let fy = super::string::fold_case(vm, &y, co)?;
                return Ok(Value::bool(fx == fy));
            }
            Ok(Value::bool(casecmp(vm, s, a[0]) == Some(0)))
        }),
        ("+@", |vm, s, _a, _b| { if frozen(vm, s) { Ok(dup(vm, s)) } else { Ok(s) } }),
        ("-@", |vm, s, _a, _b| { if frozen(vm, s) { return Ok(s); } let d = dup(vm, s); if let Some(o) = d.obj() { vm.heap.get_mut(o).frozen = true; } Ok(d) }),
        ("ascii_only?", |vm, s, _a, _b| Ok(Value::bool(bytes(vm, s).iter().all(|c| *c < 0x80)))),
        // `str_b`: a copy read as bytes, which counts and cuts one position per byte whatever its
        // bytes are (`MRB_STR_ENCODING_BINARY`)
        ("b", |vm, s, _a, _b| { let d = dup(vm, s); vm.str_set_binary(d, true); Ok(d) }),
        ("__lines", |vm, s, _a, _b| { let b = bytes(vm, s); let mut out = vec![]; let mut start = 0; for (i, &c) in b.iter().enumerate() { if c == b'\n' { let l = vm.str_new_like(&b[start..=i].to_vec(), s); out.push(l); start = i + 1; } } if start < b.len() { let l = vm.str_new_like(&b[start..].to_vec(), s); out.push(l); } Ok(vm.ary_new(out)) }),
        ("__codepoints", |vm, s, _a, _b| { let chars = char_mode(vm, s); let b = bytes(vm, s); let mut v = Vec::new(); let mut i = 0; while i < b.len() { v.push(Value::Int(super::string::char_code(vm, &b, i, chars)?)); i += super::string::utf8len(&b, i, chars); } Ok(vm.ary_new(v)) }),
        // `str_scrub_core`: each maximal subpart of bytes that spells no character becomes the
        // replacement (U+FFFD by default). A byte-read string holds no characters to be wrong
        // about, and neither does a build without the feature `utf8`, so both are a copy.
        ("__scrub", |vm, s, a, _b| {
            argc!(vm, a, 0, 1);
            let repl: Option<Vec<u8>> = match a.first() {
                None | Some(Value::Nil) => None,
                Some(r) if is_str(vm, *r) => Some(bytes(vm, *r)),
                Some(r) => { let d = vm.describe_for_type_error(*r); return Err(vm.raise_type(&format!("{d} cannot be converted to String"))); }
            };
            let chars = char_mode(vm, s);
            let repl = match repl {
                Some(r) => {
                    if !valid_utf8(&r, chars) { return Err(vm.raise_arg("replacement must be valid UTF-8")); }
                    r
                }
                None => "\u{fffd}".as_bytes().to_vec(),
            };
            let b = bytes(vm, s);
            if !chars || valid_utf8(&b, chars) { return Ok(dup(vm, s)); }
            let mut out = Vec::with_capacity(b.len());
            let mut i = 0;
            while i < b.len() {
                let n = super::string::utf8len(&b, i, chars);
                if n == 1 && b[i] >= 0x80 {
                    out.extend_from_slice(&repl);
                    i += subpart_len(&b, i);
                } else {
                    out.extend_from_slice(&b[i..i + n]);
                    i += n;
                }
            }
            Ok(vm.str_new_like(&out, s))
        }),
        // `str_scrub_chunks`: the string split into alternating runs of bytes that spell
        // characters and bytes that spell none ([valid, invalid, valid, …]), which is what the
        // block form of `scrub` in mrblib hands the block.
        ("__scrub_chunks", |vm, s, _a, _b| {
            let chars = char_mode(vm, s);
            let b = bytes(vm, s);
            if !chars { let d = dup(vm, s); return Ok(vm.ary_new(vec![d])); }
            let mut parts: Vec<Vec<u8>> = Vec::new();
            let (mut valid_start, mut i) = (0usize, 0usize);
            while i < b.len() {
                let n = super::string::utf8len(&b, i, chars);
                if n == 1 && b[i] >= 0x80 {
                    parts.push(b[valid_start..i].to_vec());
                    let start = i;
                    i += subpart_len(&b, i);
                    parts.push(b[start..i].to_vec());
                    valid_start = i;
                } else {
                    i += n;
                }
            }
            parts.push(b[valid_start..].to_vec());
            let items: Vec<Value> = parts.iter().map(|c| vm.str_new_like(c, s)).collect();
            Ok(vm.ary_new(items))
        }),
        ("lstrip", |vm, s, _a, _b| { let b = bytes(vm, s); let start = b.iter().position(|c| !is_ws(*c)).unwrap_or(b.len()); let t = b[start..].to_vec(); Ok(vm.str_new_like(&t, s)) }),
        ("rstrip", |vm, s, _a, _b| { let b = bytes(vm, s); let end = b.iter().rposition(|c| !is_ws(*c)).map(|i| i + 1).unwrap_or(0); let t = b[..end].to_vec(); Ok(vm.str_new_like(&t, s)) }),
        ("strip", |vm, s, _a, _b| { let b = bytes(vm, s); let start = b.iter().position(|c| !is_ws(*c)).unwrap_or(b.len()); let end = b.iter().rposition(|c| !is_ws(*c)).map(|i| i + 1).unwrap_or(0); if start >= end { return Ok(vm.str_new_like(b"", s)); } let t = b[start..end].to_vec(); Ok(vm.str_new_like(&t, s)) }),
        ("lstrip!", |vm, s, _a, _b| { check_frozen(vm, s)?; let b = bytes(vm, s); let start = b.iter().position(|c| !is_ws(*c)).unwrap_or(b.len()); if start == 0 { return Ok(Value::Nil); } set(vm, s, b[start..].to_vec())?; Ok(s) }),
        ("rstrip!", |vm, s, _a, _b| { check_frozen(vm, s)?; let b = bytes(vm, s); let end = b.iter().rposition(|c| !is_ws(*c)).map(|i| i + 1).unwrap_or(0); if end == b.len() { return Ok(Value::Nil); } set(vm, s, b[..end].to_vec())?; Ok(s) }),
        ("strip!", |vm, s, _a, _b| { check_frozen(vm, s)?; let b = bytes(vm, s); let start = b.iter().position(|c| !is_ws(*c)).unwrap_or(b.len()); let end = b.iter().rposition(|c| !is_ws(*c)).map(|i| i + 1).unwrap_or(0); if start == 0 && end == b.len() { return Ok(Value::Nil); } let out = if start >= end { vec![] } else { b[start..end].to_vec() }; set(vm, s, out)?; Ok(s) }),
        ("__chars", |vm, s, _a, _b| { let chars = char_mode(vm, s); let b = bytes(vm, s); let parts: Vec<Vec<u8>> = chars_of(&b, chars).into_iter().map(|c| c.to_vec()).collect(); let out: Vec<Value> = parts.iter().map(|c| vm.str_new_like(c, s)).collect(); Ok(vm.ary_new(out)) }),
        ("ljust", |vm, s, a, _b| just(vm, s, a, b'l')),
        ("rjust", |vm, s, a, _b| just(vm, s, a, b'r')),
        ("center", |vm, s, a, _b| just(vm, s, a, b'c')),
        ("partition", |vm, s, a, _b| {
            argc!(vm, a, 1);
            let sep = str_arg(vm, a[0])?;
            let b = bytes(vm, s);
            // a separator whose bytes spell no character splits nothing (`str_partition`)
            let found = if super::string::valid_chars(&sep, char_mode(vm, a[0])) { find(&b, &sep) } else { None };
            let (x, y, z) = if sep.is_empty() { (vec![], sep.clone(), b.clone()) } else { match found { Some(i) => (b[..i].to_vec(), sep.clone(), b[i + sep.len()..].to_vec()), None => (b.clone(), vec![], vec![]) } };
            let (x, y, z) = (vm.str_new_like(&x, s), vm.str_new_like(&y, s), vm.str_new_like(&z, s));
            Ok(vm.ary_new(vec![x, y, z]))
        }),
        ("rpartition", |vm, s, a, _b| {
            argc!(vm, a, 1);
            let sep = str_arg(vm, a[0])?;
            let b = bytes(vm, s);
            // see `partition`
            let found = if super::string::valid_chars(&sep, char_mode(vm, a[0])) { rfind(&b, &sep) } else { None };
            let (x, y, z) = if sep.is_empty() { (b.clone(), sep.clone(), vec![]) } else { match found { Some(i) => (b[..i].to_vec(), sep.clone(), b[i + sep.len()..].to_vec()), None => (vec![], vec![], b.clone()) } };
            let (x, y, z) = (vm.str_new_like(&x, s), vm.str_new_like(&y, s), vm.str_new_like(&z, s));
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
            // see `concat_enc`: byte-read bytes above ASCII hand their reading to the string
            // they land in
            if !vm.str_binary(s) && vm.str_binary(a[1]) && ins.iter().any(|c| *c >= 0x80) { vm.str_set_binary(s, true); }
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
            // `"UTF-8"` is accepted only by a build that has the encoding (`MRB_UTF8_STRING`)
            if cfg!(feature = "utf8") && name == "UTF-8" {
                return int_chr_utf8(vm, s);
            }
            if name != "ASCII-8BIT" && name != "BINARY" {
                let shown = alloc::string::String::from_utf8_lossy(&e).into_owned();
                return Err(vm.raise_arg(&format!("unknown encoding name - {shown}")));
            }
        }
        int_chr_binary(vm, s)
    });
}
