//! The `Player` of `docs/plans/host-bridge-plan.md`, end to end: a Rust struct and its `impl`
//! block become a Ruby class, and Ruby drives it.
//!
//! What is checked is everything the generated code is responsible for — the class method, the
//! reading and writing instance methods, `Method#arity`, the type errors, the handle going back
//! to the store when the object is collected, `dup` refusing to copy one, two types side by
//! side, and a method that is handed the `&mut Vm` and calls back into Ruby with it.

use sabiruby::host_store::RubyClass;
use sabiruby::{Value, Vm};
use sabiruby_macros::{RubyClass, ruby_methods};

#[derive(Debug, RubyClass)]
struct Player {
    hp: i64,
    name: String,
}

#[ruby_methods]
impl Player {
    /// `Player.new(100)`: no `self`, so a class method; `Self` goes into the store.
    fn new(hp: i64) -> Self {
        Player { hp, name: String::from("anon") }
    }
    /// A class method that is not a constructor, and one that is given the VM: it reads a
    /// global of the running program.
    fn strongest(vm: &mut Vm, a: i64, b: i64) -> Self {
        let v = vm.global_get("$default_name");
        let name = match vm.str_bytes(v) {
            Some(b) => String::from_utf8_lossy(b).into_owned(),
            None => String::from("anon"),
        };
        Player { hp: a.max(b), name }
    }
    fn hp(&self) -> i64 {
        self.hp
    }
    fn damage(&mut self, n: i64) {
        self.hp -= n;
    }
    fn rename(&mut self, to: String) -> String {
        self.name = to.clone();
        to
    }
    fn name(&self) -> String {
        self.name.clone()
    }
    #[ruby(name = "alive?")]
    fn alive(&self) -> bool {
        self.hp > 0
    }
    /// Given the VM, so it can call back into Ruby: the value is out of the store while it does.
    fn greet(&self, vm: &mut Vm, other: Value) -> sabiruby::error::VmResult<String> {
        let mid = vm.intern("name");
        let them = vm.funcall(other, mid, &[], Value::Nil)?;
        let them = String::from_ruby(vm, them)?;
        Ok(format!("{} greets {}", self.name, them))
    }
    /// The one the host keeps to itself.
    #[ruby(skip)]
    fn secret(&self) -> i64 {
        self.hp * 2
    }
}

/// A second type, registered in the same VM: its own tag, its own store.
#[derive(RubyClass)]
#[ruby(name = "Monster")]
struct Beast {
    kind: String,
}

#[ruby_methods]
impl Beast {
    fn new(kind: String) -> Self {
        Beast { kind }
    }
    fn kind(&self) -> String {
        self.kind.clone()
    }
    fn hp(&self) -> i64 {
        1
    }
}

use sabiruby::FromRuby;

fn compile(src: &str) -> Vec<u8> {
    sabiruby_compiler::compile(src.as_bytes(), &sabiruby_compiler::Options {
        filename: "(test)".into(),
        debug_info: true,
        ..Default::default()
    })
    .expect("compile")
}

fn run(vm: &mut Vm, src: &str) -> String {
    let bin = compile(src);
    vm.load_and_run(&bin).expect("run");
    String::from_utf8_lossy(&vm.take_output()).into_owned()
}

fn vm_with_player() -> Vm {
    let mut vm = Vm::with_mrblib().expect("vm");
    Player::register(&mut vm).expect("Player registers");
    vm
}

#[test]
fn ruby_makes_a_player_and_drives_it() {
    let mut vm = vm_with_player();
    let out = run(&mut vm, r#"
      p Player
      pl = Player.new(100)
      p pl.class, pl.is_a?(Player)
      p pl.hp
      pl.damage(30)
      p pl.hp, pl.alive?
      p pl.rename("ann")
      p pl.name
      pl.damage(80)
      p pl.hp, pl.alive?
      p Player.strongest(3, 9).hp
      $default_name = "hero"
      p Player.strongest(3, 9).name
    "#);
    assert_eq!(out, concat!(
        "Player\n",
        "Player\ntrue\n",
        "100\n",
        "70\ntrue\n",
        "\"ann\"\n",
        "\"ann\"\n",
        "-10\nfalse\n",
        "9\n",
        "\"hero\"\n",
    ));
    // the values are in the store, not in the VM's heap
    assert_eq!(vm.host_store::<Player>().unwrap().len(), 3);
}

#[test]
fn a_skipped_fn_is_not_a_method_and_the_rust_side_still_has_it() {
    let mut vm = vm_with_player();
    assert_eq!(Player::new(21).secret(), 42);
    let out = run(&mut vm, r#"
      pl = Player.new(21)
      p pl.respond_to?(:secret), pl.respond_to?(:hp), pl.respond_to?(:alive?)
      begin; pl.secret; rescue NoMethodError => e; p e.message; end
    "#);
    assert_eq!(out, "false\ntrue\ntrue\n\"undefined method 'secret' for Player\"\n");
}

#[test]
fn arity_is_the_number_of_arguments_the_rust_fn_declares() {
    let mut vm = vm_with_player();
    let out = run(&mut vm, r#"
      pl = Player.new(1)
      p pl.method(:hp).arity, pl.method(:damage).arity, pl.method(:rename).arity
      p pl.method(:greet).arity          # the &mut Vm is context, not an argument
      p Player.method(:new).arity, Player.method(:strongest).arity
      p pl.method(:hp).owner, pl.method(:damage).name
      begin; pl.damage; rescue ArgumentError => e; p e.message; end
      begin; pl.damage(1, 2); rescue ArgumentError => e; p e.message; end
    "#);
    assert_eq!(out, concat!(
        "0\n1\n1\n",
        "1\n",
        "1\n2\n",
        "Player\n:damage\n",
        "\"wrong number of arguments (given 0, expected 1)\"\n",
        "\"wrong number of arguments (given 2, expected 1)\"\n",
    ));
}

#[test]
fn an_argument_of_the_wrong_type_raises_where_the_vms_own_natives_do() {
    let mut vm = vm_with_player();
    let out = run(&mut vm, r#"
      pl = Player.new(1)
      begin; pl.damage("x"); rescue TypeError => e; p e.message; end
      begin; pl.rename(5); rescue TypeError => e; p e.message; end
      begin; pl.rename(nil); rescue TypeError => e; p e.message; end
    "#);
    assert_eq!(out, concat!(
        "\"no implicit conversion of String into Integer\"\n",
        "\"Integer cannot be converted to String\"\n",
        "\"nil cannot be converted to String\"\n",
    ));
}

#[test]
fn a_receiver_that_is_not_one_of_ours_is_a_type_error_naming_the_class() {
    let mut vm = vm_with_player();
    Beast::register(&mut vm).expect("Beast registers");
    // `allocate` makes an ordinary object of the class, with no handle in it: the one way
    // Ruby can reach a Player method with something that is not one. It is worth its own
    // sentence — "wrong argument type Player (expected Player)" reads like a bug in the VM —
    // and mruby words it the same way (`uninitialized %t (expected %s)`, src/etc.c), naming
    // the object's own class, so a subclass says Ghost and the expected one says Player.
    // Reaching the same method through `instance_exec` on something else is the other half:
    // there the class really is wrong, and the message stays the one for that.
    let out = run(&mut vm, r#"
      pl = Player.new(1)
      class Ghost < Player; end
      begin; Ghost.allocate.hp; rescue TypeError => e; p e.message; end
      begin; Player.allocate.damage(1); rescue TypeError => e; p e.message; end
      p pl.hp
    "#);
    assert_eq!(out, concat!(
        "\"uninitialized Ghost (expected Player): the object has no Player behind it \
           — `Player.allocate` makes one without running `initialize`\"\n",
        "\"uninitialized Player (expected Player): the object has no Player behind it \
           — `Player.allocate` makes one without running `initialize`\"\n",
        "1\n",
    ));
    // something that is not a Player at all keeps the message for that, allocated or not:
    // the class is what is wrong, not the missing value
    run(&mut vm, r#"$s = "s""#);
    let s = vm.global_get("$s");
    let err = Player::borrow(&mut vm, s).expect_err("not a Player");
    assert_eq!(vm.describe_error(&err), "wrong argument type String (expected Player) (TypeError)");
    // and a handle of the other type is not this one's, tag against tag
    run(&mut vm, r#"$beast = Monster.new("orc")"#);
    let beast = vm.global_get("$beast");
    let err = Player::borrow(&mut vm, beast).expect_err("not a Player");
    assert_eq!(vm.describe_error(&err), "wrong argument type Monster (expected Player) (TypeError)");
}

#[test]
fn two_types_in_one_vm_keep_their_own_tags_and_stores() {
    let mut vm = vm_with_player();
    Beast::register(&mut vm).expect("Beast registers");
    let p_tag = Player::tag(&mut vm);
    let b_tag = Beast::tag(&mut vm);
    assert_ne!(p_tag, b_tag);
    let out = run(&mut vm, r#"
      $pl = Player.new(10)
      $b = Monster.new("orc")
      p $pl.hp, $b.kind, $b.hp
      p $pl.class, $b.class
      p $pl == $b
    "#);
    assert_eq!(out, "10\n\"orc\"\n1\nPlayer\nMonster\nfalse\n");
    // the handles are both 0, in two different stores: the tag is what keeps them apart
    let pl = vm.global_get("$pl");
    let b = vm.global_get("$b");
    assert_eq!(vm.data_of(pl), Some((p_tag, 0)));
    assert_eq!(vm.data_of(b), Some((b_tag, 0)));
    assert_eq!(vm.host_store::<Player>().unwrap().len(), 1);
    assert_eq!(vm.host_store::<Beast>().unwrap().len(), 1);
}

#[test]
fn collecting_the_object_takes_the_value_out_of_the_store() {
    let mut vm = vm_with_player();
    run(&mut vm, r#"
      $kept = Player.new(100)
      10.times { |i| Player.new(i) }
    "#);
    assert_eq!(vm.host_store::<Player>().unwrap().len(), 11);
    run(&mut vm, "GC.start");
    assert_eq!(vm.host_store::<Player>().unwrap().len(), 1, "the ten on the floor are gone");
    assert_eq!(vm.host_store::<Player>().unwrap().get(0).unwrap().hp, 100);
    run(&mut vm, "$kept = nil; GC.start");
    assert!(vm.host_store::<Player>().unwrap().is_empty());
}

#[test]
fn a_handle_is_not_copied_behind_the_hosts_back() {
    let mut vm = vm_with_player();
    let out = run(&mut vm, r#"
      pl = Player.new(1)
      begin; pl.dup; rescue TypeError => e; p e.message; end
      begin; pl.clone; rescue TypeError => e; p e.message; end
    "#);
    assert_eq!(out, "\"can't dup Player\"\n\"can't clone Player\"\n");
    assert_eq!(vm.host_store::<Player>().unwrap().len(), 1);
}

#[test]
fn a_method_given_the_vm_can_call_back_into_ruby() {
    let mut vm = vm_with_player();
    let out = run(&mut vm, r#"
      a = Player.new(1).tap { |p| p.rename("ann") }
      b = Player.new(1).tap { |p| p.rename("bob") }
      p a.greet(b)
      # anything that answers to `name` will do
      class Dog; def name; "rex"; end; end
      p a.greet(Dog.new)
      # and an exception on the way back out is the caller's
      begin; a.greet(nil); rescue NoMethodError => e; p e.message; end
      p a.name                      # the value went back into the store
    "#);
    assert_eq!(out, concat!(
        "\"ann greets bob\"\n",
        "\"ann greets rex\"\n",
        "\"undefined method 'name' for NilClass\"\n",
        "\"ann\"\n",
    ));
}

#[test]
fn a_call_that_re_enters_the_same_object_raises_rather_than_seeing_half_of_it() {
    let mut vm = vm_with_player();
    let out = run(&mut vm, r#"
      a = Player.new(1).tap { |p| p.rename("ann") }
      # `greet` has `a` out of the store; `other.name` reaches it again
      class Trap
        def initialize(a); @a = a; end
        def name; @a.name; end
      end
      begin; a.greet(Trap.new(a)); rescue RuntimeError => e; p e.message; end
      p a.name, a.hp               # and it is whole again afterwards
    "#);
    assert_eq!(out, concat!(
        "\"Player is already in use by a call on the same object\"\n",
        "\"ann\"\n1\n",
    ));
}

#[test]
fn the_same_type_in_two_vms_does_not_share_anything() {
    let mut a = vm_with_player();
    let mut b = vm_with_player();
    run(&mut a, "$pl = Player.new(1)");
    run(&mut b, "$pl = Player.new(2); $pl2 = Player.new(3)");
    assert_eq!(a.host_store::<Player>().unwrap().len(), 1);
    assert_eq!(b.host_store::<Player>().unwrap().len(), 2);
    assert_eq!(a.host_store::<Player>().unwrap().get(0).unwrap().hp, 1);
    assert_eq!(b.host_store::<Player>().unwrap().get(0).unwrap().hp, 2);
}

#[test]
fn the_host_can_reach_the_values_from_rust_too() {
    let mut vm = vm_with_player();
    run(&mut vm, "$pl = Player.new(50)");
    let pl = vm.global_get("$pl");
    assert_eq!(Player::borrow(&mut vm, pl).expect("borrow").hp, 50);
    Player::borrow_mut(&mut vm, pl).expect("borrow_mut").hp = 7;
    assert_eq!(run(&mut vm, "p $pl.hp"), "7\n");
    // and put one in from Rust
    let made = sabiruby::IntoRuby::into_ruby(Player { hp: 3, name: "made".into() }, &mut vm);
    let g = vm.intern("$made");
    vm.globals.insert(g, sabiruby::value::Slot::from(made));
    assert_eq!(run(&mut vm, "p $made.name, $made.hp, $made.class"), "\"made\"\n3\nPlayer\n");
}
