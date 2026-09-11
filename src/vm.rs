//! The interpreter: register machine executing mruby 4.1.0 bytecode.
//!
//! Frame layout follows mruby: a callee's `R0` is the caller's `R[a]`
//! (`base = caller.base + a`), so a method's return value lands in the
//! caller's target register by writing `stack[callee.base]`.

use alloc::{format, string::String, string::ToString, vec, vec::Vec};

use hashbrown::HashMap;

use crate::error::{VmError, VmResult};
use crate::object::{BreakTag, ClassData, EnvData, Heap, IrepId, Method, ObjKind, ProcData};
use crate::opcode::{Op, Operands};
use crate::rite::{self, CatchType, Pool};
use crate::symbol::{Interner, Sym};
use crate::value::{ObjId, Value};

pub struct VmIrep {
    pub nlocals: usize,
    pub nregs: usize,
    pub iseq: Vec<u8>,
    pub catch: Vec<rite::CatchHandler>,
    pub pool: Vec<Pool>,
    pub syms: Vec<Sym>,
    pub reps: Vec<IrepId>,
    pub lv: Vec<Option<Sym>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cci {
    /// Ordinary Ruby frame.
    None,
    /// Frame started from native code (`Vm::call_proc`); the interpreter loop
    /// returns to the native caller when this frame is popped.
    Skip,
}

#[derive(Clone, Copy, Debug)]
pub struct CallInfo {
    pub base: usize,
    pub pc: usize,
    pub irep: IrepId,
    pub proc_: ObjId,
    /// Number of positional args (15 = packed into an array at R1).
    pub n: u8,
    pub kw: bool,
    pub mid: Option<Sym>,
    pub target_class: ObjId,
    pub env: Option<ObjId>,
    pub cci: Cci,
}

/// Well-known classes and modules.
#[derive(Clone, Copy)]
pub struct Core {
    pub basic_object: ObjId,
    pub object: ObjId,
    pub module: ObjId,
    pub class: ObjId,
    pub kernel: ObjId,
    pub comparable: ObjId,
    pub enumerable: ObjId,
    pub nil_class: ObjId,
    pub true_class: ObjId,
    pub false_class: ObjId,
    pub numeric: ObjId,
    pub integer: ObjId,
    pub float: ObjId,
    pub symbol: ObjId,
    pub string: ObjId,
    pub array: ObjId,
    pub hash: ObjId,
    pub range: ObjId,
    pub proc_: ObjId,
    pub exception: ObjId,
    pub standard_error: ObjId,
    pub runtime_error: ObjId,
    pub argument_error: ObjId,
    pub type_error: ObjId,
    pub name_error: ObjId,
    pub no_method_error: ObjId,
    pub zero_division_error: ObjId,
    pub local_jump_error: ObjId,
    pub index_error: ObjId,
    pub range_error: ObjId,
    pub key_error: ObjId,
    pub not_implemented_error: ObjId,
    pub stop_iteration: ObjId,
    pub frozen_error: ObjId,
    pub float_domain_error: ObjId,
    pub no_matching_pattern_error: ObjId,
    pub system_stack_error: ObjId,
}

/// Frequently used symbols.
#[derive(Clone, Copy)]
pub struct Syms {
    pub initialize: Sym,
    pub to_s: Sym,
    pub inspect: Sym,
    pub call: Sym,
    pub mesg: Sym,
    pub method_missing: Sym,
    pub eq: Sym,
    pub eqq: Sym,
    pub hash: Sym,
    pub eql: Sym,
    pub each: Sym,
    pub plus: Sym,
    pub minus: Sym,
    pub mul: Sym,
    pub div: Sym,
    pub lt: Sym,
    pub le: Sym,
    pub gt: Sym,
    pub ge: Sym,
    pub aref: Sym,
    pub aset: Sym,
    pub attached: Sym,
}

pub struct Vm {
    pub heap: Heap,
    pub syms: Interner,
    pub ireps: Vec<VmIrep>,
    pub stack: Vec<Value>,
    pub ci: Vec<CallInfo>,
    pub globals: HashMap<Sym, Value>,
    /// The exception being propagated (`mrb->exc`) between `L_RAISE` and `EXCEPT`.
    pub exc: Option<Value>,
    out: Vec<u8>,
    pub core: Core,
    pub s: Syms,
    pub top_self: ObjId,
    /// Instruction budget for [`Vm::step`]; `None` = unlimited.
    step_left: Option<u64>,
    /// The Proc whose body is a single `OP_CALL` (mruby `call_proc`): the
    /// method body of `Proc#call`, so calling a block does not re-enter the VM.
    pub call_proc: ObjId,
    pub instructions: u64,
    /// Executions per opcode (index = opcode number); the test runner reports
    /// which opcodes a workload never reached.
    pub op_counts: Vec<u64>,
    /// Nesting of native -> VM re-entries (`call_proc_with`); bounded to protect the host stack.
    native_depth: u32,
    /// Objects whose `inspect` is in progress (recursive containers print `[...]`).
    pub inspect_guard: Vec<ObjId>,
}

/// Result of [`Vm::step`].
#[derive(Debug, PartialEq)]
pub enum Step {
    /// The budget ran out; call `step` again to continue.
    Paused,
    /// The top-level program finished with this value.
    Finished(Value),
}

impl Vm {
    /// A VM with the core classes and native methods, but without `mrblib`
    /// (so `Array#each`, `Integer#times`, `Enumerable` etc. are missing).
    pub fn new() -> Vm {
        let mut syms = Interner::default();
        let mut heap = Heap::default();
        // Bootstrap: BasicObject < Object < Module < Class, then fix up their classes.
        let mk = |heap: &mut Heap, syms: &mut Interner, name: &str, sup: Option<ObjId>, module: bool| {
            let n = syms.intern_str(name);
            heap.alloc_raw(ObjKind::Class(ClassData {
                name: Some(n),
                superclass: sup,
                is_module: module,
                ..Default::default()
            }))
        };
        let basic_object = mk(&mut heap, &mut syms, "BasicObject", None, false);
        let object = mk(&mut heap, &mut syms, "Object", Some(basic_object), false);
        let module = mk(&mut heap, &mut syms, "Module", Some(object), false);
        let class = mk(&mut heap, &mut syms, "Class", Some(module), false);
        let kernel = mk(&mut heap, &mut syms, "Kernel", None, true);
        let comparable = mk(&mut heap, &mut syms, "Comparable", None, true);
        let enumerable = mk(&mut heap, &mut syms, "Enumerable", None, true);
        let c = |heap: &mut Heap, syms: &mut Interner, name: &str, sup: ObjId| mk(heap, syms, name, Some(sup), false);
        let nil_class = c(&mut heap, &mut syms, "NilClass", object);
        let true_class = c(&mut heap, &mut syms, "TrueClass", object);
        let false_class = c(&mut heap, &mut syms, "FalseClass", object);
        let numeric = c(&mut heap, &mut syms, "Numeric", object);
        let integer = c(&mut heap, &mut syms, "Integer", numeric);
        let float = c(&mut heap, &mut syms, "Float", numeric);
        let symbol = c(&mut heap, &mut syms, "Symbol", object);
        let string = c(&mut heap, &mut syms, "String", object);
        let array = c(&mut heap, &mut syms, "Array", object);
        let hash = c(&mut heap, &mut syms, "Hash", object);
        let range = c(&mut heap, &mut syms, "Range", object);
        let proc_ = c(&mut heap, &mut syms, "Proc", object);
        let exception = c(&mut heap, &mut syms, "Exception", object);
        let standard_error = c(&mut heap, &mut syms, "StandardError", exception);
        let runtime_error = c(&mut heap, &mut syms, "RuntimeError", standard_error);
        let argument_error = c(&mut heap, &mut syms, "ArgumentError", standard_error);
        let type_error = c(&mut heap, &mut syms, "TypeError", standard_error);
        let name_error = c(&mut heap, &mut syms, "NameError", standard_error);
        let no_method_error = c(&mut heap, &mut syms, "NoMethodError", name_error);
        let zero_division_error = c(&mut heap, &mut syms, "ZeroDivisionError", standard_error);
        let local_jump_error = c(&mut heap, &mut syms, "LocalJumpError", standard_error);
        let index_error = c(&mut heap, &mut syms, "IndexError", standard_error);
        let range_error = c(&mut heap, &mut syms, "RangeError", standard_error);
        let key_error = c(&mut heap, &mut syms, "KeyError", index_error);
        let script_error = c(&mut heap, &mut syms, "ScriptError", exception);
        let not_implemented_error = c(&mut heap, &mut syms, "NotImplementedError", script_error);
        let stop_iteration = c(&mut heap, &mut syms, "StopIteration", index_error);
        let frozen_error = c(&mut heap, &mut syms, "FrozenError", runtime_error);
        let float_domain_error = c(&mut heap, &mut syms, "FloatDomainError", range_error);
        let no_matching_pattern_error = c(&mut heap, &mut syms, "NoMatchingPatternError", standard_error);
        let system_stack_error = c(&mut heap, &mut syms, "SystemStackError", exception);
        let core = Core {
            basic_object, object, module, class, kernel, comparable, enumerable, nil_class, true_class,
            false_class, numeric, integer, float, symbol, string, array, hash, range, proc_, exception,
            standard_error, runtime_error, argument_error, type_error, name_error, no_method_error,
            zero_division_error, local_jump_error, index_error, range_error, key_error,
            not_implemented_error, stop_iteration, frozen_error, float_domain_error,
            no_matching_pattern_error, system_stack_error,
        };
        // Every object allocated so far is a class or module: set its class.
        for i in 0..heap.len() {
            let id = ObjId(i as u32);
            let is_mod = heap.class(id).is_module;
            heap.get_mut(id).class = if is_mod { module } else { class };
        }
        let s = Syms {
            initialize: syms.intern_str("initialize"),
            to_s: syms.intern_str("to_s"),
            inspect: syms.intern_str("inspect"),
            call: syms.intern_str("call"),
            mesg: syms.intern_str("mesg"),
            method_missing: syms.intern_str("method_missing"),
            eq: syms.intern_str("=="),
            eqq: syms.intern_str("==="),
            hash: syms.intern_str("hash"),
            eql: syms.intern_str("eql?"),
            each: syms.intern_str("each"),
            plus: syms.intern_str("+"),
            minus: syms.intern_str("-"),
            mul: syms.intern_str("*"),
            div: syms.intern_str("/"),
            lt: syms.intern_str("<"),
            le: syms.intern_str("<="),
            gt: syms.intern_str(">"),
            ge: syms.intern_str(">="),
            aref: syms.intern_str("[]"),
            aset: syms.intern_str("[]="),
            attached: syms.intern_str("__attached__"),
        };
        let top_self = heap.alloc(object, ObjKind::Object);
        let call_irep = VmIrep { nlocals: 1, nregs: 4, iseq: vec![Op::Call as u8], catch: vec![], pool: vec![], syms: vec![], reps: vec![], lv: vec![] };
        let call_proc = heap.alloc(core.proc_, ObjKind::Proc(ProcData { irep: 0, upper: None, env: None, target_class: Some(core.proc_), strict: true, scope: true, orphan: false }));
        let mut vm = Vm {
            heap, syms, ireps: vec![call_irep], stack: Vec::new(), ci: Vec::new(), globals: HashMap::new(),
            exc: None, out: Vec::new(), core, s, top_self, step_left: None, instructions: 0, op_counts: vec![0; crate::opcode::OP_COUNT], native_depth: 0, inspect_guard: Vec::new(), call_proc,
        };
        // Constants for the core classes, Object includes Kernel.
        for i in 0..vm.heap.len() {
            let id = ObjId(i as u32);
            if vm.heap.is_class(id) {
                if let Some(n) = vm.heap.class(id).name {
                    vm.heap.class_mut(object).consts.insert(n, Value::Obj(id));
                }
            }
        }
        vm.include_module(object, kernel);
        // Every class gets its metaclass up front (mruby `make_metaclass`), so
        // that `Sub.exception` finds the singleton method defined on `Exception`.
        for i in 0..vm.heap.len() {
            let id = ObjId(i as u32);
            if vm.heap.is_class(id) && !vm.heap.class(id).is_module {
                vm.singleton_class(Value::Obj(id)).expect("metaclass");
            }
        }
        crate::builtins::init(&mut vm);
        vm
    }

    /// A VM with `mrblib` (the Ruby part of mruby's core library) loaded.
    pub fn with_mrblib() -> VmResult<Vm> {
        let mut vm = Vm::new();
        vm.load_and_run(crate::MRBLIB_MRB)?;
        Ok(vm)
    }

    // ------------------------------------------------------------------ output

    /// Bytes written by `puts`/`p`/`print` since the last [`Vm::take_output`].
    pub fn take_output(&mut self) -> Vec<u8> {
        core::mem::take(&mut self.out)
    }
    pub fn write_out(&mut self, bytes: &[u8]) {
        self.out.extend_from_slice(bytes);
    }

    // ------------------------------------------------------------------ symbols

    pub fn intern(&mut self, name: &str) -> Sym {
        self.syms.intern_str(name)
    }
    pub fn sym_name(&self, s: Sym) -> String {
        self.syms.name_str(s)
    }

    // ------------------------------------------------------------------ objects

    pub fn str_new(&mut self, bytes: &[u8]) -> Value {
        Value::Obj(self.heap.alloc(self.core.string, ObjKind::String(bytes.to_vec())))
    }
    pub fn str_from(&mut self, s: String) -> Value {
        Value::Obj(self.heap.alloc(self.core.string, ObjKind::String(s.into_bytes())))
    }
    pub fn ary_new(&mut self, v: Vec<Value>) -> Value {
        Value::Obj(self.heap.alloc(self.core.array, ObjKind::Array(v)))
    }
    pub fn hash_new(&mut self) -> Value {
        Value::Obj(self.heap.alloc(self.core.hash, ObjKind::Hash(Default::default())))
    }
    pub fn range_new(&mut self, begin: Value, end: Value, excl: bool) -> Value {
        Value::Obj(self.heap.alloc(self.core.range, ObjKind::Range { begin, end, excl }))
    }
    pub fn exc_new(&mut self, class: ObjId, msg: &str) -> Value {
        let e = self.heap.alloc(class, ObjKind::Exception);
        let m = self.str_new(msg.as_bytes());
        let mesg = self.s.mesg;
        self.heap.ivar_set(e, mesg, m);
        Value::Obj(e)
    }
    /// Builds an error to return with `?`: `Err(vm.raise(class, "msg"))`.
    pub fn raise(&mut self, class: ObjId, msg: &str) -> VmError {
        VmError::Raise(self.exc_new(class, msg))
    }
    pub fn raise_type(&mut self, msg: &str) -> VmError {
        self.raise(self.core.type_error, msg)
    }
    pub fn raise_arg(&mut self, msg: &str) -> VmError {
        self.raise(self.core.argument_error, msg)
    }
    pub fn argnum_error(&mut self, given: usize, expected: &str) -> VmError {
        self.raise(self.core.argument_error, &format!("wrong number of arguments (given {given}, expected {expected})"))
    }
    pub fn str_bytes(&self, v: Value) -> Option<&[u8]> {
        v.obj().and_then(|o| self.heap.string(o))
    }
    pub fn ary(&self, v: Value) -> Option<&Vec<Value>> {
        v.obj().and_then(|o| self.heap.array(o))
    }
    pub fn expect_str(&mut self, v: Value, what: &str) -> VmResult<Vec<u8>> {
        match self.str_bytes(v) {
            Some(b) => Ok(b.to_vec()),
            None => Err(self.raise_type(&format!("{what} cannot be converted to String"))),
        }
    }
    pub fn expect_int(&mut self, v: Value, what: &str) -> VmResult<i64> {
        match v {
            Value::Int(i) => Ok(i),
            Value::Float(f) => Ok(f as i64),
            _ => Err(self.raise_type(&format!("{what} cannot be converted to Integer"))),
        }
    }

    // ------------------------------------------------------------------ classes

    pub fn class_of(&self, v: Value) -> ObjId {
        match v {
            Value::Nil => self.core.nil_class,
            Value::False => self.core.false_class,
            Value::True => self.core.true_class,
            Value::Int(_) => self.core.integer,
            Value::Float(_) => self.core.float,
            Value::Sym(_) => self.core.symbol,
            Value::Obj(o) => self.heap.get(o).class,
        }
    }
    /// The class as seen by Ruby (skipping singleton and include classes).
    pub fn real_class_of(&self, v: Value) -> ObjId {
        let mut c = self.class_of(v);
        loop {
            let cd = self.heap.class(c);
            if cd.is_singleton || cd.iclass_of.is_some() {
                c = cd.superclass.expect("singleton without superclass");
            } else {
                return c;
            }
        }
    }
    pub fn class_name(&self, c: ObjId) -> String {
        let cd = self.heap.class(c);
        if let Some(m) = cd.iclass_of {
            return self.class_name(m);
        }
        match cd.name {
            Some(n) => self.syms.name_str(n),
            None => {
                if cd.is_singleton {
                    "#<Class>".to_string()
                } else {
                    "#<Class:anonymous>".to_string()
                }
            }
        }
    }
    pub fn define_class(&mut self, name: &str, superclass: ObjId) -> ObjId {
        let n = self.intern(name);
        if let Some(Value::Obj(c)) = self.heap.class(self.core.object).consts.get(&n).copied() {
            return c;
        }
        let c = self.heap.alloc(self.core.class, ObjKind::Class(ClassData { name: Some(n), superclass: Some(superclass), ..Default::default() }));
        self.heap.class_mut(self.core.object).consts.insert(n, Value::Obj(c));
        self.singleton_class(Value::Obj(c)).expect("metaclass");
        c
    }
    pub fn define_module(&mut self, name: &str) -> ObjId {
        let n = self.intern(name);
        if let Some(Value::Obj(c)) = self.heap.class(self.core.object).consts.get(&n).copied() {
            return c;
        }
        let c = self.heap.alloc(self.core.module, ObjKind::Class(ClassData { name: Some(n), is_module: true, ..Default::default() }));
        self.heap.class_mut(self.core.object).consts.insert(n, Value::Obj(c));
        c
    }
    pub fn define_method(&mut self, class: ObjId, name: &str, f: crate::object::NativeFn) {
        let n = self.intern(name);
        self.heap.class_mut(class).methods.insert(n, Method::Native(f));
    }
    /// Defines the same native method on several classes.
    pub fn define_methods(&mut self, class: ObjId, list: &[(&str, crate::object::NativeFn)]) {
        for (n, f) in list {
            self.define_method(class, n, *f);
        }
    }
    /// Inserts an include class for `module` right above `class` in the chain.
    pub fn include_module(&mut self, class: ObjId, module: ObjId) {
        // Already included?
        let mut c = self.heap.class(class).superclass;
        while let Some(x) = c {
            if self.heap.class(x).iclass_of == Some(module) {
                return;
            }
            c = self.heap.class(x).superclass;
        }
        let sup = self.heap.class(class).superclass;
        let ic = self.heap.alloc(self.core.class, ObjKind::Class(ClassData { superclass: sup, iclass_of: Some(module), ..Default::default() }));
        self.heap.class_mut(class).superclass = Some(ic);
    }
    /// The singleton class of `v`, created on demand (`prepare_singleton_class`).
    pub fn singleton_class(&mut self, v: Value) -> VmResult<ObjId> {
        let o = match v {
            Value::Obj(o) => o,
            _ => return Err(self.raise_type("can't define singleton")),
        };
        let cur = self.heap.get(o).class;
        if self.heap.class(cur).is_singleton && self.heap.class(cur).attached == Some(v) {
            return Ok(cur);
        }
        // For a class, the metaclass's superclass is the superclass's metaclass.
        let sup = if self.heap.is_class(o) && !self.heap.class(o).is_module {
            match self.heap.class(o).superclass {
                Some(s) if !self.heap.class(s).is_singleton => Some(self.singleton_class(Value::Obj(s))?),
                Some(s) => Some(s),
                None => Some(cur),
            }
        } else {
            Some(cur)
        };
        let sc = self.heap.alloc(self.core.class, ObjKind::Class(ClassData { superclass: sup, is_singleton: true, attached: Some(v), ..Default::default() }));
        self.heap.get_mut(o).class = sc;
        Ok(sc)
    }
    pub fn const_get(&self, class: ObjId, name: Sym) -> Option<Value> {
        let mut c = Some(class);
        while let Some(x) = c {
            let cd = self.heap.class(x);
            let tbl = match cd.iclass_of {
                Some(m) => &self.heap.class(m).consts,
                None => &cd.consts,
            };
            if let Some(v) = tbl.get(&name) {
                return Some(*v);
            }
            c = cd.superclass;
        }
        None
    }
    pub fn obj_is_kind_of(&self, v: Value, class: ObjId) -> bool {
        let mut c = Some(self.class_of(v));
        while let Some(x) = c {
            let cd = self.heap.class(x);
            if x == class || cd.iclass_of == Some(class) {
                return true;
            }
            c = cd.superclass;
        }
        false
    }
    /// Method lookup along the superclass chain; returns the method and the
    /// class that owns it (for `super`).
    pub fn find_method(&self, class: ObjId, mid: Sym) -> Option<(Method, ObjId)> {
        let mut c = Some(class);
        while let Some(x) = c {
            let cd = self.heap.class(x);
            let tbl = match cd.iclass_of {
                Some(m) => &self.heap.class(m).methods,
                None => &cd.methods,
            };
            if let Some(m) = tbl.get(&mid) {
                return match m {
                    Method::Undef => None,
                    _ => Some((m.clone(), x)),
                };
            }
            c = cd.superclass;
        }
        None
    }
    pub fn respond_to(&self, v: Value, mid: Sym) -> bool {
        self.find_method(self.class_of(v), mid).is_some()
    }

    // ------------------------------------------------------------------ loading

    /// Loads a RITE binary; returns the id of its top-level irep.
    pub fn load(&mut self, bin: &[u8]) -> VmResult<IrepId> {
        let rite = rite::parse(bin)?;
        let offset = self.ireps.len();
        for ir in &rite.ireps {
            let syms = ir.syms.iter().map(|s| match s {
                Some(b) => self.syms.intern(b),
                None => self.syms.intern(b""),
            }).collect();
            let lv = ir.lv.iter().map(|s| s.as_ref().map(|b| self.syms.intern(b))).collect();
            self.ireps.push(VmIrep {
                nlocals: ir.nlocals as usize,
                nregs: ir.nregs as usize,
                iseq: ir.iseq.clone(),
                catch: ir.catch.clone(),
                pool: ir.pool.clone(),
                syms,
                reps: ir.reps.iter().map(|r| r + offset).collect(),
                lv,
            });
        }
        Ok(rite.root + offset)
    }

    /// Loads and runs a binary to completion at the top level.
    pub fn load_and_run(&mut self, bin: &[u8]) -> VmResult<Value> {
        let irep = self.load(bin)?;
        self.run_irep(irep)
    }

    /// Runs a top-level irep with `self` = main.
    pub fn run_irep(&mut self, irep: IrepId) -> VmResult<Value> {
        let proc_ = self.heap.alloc(self.core.proc_, ObjKind::Proc(ProcData {
            irep, upper: None, env: None, target_class: Some(self.core.object), strict: false, scope: true, orphan: false,
        }));
        let base = self.stack.len();
        let nregs = self.ireps[irep].nregs.max(4);
        self.stack.resize(base + nregs, Value::Nil);
        self.stack[base] = Value::Obj(self.top_self);
        let depth = self.ci.len();
        self.ci.push(CallInfo { base, pc: 0, irep, proc_, n: 0, kw: false, mid: None, target_class: self.core.object, env: None, cci: Cci::Skip });
        let r = self.run_loop(depth);
        self.stack.truncate(base);
        r
    }

    /// Prepares a top-level irep for stepped execution ([`Vm::step`]).
    pub fn start(&mut self, irep: IrepId) {
        let proc_ = self.heap.alloc(self.core.proc_, ObjKind::Proc(ProcData {
            irep, upper: None, env: None, target_class: Some(self.core.object), strict: false, scope: true, orphan: false,
        }));
        let base = self.stack.len();
        let nregs = self.ireps[irep].nregs.max(4);
        self.stack.resize(base + nregs, Value::Nil);
        self.stack[base] = Value::Obj(self.top_self);
        self.ci.push(CallInfo { base, pc: 0, irep, proc_, n: 0, kw: false, mid: None, target_class: self.core.object, env: None, cci: Cci::Skip });
    }

    /// Executes at most `budget` instructions of a program started with
    /// [`Vm::start`]. This is the instruction-boundary suspension point
    /// (mruby's `RETURN_IF_TASK_STOPPED`).
    pub fn step(&mut self, budget: u64) -> VmResult<Step> {
        if self.ci.is_empty() {
            return Ok(Step::Finished(Value::Nil));
        }
        self.step_left = Some(budget);
        let r = self.run_loop(0);
        self.step_left = None;
        match r {
            Ok(v) if self.ci.is_empty() => { self.stack.clear(); Ok(Step::Finished(v)) }
            Ok(_) => Ok(Step::Paused),
            Err(e) => { self.ci.clear(); self.stack.clear(); Err(e) }
        }
    }

    // ------------------------------------------------------------------ calls from native code

    /// Calls a method on `recv` from native code.
    pub fn funcall(&mut self, recv: Value, mid: Sym, args: &[Value], blk: Value) -> VmResult<Value> {
        let cls = self.class_of(recv);
        match self.find_method(cls, mid) {
            Some((Method::Native(f), _)) => {
                // native -> native recursion (e.g. inspect of nested containers) also uses the host stack
                if self.native_depth >= NATIVE_DEPTH_MAX { return Err(self.raise(self.core.system_stack_error, "stack level too deep")); }
                self.native_depth += 1;
                let r = f(self, recv, args, blk);
                self.native_depth -= 1;
                r
            }
            Some((Method::AttrReader(iv), _)) => Ok(recv.obj().map(|o| self.heap.ivar_get(o, iv)).unwrap_or(Value::Nil)),
            Some((Method::AttrWriter(iv), _)) => {
                let v = args.first().copied().unwrap_or(Value::Nil);
                if let Some(o) = recv.obj() { self.heap.ivar_set(o, iv, v); }
                Ok(v)
            }
            Some((Method::Ruby(p), owner)) => self.call_proc(p, recv, args, blk, Some(mid), owner),
            Some((Method::Undef, _)) | None => {
                let name = self.sym_name(mid);
                let desc = self.describe_for_error(recv);
                Err(self.raise(self.core.no_method_error, &format!("undefined method '{name}' for {desc}")))
            }
        }
    }

    /// Calls a block/proc from native code.
    pub fn call_block(&mut self, blk: Value, args: &[Value]) -> VmResult<Value> {
        let p = match blk {
            Value::Obj(o) if matches!(self.heap.get(o).kind, ObjKind::Proc(_)) => o,
            Value::Nil => return Err(self.raise(self.core.local_jump_error, "no block given (yield)")),
            _ => return Err(self.raise_type("wrong type (expected Proc)")),
        };
        let pd = self.heap.proc_data(p);
        let self_ = match pd.env {
            Some(e) => self.env_get(e, 0),
            None => Value::Nil,
        };
        let tc = pd.target_class.unwrap_or(self.core.object);
        let mid = pd.env.and_then(|e| self.heap.env(e).mid);
        self.call_proc(p, self_, args, Value::Nil, mid, tc)
    }

    /// Calls a block with an explicit `self` (`instance_eval`, `class_eval`).
    pub fn call_block_with_self(&mut self, blk: Value, self_: Value, args: &[Value]) -> VmResult<Value> {
        let p = match blk {
            Value::Obj(o) if matches!(self.heap.get(o).kind, ObjKind::Proc(_)) => o,
            _ => return Err(self.raise_type("wrong type (expected Proc)")),
        };
        let tc = match self_ { Value::Obj(o) if self.heap.is_class(o) => o, _ => self.singleton_class(self_)? };
        self.call_proc_with(p, self_, args, Value::Nil, None, Some(tc))
    }

    /// Pushes a frame for `proc_` and runs it to completion (re-entrant
    /// execution; native code waits for the result).
    pub fn call_proc(&mut self, proc_: ObjId, self_: Value, args: &[Value], blk: Value, mid: Option<Sym>, target_class: ObjId) -> VmResult<Value> {
        self.call_proc_with(proc_, self_, args, blk, mid, if self.heap.proc_data(proc_).env.is_some() { None } else { Some(target_class) })
    }

    /// `override_tc = Some(c)` forces the target class (instance_eval); `None` takes it from the env.
    fn call_proc_with(&mut self, proc_: ObjId, self_: Value, args: &[Value], blk: Value, mid: Option<Sym>, override_tc: Option<ObjId>) -> VmResult<Value> {
        // mruby: MRB_CALL_LEVEL_MAX (512) frames; here also a cap on host-stack re-entry.
        if self.ci.len() >= CALL_LEVEL_MAX || self.native_depth >= NATIVE_DEPTH_MAX {
            return Err(self.raise(self.core.system_stack_error, "stack level too deep"));
        }
        self.native_depth += 1;
        let r = self.call_proc_inner(proc_, self_, args, blk, mid, override_tc);
        self.native_depth -= 1;
        r
    }

    fn call_proc_inner(&mut self, proc_: ObjId, self_: Value, args: &[Value], blk: Value, mid: Option<Sym>, override_tc: Option<ObjId>) -> VmResult<Value> {
        let pd = self.heap.proc_data(proc_);
        let irep = pd.irep;
        let env = pd.env;
        let base = self.stack.len();
        let nregs = self.ireps[irep].nregs.max(args.len() + 2).max(4);
        self.stack.resize(base + nregs, Value::Nil);
        self.stack[base] = self_;
        let mut n = args.len();
        if n >= 15 {
            let packed = self.ary_new(args.to_vec());
            self.stack[base + 1] = packed;
            n = 15;
            self.stack[base + 2] = blk;
        } else {
            self.stack[base + 1..base + 1 + args.len()].copy_from_slice(args);
            self.stack[base + 1 + args.len()] = blk;
        }
        let depth = self.ci.len();
        let tc = match (override_tc, env) {
            (Some(tc), _) => tc,
            (None, Some(e)) => self.heap.env(e).target_class.unwrap_or(self.core.object),
            (None, None) => self.heap.proc_data(proc_).target_class.unwrap_or(self.core.object),
        };
        self.ci.push(CallInfo { base, pc: 0, irep, proc_, n: n as u8, kw: false, mid, target_class: tc, env: None, cci: Cci::Skip });
        let r = self.run_loop(depth);
        self.stack.truncate(base);
        r
    }

    // ------------------------------------------------------------------ environments

    fn env_get(&self, env: ObjId, idx: usize) -> Value {
        let e = self.heap.env(env);
        if e.attached { self.stack.get(e.base + idx).copied().unwrap_or(Value::Nil) } else { e.values.get(idx).copied().unwrap_or(Value::Nil) }
    }
    fn env_set(&mut self, env: ObjId, idx: usize, v: Value) {
        let (attached, base) = { let e = self.heap.env(env); (e.attached, e.base) };
        if attached {
            if base + idx < self.stack.len() { self.stack[base + idx] = v; }
        } else {
            let e = self.heap.env_mut(env);
            if idx < e.values.len() { e.values[idx] = v; }
        }
    }
    /// `uvenv`: the environment `up` procs above the current one.
    fn uvenv(&self, up: usize) -> Option<ObjId> {
        let ci = self.ci.last()?;
        let mut p = ci.proc_;
        for _ in 0..up {
            p = self.heap.proc_data(p).upper?;
        }
        self.heap.proc_data(p).env
    }
    /// Ensures the current frame has an environment (mruby `closure_setup`).
    fn frame_env(&mut self) -> ObjId {
        let i = self.ci.len() - 1;
        if let Some(e) = self.ci[i].env {
            return e;
        }
        let ci = &self.ci[i];
        let len = self.ireps[ci.irep].nlocals;
        let e = self.heap.alloc(self.core.object, ObjKind::Env(EnvData {
            base: ci.base, len, attached: true, values: Vec::new(), mid: ci.mid, target_class: Some(ci.target_class),
        }));
        self.ci[i].env = Some(e);
        e
    }
    /// Pops the current frame, detaching its environment (`cipop`).
    fn pop_frame(&mut self) -> CallInfo {
        let ci = self.ci.pop().expect("pop on empty callinfo");
        if let Some(e) = ci.env {
            let (base, len) = { let ed = self.heap.env(e); (ed.base, ed.len) };
            let end = (base + len).min(self.stack.len());
            let vals = self.stack[base..end].to_vec();
            let ed = self.heap.env_mut(e);
            ed.values = vals;
            ed.attached = false;
        }
        // Orphan blocks whose env belonged to this frame? (`MRB_PROC_ORPHAN`) — not tracked yet.
        ci
    }

    // ------------------------------------------------------------------ the loop

    /// Runs until the frame at index `stop_depth` returns, and returns its value.
    fn run_loop(&mut self, stop_depth: usize) -> VmResult<Value> {
        let mut pending: Option<VmResult<Value>> = None;
        loop {
            let r = match pending.take() { Some(r) => r, None => self.exec_frames(stop_depth) };
            match r {
                Ok(v) => return Ok(v),
                Err(VmError::Raise(exc)) => {
                    // Unwind: look for a catch handler in frames >= stop_depth.
                    if self.handle_raise(exc, stop_depth) {
                        continue;
                    }
                    return Err(VmError::Raise(exc));
                }
                Err(VmError::Unimplemented(what)) => {
                    // Surface as a Ruby NotImplementedError so scripts (and the
                    // mruby test suite) can rescue it and continue.
                    let exc = self.exc_new(self.core.not_implemented_error, &format!("not implemented in SabiRuby: {what}"));
                    pending = Some(Err(VmError::Raise(exc)));
                    continue;
                }
                Err(VmError::Break(brk)) => {
                    // A return/break came back through native code: keep unwinding here.
                    if self.ci.len() <= stop_depth { return Err(VmError::Break(brk)); }
                    match self.resume_break(brk, stop_depth) {
                        Ok(Some(v)) => return Ok(v),
                        Ok(None) => continue,
                        Err(e) => { pending = Some(Err(e)); continue; }
                    }
                }
                Err(e) => return Err(e),
            }
        }
    }

    fn catch_find(&self, irep: IrepId, pc: usize, ensure_only: bool) -> Option<rite::CatchHandler> {
        let pc = pc as u32;
        self.ireps[irep].catch.iter().rev().find(|h| pc > h.begin && pc <= h.end && (!ensure_only || h.kind == CatchType::Ensure)).copied()
    }

    fn break_new(&mut self, tag: BreakTag, ci_index: usize, value: Value) -> ObjId {
        // Reuse a pending break object of the same tag (mruby `prepare_tagged_break`).
        if let Some(Value::Obj(o)) = self.exc {
            if let ObjKind::Break { tag: t, .. } = self.heap.get(o).kind { if t == tag { return o; } }
        }
        self.heap.alloc(self.core.object, ObjKind::Break { tag, ci_index, value })
    }

    /// Enters the ensure handler `h` of the top frame with `brk` pending.
    fn enter_ensure(&mut self, h: rite::CatchHandler, brk: ObjId) {
        let top = self.ci.len() - 1;
        self.exc = Some(Value::Obj(brk));
        let (base, nregs) = { let ci = &self.ci[top]; (ci.base, self.ireps[ci.irep].nregs) };
        if self.stack.len() < base + nregs { self.stack.resize(base + nregs, Value::Nil); }
        self.ci[top].pc = h.target as usize;
    }

    /// Continues a pending non-local exit (`L_BREAK` dispatch by tag).
    fn resume_break(&mut self, brk: ObjId, stop_depth: usize) -> VmResult<Option<Value>> {
        let (tag, idx, value) = match self.heap.get(brk).kind { ObjKind::Break { tag, ci_index, value } => (tag, ci_index, value), _ => return Err(VmError::Internal("not a break object".into())) };
        self.exc = Some(Value::Obj(brk));
        match tag {
            BreakTag::Break => self.unwind_return(idx, value, stop_depth),
            BreakTag::Jump => { let target = match value { Value::Int(t) => t as usize, _ => 0 }; self.jmpuw(target); Ok(None) }
        }
    }

    /// `OP_JMPUW`: jump to `target`, running any ensure body in between first.
    fn jmpuw(&mut self, target: usize) {
        let top = self.ci.len() - 1;
        let (irep, pc) = { let ci = &self.ci[top]; (ci.irep, ci.pc) };
        if let Some(h) = self.catch_find(irep, pc, true) {
            // avoid jumping from a handler into the same handler
            if (target as u32) < h.begin || (target as u32) > h.end {
                let brk = self.break_new(BreakTag::Jump, top, Value::Int(target as i64));
                self.enter_ensure(h, brk);
                return;
            }
        }
        self.exc = None;
        self.ci[top].pc = target;
    }

    /// Unwinds to frame `return_idx` (running ensure bodies on the way, mruby
    /// `L_RETURN`/`UNWIND_ENSURE`) and returns `v` from it. `Ok(Some(v))` means
    /// the interpreter loop must hand `v` to its native caller.
    fn unwind_return(&mut self, return_idx: usize, v: Value, stop_depth: usize) -> VmResult<Option<Value>> {
        loop {
            let top = self.ci.len() - 1;
            let (irep, pc) = { let ci = &self.ci[top]; (ci.irep, ci.pc) };
            if let Some(h) = self.catch_find(irep, pc, true) {
                let brk = self.break_new(BreakTag::Break, return_idx, v);
                self.enter_ensure(h, brk);
                return Ok(None);
            }
            if top == return_idx { break; }
            let popped = self.pop_frame();
            if popped.cci == Cci::Skip || top <= stop_depth {
                // crossing a native frame: let the native caller propagate it
                let brk = self.break_new(BreakTag::Break, return_idx, v);
                self.exc = None;
                return Err(VmError::Break(brk));
            }
        }
        self.exc = None;
        let popped = self.pop_frame();
        if popped.cci == Cci::Skip || return_idx <= stop_depth {
            return Ok(Some(v));
        }
        // the callee's R0 is the caller's R[a]
        self.stack[popped.base] = v;
        Ok(None)
    }

    /// Finds a rescue/ensure handler for `exc`; on success sets pc and `exc`
    /// and returns true. Otherwise pops frames down to `stop_depth` and returns false.
    fn handle_raise(&mut self, exc: Value, stop_depth: usize) -> bool {
        loop {
            let i = self.ci.len() - 1;
            let (irep, pc) = { let ci = &self.ci[i]; (ci.irep, ci.pc) };
            if let Some(h) = self.catch_find(irep, pc, false) {
                self.ci[i].pc = h.target as usize;
                let nregs = self.ireps[irep].nregs;
                let base = self.ci[i].base;
                if self.stack.len() < base + nregs { self.stack.resize(base + nregs, Value::Nil); }
                self.exc = Some(exc);
                return true;
            }
            if i <= stop_depth {
                self.pop_frame();
                return false;
            }
            self.pop_frame();
        }
    }

    #[inline]
    fn read_b(&self, ci: &CallInfo, pc: &mut usize) -> u32 {
        let v = self.ireps[ci.irep].iseq[*pc] as u32;
        *pc += 1;
        v
    }
    #[inline]
    fn read_s(&self, ci: &CallInfo, pc: &mut usize) -> u32 {
        let s = &self.ireps[ci.irep].iseq;
        let v = ((s[*pc] as u32) << 8) | s[*pc + 1] as u32;
        *pc += 2;
        v
    }
    #[inline]
    fn read_w(&self, ci: &CallInfo, pc: &mut usize) -> u32 {
        let s = &self.ireps[ci.irep].iseq;
        let v = ((s[*pc] as u32) << 16) | ((s[*pc + 1] as u32) << 8) | s[*pc + 2] as u32;
        *pc += 3;
        v
    }

    fn exec_frames(&mut self, stop_depth: usize) -> VmResult<Value> {
        let mut ext: u8 = 0;
        loop {
            if let Some(left) = self.step_left {
                // Suspend only in the outermost loop: a nested loop (native code
                // waiting for a block) must run to completion, so the pause lands
                // at the next instruction boundary of the top-level program.
                if left == 0 && stop_depth == 0 {
                    return Ok(Value::Nil);
                }
                self.step_left = Some(left.saturating_sub(1));
            }
            self.instructions += 1;
            let ci = self.ci.last().unwrap().clone();
            let mut pc = ci.pc;
            let byte = self.ireps[ci.irep].iseq[pc];
            self.op_counts[byte as usize] += 1;
            let op = Op::from_u8(byte).ok_or_else(|| VmError::Internal(format!("bad opcode {byte}")))?;
            pc += 1;
            let (mut a, mut b, mut c) = (0u32, 0u32, 0u32);
            let a_wide = ext == 1 || ext == 3;
            let b_wide = ext == 2 || ext == 3;
            match op.operands() {
                Operands::Z => {}
                Operands::B => a = if a_wide { self.read_s(&ci, &mut pc) } else { self.read_b(&ci, &mut pc) },
                Operands::BB => { a = if a_wide { self.read_s(&ci, &mut pc) } else { self.read_b(&ci, &mut pc) }; b = if b_wide { self.read_s(&ci, &mut pc) } else { self.read_b(&ci, &mut pc) }; }
                Operands::BBB => { a = if a_wide { self.read_s(&ci, &mut pc) } else { self.read_b(&ci, &mut pc) }; b = if b_wide { self.read_s(&ci, &mut pc) } else { self.read_b(&ci, &mut pc) }; c = self.read_b(&ci, &mut pc); }
                Operands::BS => { a = if a_wide { self.read_s(&ci, &mut pc) } else { self.read_b(&ci, &mut pc) }; b = self.read_s(&ci, &mut pc); }
                Operands::BSS => { a = if a_wide { self.read_s(&ci, &mut pc) } else { self.read_b(&ci, &mut pc) }; b = self.read_s(&ci, &mut pc); c = self.read_s(&ci, &mut pc); }
                Operands::S => a = self.read_s(&ci, &mut pc),
                Operands::W => a = self.read_w(&ci, &mut pc),
            }
            ext = 0;
            // pc now points at the next instruction (like mruby's DECODE_OPERANDS).
            let top = self.ci.len() - 1;
            self.ci[top].pc = pc;
            let base = ci.base;
            let (a, b, c) = (a as usize, b as usize, c as usize);
            macro_rules! reg { ($i:expr) => { self.stack[base + $i] } }
            match op {
                Op::Nop => {}
                Op::Move => { reg!(a) = reg!(b); }
                Op::Loadl => {
                    let v = match &self.ireps[ci.irep].pool[b] {
                        Pool::Int(i) => Value::Int(*i),
                        Pool::Float(f) => Value::Float(*f),
                        Pool::Str(s) => { let s = s.clone(); self.str_new(&s) }
                        Pool::BigInt(_) => return Err(VmError::Unimplemented("bigint literal".into())),
                    };
                    reg!(a) = v;
                }
                Op::Loadi8 => { reg!(a) = Value::Int(b as i64); }
                Op::Loadineg => { reg!(a) = Value::Int(-(b as i64)); }
                Op::LoadiM1 => { reg!(a) = Value::Int(-1); }
                Op::Loadi0 => { reg!(a) = Value::Int(0); }
                Op::Loadi1 => { reg!(a) = Value::Int(1); }
                Op::Loadi2 => { reg!(a) = Value::Int(2); }
                Op::Loadi3 => { reg!(a) = Value::Int(3); }
                Op::Loadi4 => { reg!(a) = Value::Int(4); }
                Op::Loadi5 => { reg!(a) = Value::Int(5); }
                Op::Loadi6 => { reg!(a) = Value::Int(6); }
                Op::Loadi7 => { reg!(a) = Value::Int(7); }
                Op::Loadi16 => { reg!(a) = Value::Int(b as u16 as i16 as i64); }
                Op::Loadi32 => { reg!(a) = Value::Int((((b as u32) << 16) | c as u32) as i32 as i64); }
                Op::Loadsym => { reg!(a) = Value::Sym(self.ireps[ci.irep].syms[b]); }
                Op::Loadnil => { reg!(a) = Value::Nil; }
                Op::Loadself => { reg!(a) = reg!(0); }
                Op::Loadtrue => { reg!(a) = Value::True; }
                Op::Loadfalse => { reg!(a) = Value::False; }
                Op::Getsv | Op::Setsv => { return Err(VmError::Unimplemented("special variables ($~, $_)".into())); }
                Op::Getgv => { let s = self.ireps[ci.irep].syms[b]; reg!(a) = self.globals.get(&s).copied().unwrap_or(Value::Nil); }
                Op::Setgv => { let s = self.ireps[ci.irep].syms[b]; let v = reg!(a); self.globals.insert(s, v); }
                Op::Getiv => {
                    let s = self.ireps[ci.irep].syms[b];
                    let v = match reg!(0) { Value::Obj(o) => self.heap.ivar_get(o, s), _ => Value::Nil };
                    reg!(a) = v;
                }
                Op::Setiv => {
                    let s = self.ireps[ci.irep].syms[b];
                    let v = reg!(a);
                    match reg!(0) {
                        Value::Obj(o) => self.heap.ivar_set(o, s, v),
                        _ => return Err(self.raise_type("can't set instance variable on an immediate")),
                    }
                }
                Op::Getcv => {
                    let s = self.ireps[ci.irep].syms[b];
                    let cls = self.cvar_class(ci.proc_);
                    let v = match self.cvar_get(cls, s) {
                        Some(v) => v,
                        None => { let n = self.sym_name(s); let cn = self.class_name(cls); return Err(self.raise(self.core.name_error, &format!("uninitialized class variable {n} in {cn}"))); }
                    };
                    reg!(a) = v;
                }
                Op::Setcv => {
                    let s = self.ireps[ci.irep].syms[b];
                    let v = reg!(a);
                    let cls = self.cvar_class(ci.proc_);
                    self.cvar_set(cls, s, v);
                }
                Op::Getconst => {
                    let s = self.ireps[ci.irep].syms[b];
                    let v = self.const_lookup(&ci, s)?;
                    reg!(a) = v;
                }
                Op::Setconst => {
                    let s = self.ireps[ci.irep].syms[b];
                    let v = reg!(a);
                    let tc = ci.target_class;
                    if self.heap.is_class(tc) {
                        self.heap.class_mut(tc).consts.insert(s, v);
                        // name anonymous classes on first assignment
                        if let Value::Obj(o) = v {
                            if self.heap.is_class(o) && self.heap.class(o).name.is_none() {
                                self.heap.class_mut(o).name = Some(s);
                            }
                        }
                    }
                }
                Op::Getmcnst => {
                    let s = self.ireps[ci.irep].syms[b];
                    let base_v = reg!(a);
                    let cls = match base_v { Value::Obj(o) if self.heap.is_class(o) => o, _ => return Err(self.raise_type("not a class/module")) };
                    let v = match self.const_get(cls, s) {
                        Some(v) => v,
                        None => { let n = self.sym_name(s); let cn = self.class_name(cls); return Err(self.raise(self.core.name_error, &format!("uninitialized constant {cn}::{n}"))); }
                    };
                    reg!(a) = v;
                }
                Op::Setmcnst => {
                    let s = self.ireps[ci.irep].syms[b];
                    let v = reg!(a);
                    match reg!(a + 1) { Value::Obj(o) if self.heap.is_class(o) => { self.heap.class_mut(o).consts.insert(s, v); } _ => return Err(self.raise_type("not a class/module")) }
                }
                Op::Getupvar => {
                    let v = match self.uvenv(c) { Some(e) if b < self.heap.env(e).len => self.env_get(e, b), _ => Value::Nil };
                    reg!(a) = v;
                }
                Op::Setupvar => {
                    if let Some(e) = self.uvenv(c) {
                        if b < self.heap.env(e).len { let v = reg!(a); self.env_set(e, b, v); }
                    }
                }
                Op::Getidx => {
                    let recv = reg!(a); let idx = reg!(a + 1);
                    let v = self.funcall(recv, self.s.aref, &[idx], Value::Nil)?;
                    reg!(a) = v;
                }
                Op::Getidx0 => {
                    let recv = reg!(b);
                    let v = self.funcall(recv, self.s.aref, &[Value::Int(0)], Value::Nil)?;
                    reg!(a) = v;
                }
                Op::Setidx => {
                    let recv = reg!(a); let idx = reg!(a + 1); let val = reg!(a + 2);
                    self.funcall(recv, self.s.aset, &[idx, val], Value::Nil)?;
                }
                Op::Jmp => { self.ci[top].pc = jump(pc, a); }
                Op::Jmpif => { if reg!(a).truthy() { self.ci[top].pc = jump(pc, b); } }
                Op::Jmpnot => { if !reg!(a).truthy() { self.ci[top].pc = jump(pc, b); } }
                Op::Jmpnil => { if reg!(a).is_nil() { self.ci[top].pc = jump(pc, b); } }
                Op::Jmpuw => { self.jmpuw(jump(pc, a)); }
                Op::Except => { reg!(a) = self.exc.take().unwrap_or(Value::Nil); }
                Op::Rescue => {
                    let exc = reg!(a); let cls_v = reg!(b);
                    let cls = match cls_v { Value::Obj(o) if self.heap.is_class(o) => o, _ => return Err(self.raise_type("class or module required for rescue clause")) };
                    reg!(b) = Value::bool(self.obj_is_kind_of(exc, cls));
                }
                Op::Raiseif => {
                    let exc = reg!(a);
                    match exc {
                        Value::Nil => { self.exc = None; }
                        Value::Obj(o) if matches!(self.heap.get(o).kind, ObjKind::Break { .. }) => {
                            if let Some(r) = self.resume_break(o, stop_depth)? { return Ok(r); }
                        }
                        _ => return Err(VmError::Raise(exc)),
                    }
                }
                Op::Matcherr => { return Err(self.raise(self.core.no_matching_pattern_error, "pattern not matched")); }
                Op::Ssend | Op::Ssend0 | Op::Ssendb | Op::Send | Op::Send0 | Op::Sendb => {
                    let mid = self.ireps[ci.irep].syms[b];
                    let argc = if matches!(op, Op::Send0 | Op::Ssend0) { 0 } else { c };
                    let has_blk = matches!(op, Op::Sendb | Op::Ssendb);
                    if matches!(op, Op::Ssend | Op::Ssend0 | Op::Ssendb) { reg!(a) = reg!(0); }
                    self.op_send(base, a, mid, argc, has_blk, false)?;
                }
                Op::Super => {
                    let argc = b;
                    reg!(a) = reg!(0);
                    let mid = ci.mid.ok_or_else(|| self.raise(self.core.no_method_error, "super called outside of method"))?;
                    self.op_send(base, a, mid, argc, true, true)?;
                }
                Op::Call => {
                    // `Proc#call`: replace this frame (pushed by SEND) with the proc's body.
                    let p = match reg!(0) { Value::Obj(o) if matches!(self.heap.get(o).kind, ObjKind::Proc(_)) => o, _ => return Err(self.raise_type("wrong type (expected Proc)")) };
                    let n = ci.n as usize;
                    let nargs = (if n == 15 { 1 } else { n }) + 1 + 1;
                    self.vm_call_proc(p, nargs);
                }
                Op::Blkcall => {
                    // Direct block call: R[a] = R[a].call(R[a+1..a+b])
                    let p = match reg!(a) { Value::Obj(o) if matches!(self.heap.get(o).kind, ObjKind::Proc(_)) => o, _ => return Err(self.raise_type("wrong type (expected Proc)")) };
                    let nbase = base + a;
                    if self.stack.len() < nbase + b + 2 { self.stack.resize(nbase + b + 2, Value::Nil); }
                    self.stack[nbase + b + 1] = Value::Nil;
                    self.ci.push(CallInfo { base: nbase, pc: 0, irep: 0, proc_: p, n: b as u8, kw: false, mid: None, target_class: ci.target_class, env: None, cci: Cci::None });
                    self.vm_call_proc(p, b + 1);
                }
                Op::Argary => { self.op_argary(base, a, b)?; }
                Op::Enter => { self.op_enter(a as u32)?; }
                Op::KeyP | Op::Keyend | Op::Karg => { return Err(VmError::Unimplemented("keyword arguments".into())); }
                Op::Return => { let v = reg!(a); if let Some(r) = self.op_return(v, stop_depth)? { return Ok(r); } }
                Op::ReturnBlk => {
                    let v = reg!(a);
                    if let Some(r) = self.op_return_blk(v, stop_depth)? { return Ok(r); }
                }
                Op::Retself => { let v = reg!(0); if let Some(r) = self.op_return(v, stop_depth)? { return Ok(r); } }
                Op::Retnil => { if let Some(r) = self.op_return(Value::Nil, stop_depth)? { return Ok(r); } }
                Op::Rettrue => { if let Some(r) = self.op_return(Value::True, stop_depth)? { return Ok(r); } }
                Op::Retfalse => { if let Some(r) = self.op_return(Value::False, stop_depth)? { return Ok(r); } }
                Op::Break => {
                    let v = reg!(a);
                    if let Some(r) = self.op_break(v, stop_depth)? { return Ok(r); }
                }
                Op::Blkpush => { let v = self.op_blkpush(base, b)?; reg!(a) = v; }
                Op::Add => { self.op_arith(base, a, self.s.plus)?; }
                Op::Sub => { self.op_arith(base, a, self.s.minus)?; }
                Op::Mul => { self.op_arith(base, a, self.s.mul)?; }
                Op::Div => { self.op_arith(base, a, self.s.div)?; }
                Op::Addi => { reg!(a + 1) = Value::Int(b as i64); self.op_arith(base, a, self.s.plus)?; }
                Op::Subi => { reg!(a + 1) = Value::Int(b as i64); self.op_arith(base, a, self.s.minus)?; }
                Op::Addilv => { reg!(b) = reg!(a); reg!(b + 1) = Value::Int(c as i64); self.op_arith(base, b, self.s.plus)?; let v = reg!(b); reg!(a) = v; }
                Op::Subilv => { reg!(b) = reg!(a); reg!(b + 1) = Value::Int(c as i64); self.op_arith(base, b, self.s.minus)?; let v = reg!(b); reg!(a) = v; }
                Op::Eq => { self.op_compare(base, a, self.s.eq)?; }
                Op::Lt => { self.op_compare(base, a, self.s.lt)?; }
                Op::Le => { self.op_compare(base, a, self.s.le)?; }
                Op::Gt => { self.op_compare(base, a, self.s.gt)?; }
                Op::Ge => { self.op_compare(base, a, self.s.ge)?; }
                Op::Array => { let v: Vec<Value> = self.stack[base + a..base + a + b].to_vec(); reg!(a) = self.ary_new(v); }
                Op::Array2 => { let v: Vec<Value> = self.stack[base + b..base + b + c].to_vec(); reg!(a) = self.ary_new(v); }
                Op::Arycat => {
                    // R[a] == nil means "start the argument accumulator": a fresh
                    // array (independent of R[a+1]) that later ARYPUSH/ARYCAT extend.
                    let (dst, src) = (reg!(a), reg!(a + 1));
                    let items = match src { Value::Nil => vec![], _ => self.to_array(src)? };
                    match dst {
                        Value::Nil => { reg!(a) = self.ary_new(items); }
                        Value::Obj(o) if matches!(self.heap.get(o).kind, ObjKind::Array(_)) => { if let ObjKind::Array(v) = &mut self.heap.get_mut(o).kind { v.extend(items); } }
                        _ => return Err(self.raise_type("not an array")),
                    }
                }
                Op::Arypush => {
                    let dst = reg!(a);
                    let items: Vec<Value> = self.stack[base + a + 1..base + a + 1 + b].to_vec();
                    match dst { Value::Obj(o) => { if let ObjKind::Array(v) = &mut self.heap.get_mut(o).kind { v.extend(items); } } _ => return Err(self.raise_type("not an array")) }
                }
                Op::Arysplat => { let v = reg!(a); let items = self.to_array(v)?; reg!(a) = self.ary_new(items); }
                Op::Aref => {
                    let v = match reg!(b) { Value::Obj(o) => match self.heap.array(o) { Some(arr) => arr.get(c).copied().unwrap_or(Value::Nil), None => reg!(b) }, other => if c == 0 { other } else { Value::Nil } };
                    reg!(a) = v;
                }
                Op::Aset => {
                    let v = reg!(a);
                    match reg!(b) { Value::Obj(o) => { if let ObjKind::Array(arr) = &mut self.heap.get_mut(o).kind { if arr.len() <= c { arr.resize(c + 1, Value::Nil); } arr[c] = v; } } _ => return Err(self.raise_type("not an array")) }
                }
                Op::Apost => {
                    let src = reg!(a);
                    let items = match self.ary(src) { Some(v) => v.clone(), None => vec![src] };
                    let pre = b; let post = c;
                    let len = items.len();
                    if len > pre + post {
                        let rest = items[pre..len - post].to_vec();
                        reg!(a) = self.ary_new(rest);
                        for i in 0..post { reg!(a + 1 + i) = items[len - post + i]; }
                    } else {
                        reg!(a) = self.ary_new(vec![]);
                        for i in 0..post { reg!(a + 1 + i) = items.get(pre + i).copied().unwrap_or(Value::Nil); }
                    }
                }
                Op::Intern => { let v = reg!(a); let bytes = self.expect_str(v, "value")?; reg!(a) = Value::Sym(self.syms.intern(&bytes)); }
                Op::Symbol => {
                    let bytes = match &self.ireps[ci.irep].pool[b] { Pool::Str(s) => s.clone(), _ => return Err(VmError::Internal("SYMBOL pool".into())) };
                    reg!(a) = Value::Sym(self.syms.intern(&bytes));
                }
                Op::String => {
                    let bytes = match &self.ireps[ci.irep].pool[b] { Pool::Str(s) => s.clone(), _ => return Err(VmError::Internal("STRING pool".into())) };
                    reg!(a) = self.str_new(&bytes);
                }
                Op::Strcat => {
                    let (dst, src) = (reg!(a), reg!(a + 1));
                    let bytes = self.as_string(src)?;
                    match dst { Value::Obj(o) => { if let ObjKind::String(s) = &mut self.heap.get_mut(o).kind { s.extend_from_slice(&bytes); } } _ => return Err(self.raise_type("not a string")) }
                }
                Op::Hash => {
                    let h = self.hash_new();
                    let pairs: Vec<(Value, Value)> = (0..b).map(|i| (self.stack[base + a + i * 2], self.stack[base + a + i * 2 + 1])).collect();
                    for (k, v) in pairs { self.hash_set(h, k, v)?; }
                    reg!(a) = h;
                }
                Op::Hashadd => {
                    let h = reg!(a);
                    let pairs: Vec<(Value, Value)> = (0..b).map(|i| (self.stack[base + a + 1 + i * 2], self.stack[base + a + 2 + i * 2])).collect();
                    for (k, v) in pairs { self.hash_set(h, k, v)?; }
                }
                Op::Hashcat => {
                    let (h, other) = (reg!(a), reg!(a + 1));
                    let entries = match other.obj().map(|o| &self.heap.get(o).kind) { Some(ObjKind::Hash(hd)) => hd.entries.clone(), _ => return Err(self.raise_type("not a hash")) };
                    for (k, v) in entries { self.hash_set(h, k, v)?; }
                }
                Op::Lambda | Op::Block | Op::Method => {
                    let nirep = self.ireps[ci.irep].reps[b];
                    let capture = !matches!(op, Op::Method);
                    let strict = matches!(op, Op::Lambda | Op::Method);
                    let env = if capture { Some(self.frame_env()) } else { None };
                    let p = self.heap.alloc(self.core.proc_, ObjKind::Proc(ProcData {
                        irep: nirep, upper: Some(ci.proc_), env, target_class: Some(ci.target_class), strict, scope: matches!(op, Op::Method), orphan: false,
                    }));
                    reg!(a) = Value::Obj(p);
                }
                Op::RangeInc => { let (x, y) = (reg!(a), reg!(a + 1)); reg!(a) = self.range_new(x, y, false); }
                Op::RangeExc => { let (x, y) = (reg!(a), reg!(a + 1)); reg!(a) = self.range_new(x, y, true); }
                Op::Oclass => { reg!(a) = Value::Obj(self.core.object); }
                Op::Class | Op::Module => {
                    let s = self.ireps[ci.irep].syms[b];
                    let base_v = reg!(a);
                    let outer = match base_v { Value::Nil => self.heap.proc_data(ci.proc_).target_class.unwrap_or(self.core.object), Value::Obj(o) if self.heap.is_class(o) => o, _ => return Err(self.raise_type("not a class/module")) };
                    let existing = self.heap.class(outer).consts.get(&s).copied();
                    let cls = match existing {
                        Some(Value::Obj(o)) if self.heap.is_class(o) => o,
                        _ => {
                            let is_module = matches!(op, Op::Module);
                            let sup = if is_module { None } else {
                                match reg!(a + 1) { Value::Nil => Some(self.core.object), Value::Obj(o) if self.heap.is_class(o) => Some(o), _ => return Err(self.raise_type("superclass must be a Class")) }
                            };
                            let meta = if is_module { self.core.module } else { self.core.class };
                            let ncls = self.heap.alloc(meta, ObjKind::Class(ClassData { name: Some(s), superclass: sup, is_module, ..Default::default() }));
                            self.heap.class_mut(outer).consts.insert(s, Value::Obj(ncls));
                            if !is_module { self.singleton_class(Value::Obj(ncls))?; }
                            if !is_module { if let Some(sup) = sup { let inh = self.intern("inherited"); if self.respond_to(Value::Obj(sup), inh) { /* hook not implemented */ } } }
                            ncls
                        }
                    };
                    reg!(a) = Value::Obj(cls);
                }
                Op::Exec => {
                    let nirep = self.ireps[ci.irep].reps[b];
                    let cls = match reg!(a) { Value::Obj(o) if self.heap.is_class(o) => o, _ => return Err(self.raise_type("not a class/module")) };
                    let p = self.heap.alloc(self.core.proc_, ObjKind::Proc(ProcData { irep: nirep, upper: Some(ci.proc_), env: None, target_class: Some(cls), strict: false, scope: true, orphan: false }));
                    let nbase = base + a;
                    let nregs = self.ireps[nirep].nregs.max(4);
                    if self.stack.len() < nbase + nregs { self.stack.resize(nbase + nregs, Value::Nil); }
                    for i in 1..nregs { self.stack[nbase + i] = Value::Nil; }
                    self.ci.push(CallInfo { base: nbase, pc: 0, irep: nirep, proc_: p, n: 0, kw: false, mid: None, target_class: cls, env: None, cci: Cci::None });
                }
                Op::Def => {
                    let s = self.ireps[ci.irep].syms[b];
                    let target = match reg!(a) { Value::Obj(o) if self.heap.is_class(o) => o, _ => return Err(self.raise_type("not a class/module")) };
                    let p = match reg!(a + 1) { Value::Obj(o) if matches!(self.heap.get(o).kind, ObjKind::Proc(_)) => o, _ => return Err(self.raise_type("not a proc")) };
                    if let ObjKind::Proc(pd) = &mut self.heap.get_mut(p).kind { pd.target_class = Some(target); }
                    self.heap.class_mut(target).methods.insert(s, Method::Ruby(p));
                    reg!(a) = Value::Sym(s);
                }
                Op::Tdef | Op::Sdef => {
                    let s = self.ireps[ci.irep].syms[b];
                    let nirep = self.ireps[ci.irep].reps[c];
                    let target = if matches!(op, Op::Tdef) { ci.target_class } else { let v = reg!(a); self.singleton_class(v)? };
                    let p = self.heap.alloc(self.core.proc_, ObjKind::Proc(ProcData { irep: nirep, upper: Some(ci.proc_), env: None, target_class: Some(target), strict: true, scope: true, orphan: false }));
                    self.heap.class_mut(target).methods.insert(s, Method::Ruby(p));
                    reg!(a) = Value::Sym(s);
                }
                Op::Alias => {
                    let (new, old) = (self.ireps[ci.irep].syms[a], self.ireps[ci.irep].syms[b]);
                    let tc = ci.target_class;
                    match self.find_method(tc, old) { Some((m, _)) => { self.heap.class_mut(tc).methods.insert(new, m); } None => { let n = self.sym_name(old); return Err(self.raise(self.core.name_error, &format!("undefined method '{n}'"))); } }
                }
                Op::Undef => { let s = self.ireps[ci.irep].syms[a]; self.heap.class_mut(ci.target_class).methods.insert(s, Method::Undef); }
                Op::Sclass => { let v = reg!(a); reg!(a) = Value::Obj(self.singleton_class(v)?); }
                Op::Tclass => { reg!(a) = Value::Obj(ci.target_class); }
                Op::Debug => {}
                Op::Err => {
                    let msg = match &self.ireps[ci.irep].pool[a] { Pool::Str(s) => String::from_utf8_lossy(s).into_owned(), _ => "error".into() };
                    return Err(self.raise(self.core.local_jump_error, &msg));
                }
                Op::Ext1 => { ext = 1; }
                Op::Ext2 => { ext = 2; }
                Op::Ext3 => { ext = 3; }
                Op::Stop => {
                    // mruby returns regs[irep->nlocals] (the last expression's register).
                    let nlocals = self.ireps[ci.irep].nlocals;
                    let v = self.stack.get(base + nlocals).copied().unwrap_or(Value::Nil);
                    let _ = self.pop_frame();
                    return Ok(v);
                }
            }
        }
    }

    // ------------------------------------------------------------------ helpers used by the loop

    fn const_lookup(&mut self, ci: &CallInfo, s: Sym) -> VmResult<Value> {
        // 1. target class and its ancestors, 2. lexical scopes (upper procs), 3. Object
        let mut c = Some(ci.target_class);
        if let Some(v) = c.and_then(|c| self.const_get(c, s)) { return Ok(v); }
        let mut p = Some(ci.proc_);
        while let Some(pid) = p {
            let pd = self.heap.proc_data(pid);
            if let Some(tc) = pd.target_class { if let Some(v) = self.const_get(tc, s) { return Ok(v); } }
            p = pd.upper;
        }
        c = Some(self.core.object);
        if let Some(v) = c.and_then(|c| self.const_get(c, s)) { return Ok(v); }
        let n = self.sym_name(s);
        Err(self.raise(self.core.name_error, &format!("uninitialized constant {n}")))
    }

    /// `mrb_vm_cv_get`: the lexically enclosing non-singleton class (walks `upper`).
    fn cvar_class(&self, proc_: ObjId) -> ObjId {
        let mut p = Some(proc_);
        while let Some(pid) = p {
            let pd = self.heap.proc_data(pid);
            if let Some(tc) = pd.target_class { if !self.heap.class(tc).is_singleton { return tc; } }
            p = pd.upper;
        }
        self.core.object
    }
    fn cvar_get(&self, class: ObjId, s: Sym) -> Option<Value> {
        let mut c = Some(class);
        while let Some(x) = c {
            let cd = self.heap.class(x);
            if let Some(v) = cd.cvars.get(&s) { return Some(*v); }
            c = cd.superclass;
        }
        None
    }
    fn cvar_set(&mut self, class: ObjId, s: Sym, v: Value) {
        let mut c = Some(class);
        while let Some(x) = c {
            if self.heap.class(x).cvars.contains_key(&s) { self.heap.class_mut(x).cvars.insert(s, v); return; }
            c = self.heap.class(x).superclass;
        }
        self.heap.class_mut(class).cvars.insert(s, v);
    }

    pub fn to_array(&mut self, v: Value) -> VmResult<Vec<Value>> {
        if let Some(a) = self.ary(v) { return Ok(a.clone()); }
        let to_a = self.intern("to_a");
        if self.respond_to(v, to_a) {
            let r = self.funcall(v, to_a, &[], Value::Nil)?;
            if let Some(a) = self.ary(r) { return Ok(a.clone()); }
        }
        Ok(vec![v])
    }

    /// `mrb_obj_as_string`: String as is, otherwise `to_s`.
    pub fn as_string(&mut self, v: Value) -> VmResult<Vec<u8>> {
        if let Some(b) = self.str_bytes(v) { return Ok(b.to_vec()); }
        let r = self.funcall(v, self.s.to_s, &[], Value::Nil)?;
        match self.str_bytes(r) { Some(b) => Ok(b.to_vec()), None => Ok(b"".to_vec()) }
    }

    /// `eql?`-style equality used for Hash keys.
    pub fn eql(&self, a: Value, b: Value) -> bool {
        match (a, b) {
            (Value::Obj(x), Value::Obj(y)) => {
                if x == y { return true; }
                match (&self.heap.get(x).kind, &self.heap.get(y).kind) {
                    (ObjKind::String(p), ObjKind::String(q)) => p == q,
                    _ => false,
                }
            }
            _ => a == b,
        }
    }
    pub fn hash_get(&self, h: Value, k: Value) -> Option<Value> {
        let o = h.obj()?;
        match &self.heap.get(o).kind {
            ObjKind::Hash(hd) => hd.entries.iter().find(|(ek, _)| self.eql(*ek, k)).map(|(_, v)| *v),
            _ => None,
        }
    }
    pub fn hash_set(&mut self, h: Value, k: Value, v: Value) -> VmResult<()> {
        let o = match h.obj() { Some(o) => o, None => return Err(self.raise_type("not a hash")) };
        // String keys are copied (and frozen in mruby); we copy.
        let k = if let Some(b) = self.str_bytes(k) { let b = b.to_vec(); self.str_new(&b) } else { k };
        let pos = match &self.heap.get(o).kind { ObjKind::Hash(hd) => hd.entries.iter().position(|(ek, _)| self.eql(*ek, k)), _ => return Err(self.raise_type("not a hash")) };
        if let ObjKind::Hash(hd) = &mut self.heap.get_mut(o).kind {
            match pos { Some(i) => hd.entries[i].1 = v, None => hd.entries.push((k, v)) }
        }
        Ok(())
    }

    fn op_arith(&mut self, base: usize, a: usize, mid: Sym) -> VmResult<()> {
        let (x, y) = (self.stack[base + a], self.stack[base + a + 1]);
        let r = match (x, y) {
            (Value::Int(p), Value::Int(q)) => {
                let s = self.s;
                if mid == s.plus { p.checked_add(q).map(Value::Int) }
                else if mid == s.minus { p.checked_sub(q).map(Value::Int) }
                else if mid == s.mul { p.checked_mul(q).map(Value::Int) }
                else { if q == 0 { return Err(self.raise(self.core.zero_division_error, "divided by 0")); } Some(Value::Int(crate::builtins::numeric::div_floor(p, q))) }
            }
            (Value::Int(p), Value::Float(q)) => Some(float_op(mid, self.s, p as f64, q)),
            (Value::Float(p), Value::Int(q)) => Some(float_op(mid, self.s, p, q as f64)),
            (Value::Float(p), Value::Float(q)) => Some(float_op(mid, self.s, p, q)),
            _ => None,
        };
        match r {
            Some(v) => { self.stack[base + a] = v; Ok(()) }
            None => {
                if let (Value::Int(_), Value::Int(_)) = (x, y) { return Err(self.raise(self.core.range_error, "integer overflow")); }
                self.op_send(base, a, mid, 1, false, false)
            }
        }
    }

    fn op_compare(&mut self, base: usize, a: usize, mid: Sym) -> VmResult<()> {
        let (x, y) = (self.stack[base + a], self.stack[base + a + 1]);
        let s = self.s;
        let num = |p: f64, q: f64| -> Value {
            Value::bool(if mid == s.eq { p == q } else if mid == s.lt { p < q } else if mid == s.le { p <= q } else if mid == s.gt { p > q } else { p >= q })
        };
        let r = match (x, y) {
            (Value::Int(p), Value::Int(q)) => Some(Value::bool(if mid == s.eq { p == q } else if mid == s.lt { p < q } else if mid == s.le { p <= q } else if mid == s.gt { p > q } else { p >= q })),
            (Value::Int(p), Value::Float(q)) => Some(num(p as f64, q)),
            (Value::Float(p), Value::Int(q)) => Some(num(p, q as f64)),
            (Value::Float(p), Value::Float(q)) => Some(num(p, q)),
            _ if mid == s.eq => {
                // fast path: identical immediates / same object
                if x == y && !matches!(x, Value::Obj(_)) { Some(Value::True) } else { None }
            }
            _ => None,
        };
        match r {
            Some(v) => { self.stack[base + a] = v; Ok(()) }
            None => self.op_send(base, a, mid, 1, false, false),
        }
    }

    /// Shared body of SEND/SSEND/SUPER: receiver at `R[a]`, args at `R[a+1..]`,
    /// block at `R[a+argc+1]` when `has_blk`.
    fn op_send(&mut self, base: usize, a: usize, mid: Sym, c: usize, has_blk: bool, is_super: bool) -> VmResult<()> {
        let recv = self.stack[base + a];
        let n = c & 0xf;
        let nk = (c >> 4) & 0xf;
        let npos = if n == 15 { 1 } else { n };
        let bidx = base + a + npos + (if nk == 15 { 1 } else { nk * 2 }) + 1;
        if self.stack.len() <= bidx { self.stack.resize(bidx + 1, Value::Nil); }
        let blk = if has_blk { let b = self.stack[bidx]; self.ensure_block(b)? } else { Value::Nil };
        // Keyword arguments: pack `nk` pairs into a Hash (mruby `hash_new_from_regs`).
        // Keyword *parameters* are not implemented yet, so the Hash is passed as a
        // trailing positional argument, which is also what mruby does for callees
        // without keyword parameters.
        let mut argc = n;
        if nk > 0 {
            let kidx = base + a + npos + 1;
            let kdict = if nk == 15 {
                let h = self.stack[kidx];
                if !matches!(h.obj().map(|o| &self.heap.get(o).kind), Some(ObjKind::Hash(_))) { return Err(self.raise_type("keyword argument hash expected")); }
                h
            } else {
                let h = self.hash_new();
                for i in 0..nk { let (k, v) = (self.stack[kidx + i * 2], self.stack[kidx + i * 2 + 1]); self.hash_set(h, k, v)?; }
                h
            };
            if n == 15 {
                let mut all = self.ary(self.stack[base + a + 1]).cloned().unwrap_or_default();
                all.push(kdict);
                self.stack[base + a + 1] = self.ary_new(all);
            } else if n + 1 >= 15 {
                let mut all: Vec<Value> = self.stack[base + a + 1..base + a + 1 + n].to_vec();
                all.push(kdict);
                self.stack[base + a + 1] = self.ary_new(all);
                argc = 15;
            } else {
                self.stack[kidx] = kdict;
                argc = n + 1;
            }
        }
        let bidx = base + a + (if argc == 15 { 1 } else { argc }) + 1;
        if self.stack.len() <= bidx { self.stack.resize(bidx + 1, Value::Nil); }
        self.stack[bidx] = blk;
        let start_class = if is_super {
            let owner = self.ci.last().unwrap().target_class;
            match self.heap.class(owner).superclass { Some(s) => s, None => return Err(self.raise(self.core.no_method_error, "super: no superclass method")) }
        } else { self.class_of(recv) };
        let found = self.find_method(start_class, mid);
        let (m, owner) = match found {
            Some(x) => x,
            None => {
                // method_missing?
                let mm = self.s.method_missing;
                if let Some((m, owner)) = self.find_method(start_class, mm) {
                    if !matches!(m, Method::Native(_)) || self.class_of(recv) != self.core.basic_object {
                        // insert the method name as the first argument
                        let args = self.send_args(base + a, argc);
                        let mut nargs = vec![Value::Sym(mid)];
                        nargs.extend(args);
                        let r = match m { Method::Native(f) => f(self, recv, &nargs, blk)?, Method::Ruby(p) => self.call_proc(p, recv, &nargs, blk, Some(mm), owner)?, _ => Value::Nil };
                        self.stack[base + a] = r;
                        return Ok(());
                    }
                }
                let name = self.sym_name(mid);
                let desc = self.describe_for_error(recv);
                return Err(self.raise(self.core.no_method_error, &format!("undefined method '{name}' for {desc}")));
            }
        };
        match m {
            Method::Native(f) => {
                let args = self.send_args(base + a, argc);
                let r = f(self, recv, &args, blk)?;
                self.stack[base + a] = r;
            }
            Method::AttrReader(iv) => {
                let args = self.send_args(base + a, argc);
                if !args.is_empty() { return Err(self.argnum_error(args.len(), "0")); }
                self.stack[base + a] = recv.obj().map(|o| self.heap.ivar_get(o, iv)).unwrap_or(Value::Nil);
            }
            Method::AttrWriter(iv) => {
                let args = self.send_args(base + a, argc);
                if args.len() != 1 { return Err(self.argnum_error(args.len(), "1")); }
                let v = args[0];
                match recv { Value::Obj(o) => self.heap.ivar_set(o, iv, v), _ => return Err(self.raise_type("can't set instance variable")) }
                self.stack[base + a] = v;
            }
            Method::Ruby(p) => {
                if self.ci.len() >= CALL_LEVEL_MAX { return Err(self.raise(self.core.system_stack_error, "stack level too deep")); }
                let nbase = base + a;
                let nirep = self.heap.proc_data(p).irep;
                let nregs = self.ireps[nirep].nregs.max(argc + 2).max(4);
                if self.stack.len() < nbase + nregs { self.stack.resize(nbase + nregs, Value::Nil); }
                let nargs = if argc == 15 { 1 } else { argc };
                for i in nargs + 2..nregs { self.stack[nbase + i] = Value::Nil; }
                self.ci.push(CallInfo { base: nbase, pc: 0, irep: nirep, proc_: p, n: argc as u8, kw: false, mid: Some(mid), target_class: owner, env: None, cci: Cci::None });
            }
            Method::Undef => unreachable!(),
        }
        Ok(())
    }

    /// Positional arguments of a SEND at `nbase` (`argc == 15` = packed array).
    fn send_args(&self, nbase: usize, argc: usize) -> Vec<Value> {
        if argc == 15 { self.ary(self.stack[nbase + 1]).cloned().unwrap_or_default() } else { self.stack[nbase + 1..nbase + 1 + argc].to_vec() }
    }

    fn ensure_block(&mut self, b: Value) -> VmResult<Value> {
        match b {
            Value::Nil => Ok(Value::Nil),
            Value::Obj(o) if matches!(self.heap.get(o).kind, ObjKind::Proc(_)) => Ok(b),
            _ => {
                let to_proc = self.intern("to_proc");
                if self.respond_to(b, to_proc) { return self.funcall(b, to_proc, &[], Value::Nil); }
                Err(self.raise_type("not a block"))
            }
        }
    }

    /// mruby `vm_call_proc`: make the top frame execute `p` (self/mid/target
    /// class come from the proc's environment). `nargs` = registers to keep
    /// (self + args + block); the rest of the frame is cleared.
    fn vm_call_proc(&mut self, p: ObjId, nargs: usize) {
        let top = self.ci.len() - 1;
        let base = self.ci[top].base;
        let pd = self.heap.proc_data(p);
        let (irep, env, ptc) = (pd.irep, pd.env, pd.target_class);
        let nregs = self.ireps[irep].nregs.max(4);
        if self.stack.len() < base + nregs { self.stack.resize(base + nregs, Value::Nil); }
        for i in nargs..nregs { self.stack[base + i] = Value::Nil; }
        let (mid, tc, self_) = match env {
            Some(e) => { let ed = self.heap.env(e); (ed.mid, ed.target_class.or(ptc).unwrap_or(self.core.object), self.env_get(e, 0)) }
            None => (self.ci[top].mid, ptc.unwrap_or(self.core.object), self.stack[base]),
        };
        let ci = &mut self.ci[top];
        ci.irep = irep; ci.proc_ = p; ci.pc = 0; ci.mid = mid; ci.target_class = tc; ci.env = None;
        self.stack[base] = self_;
    }

    fn op_enter(&mut self, spec: u32) -> VmResult<()> {
        let m1 = ((spec >> 18) & 0x1f) as usize;
        let o = ((spec >> 13) & 0x1f) as usize;
        let r = ((spec >> 12) & 1) as usize;
        let m2 = ((spec >> 7) & 0x1f) as usize;
        let k = (spec >> 2) & 0x1f;
        let kd = ((spec >> 1) & 1) as usize;
        if k > 0 || kd > 0 { return Err(VmError::Unimplemented("keyword parameters".into())); }
        let top = self.ci.len() - 1;
        let (base, n, proc_, irep) = { let ci = &self.ci[top]; (ci.base, ci.n as usize, ci.proc_, ci.irep) };
        let strict = self.heap.proc_data(proc_).strict;
        let len = m1 + o + r + m2;
        let bidx = if n == 15 { 2 } else { n + 1 };
        let blk = self.stack[base + bidx];
        let mut argv: Vec<Value> = if n == 15 { self.ary(self.stack[base + 1]).cloned().unwrap_or_default() } else { self.stack[base + 1..base + 1 + n].to_vec() };
        let mut argc = argv.len();
        if strict {
            if argc < m1 + m2 || (r == 0 && argc > len) {
                let exp = if r == 0 && o == 0 { format!("{}", m1 + m2) } else if r == 0 { format!("{}..{}", m1 + m2, len) } else { format!("{}+", m1 + m2) };
                return Err(self.argnum_error(argc, &exp));
            }
        } else if len > 1 && argc == 1 {
            if let Some(arr) = self.ary(argv[0]) { argv = arr.clone(); argc = argv.len(); }
        }
        let nregs = self.ireps[irep].nregs.max(len + 3);
        if self.stack.len() < base + nregs { self.stack.resize(base + nregs, Value::Nil); }
        let mut pc = self.ci[top].pc;
        if argc < len {
            let mlen = if argc < m1 + m2 { if m1 < argc { argc - m1 } else { 0 } } else { m2 };
            for i in 0..argc.saturating_sub(mlen).min(m1 + o) { self.stack[base + 1 + i] = argv[i]; }
            for i in argc..m1 { self.stack[base + 1 + i] = Value::Nil; }
            for i in 0..mlen { self.stack[base + len - m2 + 1 + i] = argv[argc - mlen + i]; }
            for i in mlen..m2 { self.stack[base + len - m2 + 1 + i] = Value::Nil; }
            if r == 1 { let rest = self.ary_new(vec![]); self.stack[base + m1 + o + 1] = rest; }
            if o > 0 && argc > m1 + m2 { pc += (argc - m1 - m2) * 3; }
        } else {
            for i in 0..m1 + o { self.stack[base + 1 + i] = argv[i]; }
            let mut rnum = 0;
            if r == 1 { rnum = argc - m1 - o - m2; let rest = self.ary_new(argv[m1 + o..m1 + o + rnum].to_vec()); self.stack[base + m1 + o + 1] = rest; }
            if m2 > 0 { for i in 0..m2 { self.stack[base + m1 + o + r + 1 + i] = argv[m1 + o + rnum + i]; } }
            pc += o * 3;
        }
        self.stack[base + len + 1] = blk;
        let nlocals = self.ireps[irep].nlocals;
        for i in len + 2..nlocals { if base + i < self.stack.len() { self.stack[base + i] = Value::Nil; } }
        self.ci[top].n = len as u8;
        self.ci[top].pc = pc;
        Ok(())
    }

    fn op_blkpush(&mut self, base: usize, b: usize) -> VmResult<Value> {
        let m1 = (b >> 11) & 0x3f;
        let r = (b >> 10) & 1;
        let m2 = (b >> 5) & 0x1f;
        let kd = (b >> 4) & 1;
        let lv = b & 0xf;
        let offset = m1 + r + m2 + kd;
        let v = if lv == 0 { self.stack[base + 1 + offset] } else {
            match self.uvenv(lv - 1) { Some(e) if self.heap.env(e).len > offset + 1 => self.env_get(e, 1 + offset), _ => return Err(self.raise(self.core.local_jump_error, "unexpected yield")) }
        };
        if v.is_nil() { return Err(self.raise(self.core.local_jump_error, "unexpected yield")); }
        Ok(v)
    }

    /// Returns `Some(value)` when the loop should return to its caller.
    fn op_return(&mut self, v: Value, stop_depth: usize) -> VmResult<Option<Value>> {
        let top = self.ci.len() - 1;
        self.unwind_return(top, v, stop_depth)
    }

    fn op_return_blk(&mut self, v: Value, stop_depth: usize) -> VmResult<Option<Value>> {
        let top = self.ci.len() - 1;
        let p = self.ci[top].proc_;
        let pd = self.heap.proc_data(p);
        if pd.env.is_none() || pd.strict { return self.op_return(v, stop_depth); }
        // top_proc: walk `upper` until the method/lambda that owns the locals;
        // its env identifies the frame to return from.
        let mut cur = p;
        let mut env = pd.env;
        loop {
            let cd = self.heap.proc_data(cur);
            if cd.scope || cd.strict { break; }
            match cd.upper { Some(up) => { env = self.heap.proc_data(up).env.or(env); cur = up; } None => break }
        }
        let target_env = env;
        let mut idx = None;
        for i in (0..self.ci.len()).rev() {
            if self.ci[i].env.is_some() && self.ci[i].env == target_env { idx = Some(i); break; }
        }
        match idx {
            Some(i) => self.unwind_return(i, v, stop_depth),
            None => Err(self.raise(self.core.local_jump_error, "unexpected return")),
        }
    }

    fn op_break(&mut self, v: Value, stop_depth: usize) -> VmResult<Option<Value>> {
        let top = self.ci.len() - 1;
        let p = self.ci[top].proc_;
        let pd = self.heap.proc_data(p);
        if pd.strict { return self.op_return(v, stop_depth); }
        let dst = match (pd.orphan, pd.env, pd.upper) { (false, Some(_), Some(u)) => u, _ => return Err(self.raise(self.core.local_jump_error, "break from proc-closure")) };
        // return from the frame whose *caller* runs `dst` (the method that received the block)
        let mut idx = None;
        for i in (1..self.ci.len()).rev() {
            if self.ci[i - 1].proc_ == dst { idx = Some(i); break; }
        }
        match idx {
            Some(i) => self.unwind_return(i, v, stop_depth),
            None => Err(self.raise(self.core.local_jump_error, "break from proc-closure")),
        }
    }

    /// `OP_ARGARY`: rebuild the argument list of the enclosing method for `super` without arguments.
    fn op_argary(&mut self, base: usize, a: usize, b: usize) -> VmResult<()> {
        let m1 = (b >> 11) & 0x3f;
        let r = (b >> 10) & 1;
        let m2 = (b >> 5) & 0x1f;
        let kd = (b >> 4) & 1;
        let lv = b & 0xf;
        let ci = self.ci.last().unwrap().clone();
        if ci.mid.is_none() { return Err(self.raise(self.core.no_method_error, "super called outside of method")); }
        let get = |vm: &Vm, i: usize| -> VmResult<Value> {
            if lv == 0 { Ok(vm.stack.get(base + 1 + i).copied().unwrap_or(Value::Nil)) } else {
                match vm.uvenv(lv - 1) {
                    Some(e) if vm.heap.env(e).len > m1 + r + m2 + 1 => Ok(vm.env_get(e, 1 + i)),
                    _ => Err(VmError::Raise(Value::Nil)), // replaced below
                }
            }
        };
        let mut args = Vec::with_capacity(m1 + m2 + 1);
        let fail = |vm: &mut Vm| vm.raise(vm.core.no_method_error, "super called outside of method");
        for i in 0..m1 { args.push(get(self, i).map_err(|_| fail(self))?); }
        if r == 1 {
            let rest = get(self, m1).map_err(|_| fail(self))?;
            if let Some(v) = self.ary(rest) { args.extend(v.iter().copied()); }
        }
        for i in 0..m2 { args.push(get(self, m1 + r + i).map_err(|_| fail(self))?); }
        let blk_or_kd = get(self, m1 + r + m2).map_err(|_| fail(self))?;
        let need = base + a + 3;
        if self.stack.len() < need { self.stack.resize(need, Value::Nil); }
        self.stack[base + a] = self.ary_new(args);
        if kd == 1 {
            let blk = get(self, m1 + r + m2 + 1).map_err(|_| fail(self))?;
            self.stack[base + a + 1] = blk_or_kd;
            self.stack[base + a + 2] = blk;
        } else {
            self.stack[base + a + 1] = blk_or_kd;
        }
        Ok(())
    }

    // ------------------------------------------------------------------ error reporting

    /// mruby's `%T`: the receiver's class name (`NilClass` for nil, `Class` for a class).
    pub fn describe_for_error(&mut self, v: Value) -> String {
        let c = self.real_class_of(v);
        self.class_name(c)
    }
    /// mruby's `%Y`: `nil`/`true`/`false` for those, otherwise the class name.
    pub fn describe_for_type_error(&mut self, v: Value) -> String {
        match v {
            Value::Nil => "nil".into(),
            Value::True => "true".into(),
            Value::False => "false".into(),
            _ => self.describe_for_error(v),
        }
    }
    /// Human-readable description of an error (like mruby's `mrb_print_error`).
    pub fn describe_error(&mut self, e: &VmError) -> String {
        match e {
            VmError::Raise(exc) => {
                let cls = self.real_class_of(*exc);
                let cn = self.class_name(cls);
                let msg = self.exception_message(*exc);
                format!("{msg} ({cn})")
            }
            other => other.to_string(),
        }
    }
    pub fn exception_message(&mut self, exc: Value) -> String {
        if let Value::Obj(o) = exc {
            let m = self.heap.ivar_get(o, self.s.mesg);
            if let Some(b) = self.str_bytes(m) { return String::from_utf8_lossy(b).into_owned(); }
            let c = self.real_class_of(exc);
            return self.class_name(c);
        }
        "?".into()
    }
}

impl Default for Vm {
    fn default() -> Self { Vm::new() }
}

/// `MRB_CALL_LEVEL_MAX`.
pub const CALL_LEVEL_MAX: usize = 512;
/// Nested native -> VM re-entries allowed (each one uses host stack).
pub const NATIVE_DEPTH_MAX: u32 = 96;

#[inline]
fn jump(pc: usize, off: usize) -> usize {
    // 16-bit signed offset relative to the next instruction
    (pc as i64 + (off as u16 as i16) as i64) as usize
}

fn float_op(mid: Sym, s: Syms, p: f64, q: f64) -> Value {
    if mid == s.plus { Value::Float(p + q) } else if mid == s.minus { Value::Float(p - q) } else if mid == s.mul { Value::Float(p * q) } else { Value::Float(p / q) }
}

/// Renders an instruction listing in the style of `mrbc --verbose`.
pub fn dump(rite: &rite::Rite) -> String {
    let mut out = String::new();
    fn dump_irep(rite: &rite::Rite, i: usize, out: &mut String) {
        let ir = &rite.ireps[i];
        out.push_str(&format!("irep {} nregs={} nlocals={} pools={} syms={} reps={} ilen={}\n", i, ir.nregs, ir.nlocals, ir.pool.len(), ir.syms.len(), ir.reps.len(), ir.iseq.len()));
        for h in &ir.catch {
            out.push_str(&format!("catch type: {} begin: {:04} end: {:04} target: {:04}\n", match h.kind { CatchType::Rescue => "rescue", CatchType::Ensure => "ensure" }, h.begin, h.end, h.target));
        }
        let mut pc = 0;
        while pc < ir.iseq.len() {
            match ir.decode(pc) {
                Some((op, a, b, c, next)) => {
                    let mut line = format!("  {:03} {}", pc, op.name());
                    match op.operands() {
                        Operands::Z => {}
                        Operands::B | Operands::S | Operands::W => line.push_str(&format!("\t{a}")),
                        Operands::BB | Operands::BS => line.push_str(&format!("\t{a}\t{b}")),
                        Operands::BBB | Operands::BSS => line.push_str(&format!("\t{a}\t{b}\t{c}")),
                    }
                    match op {
                        Op::Loadsym | Op::Getgv | Op::Setgv | Op::Getiv | Op::Setiv | Op::Getcv | Op::Setcv | Op::Getconst | Op::Setconst | Op::Getmcnst | Op::Setmcnst | Op::Send | Op::Sendb | Op::Send0 | Op::Ssend | Op::Ssendb | Op::Ssend0 | Op::Def | Op::Tdef | Op::Sdef | Op::Class | Op::Module => {
                            if let Some(Some(s)) = ir.syms.get(b as usize) { line.push_str(&format!("\t; :{}", String::from_utf8_lossy(s))); }
                        }
                        Op::String | Op::Loadl | Op::Symbol => {
                            if let Some(p) = ir.pool.get(b as usize) { line.push_str(&format!("\t; {p:?}")); }
                        }
                        Op::Jmp | Op::Jmpuw => line.push_str(&format!("\t; -> {}", jump(next, a as usize))),
                        Op::Jmpif | Op::Jmpnot | Op::Jmpnil => line.push_str(&format!("\t; -> {}", jump(next, b as usize))),
                        _ => {}
                    }
                    out.push_str(&line);
                    out.push('\n');
                    pc = next;
                }
                None => { out.push_str(&format!("  {:03} ??? 0x{:02x}\n", pc, ir.iseq[pc])); pc += 1; }
            }
        }
        for &r in &ir.reps {
            out.push('\n');
            dump_irep(rite, r, out);
        }
    }
    dump_irep(rite, rite.root, &mut out);
    out
}
