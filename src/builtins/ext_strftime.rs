//! mruby-strftime (`mrbgems/mruby-strftime/src/strftime.c`): `Time#strftime`.
//!
//! The reference hands the format to the platform's `strftime(3)`; a `no_std` VM has no C
//! library, so the conversions are written out here against the broken-down time
//! `ext_time.rs` already computes. What they mean is the C locale's, as glibc reads it, and the
//! answers were checked one by one against it (`docs/design/gems.md`).

use alloc::{format, string::String, vec::Vec};

use crate::argc;
use crate::error::VmResult;
use crate::value::Value;
use crate::vm::Vm;

use super::ext_time::{gmtime, seconds_of, DateTime, MON_NAMES, WDAY_NAMES};

const WDAY_FULL: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const MON_FULL: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July",
                              "August", "September", "October", "November", "December"];

/// How a conversion pads what it writes.
#[derive(Clone, Copy, PartialEq)]
enum Pad { Zero, Space, None }

fn put(out: &mut Vec<u8>, n: i64, width: usize, pad: Pad) {
    let text = format!("{}", n.abs());
    let sign = if n < 0 { "-" } else { "" };
    let len = sign.len() + text.len();
    if len < width && pad != Pad::None {
        let fill = if pad == Pad::Zero { b'0' } else { b' ' };
        // a zero-padded negative number keeps its sign in front of the zeros
        if pad == Pad::Zero { out.extend_from_slice(sign.as_bytes()); }
        for _ in 0..width - len { out.push(fill); }
        if pad == Pad::Space { out.extend_from_slice(sign.as_bytes()); }
    } else {
        out.extend_from_slice(sign.as_bytes());
    }
    out.extend_from_slice(text.as_bytes());
}

/// One conversion, into `out`. `None` says the letter names none, which glibc copies through as
/// it stands (`%` and all).
#[allow(clippy::too_many_arguments)]
fn convert(out: &mut Vec<u8>, c: u8, d: &DateTime, secs: i64, flag: Option<Pad>, width: Option<usize>) -> Option<()> {
    let w = |dflt: usize| width.unwrap_or(dflt);
    // a conversion pads the way it does unless a flag said otherwise
    let pad = flag.unwrap_or(Pad::Zero);
    match c {
        b'Y' => put(out, d.year, w(4), pad),
        b'C' => put(out, d.year.div_euclid(100), w(2), pad),
        b'y' => put(out, d.year.rem_euclid(100), w(2), pad),
        b'm' => put(out, d.mon as i64, w(2), pad),
        b'B' => out.extend_from_slice(MON_FULL[(d.mon - 1) as usize].as_bytes()),
        b'b' | b'h' => out.extend_from_slice(MON_NAMES[(d.mon - 1) as usize].as_bytes()),
        b'd' => put(out, d.mday as i64, w(2), pad),
        // `%e` is the day space-padded, which is `%d` with the other filler
        b'e' => put(out, d.mday as i64, w(2), flag.unwrap_or(Pad::Space)),
        b'j' => put(out, d.yday as i64, w(3), pad),
        b'H' => put(out, d.hour as i64, w(2), pad),
        b'I' => put(out, match d.hour % 12 { 0 => 12, h => h } as i64, w(2), pad),
        b'M' => put(out, d.min as i64, w(2), pad),
        b'S' => put(out, d.sec as i64, w(2), pad),
        b'p' => out.extend_from_slice(if d.hour < 12 { b"AM" } else { b"PM" }),
        b'A' => out.extend_from_slice(WDAY_FULL[d.wday as usize].as_bytes()),
        b'a' => out.extend_from_slice(WDAY_NAMES[d.wday as usize].as_bytes()),
        b'w' => put(out, d.wday as i64, w(1), flag.unwrap_or(Pad::None)),
        // `%u` counts from Monday, and Sunday is 7 rather than 0
        b'u' => put(out, if d.wday == 0 { 7 } else { d.wday as i64 }, w(1), flag.unwrap_or(Pad::None)),
        // the zone is what `Time` decided: this VM has no time zone database (`docs/design/gems.md`)
        b'Z' => out.extend_from_slice(b"UTC"),
        b'z' => out.extend_from_slice(b"+0000"),
        b's' => put(out, secs, w(1), flag.unwrap_or(Pad::None)),
        b'n' => out.push(b'\n'),
        b't' => out.push(b'\t'),
        b'%' => out.push(b'%'),
        // the compound ones, which are the C locale's
        b'F' => { convert(out, b'Y', d, secs, flag, None)?; out.push(b'-'); convert(out, b'm', d, secs, flag, None)?; out.push(b'-'); convert(out, b'd', d, secs, flag, None)?; }
        b'T' | b'X' => { convert(out, b'H', d, secs, flag, None)?; out.push(b':'); convert(out, b'M', d, secs, flag, None)?; out.push(b':'); convert(out, b'S', d, secs, flag, None)?; }
        b'R' => { convert(out, b'H', d, secs, flag, None)?; out.push(b':'); convert(out, b'M', d, secs, flag, None)?; }
        b'D' | b'x' => { convert(out, b'm', d, secs, flag, None)?; out.push(b'/'); convert(out, b'd', d, secs, flag, None)?; out.push(b'/'); convert(out, b'y', d, secs, flag, None)?; }
        b'c' => {
            convert(out, b'a', d, secs, None, None)?; out.push(b' ');
            convert(out, b'b', d, secs, None, None)?; out.push(b' ');
            convert(out, b'e', d, secs, None, None)?; out.push(b' ');
            convert(out, b'T', d, secs, None, None)?; out.push(b' ');
            convert(out, b'Y', d, secs, Some(Pad::None), None)?;
        }
        _ => return None,
    }
    Some(())
}

/// `Time#strftime` (`mrb_time_strftime`). A NUL in the format is a byte like any other and comes
/// out where it stood, which is what the reference's per-segment loop amounts to.
fn strftime(vm: &mut Vm, s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    argc!(vm, a, 1);
    let fmt = vm.expect_str(a[0], "format")?;
    let secs = seconds_of(vm, s)?;
    let d = gmtime(secs);
    let mut out: Vec<u8> = Vec::with_capacity(fmt.len() + 16);
    let mut i = 0usize;
    while i < fmt.len() {
        if fmt[i] != b'%' { out.push(fmt[i]); i += 1; continue; }
        let start = i;
        let mut j = i + 1;
        // glibc's flags, then an optional field width
        let mut pad: Option<Pad> = None;
        while let Some(&f) = fmt.get(j) {
            match f {
                b'-' => pad = Some(Pad::None),
                b'_' => pad = Some(Pad::Space),
                b'0' => pad = Some(Pad::Zero),
                _ => break,
            }
            j += 1;
        }
        let ws = j;
        while fmt.get(j).map(|c| c.is_ascii_digit()).unwrap_or(false) { j += 1; }
        let width = if j > ws { core::str::from_utf8(&fmt[ws..j]).ok().and_then(|t| t.parse().ok()) } else { None };
        match fmt.get(j) {
            // a conversion the C library does not know is copied through as written
            Some(&c) => match convert(&mut out, c, &d, secs, pad, width) {
                Some(()) => i = j + 1,
                None => { out.extend_from_slice(&fmt[start..=j]); i = j + 1; }
            },
            None => { out.extend_from_slice(&fmt[start..]); i = fmt.len(); }
        }
    }
    Ok(vm.str_new(&out))
}

pub fn init(vm: &mut Vm) {
    let tn = vm.intern("Time");
    if let Some(Value::Obj(time)) = vm.const_get(vm.core.object, tn) {
        vm.define_method(time, "strftime", strftime);
    }
    let _ = String::new;
}
