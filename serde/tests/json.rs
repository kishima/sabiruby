//! The Ruby `JSON` this crate installs, and `Serde<T>` in a `define_fn` signature.
//!
//! The case in `tests/custom/` is the one that matters: its expected output was produced by
//! CRuby (`ruby json_roundtrip.rb`, 3.2.6 with json 2.6.3), so what is compared is not this
//! crate against itself but against the JSON everybody else has. It lives here rather than in
//! the VM's own `tests/custom/` because the VM and its command line tool do not link serde,
//! and it is compiled at run time by `sabiruby-compiler` rather than checked in as bytecode,
//! because this crate's tests do not need Docker for anything else.

use std::path::Path;

use sabiruby::{Value, Vm};
use sabiruby_serde::{install_json, Serde};
use serde::{Deserialize, Serialize};

fn compile(src: &[u8], name: &str) -> Vec<u8> {
    sabiruby_compiler::compile(src, &sabiruby_compiler::Options {
        filename: name.into(), debug_info: true, ..Default::default()
    }).expect("compile")
}

fn run(vm: &mut Vm, src: &str) -> String {
    let bin = compile(src.as_bytes(), "(test)");
    match vm.load_and_run(&bin) {
        Ok(_) => String::from_utf8_lossy(&vm.take_output()).into_owned(),
        Err(e) => {
            let msg = vm.describe_error(&e);
            let mut out = String::from_utf8_lossy(&vm.take_output()).into_owned();
            out.push_str(&format!("<error: {msg}>\n"));
            out
        }
    }
}

fn json_vm() -> Vm {
    let mut vm = Vm::with_mrblib().expect("vm");
    install_json(&mut vm);
    vm
}

#[test]
fn the_case_cruby_wrote_the_expectation_for() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/custom");
    let src = std::fs::read(dir.join("json_roundtrip.rb")).expect("json_roundtrip.rb");
    let expected = std::fs::read_to_string(dir.join("json_roundtrip.expected")).expect(".expected");
    let mut vm = json_vm();
    let bin = compile(&src, "json_roundtrip.rb");
    let result = vm.load_and_run(&bin);
    let mut out = String::from_utf8_lossy(&vm.take_output()).into_owned();
    if let Err(e) = result {
        let msg = vm.describe_error(&e);
        out.push_str(&format!("<error: {msg}>\n"));
    }
    assert_eq!(out, expected);
}

#[test]
fn the_vm_has_no_json_until_it_is_installed() {
    let mut vm = Vm::with_mrblib().expect("vm");
    assert_eq!(run(&mut vm, "p defined?(JSON)\n"), "nil\n");
    install_json(&mut vm);
    assert_eq!(run(&mut vm, "p defined?(JSON)\n"), "\"constant\"\n");
    // installing twice is harmless
    install_json(&mut vm);
    assert_eq!(run(&mut vm, "puts JSON.generate([1])\n"), "[1]\n");
}

#[test]
fn a_parse_error_is_a_json_parser_error_under_standard_error() {
    let mut vm = json_vm();
    assert_eq!(run(&mut vm, "p JSON::ParserError.superclass\n"), "StandardError\n");
    assert_eq!(run(&mut vm, "p JSON::ParserError.name\n"), "\"JSON::ParserError\"\n");
    // the message is serde_json's, with the place in the text
    let out = run(&mut vm, "begin\n  JSON.parse('[1,')\nrescue => e\n  puts e.class\n  puts e.message\nend\n");
    assert!(out.starts_with("JSON::ParserError\nEOF while parsing"), "{out}");
    assert!(out.contains("line 1 column 3"), "{out}");
    // a plain `rescue` catches it, which is what being a StandardError is for
    assert_eq!(run(&mut vm, "begin\n  JSON.parse('nope')\nrescue\n  p :caught\nend\n"), ":caught\n");
}

#[test]
fn generating_something_json_has_no_room_for_is_a_generator_error() {
    let mut vm = json_vm();
    let out = run(&mut vm, "begin\n  JSON.generate([0.0 / 0.0])\nrescue => e\n  puts e.class\nend\n");
    assert_eq!(out, "JSON::GeneratorError\n");
    // an Array that contains itself stops rather than taking the host's stack down
    let out = run(&mut vm, "a = []\na << a\nbegin\n  JSON.generate(a)\nrescue => e\n  puts e.class\nend\n");
    assert_eq!(out, "JSON::GeneratorError\n");
    // an object of any other class is its to_s, as CRuby's Object#to_json is
    assert_eq!(run(&mut vm, "puts (1..3).to_json\n"), "\"1..3\"\n");
}

#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct Point { x: i64, y: i64 }

#[test]
fn serde_wraps_a_type_into_a_define_fn_signature() {
    let mut vm = json_vm();
    let object = vm.core.object;
    vm.define_fn(object, "shift_point", |p: Serde<Point>| Serde(Point { x: p.0.x + 1, y: p.0.y + 1 }));
    assert_eq!(run(&mut vm, "puts shift_point({\"x\" => 1, \"y\" => 2}).to_json\n"), r#"{"x":2,"y":3}"#.to_string() + "\n");
    // symbol keys read too
    assert_eq!(run(&mut vm, "puts shift_point({x: 1, y: 2}).to_json\n"), r#"{"x":2,"y":3}"#.to_string() + "\n");
    // and a Hash that is not a Point raises the TypeError a wrong argument always raises
    assert_eq!(run(&mut vm, "begin\n  shift_point({\"x\" => 1})\nrescue => e\n  p e.class\nend\n"), "TypeError\n");
}

#[test]
fn a_serialize_that_fails_answers_with_the_exception_rather_than_with_data() {
    struct Bad;
    impl Serialize for Bad {
        fn serialize<S: serde::Serializer>(&self, _s: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom("nope"))
        }
    }
    let mut vm = json_vm();
    let object = vm.core.object;
    vm.define_fn(object, "bad", |_v: Value| Serde(Bad));
    assert_eq!(run(&mut vm, "v = bad(nil)\np v.class, v.message\n"), "TypeError\n\"nope\"\n");
}
