//! Arbitrary-precision integers: mruby-bigint's `mpz_*` layer, in Rust.
//!
//! The representation is the reference's (`mrbgems/mruby-bigint/core/bigint.c`): a sign and
//! the absolute value in 32-bit limbs, least significant first, with no leading zero limb
//! (`mpz_t`'s `sn` and `p[0..sz]` after `trim`). The embedded-limb optimization
//! (`RBIGINT_EMBED_SIZE_MAX`) is not copied; a `BigInt` always owns its limbs.
//!
//! Only the algorithms the reference needs for correctness are here — schoolbook
//! multiplication and Knuth's algorithm D for division, not Karatsuba, Barrett or
//! Montgomery (`docs/gems.md`). Every function is pure arithmetic: normalizing a result back
//! to a `Value::Int` when it fits (the reference's `bint_norm`) is the VM's job, in
//! `builtins::numeric`.

use alloc::{string::String, vec, vec::Vec};
use core::cmp::Ordering;

/// Bits in one limb (`DIG_SIZE`).
pub const LIMB_BITS: u32 = 32;
/// `DIG_BASE` as a double-limb value.
const BASE: u64 = 1 << 32;

/// An integer of arbitrary size. `mag` is the absolute value in 32-bit limbs, least
/// significant first and trimmed; an empty `mag` is zero, and zero is never negative.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BigInt {
    pub neg: bool,
    pub mag: Vec<u32>,
}

// ---------------------------------------------------------------- magnitude helpers

fn trim(v: &mut Vec<u32>) {
    while v.last() == Some(&0) {
        v.pop();
    }
}

fn ucmp(a: &[u32], b: &[u32]) -> Ordering {
    if a.len() != b.len() {
        return a.len().cmp(&b.len());
    }
    for i in (0..a.len()).rev() {
        if a[i] != b[i] {
            return a[i].cmp(&b[i]);
        }
    }
    Ordering::Equal
}

fn uadd(a: &[u32], b: &[u32]) -> Vec<u32> {
    let (long, short) = if a.len() >= b.len() { (a, b) } else { (b, a) };
    let mut out = Vec::with_capacity(long.len() + 1);
    let mut carry = 0u64;
    for i in 0..long.len() {
        let s = long[i] as u64 + if i < short.len() { short[i] as u64 } else { 0 } + carry;
        out.push(s as u32);
        carry = s >> 32;
    }
    if carry != 0 {
        out.push(carry as u32);
    }
    out
}

/// `a - b`, for `a >= b`.
fn usub(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(a.len());
    let mut borrow = 0i64;
    for i in 0..a.len() {
        let t = a[i] as i64 - if i < b.len() { b[i] as i64 } else { 0 } - borrow;
        out.push(t as u32);
        borrow = if t < 0 { 1 } else { 0 };
    }
    debug_assert_eq!(borrow, 0);
    trim(&mut out);
    out
}

/// Schoolbook multiplication (`mpz_mul_basic`).
fn umul(a: &[u32], b: &[u32]) -> Vec<u32> {
    if a.is_empty() || b.is_empty() {
        return Vec::new();
    }
    let mut out = vec![0u32; a.len() + b.len()];
    for (i, &x) in a.iter().enumerate() {
        if x == 0 {
            continue;
        }
        let mut carry = 0u64;
        for (j, &y) in b.iter().enumerate() {
            let t = x as u64 * y as u64 + out[i + j] as u64 + carry;
            out[i + j] = t as u32;
            carry = t >> 32;
        }
        let mut k = i + b.len();
        while carry != 0 {
            let t = out[k] as u64 + carry;
            out[k] = t as u32;
            carry = t >> 32;
            k += 1;
        }
    }
    trim(&mut out);
    out
}

/// `a * d + carry_in` for a single limb, in place; returns nothing (the carry is appended).
fn umul_small(a: &mut Vec<u32>, d: u32, add: u32) {
    let mut carry = add as u64;
    for x in a.iter_mut() {
        let t = *x as u64 * d as u64 + carry;
        *x = t as u32;
        carry = t >> 32;
    }
    while carry != 0 {
        a.push(carry as u32);
        carry >>= 32;
    }
    trim(a);
}

/// Divides in place by a single limb and returns the remainder.
fn udiv_small(a: &mut Vec<u32>, d: u32) -> u32 {
    let mut r = 0u64;
    for i in (0..a.len()).rev() {
        let cur = (r << 32) | a[i] as u64;
        a[i] = (cur / d as u64) as u32;
        r = cur % d as u64;
    }
    trim(a);
    r as u32
}

fn ushl(a: &[u32], limbs: usize, bits: u32) -> Vec<u32> {
    if a.is_empty() {
        return Vec::new();
    }
    let mut out = vec![0u32; limbs];
    if bits == 0 {
        out.extend_from_slice(a);
    } else {
        let mut carry = 0u32;
        for &x in a {
            out.push((x << bits) | carry);
            carry = x >> (32 - bits);
        }
        if carry != 0 {
            out.push(carry);
        }
    }
    trim(&mut out);
    out
}

fn ushr(a: &[u32], limbs: usize, bits: u32) -> Vec<u32> {
    if limbs >= a.len() {
        return Vec::new();
    }
    let src = &a[limbs..];
    let mut out = Vec::with_capacity(src.len());
    if bits == 0 {
        out.extend_from_slice(src);
    } else {
        for i in 0..src.len() {
            let hi = if i + 1 < src.len() { src[i + 1] << (32 - bits) } else { 0 };
            out.push((src[i] >> bits) | hi);
        }
    }
    trim(&mut out);
    out
}

/// Knuth's algorithm D (`udiv`): truncated quotient and remainder of the magnitudes.
fn udivmod(a: &[u32], b: &[u32]) -> (Vec<u32>, Vec<u32>) {
    debug_assert!(!b.is_empty());
    if ucmp(a, b) == Ordering::Less {
        return (Vec::new(), a.to_vec());
    }
    if b.len() == 1 {
        let mut q = a.to_vec();
        let r = udiv_small(&mut q, b[0]);
        let mut rem = vec![r];
        trim(&mut rem);
        return (q, rem);
    }
    let s = b[b.len() - 1].leading_zeros();
    let n = b.len();
    let bn: Vec<u32> = if s == 0 {
        b.to_vec()
    } else {
        let mut v = Vec::with_capacity(n);
        let mut carry = 0u32;
        for &x in b {
            v.push((x << s) | carry);
            carry = x >> (32 - s);
        }
        debug_assert_eq!(carry, 0);
        v
    };
    // the dividend gets one extra limb so that `an[j + n]` always exists
    let mut an = vec![0u32; a.len() + 1];
    if s == 0 {
        an[..a.len()].copy_from_slice(a);
    } else {
        let mut carry = 0u32;
        for i in 0..a.len() {
            an[i] = (a[i] << s) | carry;
            carry = a[i] >> (32 - s);
        }
        an[a.len()] = carry;
    }
    let m = a.len() - n;
    let mut q = vec![0u32; m + 1];
    for j in (0..=m).rev() {
        let top = ((an[j + n] as u64) << 32) | an[j + n - 1] as u64;
        let mut qhat = top / bn[n - 1] as u64;
        let mut rhat = top % bn[n - 1] as u64;
        loop {
            if qhat >= BASE || qhat * bn[n - 2] as u64 > ((rhat << 32) | an[j + n - 2] as u64) {
                qhat -= 1;
                rhat += bn[n - 1] as u64;
                if rhat < BASE {
                    continue;
                }
            }
            break;
        }
        // an[j..j+n+1] -= qhat * bn
        let mut borrow = 0i64;
        let mut carry = 0u64;
        for i in 0..n {
            let p = qhat * bn[i] as u64 + carry;
            carry = p >> 32;
            let t = an[i + j] as i64 - (p & 0xFFFF_FFFF) as i64 - borrow;
            an[i + j] = t as u32;
            borrow = if t < 0 { 1 } else { 0 };
        }
        let t = an[j + n] as i64 - carry as i64 - borrow;
        an[j + n] = t as u32;
        if t < 0 {
            // qhat was one too large: add the divisor back
            qhat -= 1;
            let mut carry = 0u64;
            for i in 0..n {
                let s2 = an[i + j] as u64 + bn[i] as u64 + carry;
                an[i + j] = s2 as u32;
                carry = s2 >> 32;
            }
            an[j + n] = (an[j + n] as u64).wrapping_add(carry) as u32;
        }
        q[j] = qhat as u32;
    }
    let mut r = an[..n].to_vec();
    if s > 0 {
        r = ushr(&r, 0, s);
    }
    trim(&mut q);
    trim(&mut r);
    (q, r)
}

fn digit_val(c: u8) -> Option<u32> {
    match c {
        b'0'..=b'9' => Some((c - b'0') as u32),
        b'a'..=b'z' => Some((c - b'a') as u32 + 10),
        b'A'..=b'Z' => Some((c - b'A') as u32 + 10),
        _ => None,
    }
}

/// The largest power of `base` that fits in a limb, and its exponent
/// (the reference's `base_limit[]`).
fn base_chunk(base: u32) -> (u32, u32) {
    let mut p: u64 = base as u64;
    let mut k = 1;
    while p * base as u64 <= u32::MAX as u64 {
        p *= base as u64;
        k += 1;
    }
    (p as u32, k)
}

// ---------------------------------------------------------------- BigInt

impl BigInt {
    pub fn zero() -> BigInt {
        BigInt { neg: false, mag: Vec::new() }
    }
    pub fn is_zero(&self) -> bool {
        self.mag.is_empty()
    }
    /// `mrb_bint_sign`: -1, 0 or 1.
    pub fn sign(&self) -> i32 {
        if self.mag.is_empty() {
            0
        } else if self.neg {
            -1
        } else {
            1
        }
    }
    fn make(neg: bool, mut mag: Vec<u32>) -> BigInt {
        trim(&mut mag);
        BigInt { neg: neg && !mag.is_empty(), mag }
    }
    pub fn from_i64(v: i64) -> BigInt {
        BigInt::from_sign_u64(v < 0, v.unsigned_abs())
    }
    pub fn from_u64(v: u64) -> BigInt {
        BigInt::from_sign_u64(false, v)
    }
    fn from_sign_u64(neg: bool, u: u64) -> BigInt {
        let mut mag = vec![u as u32, (u >> 32) as u32];
        trim(&mut mag);
        BigInt { neg: neg && !mag.is_empty(), mag }
    }
    /// `mpz_get_int`: the value as an `i64` when it fits (the reference's `bint_norm`).
    pub fn to_i64(&self) -> Option<i64> {
        if self.mag.is_empty() {
            return Some(0);
        }
        if self.mag.len() > 2 {
            return None;
        }
        let u = self.mag[0] as u64 | if self.mag.len() > 1 { (self.mag[1] as u64) << 32 } else { 0 };
        let limit = i64::MAX as u64 + if self.neg { 1 } else { 0 };
        if u > limit {
            return None;
        }
        Some(if self.neg {
            if u == i64::MAX as u64 + 1 { i64::MIN } else { -(u as i64) }
        } else {
            u as i64
        })
    }
    /// `mrb_bint_as_uint64`.
    pub fn to_u64(&self) -> Option<u64> {
        if self.neg || self.mag.len() > 2 {
            return None;
        }
        Some(self.mag.first().copied().unwrap_or(0) as u64 | ((self.mag.get(1).copied().unwrap_or(0) as u64) << 32))
    }
    /// Bytes hashed by `Integer#hash` (`mrb_bint_hash` hashes the limbs and the sign).
    pub fn hash_bytes(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(self.mag.len() * 4 + 1);
        for l in &self.mag {
            b.extend_from_slice(&l.to_le_bytes());
        }
        b.push(if self.neg { 0xff } else { 1 });
        b
    }
    /// `mrb_bint_size`: the bytes the limbs take.
    pub fn byte_size(&self) -> usize {
        self.mag.len() * 4
    }
    /// `mpz_bits`: position of the highest set bit (0 for zero).
    pub fn bit_length(&self) -> u64 {
        match self.mag.last() {
            None => 0,
            Some(top) => (self.mag.len() as u64 - 1) * 32 + (32 - top.leading_zeros() as u64),
        }
    }
    pub fn is_even(&self) -> bool {
        self.mag.first().is_none_or(|l| l & 1 == 0)
    }

    // ------------------------------------------------------------ conversion

    /// `mrb_bint_new_str`: digits in `base` (2..=36), `_` ignored, optional sign.
    /// `None` when a character is not a digit of the base.
    pub fn from_str(s: &[u8], base: u32) -> Option<BigInt> {
        debug_assert!((2..=36).contains(&base));
        let mut i = 0;
        let mut neg = false;
        if let Some(&c) = s.first() {
            if c == b'-' {
                neg = true;
                i = 1;
            } else if c == b'+' {
                i = 1;
            }
        }
        let (chunk, k) = base_chunk(base);
        let mut mag: Vec<u32> = Vec::new();
        let mut acc: u32 = 0;
        let mut n = 0;
        let mut any = false;
        while i < s.len() {
            let c = s[i];
            i += 1;
            if c == b'_' {
                continue;
            }
            let d = digit_val(c)?;
            if d >= base {
                return None;
            }
            any = true;
            acc = acc * base + d;
            n += 1;
            if n == k {
                umul_small(&mut mag, chunk, acc);
                acc = 0;
                n = 0;
            }
        }
        if !any {
            return None;
        }
        if n > 0 {
            let mut m = base;
            for _ in 1..n {
                m *= base;
            }
            umul_small(&mut mag, m, acc);
        }
        Some(BigInt::make(neg, mag))
    }

    /// `mpz_get_str`: the digits in `base` (2..=36), lower case, with a leading `-`.
    pub fn to_string_radix(&self, base: u32) -> String {
        debug_assert!((2..=36).contains(&base));
        if self.is_zero() {
            return String::from("0");
        }
        let (chunk, k) = base_chunk(base);
        let mut t = self.mag.clone();
        let mut out: Vec<u8> = Vec::new();
        while !t.is_empty() {
            let mut r = udiv_small(&mut t, chunk);
            let last = t.is_empty();
            for _ in 0..k {
                if last && r == 0 {
                    break;
                }
                let d = r % base;
                r /= base;
                out.push(if d < 10 { b'0' + d as u8 } else { b'a' + (d - 10) as u8 });
            }
        }
        if self.neg {
            out.push(b'-');
        }
        out.reverse();
        String::from_utf8(out).expect("digits are ASCII")
    }

    /// `mrb_bint_new_float`: the integer part of a finite `f` (exact).
    pub fn from_f64(f: f64) -> BigInt {
        if !f.is_finite() {
            return BigInt::zero();
        }
        let neg = f < 0.0;
        let bits = f.abs().to_bits();
        let exp = ((bits >> 52) & 0x7ff) as i64;
        let frac = bits & 0x000f_ffff_ffff_ffff;
        if exp == 0 {
            // subnormal: |f| < 1
            return BigInt::zero();
        }
        let m = frac | (1u64 << 52);
        let e = exp - 1075;
        let base = BigInt::from_u64(m);
        let v = if e >= 0 { base.shl(e as u64) } else { base.shr((-e) as u64) };
        BigInt { neg: neg && !v.mag.is_empty(), mag: v.mag }
    }

    /// `mrb_bint_as_float`: the limbs accumulated from the top, as the reference does.
    pub fn to_f64(&self) -> f64 {
        let mut val = 0.0f64;
        for &l in self.mag.iter().rev() {
            val = val * 4294967296.0 + l as f64;
        }
        if self.neg {
            -val
        } else {
            val
        }
    }

    /// `mrb_bint_from_bytes`: a non-negative integer from little-endian bytes.
    pub fn from_bytes_le(bytes: &[u8]) -> BigInt {
        let mut mag = Vec::with_capacity(bytes.len().div_ceil(4));
        for c in bytes.chunks(4) {
            let mut w = [0u8; 4];
            w[..c.len()].copy_from_slice(c);
            mag.push(u32::from_le_bytes(w));
        }
        BigInt::make(false, mag)
    }

    // ------------------------------------------------------------ arithmetic

    pub fn cmp(&self, other: &BigInt) -> Ordering {
        match (self.sign(), other.sign()) {
            (a, b) if a != b => a.cmp(&b),
            (s, _) => {
                let o = ucmp(&self.mag, &other.mag);
                if s < 0 { o.reverse() } else { o }
            }
        }
    }
    pub fn neg(&self) -> BigInt {
        BigInt::make(!self.neg, self.mag.clone())
    }
    pub fn abs(&self) -> BigInt {
        BigInt { neg: false, mag: self.mag.clone() }
    }
    pub fn add(&self, other: &BigInt) -> BigInt {
        if self.neg == other.neg {
            BigInt::make(self.neg, uadd(&self.mag, &other.mag))
        } else {
            match ucmp(&self.mag, &other.mag) {
                Ordering::Equal => BigInt::zero(),
                Ordering::Greater => BigInt::make(self.neg, usub(&self.mag, &other.mag)),
                Ordering::Less => BigInt::make(other.neg, usub(&other.mag, &self.mag)),
            }
        }
    }
    pub fn sub(&self, other: &BigInt) -> BigInt {
        self.add(&other.neg())
    }
    pub fn mul(&self, other: &BigInt) -> BigInt {
        BigInt::make(self.neg != other.neg, umul(&self.mag, &other.mag))
    }
    /// Truncated division (`mpz_mdiv` before the rounding step): the quotient goes
    /// toward zero and the remainder takes the dividend's sign.
    pub fn divmod_trunc(&self, other: &BigInt) -> (BigInt, BigInt) {
        let (q, r) = udivmod(&self.mag, &other.mag);
        (BigInt::make(self.neg != other.neg, q), BigInt::make(self.neg, r))
    }
    /// `mpz_mdivmod`: Ruby's `divmod` (floor division, the remainder takes the divisor's sign).
    pub fn divmod_floor(&self, other: &BigInt) -> (BigInt, BigInt) {
        let (q, r) = self.divmod_trunc(other);
        if r.is_zero() || self.neg == other.neg {
            (q, r)
        } else {
            (q.sub(&BigInt::from_i64(1)), r.add(other))
        }
    }
    /// `mpz_mdiv`: Ruby's `/` and `div`.
    pub fn div_floor(&self, other: &BigInt) -> BigInt {
        self.divmod_floor(other).0
    }
    /// `mpz_mmod`: Ruby's `%` and `modulo`.
    pub fn mod_floor(&self, other: &BigInt) -> BigInt {
        self.divmod_floor(other).1
    }
    /// `mpz_mod`: `Integer#remainder` (the remainder of truncated division).
    pub fn rem_trunc(&self, other: &BigInt) -> BigInt {
        self.divmod_trunc(other).1
    }
    /// `mpz_pow`: `self ** e` for a non-negative `e`.
    pub fn pow(&self, e: u64) -> BigInt {
        let mut result = BigInt::from_i64(1);
        let mut base = self.clone();
        let mut e = e;
        while e > 0 {
            if e & 1 == 1 {
                result = result.mul(&base);
            }
            e >>= 1;
            if e > 0 {
                base = base.mul(&base);
            }
        }
        result
    }
    /// `mpz_powm`: `self ** e mod m` with a positive `m`. The reduction is `mpz_mod`, the
    /// truncated remainder, so a negative base keeps the sign of `self ** e`
    /// (`(-3).pow(3, 5)` is -2 on the reference, where CRuby answers 3).
    pub fn powm(&self, e: &BigInt, m: &BigInt) -> BigInt {
        debug_assert!(!m.is_zero());
        let one = BigInt::from_i64(1);
        if m.mag == [1] {
            return BigInt::zero();
        }
        let mut result = one.clone();
        let mut base = self.rem_trunc(m);
        let bits = e.bit_length();
        for i in 0..bits {
            if e.mag[(i / 32) as usize] >> (i % 32) & 1 == 1 {
                result = result.mul(&base).rem_trunc(m);
            }
            if i + 1 < bits {
                base = base.mul(&base).rem_trunc(m);
            }
        }
        result
    }
    pub fn shl(&self, n: u64) -> BigInt {
        BigInt::make(self.neg, ushl(&self.mag, (n / 32) as usize, (n % 32) as u32))
    }
    /// An arithmetic shift: a negative value rounds toward negative infinity, as
    /// `mpz_div_2exp` does through the floor division it stands for.
    pub fn shr(&self, n: u64) -> BigInt {
        let mag = ushr(&self.mag, (n / 32) as usize, (n % 32) as u32);
        let v = BigInt::make(self.neg, mag);
        if self.neg {
            // floor: subtract one unless the shifted-out bits were all zero
            let back = v.abs().shl(n);
            if ucmp(&back.mag, &self.mag) != Ordering::Equal {
                return v.sub(&BigInt::from_i64(1));
            }
        }
        v
    }
    pub fn sqrt(&self) -> BigInt {
        debug_assert!(!self.neg);
        if self.mag.len() <= 2 {
            let n = self.to_u64().unwrap_or(0);
            return BigInt::from_u64(isqrt_u64(n));
        }
        // Newton's method from a power-of-two start above the root
        let mut x = BigInt::from_i64(1).shl(self.bit_length().div_ceil(2));
        loop {
            let next = x.add(&self.div_floor(&x)).shr(1);
            if next.cmp(&x) != Ordering::Less {
                return x;
            }
            x = next;
        }
    }
    /// `mpz_gcd` on the magnitudes (the result is never negative).
    pub fn gcd(&self, other: &BigInt) -> BigInt {
        let mut a = self.abs();
        let mut b = other.abs();
        while !b.is_zero() {
            let r = a.rem_trunc(&b);
            a = b;
            b = r;
        }
        a
    }
    /// `mrb_bint_lcm`: `|a| / gcd * |b|`, zero when either is zero.
    pub fn lcm(&self, other: &BigInt) -> BigInt {
        if self.is_zero() || other.is_zero() {
            return BigInt::zero();
        }
        let g = self.gcd(other);
        self.abs().div_floor(&g).mul(&other.abs())
    }

    // ------------------------------------------------------------ bit operations

    /// `mpz_and`/`mpz_or`/`mpz_xor`: both operands as two's complement, the operation
    /// limb by limb, and the result read back as a sign and a magnitude. The reference
    /// works over the longer of the two magnitudes and derives the sign from a rule; one
    /// limb more is taken here, holding the sign extension, so a result that needs the
    /// extra limb (`-1 ^ 0xffffffff`) comes out whole (`docs/gems.md`).
    fn bitop(&self, other: &BigInt, op: fn(u32, u32) -> u32) -> BigInt {
        let n = self.mag.len().max(other.mag.len()) + 1;
        let (x, y) = (self.twos(n), other.twos(n));
        let mut z: Vec<u32> = (0..n).map(|i| op(x[i], y[i])).collect();
        let neg = z[n - 1] != 0;
        if neg {
            let mut carry = true;
            for w in z.iter_mut() { two_comp(w, &mut carry); }
        }
        BigInt::make(neg, z)
    }
    /// The value as `n` two's complement limbs (`n` is wider than the magnitude, so the
    /// top limb is the sign extension: 0 or all ones).
    fn twos(&self, n: usize) -> Vec<u32> {
        let mut carry = true;
        (0..n).map(|i| {
            let v = self.mag.get(i).copied().unwrap_or(0);
            if !self.neg { return v; }
            let mut v = v;
            two_comp(&mut v, &mut carry);
            v
        }).collect()
    }
    pub fn and(&self, other: &BigInt) -> BigInt {
        if self.is_zero() || other.is_zero() {
            return BigInt::zero();
        }
        if !self.neg && !other.neg {
            let n = self.mag.len().min(other.mag.len());
            return BigInt::make(false, (0..n).map(|i| self.mag[i] & other.mag[i]).collect());
        }
        self.bitop(other, |a, b| a & b)
    }
    pub fn or(&self, other: &BigInt) -> BigInt {
        if self.is_zero() {
            return other.clone();
        }
        if other.is_zero() {
            return self.clone();
        }
        if !self.neg && !other.neg {
            let n = self.mag.len().max(other.mag.len());
            return BigInt::make(false, (0..n).map(|i| self.mag.get(i).copied().unwrap_or(0) | other.mag.get(i).copied().unwrap_or(0)).collect());
        }
        self.bitop(other, |a, b| a | b)
    }
    pub fn xor(&self, other: &BigInt) -> BigInt {
        if self.is_zero() {
            return other.clone();
        }
        if other.is_zero() {
            return self.clone();
        }
        if !self.neg && !other.neg {
            let n = self.mag.len().max(other.mag.len());
            return BigInt::make(false, (0..n).map(|i| self.mag.get(i).copied().unwrap_or(0) ^ other.mag.get(i).copied().unwrap_or(0)).collect());
        }
        self.bitop(other, |a, b| a ^ b)
    }
    /// `mrb_bint_rev`: `~x` is `-x - 1`.
    pub fn not(&self) -> BigInt {
        self.neg().sub(&BigInt::from_i64(1))
    }
    /// The two's complement magnitude of a negative value (`mrb_bint_2comp`,
    /// used by `sprintf` for `%b`/`%o`/`%x`).
    pub fn two_comp_mag(&self) -> Vec<u32> {
        let mut carry = true;
        let mut out = Vec::with_capacity(self.mag.len());
        for &x in &self.mag {
            let mut v = x;
            two_comp(&mut v, &mut carry);
            out.push(v);
        }
        out
    }
}

/// `make_2comp`: `v = ~v + c; c = (v == 0 && c)`.
fn two_comp(v: &mut u32, c: &mut bool) {
    let t = (!*v).wrapping_add(*c as u32);
    *v = t;
    *c = t == 0 && *c;
}

fn isqrt_u64(n: u64) -> u64 {
    if n < 2 {
        return n;
    }
    let mut x = n;
    let mut y = x.div_ceil(2);
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}
