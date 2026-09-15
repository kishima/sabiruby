//! mruby-cmath (`mrbgems/mruby-cmath/src/cmath.c`): the `CMath` module, Math over the
//! complex plane. No Ruby part; the gem is not in the reference's `default.gembox`, so the
//! reference image cannot answer for it — the checks are its own test file and the identities
//! between the functions (`docs/design/gems.md`).
//!
//! The reference leans on C99's `<complex.h>` (`csin`, `clog`, …). There is none in a
//! `no_std` Rust library, so the functions are written out here from their definitions, with
//! the principal branch C uses. A real argument answers a Float through `libm` exactly as the
//! reference does, except that `log`, `log2`, `log10` and `sqrt` of a negative real answer a
//! Complex.

use alloc::format;

use crate::argc;
use crate::error::VmResult;
use crate::value::Value;
use crate::vm::Vm;

use super::ext_complex as cpx;

/// A complex number while it is being computed (both parts Floats, as in the reference).
#[derive(Clone, Copy)]
struct C {
    re: f64,
    im: f64,
}

impl C {
    fn new(re: f64, im: f64) -> C { C { re, im } }
    fn add(self, o: C) -> C { C::new(self.re + o.re, self.im + o.im) }
    fn sub(self, o: C) -> C { C::new(self.re - o.re, self.im - o.im) }
    fn mul(self, o: C) -> C { C::new(self.re * o.re - self.im * o.im, self.re * o.im + self.im * o.re) }
    fn div(self, o: C) -> C {
        // the reference's `CXDIVc` (Smith's algorithm)
        if libm::fabs(o.re) >= libm::fabs(o.im) {
            let r = o.im / o.re;
            let d = o.re + r * o.im;
            C::new((self.re + self.im * r) / d, (self.im - self.re * r) / d)
        } else {
            let r = o.re / o.im;
            let d = o.im + r * o.re;
            C::new((self.re * r + self.im) / d, (self.im * r - self.re) / d)
        }
    }
    fn divf(self, f: f64) -> C { C::new(self.re / f, self.im / f) }
    /// `z * i`
    fn muli(self) -> C { C::new(-self.im, self.re) }
    /// `-i * z`
    fn divi(self) -> C { C::new(self.im, -self.re) }
    fn abs(self) -> f64 { libm::hypot(self.re, self.im) }
    fn arg(self) -> f64 { libm::atan2(self.im, self.re) }
}

const ONE: C = C { re: 1.0, im: 0.0 };

fn cexp(z: C) -> C {
    let e = libm::exp(z.re);
    C::new(e * libm::cos(z.im), e * libm::sin(z.im))
}
fn clog(z: C) -> C {
    C::new(libm::log(z.abs()), z.arg())
}
/// The principal square root, in the stable form (`csqrt`).
fn csqrt(z: C) -> C {
    if z.re == 0.0 && z.im == 0.0 { return C::new(0.0, 0.0); }
    let t = libm::sqrt((libm::fabs(z.re) + z.abs()) / 2.0);
    if z.re >= 0.0 {
        C::new(t, z.im / (2.0 * t))
    } else {
        C::new(libm::fabs(z.im) / (2.0 * t), libm::copysign(t, z.im))
    }
}
fn csin(z: C) -> C { C::new(libm::sin(z.re) * libm::cosh(z.im), libm::cos(z.re) * libm::sinh(z.im)) }
fn ccos(z: C) -> C { C::new(libm::cos(z.re) * libm::cosh(z.im), -libm::sin(z.re) * libm::sinh(z.im)) }
fn ctan(z: C) -> C { csin(z).div(ccos(z)) }
fn csinh(z: C) -> C { C::new(libm::sinh(z.re) * libm::cos(z.im), libm::cosh(z.re) * libm::sin(z.im)) }
fn ccosh(z: C) -> C { C::new(libm::cosh(z.re) * libm::cos(z.im), libm::sinh(z.re) * libm::sin(z.im)) }
fn ctanh(z: C) -> C { csinh(z).div(ccosh(z)) }
/// `asin z = -i ln(iz + sqrt(1 - z^2))`
fn casin(z: C) -> C { clog(z.muli().add(csqrt(ONE.sub(z.mul(z))))).divi() }
/// `acos z = -i ln(z + i sqrt(1 - z^2))`
fn cacos(z: C) -> C { clog(z.add(csqrt(ONE.sub(z.mul(z))).muli())).divi() }
/// `atan z = (i/2)(ln(1 - iz) - ln(1 + iz))`
fn catan(z: C) -> C { clog(ONE.sub(z.muli())).sub(clog(ONE.add(z.muli()))).divf(2.0).muli() }
/// `asinh z = ln(z + sqrt(z^2 + 1))`
fn casinh(z: C) -> C { clog(z.add(csqrt(z.mul(z).add(ONE)))) }
/// `acosh z = ln(z + sqrt(z + 1) sqrt(z - 1))`, which keeps the principal branch
fn cacosh(z: C) -> C { clog(z.add(csqrt(z.add(ONE)).mul(csqrt(z.sub(ONE))))) }
/// `atanh z = (ln(1 + z) - ln(1 - z)) / 2`
fn catanh(z: C) -> C { clog(ONE.add(z)).sub(clog(ONE.sub(z))).divf(2.0) }

/// `cmath_get_complex`: the argument as two Floats, and whether it was a Complex.
fn get(vm: &mut Vm, v: Value) -> VmResult<(C, bool)> {
    if cpx::is_complex(vm, v) {
        let (r, i) = cpx::float_parts(vm, v);
        return Ok((C::new(r, i), true));
    }
    match super::numeric::num_f64(vm, v) {
        Some(f) if matches!(v, Value::Int(_) | Value::Float(_)) || vm.is_bigint(v) => Ok((C::new(f, 0.0), false)),
        _ => Err(vm.raise_type("Numeric required")),
    }
}

fn complex(vm: &mut Vm, c: C) -> VmResult<Value> {
    cpx::new_complex(vm, Value::Float(c.re), Value::Float(c.im))
}

/// `DEF_CMATH_METHOD`: a Complex argument answers a Complex, a real one a Float.
fn unary(vm: &mut Vm, a: &[Value], fc: fn(C) -> C, fr: fn(f64) -> f64) -> VmResult<Value> {
    argc!(vm, a, 1);
    let (z, is_c) = get(vm, a[0])?;
    if is_c { let r = fc(z); return complex(vm, r); }
    Ok(Value::Float(fr(z.re)))
}

/// `log`, `log2`, `log10` and `sqrt` answer a Complex for a negative real too.
fn unary_cut(vm: &mut Vm, a: &[Value], fc: fn(C) -> C, fr: fn(f64) -> f64) -> VmResult<Value> {
    argc!(vm, a, 1);
    let (z, is_c) = get(vm, a[0])?;
    if is_c || z.re < 0.0 { let r = fc(z); return complex(vm, r); }
    Ok(Value::Float(fr(z.re)))
}

fn cmath_log(vm: &mut Vm, _s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    argc!(vm, a, 1, 2);
    let (z, is_c) = get(vm, a[0])?;
    let base = match a.get(1) {
        None => None,
        Some(b) => match super::numeric::num_f64(vm, *b) {
            Some(f) => Some(f),
            None => { let d = vm.describe_for_type_error(*b); return Err(vm.raise_type(&format!("{d} cannot be converted to Float"))); }
        },
    };
    if is_c || z.re < 0.0 {
        let mut c = clog(z);
        if let Some(base) = base { c = c.div(clog(C::new(base, 0.0))); }
        return complex(vm, c);
    }
    Ok(Value::Float(match base { None => libm::log(z.re), Some(b) => libm::log(z.re) / libm::log(b) }))
}

pub fn init(vm: &mut Vm) {
    let cmath = vm.define_module("CMath");
    let fns: &[(&str, crate::object::NativeFn)] = &[
        ("sin", |vm, _s, a, _b| unary(vm, a, csin, libm::sin)),
        ("cos", |vm, _s, a, _b| unary(vm, a, ccos, libm::cos)),
        ("tan", |vm, _s, a, _b| unary(vm, a, ctan, libm::tan)),
        ("asin", |vm, _s, a, _b| unary(vm, a, casin, libm::asin)),
        ("acos", |vm, _s, a, _b| unary(vm, a, cacos, libm::acos)),
        ("atan", |vm, _s, a, _b| unary(vm, a, catan, libm::atan)),
        ("sinh", |vm, _s, a, _b| unary(vm, a, csinh, libm::sinh)),
        ("cosh", |vm, _s, a, _b| unary(vm, a, ccosh, libm::cosh)),
        ("tanh", |vm, _s, a, _b| unary(vm, a, ctanh, libm::tanh)),
        ("asinh", |vm, _s, a, _b| unary(vm, a, casinh, libm::asinh)),
        ("acosh", |vm, _s, a, _b| unary(vm, a, cacosh, libm::acosh)),
        ("atanh", |vm, _s, a, _b| unary(vm, a, catanh, libm::atanh)),
        ("exp", |vm, _s, a, _b| unary(vm, a, cexp, libm::exp)),
        ("log", cmath_log),
        ("log2", |vm, _s, a, _b| unary_cut(vm, a, |z| clog(z).divf(core::f64::consts::LN_2), libm::log2)),
        ("log10", |vm, _s, a, _b| unary_cut(vm, a, |z| clog(z).divf(core::f64::consts::LN_10), libm::log10)),
        ("sqrt", |vm, _s, a, _b| unary_cut(vm, a, csqrt, libm::sqrt)),
    ];
    // module functions: public on `CMath` itself, private as instance methods
    let sc = vm.singleton_class(Value::Obj(cmath)).expect("CMath singleton");
    vm.define_methods(sc, fns);
    vm.define_methods(cmath, fns);
    for (name, _) in fns {
        let n = vm.intern(name);
        let _ = vm.set_visibility(cmath, n, crate::object::Vis::Private);
    }
}
