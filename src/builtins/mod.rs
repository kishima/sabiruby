//! Native (Rust) implementations of the core classes. Whatever mruby writes
//! in `mrblib/*.rb` is *not* reimplemented here: that bytecode is embedded and
//! loaded by [`Vm::with_mrblib`], so `Array#each`, `Integer#times`,
//! `Enumerable`, `Comparable` and friends run as ordinary Ruby methods.

use alloc::{format, string::String, vec::Vec};

pub mod array;
pub mod exception;
pub mod hash;
pub mod kernel;
pub mod numeric;
pub mod object;
pub mod proc_;
pub mod range;
pub mod string;
pub mod symbol;

use crate::error::VmResult;
use crate::object::{InstanceKind, ObjKind};
use crate::value::{ObjId, Value};
use crate::vm::Vm;

pub fn init(vm: &mut Vm) {
    object::init(vm);
    kernel::init(vm);
    numeric::init(vm);
    symbol::init(vm);
    string::init(vm);
    array::init(vm);
    hash::init(vm);
    range::init(vm);
    proc_::init(vm);
    exception::init(vm);
}

// ---------------------------------------------------------------- shared helpers

impl Vm {
    /// `inspect` through method dispatch (so Ruby overrides are honoured).
    pub fn inspect(&mut self, v: Value) -> VmResult<Vec<u8>> {
        if let Value::Obj(o) = v {
            if self.inspect_guard.contains(&o) {
                return Ok(match self.heap.get(o).kind { ObjKind::Array(_) => b"[...]".to_vec(), ObjKind::Hash(_) => b"{...}".to_vec(), _ => b"...".to_vec() });
            }
            self.inspect_guard.push(o);
            let r = self.funcall(v, self.s.inspect, &[], Value::Nil);
            self.inspect_guard.pop();
            let r = r?;
            return Ok(self.str_bytes(r).map(|b| b.to_vec()).unwrap_or_default());
        }
        let r = self.funcall(v, self.s.inspect, &[], Value::Nil)?;
        Ok(self.str_bytes(r).map(|b| b.to_vec()).unwrap_or_default())
    }
    pub fn inspect_str(&mut self, v: Value) -> VmResult<String> {
        Ok(String::from_utf8_lossy(&self.inspect(v)?).into_owned())
    }
    /// `to_s` through method dispatch.
    pub fn to_s(&mut self, v: Value) -> VmResult<Vec<u8>> {
        self.as_string(v)
    }

    /// `==` through method dispatch, with fast paths for immediates.
    pub fn equal(&mut self, a: Value, b: Value) -> VmResult<bool> {
        match (a, b) {
            (Value::Obj(_), _) | (_, Value::Obj(_)) => {
                if a == b { return Ok(true); }
                let r = self.funcall(a, self.s.eq, &[b], Value::Nil)?;
                Ok(r.truthy())
            }
            (Value::Int(x), Value::Float(y)) => Ok(numeric::int_float_cmp(x, y) == Some(core::cmp::Ordering::Equal)),
            (Value::Float(x), Value::Int(y)) => Ok(numeric::int_float_cmp(y, x) == Some(core::cmp::Ordering::Equal)),
            _ => Ok(a == b),
        }
    }

    /// Allocates an instance of `class` with the right internal representation
    /// (`mrb_instance_alloc`): the first built-in ancestor decides the kind.
    pub fn instance_alloc(&mut self, class: ObjId) -> VmResult<Value> {
        let cd = self.heap.class(class);
        if cd.is_module { return Err(self.raise_type("can't create instance of module")); }
        if cd.is_singleton { return Err(self.raise_type("can't create instance of singleton class")); }
        let core = self.core;
        let mut c = Some(class);
        let ik = loop {
            match c {
                None => break InstanceKind::Object,
                Some(x) => match self.heap.class(x).instance_kind { Some(k) => break k, None => c = self.heap.class(x).superclass },
            }
        };
        let kind = match ik {
            InstanceKind::Object => ObjKind::Object,
            InstanceKind::Exception => ObjKind::Exception,
            InstanceKind::String => ObjKind::String(Vec::new()),
            InstanceKind::Array => ObjKind::Array(Vec::new()),
            InstanceKind::Hash => ObjKind::Hash(Default::default()),
            InstanceKind::Range => ObjKind::Range { begin: crate::value::Slot::NIL, end: crate::value::Slot::NIL, excl: false },
            InstanceKind::Proc | InstanceKind::NoAlloc => { let n = self.class_name(class); return Err(self.raise(core.no_method_error, &format!("undefined method 'new' for {n}"))); }
        };
        Ok(Value::Obj(self.heap.alloc(class, kind)))
    }

    /// `Class#new`: allocate, then `initialize`.
    pub fn class_new_instance(&mut self, class: ObjId, args: &[Value], blk: Value) -> VmResult<Value> {
        let obj = self.instance_alloc(class)?;
        let init = self.s.initialize;
        if self.respond_to(obj, init) {
            self.funcall(obj, init, args, blk)?;
        } else if !args.is_empty() {
            return Err(self.argnum_error(args.len(), "0"));
        }
        Ok(obj)
    }

    pub fn check_argc(&mut self, args: &[Value], min: usize, max: usize) -> VmResult<()> {
        if args.len() < min || args.len() > max {
            let exp = if min == max { format!("{min}") } else if max == usize::MAX { format!("{min}+") } else { format!("{min}..{max}") };
            return Err(self.argnum_error(args.len(), &exp));
        }
        Ok(())
    }
}

/// Wraps a native fn body: `native!(vm, self_, args, blk, { ... })`.
#[macro_export]
macro_rules! argc {
    ($vm:expr, $args:expr, $n:expr) => {
        $vm.check_argc($args, $n, $n)?
    };
    ($vm:expr, $args:expr, $min:expr, $max:expr) => {
        $vm.check_argc($args, $min, $max)?
    };
}

/// `Object#dup` as a function (used by `Exception#exception`).
pub fn object_dup(vm: &mut Vm, v: Value) -> VmResult<Value> {
    let dup = vm.intern("dup");
    vm.funcall(v, dup, &[], Value::Nil)
}
