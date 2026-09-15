//! Rust → Ruby → Rust for every shape of serde's data model, and what the Ruby value in the
//! middle actually looks like.
//!
//! The round trip alone would pass even if both halves agreed on something Ruby cannot read,
//! so each case also checks the Ruby side with a script: the Hash a struct became is the Hash
//! a script sees, with the keys it expects.

use std::collections::BTreeMap;

use sabiruby::{Value, Vm};
use sabiruby_serde::{from_value, to_value, to_value_with, Options};
use serde::{Deserialize, Serialize};

fn vm() -> Vm {
    Vm::with_mrblib().expect("vm")
}

/// `p value` as the VM prints it: the Ruby side of what the conversion built.
fn inspect(vm: &mut Vm, v: Value) -> String {
    vm.global_set("$it", v);
    let src = sabiruby_compiler::compile(b"p $it", &sabiruby_compiler::Options {
        filename: "(test)".into(), debug_info: true, ..Default::default()
    }).expect("compile");
    vm.load_and_run(&src).expect("run");
    String::from_utf8_lossy(&vm.take_output()).trim_end().to_string()
}

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
struct Config {
    name: String,
    retries: u32,
    verbose: bool,
    tags: Vec<String>,
    limit: Option<i64>,
}

fn a_config() -> Config {
    Config { name: "sabi".into(), retries: 3, verbose: true, tags: vec!["a".into(), "b".into()], limit: None }
}

#[test]
fn a_struct_is_a_hash_with_string_keys() {
    let mut vm = vm();
    let c = a_config();
    let v = to_value(&mut vm, &c).expect("to_value");
    assert_eq!(inspect(&mut vm, v), r#"{"name" => "sabi", "retries" => 3, "verbose" => true, "tags" => ["a", "b"], "limit" => nil}"#);
    let back: Config = from_value(&mut vm, v).expect("from_value");
    assert_eq!(back, c);
}

#[test]
fn symbol_keys_are_an_option_and_both_read_back() {
    let mut vm = vm();
    let c = a_config();
    let v = to_value_with(&mut vm, &c, Options::symbol_keys()).expect("to_value");
    assert_eq!(inspect(&mut vm, v), r#"{name: "sabi", retries: 3, verbose: true, tags: ["a", "b"], limit: nil}"#);
    // reading is the forgiving direction: Symbol keys fit the same struct
    let back: Config = from_value(&mut vm, v).expect("from_value");
    assert_eq!(back, c);
}

#[derive(Serialize, Deserialize, PartialEq, Debug)]
enum Shape {
    Dot,
    Radius(f64),
    Rect(i64, i64),
    Named { w: i64, h: i64 },
}

#[test]
fn a_unit_variant_is_a_symbol_and_the_others_are_tagged_hashes() {
    let mut vm = vm();
    for (shape, printed) in [
        (Shape::Dot, ":Dot"),
        (Shape::Radius(1.5), r#"{"Radius" => 1.5}"#),
        (Shape::Rect(2, 3), r#"{"Rect" => [2, 3]}"#),
        (Shape::Named { w: 4, h: 5 }, r#"{"Named" => {"w" => 4, "h" => 5}}"#),
    ] {
        let v = to_value(&mut vm, &shape).expect("to_value");
        assert_eq!(inspect(&mut vm, v), printed, "{shape:?}");
        let back: Shape = from_value(&mut vm, v).expect("from_value");
        assert_eq!(back, shape);
    }
}

#[test]
fn a_variant_may_also_be_read_from_a_string_or_a_symbol_keyed_hash() {
    let mut vm = vm();
    let s = vm.str_new(b"Dot");
    assert_eq!(from_value::<Shape>(&mut vm, s).expect("String variant"), Shape::Dot);
    let h = vm.hash_new();
    let k = Value::Sym(vm.intern("Radius"));
    vm.hash_set(h, k, Value::Float(2.0)).expect("hash_set");
    assert_eq!(from_value::<Shape>(&mut vm, h).expect("Symbol variant"), Shape::Radius(2.0));
}

#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct Nested {
    inner: Vec<Config>,
    map: BTreeMap<String, Vec<Option<i64>>>,
    pair: (i64, String),
}

#[test]
fn nested_structures_survive_the_trip() {
    let mut vm = vm();
    let mut map = BTreeMap::new();
    map.insert("x".to_string(), vec![Some(1), None, Some(-3)]);
    let n = Nested { inner: vec![a_config()], map, pair: (7, "seven".into()) };
    let v = to_value(&mut vm, &n).expect("to_value");
    let back: Nested = from_value(&mut vm, v).expect("from_value");
    assert_eq!(back, n);
    // the map is a Hash and the tuple an Array, not the other way round
    assert!(inspect(&mut vm, v).contains(r#""map" => {"x" => [1, nil, -3]}"#));
    assert!(inspect(&mut vm, v).contains(r#""pair" => [7, "seven"]"#));
}

#[test]
fn option_is_nil_and_nil_is_none() {
    let mut vm = vm();
    let some: Option<i64> = Some(5);
    let none: Option<i64> = None;
    let v = to_value(&mut vm, &some).expect("some");
    assert_eq!(v, Value::Int(5));
    assert_eq!(to_value(&mut vm, &none).expect("none"), Value::Nil);
    assert_eq!(from_value::<Option<i64>>(&mut vm, Value::Nil).expect("nil"), None);
    assert_eq!(from_value::<Option<i64>>(&mut vm, Value::Int(5)).expect("5"), Some(5));
    // Some(None) cannot be told from None once it is in Ruby; that is the mapping, not a bug
    let nested: Option<Option<i64>> = Some(None);
    let v = to_value(&mut vm, &nested).expect("nested");
    assert_eq!(from_value::<Option<Option<i64>>>(&mut vm, v).expect("back"), None);
}

#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct Numbers {
    i_min: i64,
    i_max: i64,
    u_max: u64,
    small: u8,
    f: f64,
}

#[test]
fn numeric_boundaries() {
    let mut vm = vm();
    let n = Numbers { i_min: i64::MIN, i_max: i64::MAX, u_max: u64::MAX, small: 255, f: 1.5 };
    let v = to_value(&mut vm, &n).expect("to_value");
    // u64::MAX does not fit a Value::Int, so it is a wide Integer — and prints as the number
    assert_eq!(inspect(&mut vm, v), format!(
        r#"{{"i_min" => {}, "i_max" => {}, "u_max" => {}, "small" => 255, "f" => 1.5}}"#,
        i64::MIN, i64::MAX, u64::MAX));
    let back: Numbers = from_value(&mut vm, v).expect("from_value");
    assert_eq!(back, n);

    // i128 / u128 past the wide boundary
    let big: i128 = i128::from(u64::MAX) + 1;
    let v = to_value(&mut vm, &big).expect("i128");
    assert_eq!(inspect(&mut vm, v), "18446744073709551616");
    assert_eq!(from_value::<i128>(&mut vm, v).expect("back"), big);

    // out of range is an error, not a wrap
    let too_big = Value::Int(300);
    assert!(from_value::<u8>(&mut vm, too_big).is_err());
    // and a Float is not silently rounded into an Integer
    assert!(from_value::<i64>(&mut vm, Value::Float(1.0)).is_err());
    // but an Integer does widen where a Float is asked for, as `mrb_ensure_float_type` does
    assert_eq!(from_value::<f64>(&mut vm, Value::Int(3)).expect("widen"), 3.0);
}

/// A field of bytes: `serialize_bytes` / `deserialize_byte_buf` without pulling in
/// `serde_bytes` for one test.
#[derive(PartialEq, Debug)]
struct Blob(Vec<u8>);

impl Serialize for Blob {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bytes(&self.0)
    }
}

impl<'de> Deserialize<'de> for Blob {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Blob, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = Blob;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { f.write_str("bytes") }
            fn visit_bytes<E: serde::de::Error>(self, v: &[u8]) -> Result<Blob, E> { Ok(Blob(v.to_vec())) }
            fn visit_byte_buf<E: serde::de::Error>(self, v: Vec<u8>) -> Result<Blob, E> { Ok(Blob(v)) }
        }
        d.deserialize_byte_buf(V)
    }
}

#[test]
fn bytes_that_are_not_utf8_go_through_as_bytes() {
    let mut vm = vm();
    let b = Blob(vec![0x00, 0xff, 0xfe, b'a']);
    let v = to_value(&mut vm, &b).expect("to_value");
    assert_eq!(vm.str_bytes(v).expect("a String"), &b.0[..]);
    assert!(vm.str_binary(v), "bytes are marked binary");
    assert_eq!(from_value::<Blob>(&mut vm, v).expect("from_value"), b);
    // the same String read as text is an error rather than a lossy replacement
    assert!(from_value::<String>(&mut vm, v).is_err());
}

#[test]
fn a_ruby_string_that_is_not_utf8_reaches_deserialize_any_as_bytes() {
    let mut vm = vm();
    let v = vm.str_new(&[0xff, 0xfe]);
    let e = from_value::<serde_json::Value>(&mut vm, v).expect_err("bytes are not a JSON value");
    assert_eq!(vm.describe_error(&e), "invalid type: byte array, expected any valid JSON value (TypeError)");
    // a type that does want bytes gets them
    assert_eq!(from_value::<Blob>(&mut vm, v).expect("blob").0, vec![0xff, 0xfe]);
}

#[test]
fn a_failed_conversion_raises_a_type_error_the_script_can_see() {
    let mut vm = vm();
    let e = from_value::<Config>(&mut vm, Value::Int(1)).expect_err("not a Hash");
    assert_eq!(vm.describe_error(&e), "cannot deserialize Integer as Hash (TypeError)");
    let h = vm.hash_new();
    let k = vm.str_new(b"name");
    vm.hash_set(h, k, Value::Int(1)).expect("hash_set");
    let e = from_value::<Config>(&mut vm, h).expect_err("name is not a String");
    assert_eq!(vm.describe_error(&e), "cannot deserialize Integer as String (TypeError)");
}

#[test]
fn unknown_keys_are_ignored_whatever_they_hold() {
    let mut vm = vm();
    let c = a_config();
    let v = to_value(&mut vm, &c).expect("to_value");
    // a key of a class serde has no name for must not fail a conversion that ignores it
    let k = vm.str_new(b"callback");
    let proc_ = {
        let src = sabiruby_compiler::compile(b"$p = proc { 1 }", &sabiruby_compiler::Options {
            filename: "(test)".into(), debug_info: true, ..Default::default()
        }).expect("compile");
        vm.load_and_run(&src).expect("run");
        vm.global_get("$p")
    };
    vm.hash_set(v, k, proc_).expect("hash_set");
    let back: Config = from_value(&mut vm, v).expect("from_value");
    assert_eq!(back, c);
}
