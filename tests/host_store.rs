//! The store the host keeps its own values in (`sabiruby::host_store`): the slab itself, the
//! one-per-type stores a `Vm` carries, and what the collector does to them.
//!
//! The point of the stores being in the VM is the last of those: a `Data` object collected
//! takes the value its handle named with it, without the host having to hold a lock the free
//! hook can reach. What is checked here is that this happens, that it happens to the right
//! store when several types are registered, and that the host's own hook still hears about it.

use std::sync::{Arc, Mutex};

use sabiruby::host_store::{HostStore, RubyClass};
use sabiruby::{IntoRuby, Value, Vm};

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
fn the_slab_hands_a_removed_number_out_again() {
    let mut s: HostStore<String> = HostStore::new();
    assert!(s.is_empty());
    let a = s.insert("ann".into());
    let b = s.insert("bob".into());
    let c = s.insert("cid".into());
    assert_eq!((a, b, c), (0, 1, 2));
    assert_eq!(s.len(), 3);
    assert_eq!(s.get(b).map(String::as_str), Some("bob"));

    s.get_mut(b).unwrap().push_str("by");
    assert_eq!(s.get(b).map(String::as_str), Some("bobby"));

    assert_eq!(s.remove(b), Some("bobby".into()));
    assert_eq!(s.get(b), None);
    assert!(!s.contains(b));
    assert_eq!(s.len(), 2);
    assert_eq!(s.remove(b), None, "removing twice gives nothing, and no second free number");
    assert_eq!(s.insert("dee".into()), b, "bob's number, given out again");
    assert_eq!(s.insert("eve".into()), 3);

    assert_eq!(s.get(99), None);
    assert_eq!(s.remove(99), None);

    let seen: Vec<(u64, &str)> = s.iter().map(|(h, v)| (h, v.as_str())).collect();
    assert_eq!(seen, vec![(0, "ann"), (1, "dee"), (2, "cid"), (3, "eve")]);
    for (_, v) in s.iter_mut() { v.make_ascii_uppercase(); }
    assert_eq!(s.get(0).map(String::as_str), Some("ANN"));
}

#[test]
fn take_keeps_the_number_reserved_until_it_is_restored() {
    let mut s: HostStore<i64> = HostStore::new();
    let a = s.insert(1);
    let b = s.insert(2);
    let out = s.take(a).expect("taken");
    assert_eq!(out, 1);
    assert_eq!(s.len(), 1);
    assert!(s.is_out(a), "the entry says the value is out on loan, not gone");
    assert_eq!(s.take(a), None, "a second take sees nothing rather than the same value twice");
    assert_eq!(s.remove(a), None, "nor is it removed and its number given up while it is out");
    assert_eq!(s.insert(3), 2, "the reserved number is not handed out");
    s.restore(a, out + 10);
    assert_eq!(s.get(a), Some(&11));
    assert_eq!(s.len(), 3);
    // restoring into a number nobody reserved drops the value rather than growing the slab
    s.restore(99, 7);
    assert_eq!(s.len(), 3);
    assert_eq!(s.get(b), Some(&2));
}

#[test]
fn a_vm_carries_one_store_per_type() {
    struct Player { hp: i64 }
    struct Monster { name: String }

    let mut vm = Vm::with_mrblib().expect("vm");
    assert!(vm.host_store::<Player>().is_none(), "nothing is installed until it is asked for");
    assert_eq!(vm.host_store_tag::<Player>(), None);

    let p_tag = vm.install_host_store::<Player>();
    let m_tag = vm.install_host_store::<Monster>();
    assert_ne!(p_tag, m_tag, "two types, two tags");
    assert_eq!(vm.install_host_store::<Player>(), p_tag, "installing again keeps the tag");
    assert_eq!(vm.host_store_tag::<Monster>(), Some(m_tag));

    let h = vm.host_store_mut::<Player>().unwrap().insert(Player { hp: 100 });
    vm.host_store_mut::<Monster>().unwrap().insert(Monster { name: "orc".into() });
    assert_eq!(vm.install_host_store::<Player>(), p_tag);
    assert_eq!(vm.host_store::<Player>().unwrap().get(h).unwrap().hp, 100, "and keeps what is stored");
    assert_eq!(vm.host_store::<Monster>().unwrap().len(), 1);
    assert_eq!(vm.host_store::<Monster>().unwrap().get(0).unwrap().name, "orc");

    // the tags are the same numbers `next_data_tag` hands out, so a host mixing the two schemes
    // does not collide with itself
    let by_hand = vm.next_data_tag();
    assert!(by_hand != p_tag && by_hand != m_tag);
}

#[test]
fn a_vm_with_stores_is_still_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>(_: &T) {}
    struct Player { hp: i64 }
    let mut vm = Vm::with_mrblib().expect("vm");
    vm.install_host_store::<Player>();
    let h = vm.host_store_mut::<Player>().unwrap().insert(Player { hp: 1 });
    assert_eq!(vm.host_store::<Player>().unwrap().get(h).unwrap().hp, 1);
    assert_send_sync(&vm);
}

// ------------------------------------------------------------------ RubyClass by hand

#[derive(Debug)]
struct Player { hp: i64 }
impl RubyClass for Player { const NAME: &'static str = "Player"; }
impl IntoRuby for Player {
    fn into_ruby(self, vm: &mut Vm) -> Value { self.into_handle(vm) }
}

#[derive(Debug)]
struct Monster { name: String }
impl RubyClass for Monster { const NAME: &'static str = "Monster"; }
impl IntoRuby for Monster {
    fn into_ruby(self, vm: &mut Vm) -> Value { self.into_handle(vm) }
}

#[test]
fn a_value_goes_into_the_store_and_the_handle_into_ruby() {
    let mut vm = Vm::with_mrblib().expect("vm");
    Player::register_class(&mut vm);
    let v = Player { hp: 100 }.into_ruby(&mut vm);
    let tag = Player::tag(&mut vm);
    assert_eq!(vm.data_of(v), Some((tag, 0)));
    assert_eq!(Player::borrow(&mut vm, v).expect("borrow").hp, 100);
    Player::borrow_mut(&mut vm, v).expect("borrow_mut").hp -= 30;
    assert_eq!(Player::borrow(&mut vm, v).expect("borrow").hp, 70);

    // taken out for the duration of a call that is given the VM, and put back
    let (h, mut p) = Player::take_out(&mut vm, v).expect("take_out");
    assert!(Player::borrow(&mut vm, v).is_err(), "while it is out, nothing else may have it");
    p.hp = 5;
    Player::give_back(&mut vm, h, p);
    assert_eq!(Player::borrow(&mut vm, v).expect("borrow").hp, 5);
}

#[test]
fn a_handle_of_another_kind_is_a_type_error_naming_the_class() {
    let mut vm = Vm::with_mrblib().expect("vm");
    let monster = Monster { name: "orc".into() }.into_ruby(&mut vm);
    assert_eq!(Monster::borrow(&mut vm, monster).expect("borrow").name, "orc");
    let err = Player::borrow(&mut vm, monster).expect_err("not a Player");
    assert_eq!(vm.describe_error(&err), "wrong argument type Monster (expected Player) (TypeError)");
    let err = Player::borrow(&mut vm, Value::Int(5)).expect_err("not a Player");
    assert_eq!(vm.describe_error(&err), "wrong argument type Integer (expected Player) (TypeError)");
    // and a handle whose value the host dropped itself is a RuntimeError, not a wrong value
    let p = Player { hp: 1 }.into_ruby(&mut vm);
    let (_tag, h) = vm.data_of(p).unwrap();
    Player::store(&mut vm).remove(h);
    let err = Player::borrow(&mut vm, p).expect_err("gone");
    assert_eq!(vm.describe_error(&err), "stale Player handle: the host dropped the value (RuntimeError)");
}

#[test]
fn collecting_the_ruby_object_drops_the_value_it_named() {
    let mut vm = Vm::with_mrblib().expect("vm");
    // the host's own hook still hears about it, after the store has dropped the value
    let freed: Arc<Mutex<Vec<(u32, u64)>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = freed.clone();
    vm.set_on_free(Box::new(move |tag, handle| sink.lock().unwrap().push((tag, handle))));

    Player::register_class(&mut vm);
    Monster::register_class(&mut vm);
    let object = vm.core.object;
    vm.define_fn(object, "player", |hp: i64| Player { hp });
    vm.define_fn(object, "monster", |name: String| Monster { name });
    vm.define_fn(object, "hp", |vm: &mut Vm, v: Value| -> sabiruby::error::VmResult<i64> {
        Ok(Player::borrow(vm, v)?.hp)
    });

    let out = run(&mut vm, r#"
      $kept = player(100)
      player(1); player(2)
      monster("orc")
      p hp($kept)
      GC.start
    "#);
    assert_eq!(out, "100\n");
    let p_tag = Player::tag(&mut vm);
    assert_eq!(vm.host_store::<Player>().unwrap().len(), 1, "the two on the floor are gone");
    assert_eq!(vm.host_store::<Monster>().unwrap().len(), 0, "so is the monster, from its own store");
    assert_eq!(vm.host_store::<Player>().unwrap().get(0).unwrap().hp, 100);
    let seen = freed.lock().unwrap().clone();
    assert_eq!(seen.len(), 3, "the host's hook heard about all three: {seen:?}");
    assert!(seen.contains(&(p_tag, 1)) && seen.contains(&(p_tag, 2)));

    run(&mut vm, "$kept = nil; GC.start");
    assert_eq!(vm.host_store::<Player>().unwrap().len(), 0);
    assert_eq!(freed.lock().unwrap().len(), 4);
}

#[test]
fn a_handle_the_collector_reclaimed_is_given_out_again() {
    let mut vm = Vm::with_mrblib().expect("vm");
    Player::register_class(&mut vm);
    let object = vm.core.object;
    vm.define_fn(object, "player", |hp: i64| Player { hp });
    run(&mut vm, "$a = player(1); player(2); $a = nil; GC.start");
    assert_eq!(vm.host_store::<Player>().unwrap().len(), 0);
    let v = Player { hp: 3 }.into_ruby(&mut vm);
    assert_eq!(vm.data_of(v).unwrap().1, 1, "the last number freed comes back first");
}
