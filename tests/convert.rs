//! `Vm::define_fn`: a Rust function registered as a Ruby method, with its arguments and its
//! answer converted (`sabiruby::convert`).
//!
//! What is checked here is the round trip of each `FromRuby` / `IntoRuby` impl, the exception
//! class and wording when a conversion cannot be made, the argument count (which a method
//! built this way knows, unlike a `define_closure` one, so `Method#arity` answers with it),
//! and each of the shapes a function may have.

use sabiruby::convert::{Block, Bytes, This};
use sabiruby::error::VmResult;
use sabiruby::{Value, Vm};

fn compile(src: &str) -> Vec<u8> {
    sabiruby_compiler::compile(src.as_bytes(), &sabiruby_compiler::Options {
        filename: "(test)".into(), debug_info: true, ..Default::default()
    }).expect("compile")
}

fn run(vm: &mut Vm, src: &str) -> String {
    let bin = compile(src);
    vm.load_and_run(&bin).expect("run");
    String::from_utf8_lossy(&vm.take_output()).into_owned()
}

/// Runs and gives back the error message instead of panicking.
fn run_err(vm: &mut Vm, src: &str) -> String {
    let bin = compile(src);
    match vm.load_and_run(&bin) {
        Ok(_) => panic!("expected a raise, got none"),
        Err(e) => vm.describe_error(&e),
    }
}

#[test]
fn arguments_are_converted_to_the_rust_types_asked_for() {
    let mut vm = Vm::with_mrblib().expect("vm");
    let o = vm.core.object;
    vm.define_fn(o, "add", |a: i64, b: i64| a + b);
    vm.define_fn(o, "half", |a: f64| a / 2.0);
    vm.define_fn(o, "not", |a: bool| !a);
    vm.define_fn(o, "shout", |s: String| s.to_uppercase());
    vm.define_fn(o, "sym_s", |s: sabiruby::symbol::Sym, vm2: Value| { let _ = vm2; s });
    vm.define_fn(o, "narrow", |a: i32| a * 2);
    vm.define_fn(o, "sum", |v: Vec<i64>| v.iter().sum::<i64>());
    vm.define_fn(o, "nested", |v: Vec<Vec<i64>>| v.iter().map(|x| x.len() as i64).sum::<i64>());
    vm.define_fn(o, "maybe", |a: Option<i64>| a.unwrap_or(-1));
    vm.define_fn(o, "raw", |v: Value| v);
    vm.define_fn(o, "first_byte", |b: Bytes| b.0.first().copied().map(|x| x as i64).unwrap_or(-1));
    let out = run(&mut vm, r#"
      p add(2, 3)
      p half(5)
      p half(5.0)
      p not(nil), not(false), not(0)
      p shout("abc")
      p sym_s(:x, nil), sym_s("y", nil)
      p narrow(21)
      p sum([1, 2, 3])
      p nested([[1], [2, 3]])
      p maybe(nil), maybe(7)
      p raw(:sym)
      p first_byte("Az")
    "#);
    assert_eq!(out, concat!(
        "5\n",
        "2.5\n2.5\n",
        "true\ntrue\nfalse\n",
        "\"ABC\"\n",
        ":x\n:y\n",
        "42\n",
        "6\n",
        "3\n",
        "-1\n7\n",
        ":sym\n",
        "65\n",
    ));
}

#[test]
fn answers_are_converted_back_to_ruby() {
    let mut vm = Vm::with_mrblib().expect("vm");
    let o = vm.core.object;
    vm.define_fn(o, "nothing", || ());
    vm.define_fn(o, "an_int", || 7i64);
    vm.define_fn(o, "a_u32", || 7u32);
    vm.define_fn(o, "a_big", || u64::MAX);
    vm.define_fn(o, "a_float", || 1.5f64);
    vm.define_fn(o, "a_bool", || true);
    vm.define_fn(o, "a_str", || "hi");
    vm.define_fn(o, "a_string", || String::from("there"));
    vm.define_fn(o, "some_bytes", || Bytes(vec![0xff, 0x41]));
    vm.define_fn(o, "an_array", || vec![1i64, 2, 3]);
    vm.define_fn(o, "a_pair", || (1i64, "two"));
    vm.define_fn(o, "a_six", || (1i64, 2i64, 3i64, 4i64, 5i64, 6i64));
    vm.define_fn(o, "an_option", |b: bool| if b { Some(1i64) } else { None });
    vm.define_fn(o, "a_value", |vm: &mut Vm| vm.str_from(String::from("made")));
    let out = run(&mut vm, r#"
      p nothing, an_int, a_u32, a_big, a_float, a_bool
      p a_str, a_string
      p some_bytes.bytes
      p an_array, a_pair, a_six
      p an_option(true), an_option(false)
      p a_value
    "#);
    assert_eq!(out, concat!(
        "nil\n7\n7\n18446744073709551615\n1.5\ntrue\n",
        "\"hi\"\n\"there\"\n",
        "[255, 65]\n",
        "[1, 2, 3]\n[1, \"two\"]\n[1, 2, 3, 4, 5, 6]\n",
        "1\nnil\n",
        "\"made\"\n",
    ));
}

#[test]
fn a_conversion_that_cannot_be_made_raises_what_the_vm_raises() {
    let mut vm = Vm::with_mrblib().expect("vm");
    let o = vm.core.object;
    vm.define_fn(o, "want_int", |a: i64| a);
    vm.define_fn(o, "want_i32", |a: i32| a as i64);
    vm.define_fn(o, "want_str", |s: String| s);
    vm.define_fn(o, "want_ary", |v: Vec<i64>| v.len() as i64);
    vm.define_fn(o, "want_float", |f: f64| f);
    let out = run(&mut vm, r#"
      def err
        yield
      rescue => e
        p [e.class, e.message]
      end
      err { want_int(nil) }
      err { want_int("2") }
      err { want_i32(1 << 40) }
      err { want_str(5) }
      err { want_str(nil) }
      err { want_ary(5) }
      err { want_float(:x) }
    "#);
    assert_eq!(out, concat!(
        "[TypeError, \"no implicit conversion of nil into Integer\"]\n",
        "[TypeError, \"no implicit conversion of String into Integer\"]\n",
        "[RangeError, \"integer out of range\"]\n",
        "[TypeError, \"Integer cannot be converted to String\"]\n",
        "[TypeError, \"nil cannot be converted to String\"]\n",
        "[TypeError, \"no implicit conversion into Array\"]\n",
        "[TypeError, \"can't convert Symbol into Float\"]\n",
    ));
}

#[test]
fn the_number_of_arguments_is_checked_and_arity_answers_with_it() {
    let mut vm = Vm::with_mrblib().expect("vm");
    let c = vm.define_class("Calc", vm.core.object);
    vm.define_fn(c, "zero", || 0i64);
    vm.define_fn(c, "one", |a: i64| a);
    vm.define_fn(c, "two", |a: i64, b: i64| a + b);
    vm.define_fn(c, "six", |a: i64, b: i64, c: i64, d: i64, e: i64, f: i64| a + b + c + d + e + f);
    // a closure registered by hand still says -1: it has no declared argument list
    vm.define_closure(c, "raw", |_vm, _s, a, _b| Ok(Value::Int(a.len() as i64)));
    let out = run(&mut vm, r#"
      k = Calc.new
      p k.method(:zero).arity, k.method(:one).arity, k.method(:two).arity, k.method(:six).arity
      p k.method(:raw).arity
      p Calc.instance_method(:two).arity
      p k.six(1, 2, 3, 4, 5, 6)
      begin; k.two(1); rescue ArgumentError => e; p e.message; end
      begin; k.two(1, 2, 3); rescue ArgumentError => e; p e.message; end
      begin; k.zero(1); rescue ArgumentError => e; p e.message; end
    "#);
    assert_eq!(out, concat!(
        "0\n1\n2\n6\n",
        "-1\n",
        "2\n",
        "21\n",
        "\"wrong number of arguments (given 1, expected 2)\"\n",
        "\"wrong number of arguments (given 3, expected 2)\"\n",
        "\"wrong number of arguments (given 1, expected 0)\"\n",
    ));
}

#[test]
fn a_function_can_take_the_receiver_the_vm_and_the_block() {
    let mut vm = Vm::with_mrblib().expect("vm");
    let c = vm.define_class("Box", vm.core.object);
    // the receiver, untouched and converted
    vm.define_fn(c, "myself", |this: This<Value>| this.0);
    vm.define_fn(c, "label", |vm: &mut Vm, this: This<Value>| {
        let n = vm.intern("@n");
        match this.0 { Value::Obj(o) => vm.heap.ivar_get(o, n), _ => Value::Nil }
    });
    // the VM, to call back into Ruby
    vm.define_fn(c, "twice", |vm: &mut Vm, blk: Block| -> VmResult<Value> {
        match blk.0 {
            Some(b) => { vm.call_block(b, &[Value::Int(1)])?; vm.call_block(b, &[Value::Int(2)]) }
            None => Ok(Value::Nil),
        }
    });
    // receiver + arguments + block together
    vm.define_fn(c, "apply", |vm: &mut Vm, this: This<Value>, n: i64, blk: Block| -> VmResult<Value> {
        match blk.0 { Some(b) => vm.call_block(b, &[this.0, Value::Int(n)]), None => Ok(Value::Int(n)) }
    });
    // Integer#doubled: the receiver converted to an i64
    let int = vm.core.integer;
    vm.define_fn(int, "doubled", |this: This<i64>| this.0 * 2);
    let out = run(&mut vm, r#"
      b = Box.new
      b.instance_variable_set(:@n, :named)
      p b.myself.equal?(b)
      p b.label
      b.twice { |i| puts i }
      p b.apply(3) { |r, n| [r.class, n] }
      p b.apply(3)
      p 21.doubled
    "#);
    assert_eq!(out, concat!(
        "true\n",
        ":named\n",
        "1\n2\n",
        "[Box, 3]\n",
        "3\n",
        "42\n",
    ));
}

#[test]
fn a_function_answers_with_a_result_to_raise() {
    let mut vm = Vm::with_mrblib().expect("vm");
    let o = vm.core.object;
    // a chosen exception class needs the VM
    vm.define_fn(o, "checked", |vm: &mut Vm, n: i64| -> VmResult<i64> {
        if n < 0 { Err(vm.raise_arg("negative")) } else { Ok(n * 2) }
    });
    // without one, a String or a &'static str is a RuntimeError
    vm.define_fn(o, "owned_err", |n: i64| -> Result<i64, String> {
        if n < 0 { Err(format!("no {n}")) } else { Ok(n) }
    });
    vm.define_fn(o, "static_err", |n: i64| -> Result<i64, &'static str> {
        if n < 0 { Err("nope") } else { Ok(n) }
    });
    let out = run(&mut vm, r#"
      def err
        yield
      rescue => e
        p [e.class, e.message]
      end
      p checked(2), owned_err(2), static_err(2)
      err { checked(-1) }
      err { owned_err(-1) }
      err { static_err(-1) }
    "#);
    assert_eq!(out, concat!(
        "4\n2\n2\n",
        "[ArgumentError, \"negative\"]\n",
        "[RuntimeError, \"no -1\"]\n",
        "[RuntimeError, \"nope\"]\n",
    ));
    // unrescued, it comes back out of load_and_run
    assert!(run_err(&mut vm, "checked(-1)").contains("negative"));
}

#[test]
fn a_typed_method_is_a_native_method_like_any_other() {
    let mut vm = Vm::with_mrblib().expect("vm");
    let c = vm.define_class("Thing", vm.core.object);
    vm.define_fn(c, "spin", |a: i64| a);
    let out = run(&mut vm, r#"
      t = Thing.new
      p t.respond_to?(:spin), Thing.method_defined?(:spin)
      p Thing.instance_methods(false)
      m = t.method(:spin)
      p m.owner, m.name, m.arity
      p m.call(9)
      p m.inspect
      class Thing
        alias twirl spin
      end
      p t.twirl(3)
      p t.send(:spin, 4)
    "#);
    assert_eq!(out, concat!(
        "true\ntrue\n",
        "[:spin]\n",
        "Thing\n:spin\n1\n",
        "9\n",
        "\"#<Method: Thing#spin>\"\n",
        "3\n",
        "4\n",
    ));
}

#[test]
fn a_typed_method_keeps_the_vm_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>(_: &T) {}
    let mut vm = Vm::with_mrblib().expect("vm");
    let o = vm.core.object;
    let base = 100i64;
    vm.define_fn(o, "plus_base", move |a: i64| a + base);
    assert_send_sync(&vm);
    assert_eq!(run(&mut vm, "p plus_base(5)"), "105\n");
    // and it is reached by funcall from native code, not only from bytecode
    let mid = vm.intern("plus_base");
    let top = Value::Obj(vm.top_self);
    let v = vm.funcall(top, mid, &[Value::Int(7)], Value::Nil).expect("funcall");
    assert_eq!(v, Value::Int(107));
}
