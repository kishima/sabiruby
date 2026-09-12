//! `src/bigint.rs` on its own: the arithmetic mruby's `mpz_*` layer provides, checked
//! against `i128` where the values fit and against known answers where they do not.
//! The behaviour of `Integer` itself is checked by `tests/mrbtest` (the reference's
//! `mrbgems/mruby-bigint/test/bigint.rb`) and `tests/custom`.

use sabiruby::bigint::BigInt;

fn b(i: i64) -> BigInt { BigInt::from_i64(i) }
fn s(x: &BigInt) -> String { x.to_string_radix(10) }
fn parse(t: &str) -> BigInt { BigInt::from_str(t.as_bytes(), 10).expect("digits") }

/// A spread of magnitudes and signs, including the `i64` and limb boundaries.
const SEEDS: [i64; 17] = [0, 1, -1, 2, -3, 7, 10, 255, 65535, 4294967295, 4294967296, -4294967296, 1000000007, i32::MAX as i64, i64::MAX, i64::MIN, -9007199254740993];

#[test]
fn matches_i128_where_it_fits() {
    for x in SEEDS {
        for y in SEEDS {
            let (bx, by) = (b(x), b(y));
            let (ix, iy) = (x as i128, y as i128);
            assert_eq!(s(&bx.add(&by)), (ix + iy).to_string(), "{x} + {y}");
            assert_eq!(s(&bx.sub(&by)), (ix - iy).to_string(), "{x} - {y}");
            assert_eq!(s(&bx.mul(&by)), (ix * iy).to_string(), "{x} * {y}");
            assert_eq!(bx.cmp(&by), ix.cmp(&iy), "{x} <=> {y}");
            if y == 0 { continue; }
            let (q, r) = bx.divmod_floor(&by);
            let fq = ix.div_euclid(iy) - if ix.rem_euclid(iy) != 0 && iy < 0 { 1 } else { 0 };
            let fr = ix - fq * iy;
            assert_eq!(s(&q), fq.to_string(), "{x}.div({y})");
            assert_eq!(s(&r), fr.to_string(), "{x} % {y}");
            assert_eq!(s(&bx.rem_trunc(&by)), (ix % iy).to_string(), "{x}.remainder({y})");
            // q * y + r is the number again, at any width
            assert_eq!(s(&q.mul(&by).add(&r)), ix.to_string());
        }
    }
}

#[test]
fn bit_operations_are_twos_complement() {
    for x in SEEDS {
        for y in SEEDS {
            let (ix, iy) = (x as i128, y as i128);
            assert_eq!(s(&b(x).and(&b(y))), (ix & iy).to_string(), "{x} & {y}");
            assert_eq!(s(&b(x).or(&b(y))), (ix | iy).to_string(), "{x} | {y}");
            assert_eq!(s(&b(x).xor(&b(y))), (ix ^ iy).to_string(), "{x} ^ {y}");
        }
        assert_eq!(s(&b(x).not()), (!(x as i128)).to_string(), "~{x}");
        for n in [0u64, 1, 31, 32, 33, 64, 65] {
            // `i128` is the yardstick only while the shifted value fits in it
            let bits = 64 - x.unsigned_abs().leading_zeros() as u64;
            if bits + n < 127 { assert_eq!(s(&b(x).shl(n)), ((x as i128) << n).to_string(), "{x} << {n}"); }
            assert_eq!(s(&b(x).shr(n)), ((x as i128) >> n).to_string(), "{x} >> {n}");
        }
    }
}

#[test]
fn wide_values() {
    let two64 = b(2).pow(64);
    assert_eq!(s(&two64), "18446744073709551616");
    assert_eq!(s(&two64.mul(&two64)), "340282366920938463463374607431768211456");
    assert_eq!(two64.to_string_radix(16), "10000000000000000");
    assert_eq!(two64.to_string_radix(2).len(), 65);
    assert_eq!(two64.to_string_radix(36), "3w5e11264sgsg");
    assert_eq!(s(&b(3).pow(1000)).len(), 478);
    assert_eq!(s(&b(10).pow(40).sqrt()), "100000000000000000000");
    assert_eq!(s(&two64.neg().div_floor(&b(7))), "-2635249153387078803");
    assert_eq!(s(&two64.divmod_floor(&b(-7)).1), "-5");
    assert_eq!(s(&two64.gcd(&b(2).pow(32))), "4294967296");
    assert_eq!(s(&two64.lcm(&b(6))), "55340232221128654848");
    assert_eq!(two64.bit_length(), 65);
    assert_eq!(two64.byte_size(), 12);
    assert!(two64.is_even());
    // `2**64` is exactly a double, and back again
    assert_eq!(two64.to_f64(), 18446744073709551616.0);
    assert_eq!(s(&BigInt::from_f64(1e30)), "1000000000000000019884624838656");
    assert_eq!(s(&BigInt::from_f64(two64.to_f64())), s(&two64));
    // modular exponentiation keeps the sign of the power, as the reference does
    assert_eq!(s(&b(2).pow(64).powm(&b(2), &b(7))), "4");
    assert_eq!(s(&b(-3).powm(&b(3), &b(5))), "-2");
}

#[test]
fn division_uses_every_correction_of_algorithm_d() {
    // a divisor whose top limb is at the normalization boundary, and a dividend that forces
    // the "add the divisor back" step (`qhat` one too large)
    let cases = [
        ("340282366920938463463374607431768211455", "18446744073709551615"),
        ("18446744073709551616", "4294967297"),
        ("123456789012345678901234567890123456789", "987654321098765432109"),
        ("100000000000000000000000000000000", "3"),
        ("18446744073709551615999999999999999999", "18446744073709551616"),
    ];
    for (a, d) in cases {
        let (x, y) = (parse(a), parse(d));
        let (q, r) = x.divmod_trunc(&y);
        assert_eq!(s(&q.mul(&y).add(&r)), a, "{a} / {d}");
        assert!(r.cmp(&y).is_lt() && r.sign() >= 0, "{a} % {d} = {}", s(&r));
    }
}

#[test]
fn text_round_trips_in_every_base() {
    let v = parse("123456789012345678901234567890123456789012345678901234567890");
    for base in 2..=36u32 {
        let t = v.to_string_radix(base);
        assert_eq!(BigInt::from_str(t.as_bytes(), base), Some(v.clone()), "base {base}");
        let n = v.neg();
        assert_eq!(BigInt::from_str(n.to_string_radix(base).as_bytes(), base), Some(n), "base {base} negative");
    }
    assert_eq!(BigInt::from_str(b"1_000_000", 10), Some(b(1000000)));
    assert_eq!(BigInt::from_str(b"12", 2), None);
    assert_eq!(s(&BigInt::zero()), "0");
}

#[test]
fn normalizing_back_to_i64() {
    assert_eq!(b(0).to_i64(), Some(0));
    assert_eq!(b(i64::MAX).to_i64(), Some(i64::MAX));
    assert_eq!(b(i64::MIN).to_i64(), Some(i64::MIN));
    assert_eq!(b(i64::MAX).add(&b(1)).to_i64(), None);
    assert_eq!(b(i64::MIN).sub(&b(1)).to_i64(), None);
    assert_eq!(b(i64::MIN).neg().to_i64(), None);
    assert_eq!(b(i64::MIN).neg().sub(&b(1)).to_i64(), Some(i64::MAX));
}
