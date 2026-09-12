//! mruby-pack (`mrbgems/mruby-pack/src/pack.c`): `Array#pack`, `String#unpack`,
//! `String#unpack1`. No Ruby part.
//!
//! The template parser (`read_tmpl`) and the directives are ported one for one, including the
//! reference's own choices: `i`/`I` and `j`/`J` are aliases decided by the size of C's `int`
//! and `intptr_t` (4 and 8 here), the modifiers `_ ! < >` are accepted only after `sSiIlLqQ`,
//! `#` starts a comment to the end of the line, `p`/`P`/`%` are refused, and an unsigned 64-bit
//! value that does not fit an Integer is a RangeError rather than a wide integer (the
//! reference checks `MRB_INT_MAX` even in a build with mruby-bigint).

use alloc::{format, string::String, vec, vec::Vec};

use crate::argc;
use crate::error::{VmError, VmResult};
use crate::value::Value;
use crate::vm::Vm;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Dir { Char, Short, Long, Quad, Ber, Utf8, Double, Float, Str, Hex, Bstr, Base64, Uu, Qenc, Nul, Back, Abs, None }

#[derive(Clone, Copy, PartialEq)]
enum Type { Integer, Float, String, None }

// the reference's `PACK_FLAG_*`
const F_A: u32 = 0x02;        // `a`: NUL padding
const F_Z: u32 = 0x04;        // `Z`: NUL terminated
const F_SIGNED: u32 = 0x08;
const F_GT: u32 = 0x10;       // `>`: big endian
const F_LT: u32 = 0x20;       // `<`: little endian
const F_WIDTH: u32 = 0x40;    // the count is a width
const F_LSB: u32 = 0x80;      // low bit / low nibble first
const F_COUNT2: u32 = 0x100;  // the count belongs to one argument
const F_LE: u32 = 0x200;      // little endian after the modifiers

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const UU: &[u8; 64] = b"`!\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_";

fn base64_val(c: u8) -> Option<u8> {
    match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

fn hex2int(c: u8) -> i32 {
    match c {
        b'0'..=b'9' => (c - b'0') as i32,
        b'a'..=b'f' => (c - b'a') as i32 + 10,
        b'A'..=b'F' => (c - b'A') as i32 + 10,
        _ => -1,
    }
}

/// `qprint_encode_type`: 0 = copied, 1 = encoded, 2 = newline.
fn qprint_type(b: u8) -> u8 {
    match b {
        b'\t' => 0,
        b'\n' => 2,
        0x00..=0x1f | b'=' | 0x7f..=0xff => 1,
        _ => 0,
    }
}

fn is_space(c: u8) -> bool { matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r') }

struct Tmpl<'a> { s: &'a [u8], idx: usize }

struct Spec { dir: Dir, ty: Type, size: usize, count: i64, flags: u32 }

/// `pack_format_info`: the directive table.
fn format_info(c: u8) -> (Dir, Type, usize, u32) {
    match c {
        b'A' => (Dir::Str, Type::String, 0, F_WIDTH | F_COUNT2),
        b'a' => (Dir::Str, Type::String, 0, F_WIDTH | F_COUNT2 | F_A),
        b'B' => (Dir::Bstr, Type::String, 0, F_COUNT2),
        b'b' => (Dir::Bstr, Type::String, 0, F_COUNT2 | F_LSB),
        b'C' => (Dir::Char, Type::Integer, 1, 0),
        b'c' => (Dir::Char, Type::Integer, 1, F_SIGNED),
        b'D' | b'd' => (Dir::Double, Type::Float, 8, F_SIGNED),
        b'E' => (Dir::Double, Type::Float, 8, F_SIGNED | F_LT),
        b'e' => (Dir::Float, Type::Float, 4, F_SIGNED | F_LT),
        b'F' | b'f' => (Dir::Float, Type::Float, 4, F_SIGNED),
        b'G' => (Dir::Double, Type::Float, 8, F_SIGNED | F_GT),
        b'g' => (Dir::Float, Type::Float, 4, F_SIGNED | F_GT),
        b'H' => (Dir::Hex, Type::String, 0, F_COUNT2),
        b'h' => (Dir::Hex, Type::String, 0, F_COUNT2 | F_LSB),
        b'L' => (Dir::Long, Type::Integer, 4, 0),
        b'l' => (Dir::Long, Type::Integer, 4, F_SIGNED),
        b'M' => (Dir::Qenc, Type::String, 0, F_WIDTH | F_COUNT2),
        b'm' => (Dir::Base64, Type::String, 0, F_WIDTH | F_COUNT2),
        b'N' => (Dir::Long, Type::Integer, 4, F_GT),
        b'n' => (Dir::Short, Type::Integer, 2, F_GT),
        b'Q' => (Dir::Quad, Type::Integer, 8, 0),
        b'q' => (Dir::Quad, Type::Integer, 8, F_SIGNED),
        b'S' => (Dir::Short, Type::Integer, 2, 0),
        b's' => (Dir::Short, Type::Integer, 2, F_SIGNED),
        b'u' => (Dir::Uu, Type::String, 0, F_WIDTH | F_COUNT2),
        b'U' => (Dir::Utf8, Type::Integer, 0, 0),
        b'V' => (Dir::Long, Type::Integer, 4, F_LT),
        b'v' => (Dir::Short, Type::Integer, 2, F_LT),
        b'w' => (Dir::Ber, Type::Integer, 0, F_SIGNED),
        b'x' => (Dir::Nul, Type::None, 0, 0),
        b'X' => (Dir::Back, Type::None, 0, 0),
        b'@' => (Dir::Abs, Type::None, 0, 0),
        b'Z' => (Dir::Str, Type::String, 0, F_WIDTH | F_COUNT2 | F_Z),
        _ => (Dir::None, Type::None, 0, 0),
    }
}

/// `read_tmpl`: one directive with its modifiers and count.
fn read_tmpl(vm: &mut Vm, t: &mut Tmpl) -> VmResult<Spec> {
    let mut count: i64 = 1;
    let (dir, ty, size, mut flags);
    loop {
        if t.idx >= t.s.len() { return Ok(Spec { dir: Dir::None, ty: Type::None, size: 0, count, flags: 0 }); }
        let mut c = t.s[t.idx];
        t.idx += 1;
        if is_space(c) { continue; }
        // `i`/`I` follow C's `int` (4 bytes), `j`/`J` its `intptr_t` (8)
        c = match c {
            b'I' => b'L', b'i' => b'l',
            b'J' => b'Q', b'j' => b'q',
            b'#' => { while t.idx < t.s.len() && t.s[t.idx] != b'\n' { t.idx += 1; } continue; }
            b'p' | b'P' | b'%' => return Err(vm.raise_arg(&format!("{} is not supported", c as char))),
            c => c,
        };
        let info = format_info(c);
        if info.0 == Dir::None {
            let s = vm.str_new(&[c]);
            let d = vm.inspect_str(s).unwrap_or_default();
            return Err(vm.raise_arg(&format!("unknown unpack directive {d}")));
        }
        (dir, ty, size, flags) = info;
        // the suffix [0-9*_!<>]
        while t.idx < t.s.len() {
            let ch = t.s[t.idx];
            if ch.is_ascii_digit() {
                let mut n: i64 = 0;
                while t.idx < t.s.len() && t.s[t.idx].is_ascii_digit() {
                    n = match n.checked_mul(10).and_then(|n| n.checked_add((t.s[t.idx] - b'0') as i64)) {
                        Some(n) if n <= i32::MAX as i64 => n,
                        _ => return Err(vm.raise(vm.core.runtime_error, "too big template length")),
                    };
                    t.idx += 1;
                }
                count = n;
                continue;
            } else if ch == b'*' {
                count = if ty == Type::None { 0 } else { -1 };
            } else if matches!(ch, b'_' | b'!' | b'<' | b'>') {
                if !b"sSiIlLqQ".contains(&c) {
                    return Err(vm.raise_arg(&format!("'{}' allowed only after types sSiIlLqQ", ch as char)));
                }
                match ch { b'<' => flags |= F_LT, b'>' => flags |= F_GT, _ => {} }
            } else {
                break;
            }
            t.idx += 1;
        }
        break;
    }
    if flags & F_LT != 0 || (flags & F_GT == 0 && cfg!(target_endian = "little")) {
        flags |= F_LE;
    }
    Ok(Spec { dir, ty, size, count, flags })
}

// ------------------------------------------------------------------ pack

fn ensure(out: &mut Vec<u8>, len: usize) {
    if out.len() < len { out.resize(len, 0); }
}

fn put_int(out: &mut Vec<u8>, at: usize, n: u64, size: usize, le: bool) {
    ensure(out, at + size);
    for i in 0..size {
        let byte = (n >> (8 * i)) as u8;
        out[at + if le { i } else { size - 1 - i }] = byte;
    }
}

/// `mrb_ensure_int_type` on the argument: a Float truncates, a Rational truncates, an
/// Integer too wide for the operation is a RangeError, and anything else is a TypeError
/// with the reference's wording.
fn int_arg(vm: &mut Vm, o: Value) -> VmResult<i64> {
    match o {
        Value::Int(i) => Ok(i),
        Value::Float(_) => vm.expect_int(o, "pack"),
        Value::Obj(_) if vm.is_bigint(o) || super::ext_rational::is_rational(vm, o) || super::ext_complex::is_complex(vm, o) => vm.expect_int(o, "pack"),
        _ => { let d = vm.describe_for_type_error(o); Err(vm.raise_type(&format!("{d} cannot be converted to Integer"))) }
    }
}

fn pack_ber(vm: &mut Vm, n: i64, out: &mut Vec<u8>, at: usize) -> VmResult<usize> {
    if n < 0 { return Err(vm.raise_arg("can't compress negative numbers")); }
    let mut len = 1;
    while len < 9 {
        let shift = 7 * len as u32;
        if shift >= 63 || (n >> shift) == 0 { break; }
        len += 1;
    }
    ensure(out, at + len);
    for j in (1..=len).rev() {
        let mut x = ((n >> (7 * (j - 1))) & 0x7f) as u8;
        if j > 1 { x |= 0x80; }
        out[at + len - j] = x;
    }
    Ok(len)
}

fn utf8_to_buf(cp: i64) -> Option<Vec<u8>> {
    if cp < 0 { return None; }
    Some(match cp {
        0..=0x7f => vec![cp as u8],
        0x80..=0x7ff => vec![0xC0 | (cp >> 6) as u8, 0x80 | (cp & 0x3F) as u8],
        0x800..=0xffff => vec![0xE0 | (cp >> 12) as u8, 0x80 | ((cp >> 6) & 0x3F) as u8, 0x80 | (cp & 0x3F) as u8],
        0x10000..=0x10FFFF => vec![0xF0 | (cp >> 18) as u8, 0x80 | ((cp >> 12) & 0x3F) as u8, 0x80 | ((cp >> 6) & 0x3F) as u8, 0x80 | (cp & 0x3F) as u8],
        _ => return None,
    })
}

fn pack_str(src: &[u8], out: &mut Vec<u8>, at: usize, count: i64, flags: u32) -> usize {
    let pad = if flags & (F_A | F_Z) != 0 { 0u8 } else { b' ' };
    let (copy, padlen) = if count == 0 {
        return 0;
    } else if count == -1 {
        (src.len(), if flags & F_Z != 0 { 1 } else { 0 })
    } else if (count as usize) < src.len() {
        (count as usize, 0)
    } else {
        (src.len(), count as usize - src.len())
    };
    ensure(out, at + copy + padlen);
    out[at..at + copy].copy_from_slice(&src[..copy]);
    for i in 0..padlen { out[at + copy + i] = pad; }
    copy + padlen
}

fn pack_hex(src: &[u8], out: &mut Vec<u8>, at: usize, count: i64, flags: u32) -> usize {
    let (ashift, bshift) = if flags & F_LSB != 0 { (0, 4) } else { (4, 0) };
    let mut slen = src.len();
    let count = if count == -1 { slen as i64 } else { if slen as i64 > count { slen = count as usize; } count };
    ensure(out, at + (count as usize / 2 + (count as usize & 1)));
    let mut w = 0;
    let mut i = 0;
    while slen >= 2 {
        let a = hex2int(src[i]);
        if a < 0 { break; }
        let b = hex2int(src[i + 1]);
        if b < 0 { break; }
        out[at + w] = ((a << ashift) + (b << bshift)) as u8;
        w += 1;
        i += 2;
        slen -= 2;
    }
    if slen > 0 {
        let a = hex2int(src[i]);
        if a >= 0 { out[at + w] = (a << ashift) as u8; w += 1; }
    }
    w
}

fn pack_bstr(src: &[u8], out: &mut Vec<u8>, at: usize, count: i64, flags: u32) -> usize {
    let mut slen = src.len();
    if count != -1 && slen as i64 > count { slen = count as usize; }
    let bytes = slen.div_ceil(8);
    ensure(out, at + bytes);
    for i in 0..bytes {
        let mut byte = 0u8;
        for j in 0..8 {
            let bit = src.get(i * 8 + j).map(|c| if *c == b'1' { 1u8 } else { 0 }).filter(|_| i * 8 + j < slen).unwrap_or(0);
            if flags & F_LSB != 0 { byte |= bit << j; } else { byte |= bit << (7 - j); }
        }
        out[at + i] = byte;
    }
    bytes
}

fn pack_base64(src: &[u8], out: &mut Vec<u8>, at: usize, count: i64) -> usize {
    if src.is_empty() { return 0; }
    let count = if count != 0 && count < 3 { 45 } else if count >= 3 { count - count % 3 } else { 0 };
    let mut buf: Vec<u8> = Vec::new();
    let mut column = 3i64;
    let mut i = 0;
    while src.len() - i >= 3 {
        let l = ((src[i] as u32) << 16) | ((src[i + 1] as u32) << 8) | src[i + 2] as u32;
        i += 3;
        for sh in [18, 12, 6, 0] { buf.push(BASE64[((l >> sh) & 0x3f) as usize]); }
        if count > 0 {
            if column == count { buf.push(b'\n'); column = 0; }
            column += 3;
        }
    }
    match src.len() - i {
        1 => {
            let l = (src[i] as u32) << 16;
            buf.push(BASE64[((l >> 18) & 0x3f) as usize]);
            buf.push(BASE64[((l >> 12) & 0x3f) as usize]);
            buf.extend_from_slice(b"==");
            column += 3;
        }
        2 => {
            let l = ((src[i] as u32) << 16) | ((src[i + 1] as u32) << 8);
            for sh in [18, 12, 6] { buf.push(BASE64[((l >> sh) & 0x3f) as usize]); }
            buf.push(b'=');
            column += 3;
        }
        _ => {}
    }
    if count > 0 && column > 0 { buf.push(b'\n'); }
    ensure(out, at + buf.len());
    out[at..at + buf.len()].copy_from_slice(&buf);
    buf.len()
}

fn pack_uu(src: &[u8], out: &mut Vec<u8>, at: usize, count: i64) -> usize {
    let mut count = if count <= 2 { 45 } else if count > 63 { 63 } else { count } as usize;
    count -= count % 3;
    let mut buf: Vec<u8> = Vec::new();
    let mut s = src;
    let mut lines = 0;
    while !s.is_empty() {
        let line = s.len().min(count);
        buf.push(UU[line & 0x3F]);
        let mut p = 0;
        while p < line {
            let mut group: u32 = 0;
            let mut n = 0;
            while n < 3 && p < line { group = (group << 8) | s[p] as u32; p += 1; n += 1; }
            group <<= (3 - n) * 8;
            for sh in [18, 12, 6, 0] { buf.push(UU[((group >> sh) & 0x3F) as usize]); }
        }
        buf.push(b'\n');
        s = &s[line..];
        lines += 1;
    }
    if lines > 0 { buf.push(UU[0]); buf.push(b'\n'); }
    ensure(out, at + buf.len());
    out[at..at + buf.len()].copy_from_slice(&buf);
    buf.len()
}

fn pack_qenc(src: &[u8], out: &mut Vec<u8>, at: usize, count: i64) -> usize {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let count = if count <= 1 { 72 } else { count };
    let mut buf: Vec<u8> = Vec::new();
    let mut n: i64 = 0;
    let mut prev: i32 = -1;
    for (i, &byte) in src.iter().enumerate() {
        match qprint_type(byte) {
            0 => {
                if (byte == b' ' || byte == b'\t') && src.get(i + 1) == Some(&b'\n') {
                    buf.push(b'=');
                    buf.push(HEX[(byte >> 4) as usize]);
                    buf.push(HEX[(byte & 0xf) as usize]);
                    n += 3;
                    prev = -1;
                } else {
                    buf.push(byte);
                    n += 1;
                    prev = byte as i32;
                }
            }
            2 => {
                if prev == b' ' as i32 || prev == b'\t' as i32 { buf.push(b'='); buf.push(byte); }
                buf.push(byte);
                n = 0;
                prev = byte as i32;
            }
            _ => {
                buf.push(b'=');
                buf.push(HEX[(byte >> 4) as usize]);
                buf.push(HEX[(byte & 0xf) as usize]);
                n += 3;
                prev = -1;
            }
        }
        if n > count { buf.push(b'='); buf.push(b'\n'); n = 0; prev = b'\n' as i32; }
    }
    if n > 0 { buf.push(b'='); buf.push(b'\n'); }
    ensure(out, at + buf.len());
    out[at..at + buf.len()].copy_from_slice(&buf);
    buf.len()
}

fn pack(vm: &mut Vm, s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    argc!(vm, a, 1);
    let tmpl = match vm.str_bytes(a[0]) { Some(b) => b.to_vec(), None => { let d = vm.describe_for_type_error(a[0]); return Err(vm.raise_type(&format!("{d} cannot be converted to String"))); } };
    let items: Vec<Value> = vm.ary(s).map(|v| v.iter().map(|x| x.get()).collect()).unwrap_or_default();
    let mut t = Tmpl { s: &tmpl, idx: 0 };
    let mut out: Vec<u8> = Vec::new();
    let mut ridx: i64 = 0;
    let mut aidx = 0usize;
    while t.idx < t.s.len() {
        let sp = read_tmpl(vm, &mut t)?;
        let mut count = sp.count;
        match sp.dir {
            Dir::None => break,
            Dir::Nul => { ensure(&mut out, (ridx + count) as usize); ridx += count; continue; }
            Dir::Back => { check_x(vm, ridx, count, 'X')?; ridx -= count; continue; }
            Dir::Abs => {
                count -= ridx;
                if count > 0 { ensure(&mut out, (ridx + count) as usize); ridx += count; continue; }
                count = -count;
                check_x(vm, ridx, count, '@')?;
                ridx -= count;
                continue;
            }
            _ => {}
        }
        if sp.flags & F_WIDTH != 0 && aidx >= items.len() {
            return Err(vm.raise_arg("too few arguments"));
        }
        while aidx < items.len() {
            if count == 0 && sp.flags & F_WIDTH == 0 { break; }
            let o = items[aidx];
            let at = ridx as usize;
            let written = match sp.ty {
                Type::Integer => {
                    let n = int_arg(vm, o)?;
                    match sp.dir {
                        Dir::Char => { put_int(&mut out, at, n as u64, 1, true); 1 }
                        Dir::Short => { put_int(&mut out, at, n as u64, 2, sp.flags & F_LE != 0); 2 }
                        Dir::Long => { put_int(&mut out, at, n as u64, 4, sp.flags & F_LE != 0); 4 }
                        Dir::Quad => { put_int(&mut out, at, n as u64, 8, sp.flags & F_LE != 0); 8 }
                        Dir::Ber => pack_ber(vm, n, &mut out, at)?,
                        Dir::Utf8 => {
                            let bytes = match utf8_to_buf(n) { Some(b) => b, None => return Err(vm.raise(vm.core.range_error, "pack(U): value out of range")) };
                            ensure(&mut out, at + bytes.len());
                            out[at..at + bytes.len()].copy_from_slice(&bytes);
                            bytes.len()
                        }
                        _ => 0,
                    }
                }
                Type::Float => {
                    let f = match super::numeric::num_f64(vm, o) {
                        Some(f) if !matches!(o, Value::Obj(_)) || vm.is_bigint(o) => f,
                        _ => { let d = vm.describe_for_type_error(o); return Err(vm.raise_type(&format!("{d} cannot be converted to Float"))); }
                    };
                    match sp.dir {
                        Dir::Double => { put_int(&mut out, at, f.to_bits(), 8, sp.flags & F_LE != 0); 8 }
                        _ => { put_int(&mut out, at, (f as f32).to_bits() as u64, 4, sp.flags & F_LE != 0); 4 }
                    }
                }
                Type::String => {
                    let src = match vm.str_bytes(o) { Some(b) => b.to_vec(), None => { let d = vm.describe_for_error(o); return Err(vm.raise_type(&format!("can't convert {d} into String"))); } };
                    match sp.dir {
                        Dir::Str => pack_str(&src, &mut out, at, count, sp.flags),
                        Dir::Hex => pack_hex(&src, &mut out, at, count, sp.flags),
                        Dir::Bstr => pack_bstr(&src, &mut out, at, count, sp.flags),
                        Dir::Base64 => pack_base64(&src, &mut out, at, count),
                        Dir::Uu => pack_uu(&src, &mut out, at, count),
                        Dir::Qenc => pack_qenc(&src, &mut out, at, count),
                        _ => 0,
                    }
                }
                Type::None => 0,
            };
            ridx += written as i64;
            if sp.flags & F_COUNT2 != 0 { aidx += 1; break; }
            aidx += 1;
            if count > 0 { count -= 1; }
        }
        if ridx < 0 { return Err(vm.raise(vm.core.range_error, "negative (or overflowed) template size")); }
    }
    out.truncate(ridx as usize);
    Ok(vm.str_new(&out))
}

fn check_x(vm: &mut Vm, a: i64, count: i64, c: char) -> VmResult<()> {
    if a < count { return Err(vm.raise_arg(&format!("{c} outside of string"))); }
    Ok(())
}

// ------------------------------------------------------------------ unpack

fn get_int(src: &[u8], size: usize, le: bool) -> u64 {
    let mut n: u64 = 0;
    for i in 0..size {
        let byte = src[if le { i } else { size - 1 - i }] as u64;
        n |= byte << (8 * i);
    }
    n
}

fn unpack_str(vm: &mut Vm, src: &[u8], count: i64, flags: u32, out: &mut Vec<Value>) -> usize {
    if src.is_empty() { let v = vm.str_new(&[]); out.push(v); return 0; }
    let mut slen = src.len();
    if count != -1 && (count as usize) < slen { slen = count as usize; }
    let mut copylen = slen;
    if flags & F_Z != 0 {
        if let Some(p) = src[..slen].iter().position(|c| *c == 0) {
            copylen = p;
            if count == -1 { slen = copylen + 1; }
        }
    } else if flags & F_A == 0 {
        while copylen > 0 && matches!(src[copylen - 1], 0 | b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r') { copylen -= 1; }
    }
    let v = vm.str_new(&src[..copylen]);
    out.push(v);
    slen
}

fn unpack_hex(vm: &mut Vm, src: &[u8], count: i64, flags: u32, out: &mut Vec<Value>) -> usize {
    if src.is_empty() { let v = vm.str_new(&[]); out.push(v); return 0; }
    let (ashift, bshift) = if flags & F_LSB != 0 { (0, 4) } else { (4, 0) };
    let mut count = if count == -1 { (src.len() * 2) as i64 } else { count };
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = 0;
    let mut dst: Vec<u8> = Vec::new();
    while s < src.len() && count >= 2 {
        let byte = src[s];
        s += 1;
        dst.push(HEX[((byte >> ashift) & 0x0f) as usize]);
        dst.push(HEX[((byte >> bshift) & 0x0f) as usize]);
        count -= 2;
    }
    if s < src.len() && count > 0 {
        let byte = src[s];
        s += 1;
        dst.push(HEX[((byte >> ashift) & 0x0f) as usize]);
    }
    let v = vm.str_new(&dst);
    out.push(v);
    s
}

fn unpack_bstr(vm: &mut Vm, src: &[u8], count: i64, flags: u32, out: &mut Vec<Value>) -> usize {
    if src.is_empty() { let v = vm.str_new(&[]); out.push(v); return 0; }
    let count = if count == -1 || count > (src.len() * 8) as i64 { (src.len() * 8) as usize } else { count as usize };
    let mut dst: Vec<u8> = Vec::with_capacity(count);
    for i in 0..count {
        let byte = src[i / 8];
        let bit = if flags & F_LSB != 0 { (byte >> (i % 8)) & 1 } else { (byte >> (7 - i % 8)) & 1 };
        dst.push(b'0' + bit);
    }
    let v = vm.str_new(&dst);
    out.push(v);
    count.div_ceil(8)
}

fn unpack_base64(vm: &mut Vm, src: &[u8], out: &mut Vec<Value>) -> usize {
    if src.is_empty() { let v = vm.str_new(&[]); out.push(v); return 0; }
    let mut dst: Vec<u8> = Vec::new();
    let mut s = 0usize;
    let mut padding = 0;
    'outer: while src.len() - s >= 4 {
        let mut ch = [0u8; 4];
        for c in ch.iter_mut() {
            loop {
                if s >= src.len() { break 'outer; }
                let b = src[s];
                s += 1;
                if b == b'=' { *c = 0; padding += 1; break; }
                if let Some(v) = base64_val(b) { *c = v; break; }
            }
        }
        let l = ((ch[0] as u32) << 18) + ((ch[1] as u32) << 12) + ((ch[2] as u32) << 6) + ch[3] as u32;
        dst.push((l >> 16) as u8);
        if padding >= 2 { break; }
        dst.push((l >> 8) as u8);
        if padding == 1 { break; }
        dst.push(l as u8);
    }
    let v = vm.str_new(&dst);
    out.push(v);
    s
}

fn unpack_uu(vm: &mut Vm, src: &[u8], out: &mut Vec<Value>) -> usize {
    let mut dst: Vec<u8> = Vec::new();
    let mut s = 0usize;
    let dec = |c: u8| -> i8 { if (32..128).contains(&c) { if c == b'`' { 0 } else if c < 96 { (c - 32) as i8 } else { -1 } } else { -1 } };
    while s < src.len() {
        while s < src.len() && matches!(src[s], b'\n' | b'\r' | b' ' | b'\t') { s += 1; }
        if s >= src.len() { break; }
        let line_len = match dec(src[s]) { n if n >= 0 => { s += 1; n as usize } _ => break };
        if line_len == 0 { break; }
        let mut done = 0;
        while done < line_len && s + 3 < src.len() {
            let mut c = [0i32; 4];
            let mut valid = true;
            for ci in c.iter_mut() {
                if s >= src.len() || dec(src[s]) < 0 { valid = false; break; }
                *ci = dec(src[s]) as i32;
                s += 1;
            }
            if !valid { break; }
            let group = ((c[0] as u32) << 18) | ((c[1] as u32) << 12) | ((c[2] as u32) << 6) | c[3] as u32;
            let take = (line_len - done).min(3);
            for i in 0..take { dst.push((group >> (16 - i * 8)) as u8); done += 1; }
        }
        while s < src.len() && src[s] != b'\n' && src[s] != b'\r' { s += 1; }
    }
    let v = vm.str_new(&dst);
    out.push(v);
    src.len()
}

fn unpack_qenc(vm: &mut Vm, src: &[u8], out: &mut Vec<Value>) -> usize {
    if src.is_empty() { let v = vm.str_new(&[]); out.push(v); return 0; }
    let mut dst: Vec<u8> = Vec::new();
    let mut s = 0usize;
    while s < src.len() {
        if src[s] != b'=' { dst.push(src[s]); s += 1; continue; }
        s += 1;
        if s == src.len() { break; }
        if s + 1 < src.len() && src[s] == b'\r' && src[s + 1] == b'\n' { s += 2; continue; }
        if src[s] == b'\n' { s += 1; continue; }
        let c1 = hex2int(src[s]);
        if c1 == -1 { break; }
        s += 1;
        if s == src.len() { break; }
        let c2 = hex2int(src[s]);
        if c2 == -1 { break; }
        dst.push(((c1 << 4) | c2) as u8);
        s += 1;
    }
    let v = vm.str_new(&dst);
    out.push(v);
    src.len()
}

/// `unpack_utf8`: one code point, or the reference's two ArgumentErrors.
fn unpack_utf8(vm: &mut Vm, src: &[u8], out: &mut Vec<Value>) -> VmResult<usize> {
    if src.is_empty() { return Ok(1); }
    let first = src[0];
    if first < 0x80 { out.push(Value::Int(first as i64)); return Ok(1); }
    let malformed = |vm: &mut Vm| -> VmError { vm.raise_arg("malformed UTF-8 character") };
    let redundant = |vm: &mut Vm| -> VmError { vm.raise_arg("redundant UTF-8 sequence") };
    let len = match first { 0xC0..=0xDF => 2, 0xE0..=0xEF => 3, 0xF0..=0xF7 => 4, _ => return Err(malformed(vm)) };
    if len > src.len() { return Err(malformed(vm)); }
    if src[1..len].iter().any(|b| !(0x80..=0xBF).contains(b)) { return Err(malformed(vm)); }
    let uv: u32 = match len {
        2 => (((first & 0x1F) as u32) << 6) | (src[1] & 0x3F) as u32,
        3 => (((first & 0x0F) as u32) << 12) | (((src[1] & 0x3F) as u32) << 6) | (src[2] & 0x3F) as u32,
        _ => (((first & 0x07) as u32) << 18) | (((src[1] & 0x3F) as u32) << 12) | (((src[2] & 0x3F) as u32) << 6) | (src[3] & 0x3F) as u32,
    };
    let min = match len { 2 => 0x80, 3 => 0x800, _ => 0x10000 };
    if uv < min { return Err(redundant(vm)); }
    if len == 4 && uv > 0x10FFFF { return Err(malformed(vm)); }
    out.push(Value::Int(uv as i64));
    Ok(len)
}

fn unpack_ber(src: &[u8], out: &mut Vec<Value>) -> Result<usize, ()> {
    if src.is_empty() { return Ok(0); }
    let mut n: i64 = 0;
    let mut i = 0;
    for &b in src {
        i += 1;
        if i > 9 || n > (i64::MAX >> 7) { return Err(()); }
        n = (n << 7) | (b & 0x7f) as i64;
        if b & 0x80 == 0 { break; }
    }
    out.push(Value::Int(n));
    Ok(i)
}

fn unpack(vm: &mut Vm, s: Value, a: &[Value], single: bool) -> VmResult<Value> {
    argc!(vm, a, 1);
    let tmpl = match vm.str_bytes(a[0]) { Some(b) => b.to_vec(), None => { let d = vm.describe_for_type_error(a[0]); return Err(vm.raise_type(&format!("{d} cannot be converted to String"))); } };
    let src = match vm.str_bytes(s) { Some(b) => b.to_vec(), None => Vec::new() };
    let mut t = Tmpl { s: &tmpl, idx: 0 };
    let mut out: Vec<Value> = Vec::new();
    let mut srcidx: i64 = 0;
    let srclen = src.len() as i64;
    'tmpl: while t.idx < t.s.len() {
        let sp = read_tmpl(vm, &mut t)?;
        let mut count = sp.count;
        match sp.dir {
            Dir::None => break,
            Dir::Nul => { check_x(vm, srclen - srcidx, count, 'x')?; srcidx += count; continue; }
            Dir::Back => { check_x(vm, srcidx, count, 'X')?; srcidx -= count; continue; }
            Dir::Abs => { check_x(vm, srclen, count, '@')?; srcidx = count; continue; }
            _ => {}
        }
        let at = srcidx.clamp(0, srclen) as usize;
        let rest = &src[at..];
        let used = match sp.dir {
            Dir::Hex => Some(unpack_hex(vm, rest, count, sp.flags, &mut out)),
            Dir::Bstr => Some(unpack_bstr(vm, rest, count, sp.flags, &mut out)),
            Dir::Str => Some(unpack_str(vm, rest, count, sp.flags, &mut out)),
            Dir::Base64 => Some(unpack_base64(vm, rest, &mut out)),
            Dir::Uu => Some(unpack_uu(vm, rest, &mut out)),
            Dir::Qenc => Some(unpack_qenc(vm, rest, &mut out)),
            _ => None,
        };
        if let Some(n) = used {
            srcidx += n as i64;
            if single { break; }
            continue;
        }
        while count != 0 && srcidx < srclen {
            if srclen - srcidx < sp.size as i64 {
                while count > 0 { out.push(Value::Nil); count -= 1; }
                continue 'tmpl;
            }
            let at = srcidx as usize;
            let rest = &src[at..];
            match sp.dir {
                Dir::Char => {
                    let n = rest[0];
                    out.push(Value::Int(if sp.flags & F_SIGNED != 0 { n as i8 as i64 } else { n as i64 }));
                    srcidx += 1;
                }
                Dir::Short => {
                    let n = get_int(rest, 2, sp.flags & F_LE != 0) as u16;
                    out.push(Value::Int(if sp.flags & F_SIGNED != 0 { n as i16 as i64 } else { n as i64 }));
                    srcidx += 2;
                }
                Dir::Long => {
                    let n = get_int(rest, 4, sp.flags & F_LE != 0) as u32;
                    out.push(Value::Int(if sp.flags & F_SIGNED != 0 { n as i32 as i64 } else { n as i64 }));
                    srcidx += 4;
                }
                Dir::Quad => {
                    let n = get_int(rest, 8, sp.flags & F_LE != 0);
                    if sp.flags & F_SIGNED != 0 {
                        out.push(Value::Int(n as i64));
                    } else {
                        // the reference refuses what does not fit an Integer, bigint or not
                        if n > i64::MAX as u64 { return Err(vm.raise(vm.core.range_error, &format!("cannot unpack to Integer: {n}"))); }
                        out.push(Value::Int(n as i64));
                    }
                    srcidx += 8;
                }
                Dir::Ber => {
                    match unpack_ber(rest, &mut out) {
                        Ok(n) => srcidx += n as i64,
                        Err(()) => return Err(vm.raise(vm.core.range_error, "BER unpacking 'w' overflow")),
                    }
                }
                Dir::Float => {
                    let n = get_int(rest, 4, sp.flags & F_LE != 0) as u32;
                    out.push(Value::Float(f32::from_bits(n) as f64));
                    srcidx += 4;
                }
                Dir::Double => {
                    let n = get_int(rest, 8, sp.flags & F_LE != 0);
                    out.push(Value::Float(f64::from_bits(n)));
                    srcidx += 8;
                }
                Dir::Utf8 => { srcidx += unpack_utf8(vm, rest, &mut out)? as i64; }
                _ => return Err(vm.raise(vm.core.runtime_error, "mruby-pack's bug")),
            }
            if count > 0 { count -= 1; }
        }
    }
    if single {
        return Ok(out.first().copied().unwrap_or(Value::Nil));
    }
    Ok(vm.ary_new(out))
}

pub fn init(vm: &mut Vm) {
    vm.define_method(vm.core.array, "pack", pack);
    vm.define_method(vm.core.string, "unpack", |vm, s, a, _b| unpack(vm, s, a, false));
    vm.define_method(vm.core.string, "unpack1", |vm, s, a, _b| unpack(vm, s, a, true));
    let _ = String::new();
}
