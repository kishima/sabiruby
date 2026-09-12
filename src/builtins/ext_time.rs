//! mruby-time (`mrbgems/mruby-time/src/time.c`): `Time`. No Ruby part.
//!
//! A Time keeps `sec`/`nsec` since the epoch and its zone in hidden instance variables
//! (`__sec`, `__nsec`, `__utc`) instead of the reference's `MRB_TT_CDATA` payload. The calendar
//! (`gmtime`/`timegm`) is computed here; there is no `localtime`: the local zone is UTC with the
//! offset `+0000` (the VM is no_std and has no zone database), so `Time.local` and `Time.utc`
//! differ only in what `utc?`, `zone` and `to_s` say. `Time.now` reads the host's
//! `Vm::wall_clock`; without one it is the epoch.

use alloc::{format, vec};

use crate::argc;
use crate::error::{VmError, VmResult};
use crate::object::ObjKind;
use crate::value::{ObjId, Value};
use crate::vm::Vm;

use super::object::same_object;

const NSECS_PER_SEC: i64 = 1_000_000_000;
const USECS_PER_SEC: i64 = 1_000_000;

#[derive(Clone, Copy)]
struct Tm {
    sec: i64,
    nsec: i64,
    utc: bool,
}

/// broken-down time (`struct tm`)
struct DateTime {
    year: i64,
    mon: u32, // 1..=12
    mday: u32,
    hour: u32,
    min: u32,
    sec: u32,
    wday: u32, // 0 = Sunday
    yday: u32, // 1-based
}

fn is_leap(y: i64) -> bool {
    y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)
}

/// days since 1970-01-01 → civil date (Howard Hinnant's algorithm)
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// civil date → days since 1970-01-01; a day past the month's end rolls over, as `timegm` does
fn days_from_civil(y: i64, m: u32, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 } as i64;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn gmtime(sec: i64) -> DateTime {
    let days = sec.div_euclid(86400);
    let rem = sec.rem_euclid(86400);
    let (year, mon, mday) = civil_from_days(days);
    let jan1 = days_from_civil(year, 1, 1);
    DateTime {
        year, mon, mday,
        hour: (rem / 3600) as u32, min: ((rem % 3600) / 60) as u32, sec: (rem % 60) as u32,
        wday: (days + 4).rem_euclid(7) as u32,
        yday: (days - jan1 + 1) as u32,
    }
}

fn out_of_range(vm: &mut Vm, v: Value) -> VmError {
    let d = vm.inspect_str(v).unwrap_or_default();
    vm.raise(vm.core.range_error, &format!("{d} out of Time range"))
}

/// `mrb_to_time_t`: seconds (and, from a Float's fraction, microseconds) of a number
fn to_time_t(vm: &mut Vm, v: Value, usec: Option<&mut i64>) -> VmResult<i64> {
    match v {
        Value::Int(i) => { if let Some(u) = usec { *u = 0; } Ok(i) }
        Value::Float(f) => {
            if !f.is_finite() { return Err(vm.raise(vm.core.float_domain_error, &super::numeric::float_to_s(f))); }
            if f >= (i64::MAX as f64) - 1.0 || f < (i64::MIN as f64) + 1.0 { return Err(out_of_range(vm, v)); }
            match usec {
                Some(u) => { let tt = libm::floor(f); *u = libm::trunc((f - tt) * 1.0e6) as i64; Ok(tt as i64) }
                None => Ok(libm::round(f) as i64),
            }
        }
        _ => { let d = vm.describe_for_type_error(v); Err(vm.raise_type(&format!("cannot convert {d} to time"))) }
    }
}

/// `time_alloc_time`: nanoseconds normalised into `[0, 1e9)`
fn normalize(mut sec: i64, mut nsec: i64) -> (i64, i64) {
    if nsec < 0 {
        let adj = -((-(nsec + 1)) / NSECS_PER_SEC) - 1; // NDIV
        nsec -= adj * NSECS_PER_SEC;
        sec += adj;
    } else if nsec >= NSECS_PER_SEC {
        let adj = nsec / NSECS_PER_SEC;
        nsec -= adj * NSECS_PER_SEC;
        sec += adj;
    }
    (sec, nsec)
}

fn load(vm: &mut Vm, s: Value) -> VmResult<Tm> {
    let o = match s { Value::Obj(o) => o, _ => return Err(vm.raise_arg("uninitialized Time")) };
    let (ks, kn, ku) = (vm.intern("__sec"), vm.intern("__nsec"), vm.intern("__utc"));
    match (vm.heap.ivar_get(o, ks), vm.heap.ivar_get(o, kn), vm.heap.ivar_get(o, ku)) {
        (Value::Int(sec), Value::Int(nsec), u) => Ok(Tm { sec, nsec, utc: u.truthy() }),
        _ => Err(vm.raise_arg("uninitialized Time")),
    }
}

fn store(vm: &mut Vm, o: ObjId, t: Tm) {
    let (ks, kn, ku) = (vm.intern("__sec"), vm.intern("__nsec"), vm.intern("__utc"));
    vm.heap.ivar_set(o, ks, Value::Int(t.sec));
    vm.heap.ivar_set(o, kn, Value::Int(t.nsec));
    vm.heap.ivar_set(o, ku, Value::bool(t.utc));
}

fn time_class(vm: &mut Vm) -> ObjId {
    let n = vm.intern("Time");
    match vm.const_get(vm.core.object, n) { Some(Value::Obj(c)) => c, _ => vm.core.object }
}

fn is_time(vm: &mut Vm, v: Value) -> bool {
    let c = time_class(vm);
    vm.obj_is_kind_of(v, c) && load(vm, v).is_ok()
}

fn wrap(vm: &mut Vm, c: ObjId, sec: i64, nsec: i64, utc: bool) -> Value {
    let (sec, nsec) = normalize(sec, nsec);
    let o = vm.heap.alloc(c, ObjKind::Object);
    store(vm, o, Tm { sec, nsec, utc });
    Value::Obj(o)
}

/// `time_alloc`: seconds and microseconds from two numbers
fn make(vm: &mut Vm, c: ObjId, sec: Value, usec: Value, utc: bool) -> VmResult<Value> {
    let mut tusec = 0i64;
    let mut tsec = to_time_t(vm, sec, Some(&mut tusec))?;
    tusec += to_time_t(vm, usec, None)?;
    if tusec >= USECS_PER_SEC || tusec <= -USECS_PER_SEC {
        let adj = tusec / USECS_PER_SEC;
        tusec -= adj * USECS_PER_SEC;
        tsec += adj;
    }
    Ok(wrap(vm, c, tsec, tusec * 1000, utc))
}

fn now(vm: &mut Vm) -> (i64, i64) {
    vm.wall_clock.map(|c| c()).unwrap_or((0, 0))
}

fn time_now(vm: &mut Vm, s: Value, _a: &[Value], _b: Value) -> VmResult<Value> {
    let (sec, nsec) = now(vm);
    Ok(wrap(vm, s.obj().unwrap(), sec, nsec, false))
}

fn time_at(vm: &mut Vm, s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    argc!(vm, a, 1, 2);
    let usec = a.get(1).copied().unwrap_or(Value::Int(0));
    make(vm, s.obj().unwrap(), a[0], usec, false)
}

/// `time_mktime`: the reference's range checks, then `timegm`
fn mktime(vm: &mut Vm, c: ObjId, a: &[Value], utc: bool) -> VmResult<Value> {
    let mut f = [0i64, 1, 1, 0, 0, 0, 0];
    for (i, v) in a.iter().enumerate() { f[i] = vm.expect_int(*v, "time component")?; }
    let [year, mon, day, hour, min, sec, usec] = f;
    if year < i64::MIN + 1900 { return Err(vm.raise_arg("argument out of range")); }
    if !(1..=12).contains(&mon) || !(1..=31).contains(&day) || !(0..=24).contains(&hour)
        || (hour == 24 && (min > 0 || sec > 0)) || !(0..=59).contains(&min) || !(0..=60).contains(&sec) {
        return Err(vm.raise_arg("argument out of range"));
    }
    let days = days_from_civil(year, mon as u32, day);
    let secs = days * 86400 + hour * 3600 + min * 60 + sec;
    Ok(wrap(vm, c, secs, usec * 1000, utc))
}

fn time_gm(vm: &mut Vm, s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    argc!(vm, a, 1, 7);
    mktime(vm, s.obj().unwrap(), a, true)
}

fn time_local(vm: &mut Vm, s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    argc!(vm, a, 1, 7);
    mktime(vm, s.obj().unwrap(), a, false)
}

fn time_init(vm: &mut Vm, s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    argc!(vm, a, 0, 7);
    let o = s.obj().unwrap();
    if a.is_empty() {
        let (sec, nsec) = now(vm);
        store(vm, o, Tm { sec, nsec, utc: false });
    } else {
        let c = vm.real_class_of(s);
        let t = mktime(vm, c, a, false)?;
        let tm = load(vm, t)?;
        store(vm, o, tm);
    }
    Ok(s)
}

fn time_init_copy(vm: &mut Vm, s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    argc!(vm, a, 1);
    if same_object(s, a[0]) { return Ok(s); }
    if vm.real_class_of(a[0]) != vm.real_class_of(s) { return Err(vm.raise_type("wrong argument class")); }
    let t = load(vm, a[0])?;
    store(vm, s.obj().unwrap(), t);
    Ok(s)
}

fn time_eq(vm: &mut Vm, s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    argc!(vm, a, 1);
    let t1 = load(vm, s)?;
    if !is_time(vm, a[0]) { return Ok(Value::False); }
    let t2 = load(vm, a[0])?;
    Ok(Value::bool(t1.sec == t2.sec && t1.nsec == t2.nsec))
}

fn time_cmp(vm: &mut Vm, s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    argc!(vm, a, 1);
    let t1 = load(vm, s)?;
    if !is_time(vm, a[0]) { return Ok(Value::Nil); }
    let t2 = load(vm, a[0])?;
    Ok(Value::Int(match (t1.sec, t1.nsec).cmp(&(t2.sec, t2.nsec)) { core::cmp::Ordering::Less => -1, core::cmp::Ordering::Equal => 0, core::cmp::Ordering::Greater => 1 }))
}

fn time_plus(vm: &mut Vm, s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    argc!(vm, a, 1);
    let t = load(vm, s)?;
    let mut usec = 0i64;
    let sec = to_time_t(vm, a[0], Some(&mut usec))?;
    let sum = match t.sec.checked_add(sec) { Some(x) => x, None => return Err(vm.raise(vm.core.range_error, "Time out of range in addition")) };
    let c = vm.real_class_of(s);
    Ok(wrap(vm, c, sum, t.nsec + usec * 1000, t.utc))
}

fn time_minus(vm: &mut Vm, s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    argc!(vm, a, 1);
    let t = load(vm, s)?;
    if is_time(vm, a[0]) {
        let t2 = load(vm, a[0])?;
        return Ok(Value::Float((t.sec - t2.sec) as f64 + (t.nsec - t2.nsec) as f64 / 1.0e9));
    }
    let mut usec = 0i64;
    let sec = to_time_t(vm, a[0], Some(&mut usec))?;
    let diff = match t.sec.checked_sub(sec) { Some(x) => x, None => return Err(vm.raise(vm.core.range_error, "Time out of range in subtraction")) };
    let c = vm.real_class_of(s);
    Ok(wrap(vm, c, diff, t.nsec - usec * 1000, t.utc))
}

const WDAY_NAMES: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MON_NAMES: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

fn time_asctime(vm: &mut Vm, s: Value, _a: &[Value], _b: Value) -> VmResult<Value> {
    let t = load(vm, s)?;
    let d = gmtime(t.sec);
    let out = format!("{} {} {:2} {:02}:{:02}:{:02} {:04}", WDAY_NAMES[d.wday as usize], MON_NAMES[(d.mon - 1) as usize], d.mday, d.hour, d.min, d.sec, d.year);
    Ok(vm.str_from(out))
}

/// `%Y-%m-%d %H:%M:%S UTC` or `... +0000`
fn time_to_s(vm: &mut Vm, s: Value, _a: &[Value], _b: Value) -> VmResult<Value> {
    let t = load(vm, s)?;
    let d = gmtime(t.sec);
    let out = format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02} {}", d.year, d.mon, d.mday, d.hour, d.min, d.sec, if t.utc { "UTC" } else { "+0000" });
    Ok(vm.str_from(out))
}

fn field(vm: &mut Vm, s: Value, f: fn(&DateTime) -> i64) -> VmResult<Value> {
    let t = load(vm, s)?;
    Ok(Value::Int(f(&gmtime(t.sec))))
}

fn wday_p(vm: &mut Vm, s: Value, w: u32) -> VmResult<Value> {
    let t = load(vm, s)?;
    Ok(Value::bool(gmtime(t.sec).wday == w))
}

fn set_zone(vm: &mut Vm, s: Value, utc: bool) -> VmResult<Value> {
    let mut t = load(vm, s)?;
    t.utc = utc;
    store(vm, s.obj().unwrap(), t);
    Ok(s)
}

fn get_zone(vm: &mut Vm, s: Value, utc: bool) -> VmResult<Value> {
    let t = load(vm, s)?;
    let c = vm.real_class_of(s);
    Ok(wrap(vm, c, t.sec, t.nsec, utc))
}

fn time_hash(vm: &mut Vm, s: Value, _a: &[Value], _b: Value) -> VmResult<Value> {
    let t = load(vm, s)?;
    let mut h: u64 = 0xcbf29ce484222325;
    for b in t.sec.to_le_bytes().iter().chain(t.nsec.to_le_bytes().iter()).chain([t.utc as u8].iter()) {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    Ok(Value::Int((h as u32) as i64))
}

pub fn init(vm: &mut Vm) {
    let tc = vm.define_class("Time", vm.core.object);
    let cmp = vm.intern("Comparable");
    if let Some(Value::Obj(m)) = vm.const_get(vm.core.object, cmp) { vm.include_module(tc, m); }
    let sc = vm.singleton_class(Value::Obj(tc)).expect("Time singleton");
    vm.define_methods(sc, &[("at", time_at), ("gm", time_gm), ("local", time_local), ("mktime", time_local), ("now", time_now), ("utc", time_gm)]);
    vm.define_methods(tc, &[
        ("hash", time_hash),
        ("eql?", time_eq),
        ("==", time_eq),
        ("<=>", time_cmp),
        ("+", time_plus),
        ("-", time_minus),
        ("to_s", time_to_s),
        ("inspect", time_to_s),
        ("asctime", time_asctime),
        ("ctime", time_asctime),
        ("day", |vm, s, _a, _b| field(vm, s, |d| d.mday as i64)),
        ("mday", |vm, s, _a, _b| field(vm, s, |d| d.mday as i64)),
        ("dst?", |vm, s, _a, _b| { load(vm, s)?; Ok(Value::False) }),
        ("getgm", |vm, s, _a, _b| get_zone(vm, s, true)),
        ("getutc", |vm, s, _a, _b| get_zone(vm, s, true)),
        ("getlocal", |vm, s, _a, _b| get_zone(vm, s, false)),
        ("gmt?", |vm, s, _a, _b| { let t = load(vm, s)?; Ok(Value::bool(t.utc)) }),
        ("utc?", |vm, s, _a, _b| { let t = load(vm, s)?; Ok(Value::bool(t.utc)) }),
        ("gmtime", |vm, s, _a, _b| set_zone(vm, s, true)),
        ("utc", |vm, s, _a, _b| set_zone(vm, s, true)),
        ("localtime", |vm, s, _a, _b| set_zone(vm, s, false)),
        ("hour", |vm, s, _a, _b| field(vm, s, |d| d.hour as i64)),
        ("min", |vm, s, _a, _b| field(vm, s, |d| d.min as i64)),
        ("mon", |vm, s, _a, _b| field(vm, s, |d| d.mon as i64)),
        ("month", |vm, s, _a, _b| field(vm, s, |d| d.mon as i64)),
        ("sec", |vm, s, _a, _b| field(vm, s, |d| d.sec as i64)),
        ("year", |vm, s, _a, _b| field(vm, s, |d| d.year)),
        ("wday", |vm, s, _a, _b| field(vm, s, |d| d.wday as i64)),
        ("yday", |vm, s, _a, _b| field(vm, s, |d| d.yday as i64)),
        ("to_i", |vm, s, _a, _b| { let t = load(vm, s)?; Ok(Value::Int(t.sec)) }),
        ("to_f", |vm, s, _a, _b| { let t = load(vm, s)?; Ok(Value::Float(t.sec as f64 + t.nsec as f64 / 1.0e9)) }),
        ("usec", |vm, s, _a, _b| { let t = load(vm, s)?; Ok(Value::Int(t.nsec / 1000)) }),
        ("nsec", |vm, s, _a, _b| { let t = load(vm, s)?; Ok(Value::Int(t.nsec)) }),
        ("tv_nsec", |vm, s, _a, _b| { let t = load(vm, s)?; Ok(Value::Int(t.nsec)) }),
        ("zone", |vm, s, _a, _b| { let t = load(vm, s)?; Ok(vm.str_new(if t.utc { b"UTC" } else { b"+0000" })) }),
        ("utc_offset", |vm, s, _a, _b| { load(vm, s)?; Ok(Value::Int(0)) }),
        ("gmt_offset", |vm, s, _a, _b| { load(vm, s)?; Ok(Value::Int(0)) }),
        ("gmtoff", |vm, s, _a, _b| { load(vm, s)?; Ok(Value::Int(0)) }),
        ("initialize", time_init),
        ("initialize_copy", time_init_copy),
        ("sunday?", |vm, s, _a, _b| wday_p(vm, s, 0)),
        ("monday?", |vm, s, _a, _b| wday_p(vm, s, 1)),
        ("tuesday?", |vm, s, _a, _b| wday_p(vm, s, 2)),
        ("wednesday?", |vm, s, _a, _b| wday_p(vm, s, 3)),
        ("thursday?", |vm, s, _a, _b| wday_p(vm, s, 4)),
        ("friday?", |vm, s, _a, _b| wday_p(vm, s, 5)),
        ("saturday?", |vm, s, _a, _b| wday_p(vm, s, 6)),
    ]);
    for name in ["initialize", "initialize_copy"] {
        let n = vm.intern(name);
        let _ = vm.set_visibility(tc, n, crate::object::Vis::Private);
    }
    let _ = vec![0u8];
    let _ = is_leap;
}
