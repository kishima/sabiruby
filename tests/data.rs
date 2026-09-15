//! Data objects: a value the host owns, named in Ruby by a `(tag, handle)` pair rather than
//! held by the VM (`Vm::data_new`, `Vm::data_of`, `Vm::set_on_free`).
//!
//! What is checked here is that such an object behaves as an ordinary Ruby object where it
//! should (class, ivars, freeze, `ObjectSpace`), that `==` / `eql?` / `hash` go by the handle
//! while `equal?` stays identity, that `dup` and `clone` refuse to copy a handle, and — the
//! reason the kind exists — that the host is told when one is collected, both by `GC.start`
//! and under the stress mode, and not while it is still reachable.

use std::sync::{Arc, Mutex};

use sabiruby::convert::{DataRef, This};
use sabiruby::{Value, Vm};

/// The host's own kind number for the values in these tests.
const PLAYER: u32 = 7;

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

#[test]
fn a_data_object_carries_a_handle_the_vm_never_reads() {
    let mut vm = Vm::with_mrblib().expect("vm");
    let player = vm.define_class("Player", vm.core.object);
    let v = vm.data_new(player, PLAYER, 42);
    assert_eq!(vm.data_of(v), Some((PLAYER, 42)));
    // anything else has none
    assert_eq!(vm.data_of(Value::Int(42)), None);
    let s = vm.str_from(String::from("x"));
    assert_eq!(vm.data_of(s), None);
    // it is an ordinary object to Ruby otherwise
    let g = vm.intern("$p");
    vm.globals.insert(g, sabiruby::value::Slot::from(v));
    let out = run(&mut vm, r##"
      p $p.class, $p.is_a?(Player), $p.is_a?(Object)
      p $p.inspect.start_with?("#<Player:0x")
      $p.instance_variable_set(:@n, 3)
      p $p.instance_variable_get(:@n), $p.instance_variables
      p $p.frozen?
      $p.freeze
      p $p.frozen?
      p $p.respond_to?(:class)
    "##);
    assert_eq!(out, "Player\ntrue\ntrue\ntrue\n3\n[:@n]\nfalse\ntrue\ntrue\n");
}

#[test]
fn equality_goes_by_the_handle_and_identity_stays_identity() {
    let mut vm = Vm::with_mrblib().expect("vm");
    let player = vm.define_class("Player", vm.core.object);
    let object = vm.core.object;
    // two objects for the same host value, one for another, one of another kind
    vm.define_fn(object, "one", |vm: &mut Vm| { let c = vm.define_class("Player", vm.core.object); vm.data_new(c, PLAYER, 1) });
    vm.define_fn(object, "two", |vm: &mut Vm| { let c = vm.define_class("Player", vm.core.object); vm.data_new(c, PLAYER, 2) });
    vm.define_fn(object, "other_kind", |vm: &mut Vm| { let c = vm.define_class("Player", vm.core.object); vm.data_new(c, PLAYER + 1, 1) });
    let _ = player;
    let out = run(&mut vm, r#"
      a, b, c, d = one, one, two, other_kind
      p a == b, a.eql?(b), a.hash == b.hash
      p a.equal?(b), a.equal?(a)
      p a == c, a == d, a == 1, a == nil
      h = {}
      h[a] = :first
      h[b] = :second
      h[c] = :third
      p h.size, h[a], h[one]
      p [a, b, c].uniq.size
    "#);
    assert_eq!(out, concat!(
        "true\ntrue\ntrue\n",
        "false\ntrue\n",
        "false\nfalse\nfalse\nfalse\n",
        "2\n:second\n:second\n",
        "2\n",
    ));
}

#[test]
fn a_handle_is_not_copied_behind_the_hosts_back() {
    let mut vm = Vm::with_mrblib().expect("vm");
    let player = vm.define_class("Player", vm.core.object);
    let v = vm.data_new(player, PLAYER, 1);
    let g = vm.intern("$p");
    vm.globals.insert(g, sabiruby::value::Slot::from(v));
    let out = run(&mut vm, r#"
      begin; $p.dup; rescue TypeError => e; p e.message; end
      begin; $p.clone; rescue TypeError => e; p e.message; end
    "#);
    assert_eq!(out, "\"can't dup Player\"\n\"can't clone Player\"\n");
}

#[test]
fn the_host_is_told_when_a_data_object_is_collected() {
    let freed: Arc<Mutex<Vec<(u32, u64)>>> = Arc::new(Mutex::new(Vec::new()));
    let mut vm = Vm::with_mrblib().expect("vm");
    let sink = freed.clone();
    vm.set_on_free(Box::new(move |tag, handle| sink.lock().unwrap().push((tag, handle))));
    let object = vm.core.object;
    vm.define_fn(object, "make", |vm: &mut Vm, n: i64| {
        let c = vm.define_class("Player", vm.core.object);
        vm.data_new(c, PLAYER, n as u64)
    });
    // one kept in a global, one dropped on the floor
    run(&mut vm, "$kept = make(1); make(2); GC.start");
    assert_eq!(*freed.lock().unwrap(), vec![(PLAYER, 2)]);
    // the kept one is not freed however often the collector runs
    run(&mut vm, "GC.start; GC.start");
    assert_eq!(*freed.lock().unwrap(), vec![(PLAYER, 2)]);
    // until nothing holds it
    run(&mut vm, "$kept = nil; GC.start");
    assert_eq!(*freed.lock().unwrap(), vec![(PLAYER, 2), (PLAYER, 1)]);
}

#[test]
fn the_hook_is_called_under_the_stress_mode_too() {
    let freed: Arc<Mutex<Vec<u64>>> = Arc::new(Mutex::new(Vec::new()));
    let mut vm = Vm::with_mrblib().expect("vm");
    let sink = freed.clone();
    vm.set_on_free(Box::new(move |_tag, handle| sink.lock().unwrap().push(handle)));
    let object = vm.core.object;
    vm.define_fn(object, "make", |vm: &mut Vm, n: i64| {
        let c = vm.define_class("Player", vm.core.object);
        vm.data_new(c, PLAYER, n as u64)
    });
    // collect after every allocation, as `SABIRUBY_GC_STRESS=1` does for the test runner
    vm.set_gc_stress(true);
    assert_eq!(run(&mut vm, "$kept = make(1000); 100.times { |i| make(i) }; p :ok"), ":ok\n");
    vm.set_gc_stress(false);
    let seen = freed.lock().unwrap().clone();
    assert_eq!(seen.len(), 100, "every short-lived handle is freed once: {seen:?}");
    let mut sorted = seen.clone();
    sorted.sort();
    assert_eq!(sorted, (0..100).collect::<Vec<u64>>());
    // the one held by a global survived all of it
    assert!(!seen.contains(&1000));
}

#[test]
fn a_handle_goes_back_to_the_hosts_own_table() {
    // the host's slab: the values live here, Ruby only ever sees their index
    let slab: Arc<Mutex<Vec<Option<String>>>> = Arc::new(Mutex::new(Vec::new()));
    let mut vm = Vm::with_mrblib().expect("vm");

    let store = slab.clone();
    vm.set_on_free(Box::new(move |tag, handle| {
        if tag == PLAYER { store.lock().unwrap()[handle as usize] = None; }
    }));

    let player = vm.define_class("Player", vm.core.object);
    let sc = vm.singleton_class(Value::Obj(player)).expect("singleton");
    let store = slab.clone();
    vm.define_fn(sc, "named", move |vm: &mut Vm, name: String| {
        let handle = { let mut s = store.lock().unwrap(); s.push(Some(name)); (s.len() - 1) as u64 };
        let c = vm.define_class("Player", vm.core.object);
        vm.data_new(c, PLAYER, handle)
    });
    let store = slab.clone();
    vm.define_fn(player, "name", move |this: This<DataRef>| {
        store.lock().unwrap()[this.handle as usize].clone()
    });
    let store = slab.clone();
    vm.define_fn(player, "rename", move |this: This<DataRef>, to: String| {
        store.lock().unwrap()[this.handle as usize] = Some(to.clone());
        to
    });
    // a handle taken as an argument rather than as the receiver
    let store = slab.clone();
    let object = vm.core.object;
    vm.define_fn(object, "name_of", move |d: DataRef| store.lock().unwrap()[d.handle as usize].clone());

    let out = run(&mut vm, r#"
      $a = Player.named("ann")
      b = Player.named("bob")
      p $a.name, b.name
      p $a.rename("anna")
      p $a.name
      p name_of($a)
      # anything that is not a Data object is a TypeError, not a wrong handle
      begin; name_of(Object.new); rescue TypeError => e; p e.message; end
      begin; name_of(5); rescue TypeError => e; p e.message; end
      b = nil
      GC.start
    "#);
    assert_eq!(out, concat!(
        "\"ann\"\n\"bob\"\n",
        "\"anna\"\n",
        "\"anna\"\n",
        "\"anna\"\n",
        "\"wrong argument type Object (expected Data)\"\n",
        "\"wrong argument type Integer (expected Data)\"\n",
    ));
    // bob's slot was given back, ann's is still there
    let s = slab.lock().unwrap();
    assert_eq!(s[0], Some(String::from("anna")));
    assert_eq!(s[1], None);
}

#[test]
fn object_space_walks_over_data_objects() {
    let mut vm = Vm::with_mrblib().expect("vm");
    let object = vm.core.object;
    vm.define_fn(object, "make", |vm: &mut Vm, n: i64| {
        let c = vm.define_class("Player", vm.core.object);
        vm.data_new(c, PLAYER, n as u64)
    });
    let out = run(&mut vm, r#"
      $held = [make(1), make(2)]
      p ObjectSpace.each_object(Player) { |o| }
      before = ObjectSpace.count_objects[:T_OBJECT]
      $held = nil
      GC.start
      p ObjectSpace.count_objects[:T_OBJECT] < before
      p ObjectSpace.each_object(Player) { |o| }
    "#);
    assert_eq!(out, "2\ntrue\n0\n");
}

#[test]
fn a_vm_with_a_free_hook_is_still_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>(_: &T) {}
    let freed = Arc::new(Mutex::new(0u64));
    let sink = freed.clone();
    let mut vm = Vm::with_mrblib().expect("vm");
    vm.set_on_free(Box::new(move |_t, _h| { *sink.lock().unwrap() += 1; }));
    assert_send_sync(&vm);
    let object = vm.core.object;
    vm.define_fn(object, "make", |vm: &mut Vm| { let c = vm.core.object; vm.data_new(c, PLAYER, 1) });
    assert_eq!(run(&mut vm, "make; GC.start; p :ok"), ":ok\n");
    assert_eq!(*freed.lock().unwrap(), 1);
}
