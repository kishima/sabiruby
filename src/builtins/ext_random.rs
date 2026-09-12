//! mruby-random (`mrbgems/mruby-random/src/random.c`): `Random` (PCG-XSH-RR, 64-bit state,
//! 32-bit output, the reference's seeding so a seed gives the reference's sequence),
//! `Kernel#rand`/`srand`, `Array#shuffle`/`shuffle!`/`sample`. No Ruby part.
//!
//! A `Random` keeps its state in two hidden instance variables (`__state`, `__seed`) instead
//! of the reference's `MRB_TT_ISTRUCT` payload. The default generator is seeded with a
//! constant at start (the VM has no clock; the reference uses `time(NULL)`), so its sequence
//! is the same on every run until `srand` is called.

use alloc::{format, vec, vec::Vec};

use crate::argc;
use crate::error::VmResult;
use crate::object::ObjKind;
use crate::value::{ObjId, Value};
use crate::vm::Vm;

const PCG_MULTIPLIER: u64 = 6364136223846793005;
const PCG_INCREMENT: u64 = 1442695040888963407;

#[derive(Clone, Copy)]
struct RandState {
    state: u64,
    seed_value: u32,
}

impl RandState {
    fn init() -> RandState {
        RandState { state: 0x853c49e6748fea9b, seed_value: 521288629 }
    }
    /// `rand_seed`: PCG initialisation (state 0, step, add the seed, ten steps)
    fn seed(&mut self, seed: u32) -> u32 {
        let old = self.seed_value;
        self.state = 0;
        self.uint32();
        self.state = self.state.wrapping_add(seed as u64);
        for _ in 0..10 { self.uint32(); }
        self.seed_value = seed;
        old
    }
    fn uint32(&mut self) -> u32 {
        let old = self.state;
        self.state = old.wrapping_mul(PCG_MULTIPLIER).wrapping_add(PCG_INCREMENT);
        let xorshifted = (((old >> 18) ^ old) >> 27) as u32;
        let rot = (old >> 59) as u32;
        (xorshifted >> rot) | (xorshifted << ((32 - rot) & 31))
    }
    fn real(&mut self) -> f64 {
        self.uint32() as f64 * (1.0 / 4294967296.0)
    }
    /// uniform in `[0, max)` without modulo bias
    fn int(&mut self, max: i64) -> i64 {
        if max <= 0 { return 0; }
        if max > u32::MAX as i64 {
            let umax = max as u64;
            let threshold = (0u64.wrapping_sub(umax)) % umax;
            loop {
                let r = ((self.uint32() as u64) << 32) | self.uint32() as u64;
                if r >= threshold { return (r % umax) as i64; }
            }
        }
        let m = max as u32;
        let threshold = (0u32.wrapping_sub(m)) % m;
        loop {
            let r = self.uint32();
            if r >= threshold { return (r % m) as i64; }
        }
    }
    fn uint(&mut self) -> u64 {
        ((self.uint32() as u64) << 32) | self.uint32() as u64
    }
    /// uniform in `[0, span)`; `span == 0` is the whole 64-bit domain
    fn u(&mut self, span: u64) -> u64 {
        if span == 0 { return self.uint(); }
        let threshold = (0u64.wrapping_sub(span)) % span;
        loop {
            let r = self.uint();
            if r >= threshold { return r % span; }
        }
    }
}

fn load(vm: &mut Vm, o: ObjId) -> RandState {
    let (ks, kv) = (vm.intern("__state"), vm.intern("__seed"));
    let state = match vm.heap.ivar_get(o, ks) { Value::Int(i) => i as u64, _ => 0 };
    let seed_value = match vm.heap.ivar_get(o, kv) { Value::Int(i) => i as u32, _ => 0 };
    RandState { state, seed_value }
}

fn store(vm: &mut Vm, o: ObjId, t: RandState) {
    let (ks, kv) = (vm.intern("__state"), vm.intern("__seed"));
    vm.heap.ivar_set(o, ks, Value::Int(t.state as i64));
    vm.heap.ivar_set(o, kv, Value::Int(t.seed_value as i64));
}

fn random_class(vm: &mut Vm) -> ObjId {
    let n = vm.intern("Random");
    match vm.const_get(vm.core.object, n) { Some(Value::Obj(c)) => c, _ => vm.core.object }
}

/// the class-level `mruby_Random` instance variable holds the default generator
fn random_default(vm: &mut Vm) -> VmResult<ObjId> {
    let c = random_class(vm);
    let k = vm.intern("mruby_Random");
    match vm.heap.ivar_get(c, k) {
        Value::Obj(o) if vm.obj_is_kind_of(Value::Obj(o), c) => Ok(o),
        _ => Err(vm.raise(vm.core.runtime_error, "[BUG] default Random replaced")),
    }
}

fn range_error(vm: &mut Vm, v: Value) -> crate::error::VmError {
    let d = vm.describe_for_type_error(v);
    vm.raise_type(&format!("no implicit conversion of {d} into Integer"))
}

fn random_rand(t: &mut RandState, max: i64) -> Value {
    if max == 0 { return Value::Float(t.real()); }
    Value::Int((t.uint32() as i64) % max)
}

fn rand_range_int(t: &mut RandState, begin: i64, end: i64, excl: bool) -> Value {
    if begin > end || (excl && begin == end) { return Value::Nil; }
    let span = (end as u64).wrapping_sub(begin as u64).wrapping_add(if excl { 0 } else { 1 });
    let r = t.u(span);
    Value::Int((begin as u64).wrapping_add(r) as i64)
}

fn rand_range_float(t: &mut RandState, begin: f64, end: f64, _excl: bool) -> Value {
    let span = end - begin;
    if span <= 0.0 { return Value::Nil; }
    Value::Float(t.real() * span + begin)
}

fn random_range(vm: &mut Vm, t: &mut RandState, rv: Value) -> VmResult<Value> {
    let (b, e, x) = match rv.obj().map(|o| &vm.heap.get(o).kind) {
        Some(ObjKind::Range { begin, end, excl }) => (begin.get(), end.get(), *excl),
        _ => return Err(range_error(vm, rv)),
    };
    if let (Value::Int(b), Value::Int(e)) = (b, e) { return Ok(rand_range_int(t, b, e, x)); }
    let to_f = |vm: &mut Vm, v: Value| -> VmResult<f64> { match v { Value::Float(f) => Ok(f), Value::Int(i) => Ok(i as f64), _ => Err(range_error(vm, v)) } };
    let (bf, ef) = (to_f(vm, b)?, to_f(vm, e)?);
    Ok(rand_range_float(t, bf, ef, x))
}

fn random_rand_impl(vm: &mut Vm, t: &mut RandState, a: &[Value]) -> VmResult<Value> {
    argc!(vm, a, 0, 1);
    let Some(arg) = a.first().copied() else { return Ok(random_rand(t, 0)) };
    match arg {
        Value::Float(f) => Ok(random_rand(t, f as i64)),
        Value::Int(i) => Ok(random_rand(t, i)),
        Value::Obj(o) if matches!(vm.heap.get(o).kind, ObjKind::Range { .. }) => random_range(vm, t, arg),
        v => Err(range_error(vm, v)),
    }
}

/// `srand` without a seed: the reference mixes `time(NULL)`, a draw and the state's address
fn fresh_seed(vm: &mut Vm, t: &mut RandState, o: ObjId) -> u32 {
    let clock = vm.gc_clock.map(|c| c()).unwrap_or(0) as u32;
    clock ^ t.uint32() ^ (o.0.wrapping_mul(2654435761))
}

fn random_m_init(vm: &mut Vm, s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    argc!(vm, a, 0, 1);
    let o = s.obj().unwrap();
    let mut t = RandState::init();
    if let Some(seed) = a.first() {
        let seed = vm.expect_int(*seed, "seed")?;
        t.seed(seed as u32);
    }
    store(vm, o, t);
    Ok(s)
}

fn random_m_rand(vm: &mut Vm, s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    let o = s.obj().unwrap();
    let mut t = load(vm, o);
    let r = random_rand_impl(vm, &mut t, a);
    store(vm, o, t);
    r
}

fn srand_on(vm: &mut Vm, o: ObjId, a: &[Value]) -> VmResult<Value> {
    argc!(vm, a, 0, 1);
    let mut t = load(vm, o);
    let seed = match a.first() {
        Some(v) => vm.expect_int(*v, "seed")? as u32,
        None => fresh_seed(vm, &mut t, o),
    };
    let old = t.seed(seed);
    store(vm, o, t);
    Ok(Value::Int(old as i64))
}

fn random_m_srand(vm: &mut Vm, s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    srand_on(vm, s.obj().unwrap(), a)
}

fn bytes_on(vm: &mut Vm, o: ObjId, a: &[Value]) -> VmResult<Value> {
    argc!(vm, a, 1);
    let n = vm.expect_int(a[0], "size")?;
    if n < 0 { return Err(vm.raise_arg("negative string size")); }
    let mut t = load(vm, o);
    let mut out = Vec::with_capacity(n as usize);
    let mut left = n;
    while left >= 4 {
        let x = t.uint32();
        out.extend_from_slice(&x.to_le_bytes());
        left -= 4;
    }
    if left > 0 {
        let mut x = t.uint32();
        while left > 0 { out.push(x as u8); x >>= 8; left -= 1; }
    }
    store(vm, o, t);
    Ok(vm.str_new(&out))
}

fn random_m_bytes(vm: &mut Vm, s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    bytes_on(vm, s.obj().unwrap(), a)
}

fn random_f_rand(vm: &mut Vm, _s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    let o = random_default(vm)?;
    let mut t = load(vm, o);
    let r = random_rand_impl(vm, &mut t, a);
    store(vm, o, t);
    r
}

fn random_f_srand(vm: &mut Vm, _s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    let o = random_default(vm)?;
    srand_on(vm, o, a)
}

fn random_f_bytes(vm: &mut Vm, _s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    let o = random_default(vm)?;
    bytes_on(vm, o, a)
}

/// The `random:` keyword of `shuffle`/`sample` (`check_random_arg`): the generator to use,
/// the default one when absent; any other keyword is refused.
fn random_kw(vm: &mut Vm, a: &[Value]) -> VmResult<(Vec<Value>, ObjId)> {
    let kw = match (vm.pending_kw, a.last()) { (Some(k), Some(l)) if !k.is_nil() && k == *l => Some(k), _ => None };
    let pos = if kw.is_some() { a[..a.len() - 1].to_vec() } else { a.to_vec() };
    let Some(kw) = kw else { return Ok((pos, random_default(vm)?)) };
    let krandom = vm.intern("random");
    let entries: Vec<(Value, Value)> = match kw.obj().map(|o| &vm.heap.get(o).kind) {
        Some(ObjKind::Hash(hd)) => hd.entries.iter().map(|(k, v)| (k.get(), v.get())).collect(),
        _ => Vec::new(),
    };
    let mut r = None;
    for (k, v) in entries {
        if k == Value::Sym(krandom) { r = Some(v); continue; }
        let d = vm.inspect_str(k)?;
        return Err(vm.raise_arg(&format!("unknown keyword: {d}")));
    }
    match r {
        None => Ok((pos, random_default(vm)?)),
        Some(v) => {
            let c = random_class(vm);
            match v {
                Value::Obj(o) if vm.obj_is_kind_of(v, c) => Ok((pos, o)),
                _ => Err(vm.raise_type("Random object required")),
            }
        }
    }
}

fn ary_shuffle_bang(vm: &mut Vm, s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    let (pos, ro) = random_kw(vm, a)?;
    argc!(vm, &pos, 0);
    let o = s.obj().unwrap();
    let len = vm.ary_vals(s).map(|v| v.len()).unwrap_or(0);
    if len > 1 {
        super::ext_array::check_frozen(vm, s)?;
        let mut t = load(vm, ro);
        if let ObjKind::Array(v) = &mut vm.heap.get_mut(o).kind {
            let mut i = len - 1;
            while i > 0 {
                let j = t.int(i as i64 + 1) as usize;
                v.swap(i, j);
                i -= 1;
            }
        }
        store(vm, ro, t);
    }
    Ok(s)
}

fn ary_shuffle(vm: &mut Vm, s: Value, a: &[Value], b: Value) -> VmResult<Value> {
    let vals = vm.ary_vals(s).unwrap_or_default();
    let n = vm.ary_new(vals);
    ary_shuffle_bang(vm, n, a, b)
}

fn ary_sample(vm: &mut Vm, s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    let (pos, ro) = random_kw(vm, a)?;
    argc!(vm, &pos, 0, 1);
    let vals = vm.ary_vals(s).unwrap_or_default();
    let len = vals.len();
    let mut t = load(vm, ro);
    let r = match pos.first() {
        None => Ok(match len { 0 => Value::Nil, 1 => vals[0], _ => vals[t.int(len as i64) as usize] }),
        Some(nv) => {
            let n = vm.expect_int(*nv, "count")?;
            if n < 0 { return Err(vm.raise_arg("negative sample number")); }
            let n = (n as usize).min(len);
            let mut idx: Vec<usize> = Vec::with_capacity(n);
            for _ in 0..n {
                loop {
                    let v = t.int(len as i64) as usize;
                    if !idx.contains(&v) { idx.push(v); break; }
                }
            }
            let out: Vec<Value> = idx.iter().map(|i| vals[*i]).collect();
            Ok(vm.ary_new(out))
        }
    };
    store(vm, ro, t);
    r
}

pub fn init(vm: &mut Vm) {
    let random = vm.define_class("Random", vm.core.object);
    let rsc = vm.singleton_class(Value::Obj(random)).expect("Random singleton");
    vm.define_methods(rsc, &[("rand", random_f_rand), ("srand", random_f_srand), ("bytes", random_f_bytes)]);
    vm.define_methods(random, &[("initialize", random_m_init), ("rand", random_m_rand), ("srand", random_m_srand), ("bytes", random_m_bytes)]);
    let k = vm.core.kernel;
    vm.define_methods(k, &[("rand", random_f_rand), ("srand", random_f_srand)]);
    let ksc = vm.singleton_class(Value::Obj(k)).expect("Kernel singleton");
    for name in ["rand", "srand"] {
        let n = vm.intern(name);
        if let Some((m, _)) = vm.find_method(k, n) {
            vm.def_method_raw(ksc, n, m);
            let _ = vm.set_visibility(k, n, crate::object::Vis::Private);
        }
    }
    let ary = vm.core.array;
    vm.define_methods(ary, &[("shuffle", ary_shuffle), ("shuffle!", ary_shuffle_bang), ("sample", ary_sample)]);
    // the default generator
    let d = vm.heap.alloc(random, ObjKind::Object);
    let mut t = RandState::init();
    t.seed(0x5ab1_0001 ^ (d.0.wrapping_mul(2654435761)));
    store(vm, d, t);
    let kd = vm.intern("mruby_Random");
    vm.heap.ivar_set(random, kd, Value::Obj(d));
    let _ = vec![0u8];
}
