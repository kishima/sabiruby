//! mruby-objectspace (`mrbgems/mruby-objectspace/src/mruby_objectspace.c`):
//! `ObjectSpace.count_objects`, `ObjectSpace.each_object`. No Ruby part. Both walk the heap
//! (`Heap::ids`), as the reference's `mrb_objspace_each_objects` does.

use alloc::{format, vec::Vec};

use crate::argc;
use crate::error::VmResult;
use crate::object::ObjKind;
use crate::value::{ObjId, Value};
use crate::vm::Vm;

/// The reference's `MRB_TT_*` name of an object, in enum order (the order of the hash).
const TYPES: [&str; 15] = ["T_OBJECT", "T_CLASS", "T_MODULE", "T_ICLASS", "T_SCLASS", "T_PROC", "T_ARRAY", "T_HASH", "T_STRING", "T_RANGE", "T_EXCEPTION", "T_ENV", "T_FIBER", "T_BREAK", "T_BIGINT"];

fn type_index(vm: &Vm, id: ObjId) -> usize {
    match &vm.heap.get(id).kind {
        ObjKind::Object => 0,
        ObjKind::Class(cd) => if cd.iclass_of.is_some() || cd.origin_of.is_some() { 3 } else if cd.is_singleton { 4 } else if cd.is_module { 2 } else { 1 },
        ObjKind::Proc(_) => 5,
        ObjKind::Array(_) => 6,
        ObjKind::Hash(_) => 7,
        ObjKind::String(_) => 8,
        ObjKind::Range { .. } => 9,
        ObjKind::Exception => 10,
        // a Regexp and a MatchData are `T_CDATA` in the reference, which the table has no entry
        // for; they are counted as plain objects here (`docs/gems.md`)
        // a Task is `T_CDATA` too, and is counted the same way, as is a host Data object
        // (which is what `T_CDATA` names in the reference)
        ObjKind::Regexp(_) | ObjKind::MatchData { .. } | ObjKind::Task(_) | ObjKind::Data { .. } => 0,
        ObjKind::Env(_) => 11,
        ObjKind::Fiber(_) => 12,
        ObjKind::Break { .. } => 13,
        // `MRB_TT_BIGINT` follows `MRB_TT_BREAK` in the reference's enum (after the
        // Complex and Rational it has no counterpart for here)
        ObjKind::BigInt(_) => 14,
    }
}

/// `ObjectSpace.count_objects([hash])`: `TOTAL` is every slot, `FREE` the swept ones, then
/// one entry per type that has a live object. A given hash is emptied and filled.
fn count_objects(vm: &mut Vm, _s: Value, a: &[Value], _b: Value) -> VmResult<Value> {
    argc!(vm, a, 0, 1);
    let h = if a.is_empty() {
        vm.hash_new()
    } else {
        match a[0].obj().map(|o| &vm.heap.get(o).kind) {
            Some(ObjKind::Hash(_)) => a[0],
            _ => { let d = vm.describe_for_type_error(a[0]); return Err(vm.raise_type(&format!("{d} cannot be converted to Hash"))); }
        }
    };
    if let Some(ObjKind::Hash(hd)) = h.obj().map(|o| &mut vm.heap.get_mut(o).kind) {
        hd.clear();
    }
    let total = vm.heap.len();
    let mut free = 0i64;
    let mut counts = [0i64; TYPES.len()];
    for i in 0..total {
        let id = ObjId(i as u32);
        if vm.heap.is_free(id) { free += 1; } else { counts[type_index(vm, id)] += 1; }
    }
    let (kt, kf) = (vm.intern("TOTAL"), vm.intern("FREE"));
    vm.hash_set(h, Value::Sym(kt), Value::Int(total as i64))?;
    vm.hash_set(h, Value::Sym(kf), Value::Int(free))?;
    for (i, name) in TYPES.iter().enumerate() {
        if counts[i] != 0 {
            let k = vm.intern(name);
            vm.hash_set(h, Value::Sym(k), Value::Int(counts[i]))?;
        }
    }
    Ok(h)
}

/// `ObjectSpace.each_object([module]) { |obj| }`: every live object that Ruby code can hold
/// (no environments, break objects or include classes), answering the count.
fn each_object(vm: &mut Vm, _s: Value, a: &[Value], b: Value) -> VmResult<Value> {
    argc!(vm, a, 0, 1);
    if b.is_nil() { return Err(vm.raise_arg("no block given")); }
    let filter = match a.first() {
        None => None,
        Some(v) => Some(super::object::class_arg(vm, *v)?),
    };
    let ids: Vec<ObjId> = vm.heap.ids().collect();
    let mut count = 0i64;
    for id in ids {
        if vm.heap.is_free(id) { continue; }
        match &vm.heap.get(id).kind {
            ObjKind::Env(_) | ObjKind::Break { .. } => continue,
            ObjKind::Class(cd) if cd.iclass_of.is_some() || cd.origin_of.is_some() => continue,
            _ => {}
        }
        if let Some(m) = filter {
            if !vm.obj_is_kind_of(Value::Obj(id), m) { continue; }
        }
        vm.call_block(b, &[Value::Obj(id)])?;
        count += 1;
    }
    Ok(Value::Int(count))
}

pub fn init(vm: &mut Vm) {
    let os = vm.define_module("ObjectSpace");
    let sc = vm.singleton_class(Value::Obj(os)).expect("ObjectSpace singleton");
    vm.define_methods(sc, &[("count_objects", count_objects), ("each_object", each_object)]);
}
