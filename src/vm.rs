//! The interpreter: register machine executing mruby 4.1.0 bytecode.
//!
//! Frame layout follows mruby: a callee's `R0` is the caller's `R[a]`
//! (`base = caller.base + a`), so a method's return value lands in the
//! caller's target register by writing `stack[callee.base]`.

use alloc::{format, string::String, string::ToString, vec, vec::Vec};

use hashbrown::HashMap;

use crate::error::{VmError, VmResult};
use crate::object::{BreakTag, ClassData, EnvData, Heap, InstanceKind, IrepId, Method, ObjKind, ProcData, Vis, GC_MIN_INTERVAL};
use crate::inspect::{CatchHandlerInfo, DetachReason, SwitchKind, TraceEvent, UnwindBy};
use crate::opcode::{Op, Operands};
use crate::rite::{self, CatchType, Pool};
use crate::symbol::{Interner, Sym};
use crate::value::{slots_of, values_of, ObjId, Slot, Value};

pub struct VmIrep {
    pub nlocals: usize,
    pub nregs: usize,
    pub iseq: Vec<u8>,
    pub catch: Vec<rite::CatchHandler>,
    pub pool: Vec<Pool>,
    pub syms: Vec<Sym>,
    pub reps: Vec<IrepId>,
    pub lv: Vec<Option<Sym>>,
    /// `(start_pc, line)` from the DBG section (`mrbc -g`); empty without it.
    pub lines: Vec<(u32, u32)>,
    /// Source file name from the DBG section.
    pub filename: Option<String>,
}

impl VmIrep {
    /// The source line of `pc` (mruby `mrb_debug_get_line`).
    pub fn line_of(&self, pc: usize) -> Option<u32> {
        let pc = pc as u32;
        match self.lines.partition_point(|(start, _)| *start <= pc) {
            0 => None,
            i => Some(self.lines[i - 1].1),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cci {
    /// Ordinary Ruby frame.
    None,
    /// Frame started from native code (`Vm::call_proc`); the interpreter loop
    /// returns to the native caller when this frame is popped.
    Skip,
}

/// `mrb_fiber_state`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FiberState { Created, Running, Resumed, Suspended, Transferred, Terminated }

/// `mrb_context`: one register stack and one frame stack. The running
/// context's `stack`/`ci` live in the `Vm` fields; the entry here is empty
/// while it runs (they are swapped on every switch).
pub struct Context {
    pub stack: Vec<Slot>,
    pub ci: Vec<CallInfo>,
    pub status: FiberState,
    /// The context to return to on `Fiber.yield` / termination (`prev`).
    pub prev: Option<usize>,
    /// The Fiber object of this context, made lazily by `Fiber.current` for the root.
    pub fib: Option<ObjId>,
    /// The block the fiber runs (set by `Fiber#initialize`).
    pub proc_: Option<ObjId>,
    /// Resumed by native code (`mrb_fiber_resume`): a nested run loop is
    /// waiting on the host stack and the fiber's next yield must return from it.
    pub vmexec: bool,
    /// Register (absolute index into `stack`) of the `resume`/`yield`/`transfer`
    /// call this context is suspended in; the value it is switched back with
    /// lands there (mruby writes it to the pending C frame's `stack[0]`).
    pub pending_reg: Option<usize>,
}

impl Context {
    pub fn new(status: FiberState) -> Context {
        Context { stack: Vec::new(), ci: Vec::new(), status, prev: None, fib: None, proc_: None, vmexec: false, pending_reg: None }
    }
}

/// Index of the root context in `Vm::contexts`.
pub const ROOT: usize = 0;

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
    /// Visibility given to methods defined by `def` in this frame (`private` with no arguments).
    pub vis: Vis,
    /// `module_function` with no arguments: following `def`s also become singleton methods.
    pub modfunc: bool,
    /// `instance_eval`/`class_eval` frame: visibility lookups stop here.
    pub vis_break: bool,
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
    pub fiber: ObjId,
    pub fiber_error: ObjId,
}

impl Core {
    /// Every class in the set (GC roots).
    pub fn ids(&self) -> [ObjId; 39] {
        [self.basic_object, self.object, self.module, self.class, self.kernel, self.comparable, self.enumerable,
         self.nil_class, self.true_class, self.false_class, self.numeric, self.integer, self.float, self.symbol,
         self.string, self.array, self.hash, self.range, self.proc_, self.exception, self.standard_error,
         self.runtime_error, self.argument_error, self.type_error, self.name_error, self.no_method_error,
         self.zero_division_error, self.local_jump_error, self.index_error, self.range_error, self.key_error,
         self.not_implemented_error, self.stop_iteration, self.frozen_error, self.float_domain_error,
         self.no_matching_pattern_error, self.system_stack_error, self.fiber, self.fiber_error]
    }
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
    #[doc(hidden)]
    pub ireps: Vec<VmIrep>,
    #[doc(hidden)]
    pub stack: Vec<Slot>,
    #[doc(hidden)]
    pub ci: Vec<CallInfo>,
    pub globals: HashMap<Sym, Slot>,
    /// The exception being propagated (`mrb->exc`) between `L_RAISE` and `EXCEPT`.
    #[doc(hidden)]
    pub exc: Option<Value>,
    out: Vec<u8>,
    pub core: Core,
    pub s: Syms,
    pub top_self: ObjId,
    /// Instruction budget for [`Vm::step`]; `None` = unlimited.
    step_left: Option<u64>,
    /// The Proc whose body is a single `OP_CALL` (mruby `call_proc`): the
    /// method body of `Proc#call`, so calling a block does not re-enter the VM.
    #[doc(hidden)]
    pub call_proc: ObjId,
    pub instructions: u64,
    /// Executions per opcode (index = opcode number); the test runner reports
    /// which opcodes a workload never reached.
    pub op_counts: Vec<u64>,
    /// Nesting of native -> VM re-entries (`call_proc_with`); bounded to protect the host stack.
    native_depth: u32,
    /// Objects whose `inspect` is in progress (recursive containers print `[...]`).
    #[doc(hidden)]
    pub inspect_guard: Vec<ObjId>,
    /// Keyword hash of the native call in progress. `Vm::funcall` re-attaches it
    /// as keywords when a native forwards its arguments unchanged (`send`, `new`).
    #[doc(hidden)]
    pub pending_kw: Option<Value>,
    /// Pairs whose `==`/`eql?` is in progress (recursive containers compare equal).
    #[doc(hidden)]
    pub eq_guard: Vec<(ObjId, ObjId)>,
    /// `GC.disable` state: collections are postponed until `GC.enable`.
    pub gc_disabled: bool,
    pending_vis_break: bool,
    /// Native fns that stand for `mrb_notimplement()`: `respond_to?` answers false for them.
    #[doc(hidden)]
    pub notimpl_fns: Vec<crate::object::NativeFn>,
    pub gc_step_limit: i64,
    /// `GC.interval_ratio` (percent): the heap may grow to `live * ratio / 100` between collections.
    pub gc_interval_ratio: i64,
    /// Collect at the first instruction boundary after every allocation (`SABIRUBY_GC_STRESS`).
    #[doc(hidden)]
    pub gc_stress: bool,
    /// Natives running on the host stack (SEND -> native, `funcall` -> native).
    /// Their Rust locals may hold values the collector cannot see, so it does not
    /// run while this is non-zero (see `docs/gc.md`, "Contract for native code").
    #[doc(hidden)]
    pub native_active: u32,
    /// Objects the host keeps across calls (`mrb_gc_register`).
    #[doc(hidden)]
    pub gc_registered: Vec<ObjId>,
    /// Objects alive after the last collection.
    pub live_after_gc: usize,
    /// Collections run so far.
    pub gc_count: u64,
    /// Total time in the collector, measured with `gc_clock` when the host sets one.
    pub gc_time_ns: u64,
    /// Monotonic clock in nanoseconds (the library is no_std; the CLI supplies one).
    pub gc_clock: Option<fn() -> u64>,
    /// Recording of what the interpreter does, for debuggers (`Vm::set_trace`, `src/inspect.rs`).
    /// `None` (the default) means nothing is recorded.
    pub trace: Option<Vec<crate::inspect::TraceEvent>>,
    /// All contexts (fibers); `contexts[cur]` is the running one (its stack/ci are in `stack`/`ci`).
    #[doc(hidden)]
    pub contexts: Vec<Context>,
    #[doc(hidden)]
    pub cur: usize,
    /// True while a native method called straight from a SEND instruction runs
    /// (mruby: the frame's `cci == CINFO_NONE`); false when called through
    /// `funcall` from other native code. Decides whether a fiber switch can
    /// continue in the current run loop or needs a nested one.
    #[doc(hidden)]
    pub direct_send: bool,
    /// Absolute register the native call in progress writes its result to.
    native_ret_reg: usize,
    /// Set by a fiber switch that must end the innermost run loop (yield or
    /// termination of a fiber resumed by native code): the loop returns this value.
    loop_exit: Option<Value>,
    /// Arity of natives as the reference declares it (`MRB_ARGS_*`), for `Method#arity`.
    #[doc(hidden)]
    pub native_arity: Vec<(crate::object::NativeFn, i64)>,
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
        let _syntax_error = c(&mut heap, &mut syms, "SyntaxError", script_error);
        let not_implemented_error = c(&mut heap, &mut syms, "NotImplementedError", script_error);
        let stop_iteration = c(&mut heap, &mut syms, "StopIteration", index_error);
        let frozen_error = c(&mut heap, &mut syms, "FrozenError", runtime_error);
        let float_domain_error = c(&mut heap, &mut syms, "FloatDomainError", range_error);
        let no_matching_pattern_error = c(&mut heap, &mut syms, "NoMatchingPatternError", standard_error);
        let system_stack_error = c(&mut heap, &mut syms, "SystemStackError", exception);
        // mruby-fiber
        let fiber = c(&mut heap, &mut syms, "Fiber", object);
        let fiber_error = c(&mut heap, &mut syms, "FiberError", standard_error);
        let core = Core {
            basic_object, object, module, class, kernel, comparable, enumerable, nil_class, true_class,
            false_class, numeric, integer, float, symbol, string, array, hash, range, proc_, exception,
            standard_error, runtime_error, argument_error, type_error, name_error, no_method_error,
            zero_division_error, local_jump_error, index_error, range_error, key_error,
            not_implemented_error, stop_iteration, frozen_error, float_domain_error,
            no_matching_pattern_error, system_stack_error, fiber, fiber_error,
        };
        // Every object allocated so far is a class or module: set its class.
        for i in 0..heap.len() {
            let id = ObjId(i as u32);
            let is_mod = heap.class(id).is_module;
            heap.get_mut(id).class = if is_mod { module } else { class };
        }
        for (cls, kind) in [(basic_object, InstanceKind::Object), (string, InstanceKind::String), (array, InstanceKind::Array), (hash, InstanceKind::Hash),
                            (range, InstanceKind::Range), (exception, InstanceKind::Exception), (proc_, InstanceKind::Proc), (fiber, InstanceKind::Fiber),
                            (integer, InstanceKind::NoAlloc), (float, InstanceKind::NoAlloc), (symbol, InstanceKind::NoAlloc), (nil_class, InstanceKind::NoAlloc),
                            (true_class, InstanceKind::NoAlloc), (false_class, InstanceKind::NoAlloc)] {
            heap.class_mut(cls).instance_kind = Some(kind);
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
        let call_irep = VmIrep { nlocals: 1, nregs: 4, iseq: vec![Op::Call as u8], catch: vec![], pool: vec![], syms: vec![], reps: vec![], lv: vec![], lines: vec![], filename: None };
        let call_proc = heap.alloc(core.proc_, ObjKind::Proc(ProcData { irep: 0, upper: None, env: None, target_class: Some(core.proc_), strict: true, scope: true, orphan: false }));
        let mut vm = Vm {
            heap, syms, ireps: vec![call_irep], stack: Vec::new(), ci: Vec::new(), globals: HashMap::new(),
            exc: None, out: Vec::new(), core, s, top_self, step_left: None, instructions: 0, op_counts: vec![0; crate::opcode::OP_COUNT], native_depth: 0, inspect_guard: Vec::new(), pending_kw: None, eq_guard: Vec::new(), gc_disabled: false, pending_vis_break: false, notimpl_fns: Vec::new(), gc_step_limit: 0, gc_interval_ratio: 200, gc_stress: false, native_active: 0, gc_registered: Vec::new(), live_after_gc: 0, gc_count: 0, gc_time_ns: 0, gc_clock: None, trace: None, call_proc,
            contexts: vec![Context::new(FiberState::Running)], cur: ROOT, direct_send: false, native_ret_reg: 0, loop_exit: None, native_arity: Vec::new(),
        };
        // Constants for the core classes, Object includes Kernel.
        for i in 0..vm.heap.len() {
            let id = ObjId(i as u32);
            if vm.heap.is_class(id) {
                if let Some(n) = vm.heap.class(id).name {
                    vm.heap.class_mut(object).consts.insert(n, Slot::from(Value::Obj(id)));
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
        vm.load_mrblib()?;
        Ok(vm)
    }

    /// Loads `mrblib` and the Ruby parts of the gems into a VM made by [`Vm::new`]
    /// (so a host can set options such as `gc_stress` first).
    pub fn load_mrblib(&mut self) -> VmResult<()> {
        let vm = self;
        vm.load_and_run(crate::MRBLIB_MRB)?;
        // gems with a Ruby part, in the order of the reference gembox
        // (`mrbgems/default.gembox`: the *-ext gems before mruby-enumerator,
        // whose `Enumerable#zip` therefore wins over mruby-enum-ext's)
        for lib in [crate::MRBLIB_SPRINTF_MRB, crate::MRBLIB_ENUM_EXT_MRB, crate::MRBLIB_STRING_EXT_MRB, crate::MRBLIB_ARRAY_EXT_MRB, crate::MRBLIB_HASH_EXT_MRB, crate::MRBLIB_RANGE_EXT_MRB, crate::MRBLIB_PROC_EXT_MRB, crate::MRBLIB_ENUMERATOR_MRB, crate::MRBLIB_METHOD_MRB] {
            vm.load_and_run(lib)?;
        }
        Ok(())
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
        Value::Obj(self.heap.alloc(self.core.array, ObjKind::Array(slots_of(&v))))
    }
    pub fn hash_new(&mut self) -> Value {
        Value::Obj(self.heap.alloc(self.core.hash, ObjKind::Hash(Default::default())))
    }
    pub fn range_new(&mut self, begin: Value, end: Value, excl: bool) -> Value {
        let o = self.heap.alloc(self.core.range, ObjKind::Range { begin: Slot::from(begin), end: Slot::from(end), excl });
        self.heap.get_mut(o).frozen = true;
        Value::Obj(o)
    }
    /// `range_ptr_replace`: both ends must be comparable (`<=>` not nil).
    pub fn check_range_ends(&mut self, x: Value, y: Value) -> VmResult<()> {
        if x.is_nil() || y.is_nil() { return Ok(()); }
        let ok = match (x, y) {
            (Value::Int(_), Value::Int(_)) => true,
            (Value::Int(p), Value::Float(q)) => crate::builtins::numeric::int_float_cmp(p, q).is_some(),
            (Value::Float(p), Value::Int(q)) => crate::builtins::numeric::int_float_cmp(q, p).is_some(),
            (Value::Float(p), Value::Float(q)) => p.partial_cmp(&q).is_some(),
            _ => { let cmp = self.intern("<=>"); !self.funcall(x, cmp, &[y], Value::Nil)?.is_nil() }
        };
        if ok { Ok(()) } else { Err(self.raise_arg("bad value for range")) }
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
    /// A `NameError` carrying `@name`.
    pub fn name_error(&mut self, name: Sym, msg: &str) -> VmError {
        let e = self.exc_new(self.core.name_error, msg);
        if let Value::Obj(o) = e { let k = self.intern("@name"); self.heap.ivar_set(o, k, Value::Sym(name)); }
        VmError::Raise(e)
    }
    /// A `NoMethodError` carrying `@name` (NameError#name) and an empty `@args`.
    pub fn no_method_error(&mut self, mid: Sym, _recv: Value, msg: &str) -> VmError {
        let e = self.exc_new(self.core.no_method_error, msg);
        if let Value::Obj(o) = e {
            let k = self.intern("@name"); self.heap.ivar_set(o, k, Value::Sym(mid));
            let a = self.intern("@args"); let empty = self.ary_new(vec![]); self.heap.ivar_set(o, a, empty);
        }
        VmError::Raise(e)
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
    pub fn ary(&self, v: Value) -> Option<&Vec<Slot>> {
        v.obj().and_then(|o| self.heap.array(o))
    }
    /// The elements of an Array as values (a copy).
    pub fn ary_vals(&self, v: Value) -> Option<Vec<Value>> {
        self.ary(v).map(|a| values_of(a))
    }
    pub fn expect_str(&mut self, v: Value, what: &str) -> VmResult<Vec<u8>> {
        match self.str_bytes(v) {
            Some(b) => Ok(b.to_vec()),
            None => Err(self.raise_type(&format!("{what} cannot be converted to String"))),
        }
    }
    pub fn expect_int(&mut self, v: Value, _what: &str) -> VmResult<i64> {
        match v {
            Value::Int(i) => Ok(i),
            Value::Float(f) => {
                if f.is_nan() || f.is_infinite() { let s = crate::builtins::numeric::float_to_s(f); return Err(self.raise(self.core.float_domain_error, &s)); }
                if f >= 9223372036854775808.0 || f < -9223372036854775808.0 { let s = crate::builtins::numeric::float_to_s(f); return Err(self.raise(self.core.range_error, &format!("float {s} out of range of integer"))); }
                Ok(f as i64)
            }
            _ => { let d = self.describe_for_type_error(v); Err(self.raise_type(&format!("no implicit conversion of {d} into Integer"))) }
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
            if cd.is_singleton || cd.iclass_of.is_some() || cd.origin_of.is_some() {
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
        if let Some(o) = cd.origin_of {
            return self.class_name(o);
        }
        match cd.name {
            Some(n) => match cd.outer {
                Some(o) if o != self.core.object => format!("{}::{}", self.class_name(o), self.syms.name_str(n)),
                _ => self.syms.name_str(n),
            },
            None => {
                let kind = if cd.is_module { "Module" } else { "Class" };
                format!("#<{kind}:0x{:012x}>", (c.0 as usize + 1) * 0x40)
            }
        }
    }
    pub fn define_class(&mut self, name: &str, superclass: ObjId) -> ObjId {
        let n = self.intern(name);
        if let Some(Value::Obj(c)) = self.heap.class(self.core.object).consts.get(&n).map(|s| s.get()) {
            return c;
        }
        let c = self.heap.alloc(self.core.class, ObjKind::Class(ClassData { name: Some(n), superclass: Some(superclass), ..Default::default() }));
        self.heap.class_mut(self.core.object).consts.insert(n, Slot::from(Value::Obj(c)));
        self.singleton_class(Value::Obj(c)).expect("metaclass");
        c
    }
    pub fn define_module(&mut self, name: &str) -> ObjId {
        let n = self.intern(name);
        if let Some(Value::Obj(c)) = self.heap.class(self.core.object).consts.get(&n).map(|s| s.get()) {
            return c;
        }
        let c = self.heap.alloc(self.core.module, ObjKind::Class(ClassData { name: Some(n), is_module: true, ..Default::default() }));
        self.heap.class_mut(self.core.object).consts.insert(n, Slot::from(Value::Obj(c)));
        c
    }
    pub fn define_method(&mut self, class: ObjId, name: &str, f: crate::object::NativeFn) {
        let n = self.intern(name);
        self.def_method_raw(class, n, Method::Native(f));
    }
    pub fn alias_method(&mut self, class: ObjId, new: Sym, old: Sym) -> VmResult<()> {
        match self.find_method(class, old) {
            Some((m, owner)) => { let vis = self.method_vis(owner, old); self.def_method(class, new, m, vis) }
            None => { let n = self.sym_name(old); let cn = self.class_name(class); Err(self.name_error(old, &format!("undefined method '{n}' for class '{cn}'"))) }
        }
    }
    pub fn undef_method(&mut self, class: ObjId, mid: Sym) -> VmResult<()> {
        if self.heap.get(class).frozen { return Err(self.frozen_error(Value::Obj(class))); }
        if self.find_method(class, mid).is_none() { let n = self.sym_name(mid); let cn = self.class_name(class); return Err(self.name_error(mid, &format!("undefined method '{n}' for class '{cn}'"))); }
        let t = self.def_target(class);
        self.heap.class_mut(t).methods.insert(mid, Method::Undef);
        Ok(())
    }
    /// The table that receives definitions for `class` (its origin when modules are prepended).
    pub fn def_target(&self, class: ObjId) -> ObjId {
        self.heap.class(class).origin.unwrap_or(class)
    }
    /// Installs a method without hooks (used by initialization and internal copies).
    pub fn def_method_raw(&mut self, class: ObjId, mid: Sym, m: Method) {
        let t = self.def_target(class);
        self.heap.class_mut(t).methods.insert(mid, m);
        self.heap.class_mut(t).vis.remove(&mid);
    }
    /// Installs a method with visibility and calls the `method_added` hook.
    pub fn def_method(&mut self, class: ObjId, mid: Sym, m: Method, vis: Vis) -> VmResult<()> {
        if self.heap.get(class).frozen { return Err(self.frozen_error(Value::Obj(class))); }
        let t = self.def_target(class);
        self.heap.class_mut(t).methods.insert(mid, m);
        // initialize & co. are always private (class.c)
        let always_private = [self.s.initialize, self.intern("initialize_copy"), self.intern("respond_to_missing?")];
        let vis = if always_private.contains(&mid) { Vis::Private } else { vis };
        if vis == Vis::Public { self.heap.class_mut(t).vis.remove(&mid); } else { self.heap.class_mut(t).vis.insert(mid, vis); }
        self.method_added(class, mid)
    }
    /// Where the default visibility for `def` lives (class.c `find_visibility_scope`):
    /// the nearest scope frame or, once that frame has an environment, its env.
    /// `c` = the class being defined into (`None` = the frame's target class).
    pub fn visibility_scope(&self, c: Option<ObjId>) -> (Option<usize>, Option<ObjId>) {
        let top = self.ci.len() - 1;
        let ci = self.ci[top];
        let c = c.unwrap_or(ci.target_class);
        let brk_frame = |p: ObjId| -> bool {
            let pd = self.heap.proc_data(p);
            pd.upper.is_none() || pd.scope || pd.env.is_none() || ci.target_class != c || ci.vis_break
        };
        if brk_frame(ci.proc_) {
            return (Some(top), ci.env);
        }
        let mut p = ci.proc_;
        loop {
            let env = self.heap.proc_data(p).env;
            let up = self.heap.proc_data(p).upper;
            let stop = match up {
                None => true,
                Some(u) => { let ud = self.heap.proc_data(u); ud.upper.is_none() || ud.scope || ud.env.is_none() || match env { Some(e) => { let ed = self.heap.env(e); ed.target_class != Some(c) || ed.vis_break } None => true } }
            };
            if stop { return (None, env); }
            p = up.unwrap();
        }
    }
    /// The (visibility, module_function) that a `def` in the current frame gets.
    pub fn current_def_vis(&self, c: ObjId) -> (Vis, bool) {
        match self.visibility_scope(Some(c)) {
            (_, Some(e)) => { let ed = self.heap.env(e); (ed.vis, ed.modfunc) }
            (Some(i), None) => (self.ci[i].vis, self.ci[i].modfunc),
            _ => (Vis::Public, false),
        }
    }
    /// `private` etc. with no arguments: records the default on the scope.
    pub fn set_scope_vis(&mut self, vis: Vis, modfunc: bool) {
        match self.visibility_scope(None) {
            (_, Some(e)) => { let ed = self.heap.env_mut(e); ed.vis = vis; ed.modfunc = modfunc; }
            (Some(i), None) => { self.ci[i].vis = vis; self.ci[i].modfunc = modfunc; }
            _ => {}
        }
    }
    /// `mrb_method_added`: `method_added` / `singleton_method_added` hook.
    pub fn method_added(&mut self, class: ObjId, mid: Sym) -> VmResult<()> {
        let cd = self.heap.class(class);
        let (recv, hook) = if cd.is_singleton { (cd.attached.map(|s| s.get()).unwrap_or(Value::Nil), self.intern("singleton_method_added")) } else { (Value::Obj(class), self.intern("method_added")) };
        if let Some((Method::Native(_), owner)) = self.find_method(self.class_of(recv), hook) {
            if owner == self.core.module || owner == self.core.basic_object { return Ok(()); }
        }
        if self.respond_to(recv, hook) { self.funcall(recv, hook, &[Value::Sym(mid)], Value::Nil)?; }
        Ok(())
    }
    /// The class whose `methods`/`vis` tables a chain node reads: an include class
    /// reads its module's table (the module's origin when it has prepends).
    pub fn table_owner(&self, x: ObjId) -> ObjId {
        match self.heap.class(x).iclass_of { Some(m) => self.heap.class(m).origin.unwrap_or(m), None => x }
    }
    /// Visibility of `mid` as found from `class`.
    pub fn method_vis(&self, class: ObjId, mid: Sym) -> Vis {
        let mut c = Some(class);
        while let Some(x) = c {
            let cd = self.heap.class(x);
            let tbl_owner = self.table_owner(x);
            let t = self.heap.class(tbl_owner);
            if cd.origin.is_none() {
                if t.methods.contains_key(&mid) { return t.vis.get(&mid).copied().unwrap_or(Vis::Public); }
            }
            c = cd.superclass;
        }
        Vis::Public
    }
    /// `private :m` etc.: sets visibility in `class`, copying an inherited method first.
    pub fn set_visibility(&mut self, class: ObjId, mid: Sym, vis: Vis) -> VmResult<()> {
        if self.heap.get(class).frozen { return Err(self.frozen_error(Value::Obj(class))); }
        let t = self.def_target(class);
        if !self.heap.class(t).methods.contains_key(&mid) {
            match self.find_method(class, mid) {
                Some((m, _)) => { self.heap.class_mut(t).methods.insert(mid, m); }
                None => { let n = self.sym_name(mid); let cn = self.class_name(class); return Err(self.raise(self.core.name_error, &format!("undefined method '{n}' for class '{cn}'"))); }
            }
        }
        if vis == Vis::Public { self.heap.class_mut(t).vis.remove(&mid); } else { self.heap.class_mut(t).vis.insert(mid, vis); }
        Ok(())
    }
    /// `Module#prepend`: moves the class's own table into an origin include class once, then
    /// inserts the module's include class right below the class.
    pub fn prepend_module(&mut self, class: ObjId, module: ObjId) -> VmResult<()> {
        if self.heap.get(class).frozen { return Err(self.frozen_error(Value::Obj(class))); }
        if self.heap.class(class).origin.is_none() {
            let (methods, vis, sup) = { let c = self.heap.class_mut(class); (core::mem::take(&mut c.methods), core::mem::take(&mut c.vis), c.superclass) };
            let origin = self.heap.alloc(self.core.class, ObjKind::Class(ClassData { superclass: sup, origin_of: Some(class), methods, vis, ..Default::default() }));
            let c = self.heap.class_mut(class);
            c.superclass = Some(origin);
            c.origin = Some(origin);
        }
        // already prepended (between class and origin)?
        let origin = self.heap.class(class).origin.unwrap();
        let mut c = self.heap.class(class).superclass;
        while let Some(x) = c {
            if x == origin { break; }
            if self.heap.class(x).iclass_of == Some(module) { return Ok(()); }
            c = self.heap.class(x).superclass;
        }
        if module == class || self.module_chain_contains(module, class) { return Err(self.raise_arg("cyclic prepend detected")); }
        // the module's own chain goes in too, nearest first: insert in reverse right below the class
        // (a module already anywhere in the chain, prepended or included, is not added again)
        let mods = self.module_chain(module);
        for m in mods.iter().rev() {
            // mruby include_module_at(search_super=0): scan stops at the first real class,
            // so a module already prepended to a superclass is prepended again here
            let mut c = self.heap.class(class).superclass;
            let mut present = false;
            while let Some(x) = c {
                let cd = self.heap.class(x);
                if cd.iclass_of == Some(*m) { present = true; break; }
                if cd.iclass_of.is_none() && cd.origin_of.is_none() { break; }
                c = cd.superclass;
            }
            if present { continue; }
            let sup = self.heap.class(class).superclass;
            let ic = self.heap.alloc(self.core.class, ObjKind::Class(ClassData { superclass: sup, iclass_of: Some(*m), ..Default::default() }));
            self.heap.class_mut(class).superclass = Some(ic);
        }
        Ok(())
    }
    /// Defines the same native method on several classes.
    pub fn define_methods(&mut self, class: ObjId, list: &[(&str, crate::object::NativeFn)]) {
        for (n, f) in list {
            self.define_method(class, n, *f);
        }
    }
    /// Inserts an include class for `module` right above `class` in the chain.
    pub fn include_module(&mut self, class: ObjId, module: ObjId) {
        let mods = self.module_chain(module);
        let mut at = self.def_target(class); // below the origin when modules are prepended
        for m in mods {
            // already in the chain?
            let mut c = self.heap.class(class).superclass;
            let mut present = false;
            while let Some(x) = c { if self.heap.class(x).iclass_of == Some(m) { present = true; break; } c = self.heap.class(x).superclass; }
            if present { continue; }
            let sup = self.heap.class(at).superclass;
            let ic = self.heap.alloc(self.core.class, ObjKind::Class(ClassData { superclass: sup, iclass_of: Some(m), ..Default::default() }));
            self.heap.class_mut(at).superclass = Some(ic);
            at = ic;
        }
    }
    /// The modules a module brings along (its prepends and includes), nearest first.
    pub fn module_chain(&self, module: ObjId) -> Vec<ObjId> {
        let mut out = vec![];
        let mut c = Some(module);
        while let Some(x) = c {
            let cd = self.heap.class(x);
            if let Some(m) = cd.iclass_of { for y in self.module_chain(m) { if !out.contains(&y) { out.push(y); } } }
            else if let Some(o) = cd.origin_of { if !out.contains(&o) { out.push(o); } }
            else if x == module && cd.origin.is_none() { out.push(x); }
            c = cd.superclass;
        }
        out
    }
    /// True when `class` appears in `module`'s chain (would make a cycle).
    pub fn module_chain_contains(&self, module: ObjId, class: ObjId) -> bool {
        let mut c = Some(module);
        while let Some(x) = c { let cd = self.heap.class(x); if x == class || cd.iclass_of == Some(class) { return true; } c = cd.superclass; }
        false
    }
    /// The singleton class of `v`, created on demand (`prepare_singleton_class`).
    pub fn singleton_class(&mut self, v: Value) -> VmResult<ObjId> {
        let o = match v {
            Value::Obj(o) => o,
            Value::Nil => return Ok(self.core.nil_class),
            Value::True => return Ok(self.core.true_class),
            Value::False => return Ok(self.core.false_class),
            _ => return Err(self.raise_type("can't define singleton")),
        };
        let cur = self.heap.get(o).class;
        if self.heap.class(cur).is_singleton && self.heap.class(cur).attached == Some(Slot::from(v)) {
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
        let sc = self.heap.alloc(self.core.class, ObjKind::Class(ClassData { superclass: sup, is_singleton: true, attached: Some(Slot::from(v)), ..Default::default() }));
        self.heap.get_mut(o).class = sc;
        if self.heap.get(o).frozen { self.heap.get_mut(sc).frozen = true; }
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
                return Some(v.get());
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
            if cd.origin.is_some() { c = cd.superclass; continue; } // own table lives in the origin
            let tbl = &self.heap.class(self.table_owner(x)).methods;
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
        match self.find_method(self.class_of(v), mid) {
            Some((Method::Native(f), _)) => !self.notimpl_fns.iter().any(|g| core::ptr::fn_addr_eq(*g, f)),
            Some(_) => true,
            None => false,
        }
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
                lines: ir.lines.clone(),
                filename: ir.filename.as_ref().map(|f| String::from_utf8_lossy(f).into_owned()),
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
        self.stack.resize(base + nregs, Slot::NIL);
        self.stack[base] = Slot::from(Value::Obj(self.top_self));
        let depth = self.ci.len();
        self.ci.push(CallInfo { base, pc: 0, irep, proc_, n: 0, kw: false, mid: None, target_class: self.core.object, env: None, cci: Cci::Skip, vis: Vis::Public, modfunc: false, vis_break: false });
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
        self.stack.resize(base + nregs, Slot::NIL);
        self.stack[base] = Slot::from(Value::Obj(self.top_self));
        self.ci.push(CallInfo { base, pc: 0, irep, proc_, n: 0, kw: false, mid: None, target_class: self.core.object, env: None, cci: Cci::Skip, vis: Vis::Public, modfunc: false, vis_break: false });
    }

    /// Executes at most `budget` instructions of a program started with
    /// [`Vm::start`]. This is the instruction-boundary suspension point
    /// (mruby's `RETURN_IF_TASK_STOPPED`).
    pub fn step(&mut self, budget: u64) -> VmResult<Step> {
        if self.ci.is_empty() {
            return Ok(Step::Finished(Value::Nil));
        }
        self.step_left = Some(budget);
        let r = self.run_loop_ctx(ROOT, 0);
        self.step_left = None;
        match r {
            Ok(v) if self.cur == ROOT && self.ci.is_empty() => { self.stack.clear(); Ok(Step::Finished(v)) }
            Ok(_) => Ok(Step::Paused),
            Err(e) => { self.reset_to_root(); Err(e) }
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
                let direct = core::mem::replace(&mut self.direct_send, false);
                let r = self.call_native(f, recv, args, blk);
                self.direct_send = direct;
                self.native_depth -= 1;
                self.orphan_block_of_native(blk);
                r
            }
            Some((Method::AttrReader(iv), _)) => Ok(recv.obj().map(|o| self.heap.ivar_get(o, iv)).unwrap_or(Value::Nil)),
            Some((Method::AttrWriter(iv), _)) => {
                let v = args.first().copied().unwrap_or(Value::Nil);
                if let Some(o) = recv.obj() { self.heap.ivar_set(o, iv, v); }
                Ok(v)
            }
            Some((Method::Ruby(p), owner)) => {
                // A native forwarding its own arguments (`send`, `Class#new`) keeps
                // the caller's keywords: the trailing Hash is the pending kdict.
                let kw = match (self.pending_kw, args.last()) { (Some(k), Some(l)) if !k.is_nil() && k == *l => Some(k), _ => None };
                let tc = if self.heap.proc_data(p).env.is_some() { None } else { Some(owner) };
                match kw {
                    Some(k) => self.call_proc_with(p, recv, &args[..args.len() - 1], Some(k), blk, Some(mid), tc),
                    None => self.call_proc_with(p, recv, args, None, blk, Some(mid), tc),
                }
            }
            Some((Method::Undef, _)) | None => {
                // a user-defined method_missing takes the call (the basic one only reports)
                let mm = self.s.method_missing;
                if let Some((m, owner)) = self.find_method(cls, mm) {
                    if !matches!(m, Method::Native(_)) {
                        let mut nargs = vec![Value::Sym(mid)];
                        nargs.extend_from_slice(args);
                        if let Method::Ruby(p) = m {
                            let kw = match (self.pending_kw, args.last()) { (Some(k), Some(l)) if !k.is_nil() && k == *l => Some(k), _ => None };
                            let pos = if kw.is_some() { &nargs[..nargs.len() - 1] } else { &nargs[..] };
                            let tc = if self.heap.proc_data(p).env.is_some() { None } else { Some(owner) };
                            return self.call_proc_with(p, recv, pos, kw, blk, Some(mm), tc);
                        }
                    }
                }
                let name = self.sym_name(mid);
                let desc = self.describe_for_error(recv);
                let e = self.no_method_error(mid, recv, &format!("undefined method '{name}' for {desc}"));
                if let VmError::Raise(Value::Obj(o)) = e { let av = self.ary_new(args.to_vec()); let k = self.intern("@args"); self.heap.ivar_set(o, k, av); }
                Err(e)
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
        self.pending_vis_break = true;
        let r = self.call_proc_with(p, self_, args, None, Value::Nil, None, Some(tc));
        self.pending_vis_break = false;
        r
    }

    /// Pushes a frame for `proc_` and runs it to completion (re-entrant
    /// execution; native code waits for the result).
    /// Runs a method body proc with keywords (`mrb_exec_irep` for `Method#call`).
    pub fn call_method_proc(&mut self, proc_: ObjId, self_: Value, args: &[Value], kw: Option<Value>, blk: Value, mid: Option<Sym>, target_class: ObjId) -> VmResult<Value> {
        let tc = if self.heap.proc_data(proc_).env.is_some() { None } else { Some(target_class) };
        self.call_proc_with(proc_, self_, args, kw, blk, mid, tc)
    }
    /// `mrb_cv_set` from native code.
    pub fn cvar_store(&mut self, class: ObjId, s: Sym, v: Value) -> VmResult<()> { self.cvar_set(class, s, v) }

    pub fn call_proc(&mut self, proc_: ObjId, self_: Value, args: &[Value], blk: Value, mid: Option<Sym>, target_class: ObjId) -> VmResult<Value> {
        let tc = if self.heap.proc_data(proc_).env.is_some() { None } else { Some(target_class) };
        self.call_proc_with(proc_, self_, args, None, blk, mid, tc)
    }

    /// `override_tc = Some(c)` forces the target class (instance_eval); `None` takes it from the env.
    fn call_proc_with(&mut self, proc_: ObjId, self_: Value, args: &[Value], kw: Option<Value>, blk: Value, mid: Option<Sym>, override_tc: Option<ObjId>) -> VmResult<Value> {
        // mruby: MRB_CALL_LEVEL_MAX (512) frames; here also a cap on host-stack re-entry.
        if self.ci.len() >= CALL_LEVEL_MAX || self.native_depth >= NATIVE_DEPTH_MAX {
            return Err(self.raise(self.core.system_stack_error, "stack level too deep"));
        }
        self.native_depth += 1;
        let r = self.call_proc_inner(proc_, self_, args, kw, blk, mid, override_tc);
        self.native_depth -= 1;
        r
    }

    fn call_proc_inner(&mut self, proc_: ObjId, self_: Value, args: &[Value], kw: Option<Value>, blk: Value, mid: Option<Sym>, override_tc: Option<ObjId>) -> VmResult<Value> {
        let pd = self.heap.proc_data(proc_);
        let irep = pd.irep;
        let env = pd.env;
        let base = self.stack.len();
        let nregs = self.ireps[irep].nregs.max(args.len() + 3).max(4);
        self.stack.resize(base + nregs, Slot::NIL);
        self.stack[base] = Slot::from(self_);
        let mut n = args.len();
        let mut next;
        if n >= 15 {
            let packed = self.ary_new(args.to_vec());
            self.stack[base + 1] = Slot::from(packed);
            n = 15;
            next = base + 2;
        } else {
            for (i, v) in args.iter().enumerate() { self.stack[base + 1 + i] = Slot::from(*v); }
            next = base + 1 + args.len();
        }
        if let Some(k) = kw { self.stack[next] = Slot::from(k); next += 1; }
        self.stack[next] = Slot::from(blk);
        let depth = self.ci.len();
        let tc = match (override_tc, env) {
            (Some(tc), _) => tc,
            (None, Some(e)) => self.heap.env(e).target_class.unwrap_or(self.core.object),
            (None, None) => self.heap.proc_data(proc_).target_class.unwrap_or(self.core.object),
        };
        let vis_break = core::mem::take(&mut self.pending_vis_break);
        self.ci.push(CallInfo { base, pc: 0, irep, proc_, n: n as u8, kw: kw.is_some(), mid, target_class: tc, env: None, cci: Cci::Skip, vis: Vis::Public, modfunc: false, vis_break });
        let r = self.run_loop(depth);
        self.stack.truncate(base);
        r
    }


    // ------------------------------------------------------------------ fibers (mruby-fiber)

    /// Calls a native method on behalf of a SEND instruction. Returns the value
    /// and whether the native switched fibers; in that case the value was
    /// delivered to the register the new context waits on (or, when the fiber
    /// that yielded had been resumed by native code, the run loop is told to
    /// return it) and the caller must not write it to its own register.
    fn call_native_direct(&mut self, f: crate::object::NativeFn, recv: Value, args: &[Value], blk: Value, ret_reg: usize) -> VmResult<(Value, bool)> {
        let ctx0 = self.cur;
        let direct = core::mem::replace(&mut self.direct_send, true);
        let reg0 = core::mem::replace(&mut self.native_ret_reg, ret_reg);
        let r = self.call_native(f, recv, args, blk);
        self.native_ret_reg = reg0;
        self.direct_send = direct;
        self.orphan_block_of_native(blk);
        let v = r?;
        if self.cur == ctx0 { return Ok((v, false)); }
        if self.loop_exit.is_some() { return Ok((v, true)); }
        self.deliver(v);
        Ok((v, true))
    }

    /// Runs a native with the collector held off (its Rust locals are not roots).
    #[inline]
    pub fn call_native(&mut self, f: crate::object::NativeFn, recv: Value, args: &[Value], blk: Value) -> VmResult<Value> {
        self.native_active += 1;
        let r = f(self, recv, args, blk);
        self.native_active -= 1;
        r
    }

    /// A block made by the running frame and passed to a native that has now
    /// returned loses its home (mruby `cipop` of the C frame: MRB_PROC_ORPHAN),
    /// so `proc { break }.call` is a LocalJumpError.
    fn orphan_block_of_native(&mut self, blk: Value) {
        if let Value::Obj(b) = blk {
            if let ObjKind::Proc(pd) = &self.heap.get(b).kind {
                let caller_env = self.ci.last().and_then(|c| c.env);
                if !pd.strict && pd.env.is_some() && pd.env == caller_env {
                    if let ObjKind::Proc(pd) = &mut self.heap.get_mut(b).kind { pd.orphan = true; }
                }
            }
        }
    }

    /// `send`/`__send__` issued by a SEND: shifts the arguments down and
    /// dispatches the named method in the same frame (visibility ignored).
    fn op_send_redirect(&mut self, base: usize, a: usize, argc: usize, kw: bool, has_blk: bool, blk: Value) -> VmResult<()> {
        let (mut args, kd) = self.native_args(base + a, argc, kw);
        if kd.is_some() { args.pop(); }
        if args.is_empty() { return Err(self.argnum_error(0, "1+")); }
        let mid = match args[0] {
            Value::Sym(m) => m,
            v => match self.str_bytes(v) { Some(b) => { let n = String::from_utf8_lossy(b).into_owned(); self.intern(&n) } None => { let d = self.inspect_str(v)?; return Err(self.raise_type(&format!("{d} is not a symbol nor a string"))) } },
        };
        let rest: Vec<Value> = args[1..].to_vec();
        let (n, mut next) = if rest.len() >= 15 {
            let packed = self.ary_new(rest);
            if self.stack.len() < base + a + 2 { self.stack.resize(base + a + 2, Slot::NIL); }
            self.stack[base + a + 1] = Slot::from(packed);
            (15usize, base + a + 2)
        } else {
            if self.stack.len() < base + a + 1 + rest.len() { self.stack.resize(base + a + 1 + rest.len(), Slot::NIL); }
            for (i, v) in rest.iter().enumerate() { self.stack[base + a + 1 + i] = Slot::from(*v); }
            (rest.len(), base + a + 1 + rest.len())
        };
        let c = if let Some(k) = kd { if self.stack.len() <= next { self.stack.resize(next + 1, Slot::NIL); } self.stack[next] = Slot::from(k); next += 1; n | (15 << 4) } else { n };
        if self.stack.len() <= next { self.stack.resize(next + 1, Slot::NIL); }
        self.stack[next] = Slot::from(blk);
        self.op_send_vis(base, a, mid, c, has_blk, false, false)
    }

    /// Writes `v` into the register the current context is suspended in.
    fn deliver(&mut self, v: Value) {
        if let Some(reg) = self.contexts[self.cur].pending_reg.take() {
            if reg < self.stack.len() { self.stack[reg] = Slot::from(v); }
        }
    }

    /// Makes `to` the running context (`fiber_switch_context`).
    fn switch_context(&mut self, to: usize, kind: SwitchKind) {
        let from = self.cur;
        if from == to { return; }
        if self.trace.is_some() { self.record(TraceEvent::FiberSwitch { from, to, kind }); }
        core::mem::swap(&mut self.stack, &mut self.contexts[from].stack);
        core::mem::swap(&mut self.ci, &mut self.contexts[from].ci);
        core::mem::swap(&mut self.stack, &mut self.contexts[to].stack);
        core::mem::swap(&mut self.ci, &mut self.contexts[to].ci);
        self.contexts[to].status = FiberState::Running;
        self.cur = to;
    }

    /// Back to the root context with everything unwound (after an abort).
    pub fn reset_to_root(&mut self) {
        if self.cur != ROOT {
            self.ci.clear();
            self.stack.clear();
            self.contexts[self.cur].status = FiberState::Terminated;
            self.switch_context(ROOT, SwitchKind::Reset);
        }
        self.ci.clear();
        self.stack.clear();
        self.loop_exit = None;
    }

    /// Terminates the running fiber and switches to the context it returns to
    /// (`fiber_terminate`). Returns whether the fiber was running under a
    /// native resume, in which case the caller ends the nested run loop.
    fn fiber_terminate(&mut self) -> bool {
        let c = self.cur;
        let vmexec = core::mem::take(&mut self.contexts[c].vmexec);
        self.contexts[c].status = FiberState::Terminated;
        let prev = self.contexts[c].prev.take();
        self.switch_context(prev.unwrap_or(ROOT), SwitchKind::Terminate);
        self.contexts[c].stack = Vec::new();
        self.contexts[c].ci = Vec::new();
        vmexec
    }

    fn fiber_context(&mut self, fib: Value) -> VmResult<usize> {
        match fib.obj().map(|o| &self.heap.get(o).kind) {
            Some(ObjKind::Fiber(c)) if *c != usize::MAX => Ok(*c),
            Some(ObjKind::Fiber(_)) => Err(self.raise(self.core.fiber_error, "uninitialized Fiber")),
            _ => Err(self.raise_type("not a Fiber")),
        }
    }

    /// `Fiber#initialize`: gives the Fiber object a fresh context that will run `proc_`.
    pub fn fiber_init(&mut self, fib: Value, proc_: ObjId) -> VmResult<()> {
        let o = match fib { Value::Obj(o) => o, _ => return Err(self.raise_type("not a Fiber")) };
        if !matches!(self.heap.get(o).kind, ObjKind::Fiber(_)) { return Err(self.raise_type("not a Fiber")); }
        if let ObjKind::Fiber(c) = self.heap.get(o).kind { if c != usize::MAX { return Err(self.raise(self.core.runtime_error, "cannot initialize twice")); } }
        let mut ctx = Context::new(FiberState::Created);
        ctx.fib = Some(o);
        ctx.proc_ = Some(proc_);
        self.contexts.push(ctx);
        let id = self.contexts.len() - 1;
        if let ObjKind::Fiber(c) = &mut self.heap.get_mut(o).kind { *c = id; }
        Ok(())
    }

    /// `Fiber.current`: the Fiber object of the running context (made on first use for the root).
    pub fn fiber_current(&mut self) -> Value {
        if let Some(f) = self.contexts[self.cur].fib { return Value::Obj(f); }
        let f = self.heap.alloc(self.core.fiber, ObjKind::Fiber(self.cur));
        self.contexts[self.cur].fib = Some(f);
        Value::Obj(f)
    }

    pub fn fiber_state(&mut self, fib: Value) -> VmResult<FiberState> {
        let c = self.fiber_context(fib)?;
        Ok(self.contexts[c].status)
    }

    fn fiber_result(&mut self, args: &[Value]) -> Value {
        match args.len() { 0 => Value::Nil, 1 => args[0], _ => self.ary_new(args.to_vec()) }
    }

    /// `fiber_check_cfunc`: a context with a native frame on the host stack
    /// cannot be switched. The entry frame of a context (index 0: the fiber's
    /// block, or the top-level program of the root) does not count, like
    /// mruby's `cibase` in `task_across_c_boundary`.
    fn fiber_check_native(&self, ctx: usize) -> bool {
        let ci = if ctx == self.cur { &self.ci } else { &self.contexts[ctx].ci };
        ci.iter().skip(1).any(|c| c.cci == Cci::Skip)
    }

    /// `Fiber#resume` from native code (`mrb_fiber_resume`): runs the fiber in a
    /// nested loop until it yields or finishes, and returns that value.
    pub fn fiber_resume(&mut self, fib: Value, args: &[Value]) -> VmResult<Value> {
        self.fiber_switch(fib, args, true, true)
    }

    /// `Fiber#resume` / `Fiber#transfer` core (`fiber_switch`). With `vmexec` false the
    /// switch takes effect in the current run loop (the native returns and the
    /// loop continues in the new context); `resume` false is a transfer.
    pub fn fiber_switch(&mut self, fib: Value, args: &[Value], resume: bool, vmexec: bool) -> VmResult<Value> {
        let c = self.fiber_context(fib)?;
        let old = self.cur;
        if resume && c == old { return Err(self.raise(self.core.fiber_error, "attempt to resume the current fiber")); }
        let status = self.contexts[c].status;
        match status {
            FiberState::Transferred if resume => return Err(self.raise(self.core.fiber_error, "resuming transferred fiber")),
            FiberState::Running | FiberState::Resumed => return Err(self.raise(self.core.fiber_error, "double resume")),
            FiberState::Terminated => return Err(self.raise(self.core.fiber_error, "resuming dead fiber")),
            _ => {}
        }
        if self.fiber_check_native(c) { return Err(self.raise(self.core.fiber_error, "can't cross C function boundary")); }
        if resume {
            self.contexts[old].status = FiberState::Resumed;
            self.contexts[c].prev = Some(old);
        } else {
            self.contexts[old].status = FiberState::Transferred;
            self.contexts[c].prev = None;
        }
        if vmexec { self.contexts[c].vmexec = true; } else { self.contexts[old].pending_reg = Some(self.native_ret_reg); }
        self.switch_context(c, SwitchKind::Resume);
        let value;
        if status == FiberState::Created {
            let p = match self.contexts[c].proc_ { Some(p) => p, None => return Err(self.raise(self.core.fiber_error, "double resume (current)")) };
            let (irep, env, tc) = { let pd = self.heap.proc_data(p); (pd.irep, pd.env, pd.target_class) };
            let self_ = match env { Some(e) => self.env_get(e, 0), None => Value::Obj(self.top_self) };
            let nregs = self.ireps[irep].nregs.max(args.len() + 3).max(4);
            self.stack.clear();
            self.stack.resize(nregs, Slot::NIL);
            self.stack[0] = Slot::from(self_);
            let n = if args.len() >= 15 { let packed = self.ary_new(args.to_vec()); self.stack[1] = Slot::from(packed); 15 } else { for (i, v) in args.iter().enumerate() { self.stack[1 + i] = Slot::from(*v); } args.len() };
            let tc = tc.unwrap_or(self.core.object);
            self.ci.clear();
            self.ci.push(CallInfo { base: 0, pc: 0, irep, proc_: p, n: n as u8, kw: false, mid: None, target_class: tc, env: None, cci: Cci::None, vis: Vis::Public, modfunc: false, vis_break: false });
            value = self_;
        } else {
            value = self.fiber_result(args);
            if vmexec { self.deliver(value); }
        }
        if vmexec {
            let r = self.run_loop_ctx(c, 0);
            // the fiber yielded (loop_exit) or terminated: we are back in `old`
            debug_assert_eq!(self.cur, old);
            r
        } else {
            Ok(value)
        }
    }

    /// `Fiber.yield` (`mrb_fiber_yield`): switch back to the resumer. The value
    /// returned must be returned as-is by the native that called this.
    pub fn fiber_yield(&mut self, args: &[Value]) -> VmResult<Value> {
        let c = self.cur;
        let prev = match self.contexts[c].prev { Some(p) => p, None => return Err(self.raise(self.core.fiber_error, "attempt to yield on a not resumed fiber")) };
        if c == ROOT { return Err(self.raise(self.core.fiber_error, "can't yield from root fiber")); }
        if self.contexts[prev].status == FiberState::Transferred { return Err(self.raise(self.core.fiber_error, "attempt to yield on a not resumed fiber")); }
        if !self.direct_send || self.fiber_check_native(c) { return Err(self.raise(self.core.fiber_error, "can't cross C function boundary")); }
        let value = self.fiber_result(args);
        self.contexts[c].status = FiberState::Suspended;
        self.contexts[c].pending_reg = Some(self.native_ret_reg);
        self.contexts[c].prev = None;
        let vmexec = core::mem::take(&mut self.contexts[c].vmexec);
        self.switch_context(prev, SwitchKind::Yield);
        if vmexec { self.loop_exit = Some(value); }
        Ok(value)
    }

    /// `Fiber#transfer`.
    pub fn fiber_transfer(&mut self, fib: Value, args: &[Value]) -> VmResult<Value> {
        let c = self.fiber_context(fib)?;
        // fiber_check_cfunc_recursive: no native frame anywhere on the chain of resumers
        if !self.direct_send { return Err(self.raise(self.core.fiber_error, "can't cross C function boundary")); }
        let mut x = Some(self.cur);
        while let Some(i) = x {
            if self.fiber_check_native(i) || self.contexts[i].vmexec { return Err(self.raise(self.core.fiber_error, "can't cross C function boundary")); }
            if i == ROOT { break; }
            x = self.contexts[i].prev;
        }
        if self.contexts[c].status == FiberState::Resumed { return Err(self.raise(self.core.fiber_error, "attempt to transfer to a resuming fiber")); }
        if c == ROOT {
            let value = self.fiber_result(args);
            let cur = self.cur;
            if cur == ROOT { return Ok(value); }
            self.contexts[cur].status = FiberState::Transferred;
            self.contexts[cur].pending_reg = Some(self.native_ret_reg);
            self.switch_context(ROOT, SwitchKind::Reset);
            return Ok(value);
        }
        if c == self.cur { return Ok(self.fiber_result(args)); }
        self.fiber_switch(fib, args, false, false)
    }

    /// Source line of the instruction the innermost frame is at (`None` without debug info).
    pub fn current_line(&self) -> Option<u32> {
        let ci = self.ci.last()?;
        self.ireps[ci.irep].line_of(ci.pc)
    }

    // ------------------------------------------------------------------ garbage collection

    /// Keeps `id` alive until [`Vm::gc_unregister`] (`mrb_gc_register`): for objects a
    /// host holds across calls into the VM, which the collector cannot otherwise see.
    pub fn gc_register(&mut self, id: ObjId) {
        self.gc_registered.push(id);
    }
    /// Drops one registration made by [`Vm::gc_register`].
    pub fn gc_unregister(&mut self, id: ObjId) {
        if let Some(i) = self.gc_registered.iter().rposition(|x| *x == id) { self.gc_registered.swap_remove(i); }
    }
    /// Stress mode (mruby `MRB_GC_STRESS`): every allocation makes a collection due.
    pub fn set_gc_stress(&mut self, on: bool) {
        self.gc_stress = on;
        self.heap.alloc_threshold = if on { 1 } else { GC_MIN_INTERVAL.max(self.heap.allocated_since_gc + 1) };
    }

    /// Records that the innermost frame is being removed while unwinding.
    #[inline]
    fn unwound(&mut self, by: UnwindBy) {
        if self.trace.is_some() {
            let i = self.ci.len() - 1;
            let mid = self.ci[i].mid;
            self.record(TraceEvent::FrameUnwound { frame: i, mid, by });
        }
    }

    /// Appends an event while `set_trace(true)` (the caller checks `trace.is_some()` first,
    /// so nothing is built when recording is off).
    #[inline]
    fn record(&mut self, e: TraceEvent) {
        if let Some(t) = &mut self.trace { t.push(e); }
    }

    /// The due collection, at an instruction boundary. Postponed (it stays due)
    /// while a native is on the host stack or `GC.disable` is in effect.
    #[cold]
    #[inline(never)]
    fn gc_maybe(&mut self) {
        if self.native_active == 0 && !self.gc_disabled { self.gc_collect(); }
    }

    /// `GC.start`. Called from a SEND, the native `GC.start` is the only one on the
    /// host stack and every register is in the Vm, so it collects at once; under
    /// another native (`funcall`) it only makes the collection due.
    pub fn gc_start(&mut self) {
        if self.gc_disabled { return; }
        if self.native_active <= 1 { self.gc_collect(); } else { self.heap.gc_pending = true; }
    }

    /// Mark & sweep (stop the world). The caller guarantees that no Rust frame
    /// holds a value that is not reachable from the roots.
    pub fn gc_collect(&mut self) {
        let t0 = self.gc_clock.map(|c| c());
        let mut work: Vec<ObjId> = Vec::new();
        let mut ctxs: Vec<usize> = Vec::new();
        let mut windows: Vec<(usize, usize, usize)> = Vec::new();
        let mut ctx_marked = vec![false; self.contexts.len()];
        self.gc_mark_roots(&mut work, &mut ctxs);
        loop {
            self.heap.mark_drain(&mut work, &mut ctxs, &mut windows);
            if let Some(c) = ctxs.pop() {
                if !ctx_marked[c] {
                    ctx_marked[c] = true;
                    self.gc_mark_context(c, &mut work);
                }
            } else if let Some((c, base, len)) = windows.pop() {
                if !ctx_marked[c] {
                    let st = if c == self.cur { &self.stack } else { &self.contexts[c].stack };
                    let end = (base + len).min(st.len());
                    if base < end { self.heap.mark_slots(&st[base..end], &mut work); }
                }
            } else {
                break;
            }
        }
        // A context nothing reached (its Fiber object is garbage) can never run
        // again. The environments of its frames that are still reachable (a
        // block captured there) take their values off the stack first (mruby
        // `mrb_env_detach_all` in the sweep), then the stack and frames go.
        // The index is not reused.
        let mut detached: Vec<(ObjId, usize)> = Vec::new();
        for c in 0..self.contexts.len() {
            if ctx_marked[c] { continue; }
            let ctx = &self.contexts[c];
            if ctx.status == FiberState::Terminated && ctx.stack.is_empty() && ctx.ci.is_empty() && ctx.fib.is_none() && ctx.proc_.is_none() { continue; }
            for f in &ctx.ci {
                let Some(e) = f.env else { continue };
                if !self.heap.is_marked(e) { continue; }
                let (attached, base, len) = { let ed = self.heap.env(e); (ed.attached, ed.base, ed.len) };
                if !attached { continue; }
                let end = (base + len).min(ctx.stack.len());
                let vals = if base < end { ctx.stack[base..end].to_vec() } else { Vec::new() };
                let ed = self.heap.env_mut(e);
                ed.values = vals;
                ed.attached = false;
                detached.push((e, len));
            }
            self.contexts[c] = Context::new(FiberState::Terminated);
        }
        if self.trace.is_some() {
            for (env, len) in detached { self.record(TraceEvent::EnvDetach { env, len, reason: DetachReason::ContextSwept }); }
        }
        let (before_live, allocated_since) = (self.heap.live_count(), self.heap.allocated_since_gc);
        let swept = self.heap.sweep();
        let live = self.heap.live_count();
        if self.trace.is_some() { self.record(TraceEvent::GcCollect { before_live, after_live: live, swept, allocated_since }); }
        self.live_after_gc = live;
        self.heap.allocated_since_gc = 0;
        self.heap.malloc_increase = 0;
        self.heap.gc_pending = false;
        // `interval_ratio` 200 (the default) lets the heap double: `live` more allocations
        let ratio = self.gc_interval_ratio.max(100) as usize;
        self.heap.alloc_threshold = if self.gc_stress { 1 } else { (live * (ratio - 100) / 100).max(GC_MIN_INTERVAL) };
        self.gc_count += 1;
        if let (Some(t0), Some(c)) = (t0, self.gc_clock) { self.gc_time_ns += c().saturating_sub(t0); }
    }

    /// The root set (see `docs/gc.md`). Contexts to scan go to `ctxs`.
    fn gc_mark_roots(&mut self, work: &mut Vec<ObjId>, ctxs: &mut Vec<usize>) {
        let h = &mut self.heap;
        // the running context: its stack and frames are the Vm's
        ctxs.push(self.cur);
        // the root context, and the chain of contexts waiting for a resumed fiber
        ctxs.push(ROOT);
        for (i, c) in self.contexts.iter().enumerate() {
            if matches!(c.status, FiberState::Running | FiberState::Resumed) || c.vmexec { ctxs.push(i); }
        }
        for s in self.globals.values() { h.mark_value(s.get(), work); }
        for v in [self.exc, self.loop_exit, self.pending_kw].into_iter().flatten() { h.mark_value(v, work); }
        for id in self.core.ids() { h.mark_id(id, work); }
        h.mark_id(self.top_self, work);
        h.mark_id(self.call_proc, work);
        for id in &self.inspect_guard { h.mark_id(*id, work); }
        for (x, y) in &self.eq_guard { h.mark_id(*x, work); h.mark_id(*y, work); }
        for id in &self.gc_registered { h.mark_id(*id, work); }
    }

    /// Marks what a context holds: registers, frames, its Fiber and block.
    fn gc_mark_context(&mut self, c: usize, work: &mut Vec<ObjId>) {
        fn mark_ci(h: &mut Heap, ci: &[CallInfo], work: &mut Vec<ObjId>) {
            for f in ci {
                h.mark_id(f.proc_, work);
                h.mark_id(f.target_class, work);
                if let Some(e) = f.env { h.mark_id(e, work); }
            }
        }
        let h = &mut self.heap;
        if c == self.cur {
            h.mark_slots(&self.stack, work);
            mark_ci(h, &self.ci, work);
        }
        let ctx = &self.contexts[c];
        h.mark_slots(&ctx.stack, work);
        mark_ci(h, &ctx.ci, work);
        for id in [ctx.fib, ctx.proc_].into_iter().flatten() { h.mark_id(id, work); }
    }

    // ------------------------------------------------------------------ environments

    /// The register stack of a context: the running one is in `self.stack`.
    fn stack_of(&self, ctx: usize) -> &Vec<Slot> {
        if ctx == self.cur { &self.stack } else { &self.contexts[ctx].stack }
    }
    fn stack_of_mut(&mut self, ctx: usize) -> &mut Vec<Slot> {
        if ctx == self.cur { &mut self.stack } else { &mut self.contexts[ctx].stack }
    }
    fn env_get(&self, env: ObjId, idx: usize) -> Value {
        let e = self.heap.env(env);
        if e.attached { self.stack_of(e.ctx).get(e.base + idx).map(|s| s.get()).unwrap_or(Value::Nil) } else { e.values.get(idx).map(|s| s.get()).unwrap_or(Value::Nil) }
    }
    fn env_set(&mut self, env: ObjId, idx: usize, v: Value) {
        let (attached, base, ctx) = { let e = self.heap.env(env); (e.attached, e.base, e.ctx) };
        if attached {
            let st = self.stack_of_mut(ctx);
            if base + idx < st.len() { st[base + idx] = Slot::from(v); }
        } else {
            let e = self.heap.env_mut(env);
            if idx < e.values.len() { e.values[idx] = Slot::from(v); }
        }
    }
    /// Reads a slot of an environment whether it is still on the stack or detached.
    pub fn env_value(&self, env: ObjId, idx: usize) -> Value { self.env_get(env, idx) }
    pub fn cvar_class_of(&self, proc_: ObjId) -> ObjId { self.cvar_class(proc_) }
    pub fn cvar_lookup(&self, class: ObjId, s: Sym) -> Option<Value> { self.cvar_get(class, s) }
    /// Constant lookup in a frame's lexical scope without raising.
    pub fn const_lookup_noraise(&self, ci: &CallInfo, s: Sym) -> Option<Value> {
        if let Some(v) = self.const_get(ci.target_class, s) { return Some(v); }
        let mut p = Some(ci.proc_);
        while let Some(pid) = p {
            let pd = self.heap.proc_data(pid);
            if let Some(tc) = pd.target_class { if let Some(v) = self.const_get(tc, s) { return Some(v); } }
            p = pd.upper;
        }
        self.const_get(self.core.object, s)
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
        let bidx = Self::frame_bidx(ci);
        let e = self.heap.alloc(self.core.object, ObjKind::Env(EnvData {
            ctx: self.cur, base: ci.base, len, bidx, attached: true, values: Vec::new(), mid: ci.mid, target_class: Some(ci.target_class),
            vis: ci.vis, modfunc: ci.modfunc, vis_break: ci.vis_break,
        }));
        self.ci[i].env = Some(e);
        if self.trace.is_some() {
            let (ctx, base, mid) = (self.cur, self.ci[i].base, self.ci[i].mid);
            self.record(TraceEvent::EnvCreate { env: e, ctx, frame: i, base, len, mid });
        }
        e
    }
    /// Pops the current frame, detaching its environment (`cipop`).
    fn pop_frame(&mut self) -> CallInfo {
        let ci = self.ci.pop().expect("pop on empty callinfo");
        // A block created by the caller and passed to this frame loses its home
        // when the frame returns (mruby `cipop`: MRB_PROC_ORPHAN).
        let bidx = ci.base + Self::frame_bidx(&ci);
        if let Some(Value::Obj(b)) = self.stack.get(bidx).map(|s| s.get()) {
            if let ObjKind::Proc(pd) = &self.heap.get(b).kind {
                let caller_env = self.ci.last().and_then(|c| c.env);
                if !pd.strict && pd.env.is_some() && pd.env == caller_env {
                    if let ObjKind::Proc(pd) = &mut self.heap.get_mut(b).kind { pd.orphan = true; }
                }
            }
        }
        if let Some(e) = ci.env {
            let (base, len) = { let ed = self.heap.env(e); (ed.base, ed.len) };
            let end = (base + len).min(self.stack.len());
            let vals = self.stack[base..end].to_vec();
            let ed = self.heap.env_mut(e);
            ed.values = vals;
            ed.attached = false;
            if self.trace.is_some() { self.record(TraceEvent::EnvDetach { env: e, len, reason: DetachReason::FrameReturn }); }
        }
        // Orphan blocks whose env belonged to this frame? (`MRB_PROC_ORPHAN`) — not tracked yet.
        ci
    }

    // ------------------------------------------------------------------ the loop

    /// Runs until the frame at index `stop_depth` of the current context returns, and returns its value.
    fn run_loop(&mut self, stop_depth: usize) -> VmResult<Value> {
        self.run_loop_ctx(self.cur, stop_depth)
    }

    /// Runs until frame `stop_depth` of context `lc` returns. Frames of other
    /// contexts the loop is switched into (non-native fiber resume/yield) never
    /// end the loop; only a fiber's base frame terminating does, and then the
    /// loop carries on in the previous context.
    fn run_loop_ctx(&mut self, lc: usize, stop_depth: usize) -> VmResult<Value> {
        let mut pending: Option<VmResult<Value>> = None;
        loop {
            let r = match pending.take() { Some(r) => r, None => self.exec_frames(stop_depth, lc) };
            match r {
                Ok(v) => return Ok(v),
                Err(VmError::Raise(exc)) => {
                    if let Value::Obj(o) = exc { if matches!(self.heap.get(o).kind, ObjKind::Exception) { let k = self.intern("@__raised"); self.heap.ivar_set(o, k, Value::True); } }
                    // Unwind: look for a catch handler in frames >= stop_depth.
                    if self.handle_raise(exc, stop_depth, lc) {
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
                    if self.cur == lc && self.ci.len() <= stop_depth { return Err(VmError::Break(brk)); }
                    match self.resume_break(brk, stop_depth, lc) {
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
        if self.stack.len() < base + nregs { self.stack.resize(base + nregs, Slot::NIL); }
        self.ci[top].pc = h.target as usize;
    }

    /// Continues a pending non-local exit (`L_BREAK` dispatch by tag).
    fn resume_break(&mut self, brk: ObjId, stop_depth: usize, lc: usize) -> VmResult<Option<Value>> {
        let (tag, idx, value) = match self.heap.get(brk).kind { ObjKind::Break { tag, ci_index, value } => (tag, ci_index, value), _ => return Err(VmError::Internal("not a break object".into())) };
        self.exc = Some(Value::Obj(brk));
        match tag {
            BreakTag::Break => self.unwind_return(idx, value, stop_depth, lc, UnwindBy::Return),
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
    fn unwind_return(&mut self, return_idx: usize, v: Value, stop_depth: usize, lc: usize, by: UnwindBy) -> VmResult<Option<Value>> {
        loop {
            let top = self.ci.len() - 1;
            let (irep, pc) = { let ci = &self.ci[top]; (ci.irep, ci.pc) };
            if let Some(h) = self.catch_find(irep, pc, true) {
                let brk = self.break_new(BreakTag::Break, return_idx, v);
                self.enter_ensure(h, brk);
                return Ok(None);
            }
            if top == return_idx { break; }
            self.unwound(by);
            let popped = self.pop_frame();
            if popped.cci == Cci::Skip || (self.cur == lc && top <= stop_depth) {
                // crossing a native frame: let the native caller propagate it
                let brk = self.break_new(BreakTag::Break, return_idx, v);
                self.exc = None;
                return Err(VmError::Break(brk));
            }
        }
        self.exc = None;
        let popped = self.pop_frame();
        if self.ci.is_empty() && self.cur != ROOT {
            // the fiber's block returned (mruby: `ci == cibase` → fiber_terminate)
            let vmexec = self.fiber_terminate();
            if vmexec { return Ok(Some(v)); }
            self.deliver(v);
            return Ok(None);
        }
        if popped.cci == Cci::Skip || (self.cur == lc && return_idx <= stop_depth) {
            return Ok(Some(v));
        }
        // the callee's R0 is the caller's R[a]
        self.stack[popped.base] = Slot::from(v);
        Ok(None)
    }

    /// Finds a rescue/ensure handler for `exc`; on success sets pc and `exc`
    /// and returns true. Otherwise pops frames down to `stop_depth` and returns false.
    fn handle_raise(&mut self, exc: Value, stop_depth: usize, lc: usize) -> bool {
        if self.trace.is_some() {
            let i = self.ci.len() - 1;
            let (irep, pc) = { let ci = &self.ci[i]; (ci.irep, ci.pc) };
            let class = self.class_name(self.real_class_of(exc));
            let line = self.ireps[irep].line_of(pc);
            self.record(TraceEvent::Raise { exc: exc.obj(), class, frame: i, irep, pc, line });
        }
        loop {
            let i = self.ci.len() - 1;
            let (irep, pc) = { let ci = &self.ci[i]; (ci.irep, ci.pc) };
            if self.trace.is_some() {
                let matched = self.catch_find(irep, pc, false).map(|h| CatchHandlerInfo {
                    ensure: h.kind == CatchType::Ensure, begin: h.begin, end: h.end, target: h.target });
                let line = self.ireps[irep].line_of(pc);
                self.record(TraceEvent::CatchLook { frame: i, irep, pc, line, matched });
            }
            if let Some(h) = self.catch_find(irep, pc, false) {
                self.ci[i].pc = h.target as usize;
                let nregs = self.ireps[irep].nregs;
                let base = self.ci[i].base;
                if self.stack.len() < base + nregs { self.stack.resize(base + nregs, Slot::NIL); }
                self.exc = Some(exc);
                return true;
            }
            if i == 0 && self.cur != ROOT {
                // uncaught in a fiber: it terminates and the exception continues in
                // the context that resumed it (mruby `L_FTOP`); a fiber resumed by
                // native code hands it to that native instead
                self.pop_frame();
                let vmexec = self.fiber_terminate();
                if vmexec { return false; }
                continue;
            }
            if self.cur == lc && i <= stop_depth {
                self.unwound(UnwindBy::Raise);
                self.pop_frame();
                return false;
            }
            self.unwound(UnwindBy::Raise);
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

    fn exec_frames(&mut self, stop_depth: usize, lc: usize) -> VmResult<Value> {
        let mut ext: u8 = 0;
        loop {
            if let Some(left) = self.step_left {
                // Suspend only in the outermost loop: a nested loop (native code
                // waiting for a block) must run to completion, so the pause lands
                // at the next instruction boundary of the top-level program.
                if left == 0 && stop_depth == 0 && lc == ROOT {
                    return Ok(Value::Nil);
                }
                self.step_left = Some(left.saturating_sub(1));
            }
            // the only place the collector runs: every register and frame is in the Vm
            if self.heap.gc_pending { self.gc_maybe(); }
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
            macro_rules! reg { ($i:expr) => { self.stack[base + $i].get() } }
            macro_rules! setreg { ($i:expr, $v:expr) => { { let v = $v; self.stack[base + $i] = Slot::from(v); } } }
            match op {
                Op::Nop => {}
                Op::Move => { setreg!(a, reg!(b)); }
                Op::Loadl => {
                    let v = match &self.ireps[ci.irep].pool[b] {
                        Pool::Int(i) => Value::Int(*i),
                        Pool::Float(f) => Value::Float(*f),
                        Pool::Str(s) => { let s = s.clone(); self.str_new(&s) }
                        Pool::BigInt(_) => return Err(VmError::Unimplemented("bigint literal".into())),
                    };
                    setreg!(a, v);
                }
                Op::Loadi8 => { setreg!(a, Value::Int(b as i64)); }
                Op::Loadineg => { setreg!(a, Value::Int(-(b as i64))); }
                Op::LoadiM1 => { setreg!(a, Value::Int(-1)); }
                Op::Loadi0 => { setreg!(a, Value::Int(0)); }
                Op::Loadi1 => { setreg!(a, Value::Int(1)); }
                Op::Loadi2 => { setreg!(a, Value::Int(2)); }
                Op::Loadi3 => { setreg!(a, Value::Int(3)); }
                Op::Loadi4 => { setreg!(a, Value::Int(4)); }
                Op::Loadi5 => { setreg!(a, Value::Int(5)); }
                Op::Loadi6 => { setreg!(a, Value::Int(6)); }
                Op::Loadi7 => { setreg!(a, Value::Int(7)); }
                Op::Loadi16 => { setreg!(a, Value::Int(b as u16 as i16 as i64)); }
                Op::Loadi32 => { setreg!(a, Value::Int((((b as u32) << 16) | c as u32) as i32 as i64)); }
                Op::Loadsym => { setreg!(a, Value::Sym(self.ireps[ci.irep].syms[b])); }
                Op::Loadnil => { setreg!(a, Value::Nil); }
                Op::Loadself => { setreg!(a, reg!(0)); }
                Op::Loadtrue => { setreg!(a, Value::True); }
                Op::Loadfalse => { setreg!(a, Value::False); }
                Op::Getsv | Op::Setsv => { return Err(VmError::Unimplemented("special variables ($~, $_)".into())); }
                Op::Getgv => { let s = self.ireps[ci.irep].syms[b]; setreg!(a, self.globals.get(&s).map(|s| s.get()).unwrap_or(Value::Nil)); }
                Op::Setgv => { let s = self.ireps[ci.irep].syms[b]; let v = reg!(a); self.globals.insert(s, Slot::from(v)); }
                Op::Getiv => {
                    let s = self.ireps[ci.irep].syms[b];
                    let v = match reg!(0) { Value::Obj(o) => self.heap.ivar_get(o, s), _ => Value::Nil };
                    setreg!(a, v);
                }
                Op::Setiv => {
                    let s = self.ireps[ci.irep].syms[b];
                    let v = reg!(a);
                    match reg!(0) {
                        Value::Obj(o) => { if self.heap.get(o).frozen { let r = reg!(0); return Err(self.frozen_error(r)); } self.heap.ivar_set(o, s, v) }
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
                    setreg!(a, v);
                }
                Op::Setcv => {
                    let s = self.ireps[ci.irep].syms[b];
                    let v = reg!(a);
                    let cls = self.cvar_class(ci.proc_);
                    self.cvar_set(cls, s, v)?;
                }
                Op::Getconst => {
                    let s = self.ireps[ci.irep].syms[b];
                    let v = self.const_lookup(&ci, s)?;
                    setreg!(a, v);
                }
                Op::Setconst => {
                    let s = self.ireps[ci.irep].syms[b];
                    let v = reg!(a);
                    let tc = ci.target_class;
                    if self.heap.is_class(tc) {
                        if self.heap.get(tc).frozen { return Err(self.frozen_error(Value::Obj(tc))); }
                        self.heap.class_mut(tc).consts.insert(s, Slot::from(v));
                        // name anonymous classes on first assignment
                        if let Value::Obj(o) = v {
                            if self.heap.is_class(o) && self.heap.class(o).name.is_none() {
                                self.heap.class_mut(o).name = Some(s);
                                self.heap.class_mut(o).outer = Some(tc);
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
                    setreg!(a, v);
                }
                Op::Setmcnst => {
                    let s = self.ireps[ci.irep].syms[b];
                    let v = reg!(a);
                    match reg!(a + 1) {
                        Value::Obj(o) if self.heap.is_class(o) => {
                            self.heap.class_mut(o).consts.insert(s, Slot::from(v));
                            if let Value::Obj(c) = v { if self.heap.is_class(c) && self.heap.class(c).name.is_none() { self.heap.class_mut(c).name = Some(s); self.heap.class_mut(c).outer = Some(o); } }
                        }
                        _ => return Err(self.raise_type("not a class/module")),
                    }
                }
                Op::Getupvar => {
                    let v = match self.uvenv(c) { Some(e) if b < self.heap.env(e).len => self.env_get(e, b), _ => Value::Nil };
                    setreg!(a, v);
                }
                Op::Setupvar => {
                    if let Some(e) = self.uvenv(c) {
                        if b < self.heap.env(e).len { let v = reg!(a); self.env_set(e, b, v); }
                    }
                }
                Op::Getidx => {
                    let recv = reg!(a); let idx = reg!(a + 1);
                    let v = self.funcall(recv, self.s.aref, &[idx], Value::Nil)?;
                    setreg!(a, v);
                }
                Op::Getidx0 => {
                    let recv = reg!(b);
                    let v = self.funcall(recv, self.s.aref, &[Value::Int(0)], Value::Nil)?;
                    setreg!(a, v);
                }
                Op::Setidx => {
                    let recv = reg!(a); let idx = reg!(a + 1); let val = reg!(a + 2);
                    self.funcall(recv, self.s.aset, &[idx, val], Value::Nil)?;
                    setreg!(a, val); // the value of an index assignment is the assigned value
                }
                Op::Jmp => { self.ci[top].pc = jump(pc, a); }
                Op::Jmpif => { if reg!(a).truthy() { self.ci[top].pc = jump(pc, b); } }
                Op::Jmpnot => { if !reg!(a).truthy() { self.ci[top].pc = jump(pc, b); } }
                Op::Jmpnil => { if reg!(a).is_nil() { self.ci[top].pc = jump(pc, b); } }
                Op::Jmpuw => { self.jmpuw(jump(pc, a)); }
                Op::Except => { setreg!(a, self.exc.take().unwrap_or(Value::Nil)); }
                Op::Rescue => {
                    let exc = reg!(a); let cls_v = reg!(b);
                    let cls = match cls_v { Value::Obj(o) if self.heap.is_class(o) => o, _ => return Err(self.raise_type("class or module required for rescue clause")) };
                    setreg!(b, Value::bool(self.obj_is_kind_of(exc, cls)));
                }
                Op::Raiseif => {
                    let exc = reg!(a);
                    match exc {
                        Value::Nil => { self.exc = None; }
                        Value::Obj(o) if matches!(self.heap.get(o).kind, ObjKind::Break { .. }) => {
                            if let Some(r) = self.resume_break(o, stop_depth, lc)? { return Ok(r); }
                        }
                        _ => return Err(VmError::Raise(exc)),
                    }
                }
                Op::Matcherr => { return Err(self.raise(self.core.no_matching_pattern_error, "pattern not matched")); }
                Op::Ssend | Op::Ssend0 | Op::Ssendb | Op::Send | Op::Send0 | Op::Sendb => {
                    let mid = self.ireps[ci.irep].syms[b];
                    let argc = if matches!(op, Op::Send0 | Op::Ssend0) { 0 } else { c };
                    let has_blk = matches!(op, Op::Sendb | Op::Ssendb);
                    let explicit = matches!(op, Op::Send | Op::Send0 | Op::Sendb);
                    if !explicit { setreg!(a, reg!(0)); }
                    self.op_send_vis(base, a, mid, argc, has_blk, false, explicit)?;
                    if let Some(v) = self.loop_exit.take() { return Ok(v); }
                }
                Op::Super => {
                    let argc = b;
                    setreg!(a, reg!(0));
                    let mid = ci.mid.ok_or_else(|| self.raise(self.core.no_method_error, "super called outside of method"))?;
                    self.op_send(base, a, mid, argc, true, true)?;
                    if let Some(v) = self.loop_exit.take() { return Ok(v); }
                }
                Op::Call => {
                    // `Proc#call`: replace this frame (pushed by SEND) with the proc's body.
                    let p = match reg!(0) { Value::Obj(o) if matches!(self.heap.get(o).kind, ObjKind::Proc(_)) => o, _ => return Err(self.raise_type("wrong type (expected Proc)")) };
                    let n = ci.n as usize;
                    let nargs = (if n == 15 { 1 } else { n }) + (if ci.kw { 1 } else { 0 }) + 2;
                    self.vm_call_proc(p, nargs);
                }
                Op::Blkcall => {
                    // Direct block call: R[a] = R[a].call(R[a+1..a+b])
                    let p = match reg!(a) { Value::Obj(o) if matches!(self.heap.get(o).kind, ObjKind::Proc(_)) => o, _ => return Err(self.raise_type("wrong type (expected Proc)")) };
                    let nbase = base + a;
                    let (n, kw, _) = self.prepare_call(nbase, b, false)?;
                    let npos = if n == 15 { 1 } else { n };
                    self.ci.push(CallInfo { base: nbase, pc: 0, irep: 0, proc_: p, n: n as u8, kw, mid: None, target_class: ci.target_class, env: None, cci: Cci::None, vis: Vis::Public, modfunc: false, vis_break: false });
                    self.vm_call_proc(p, npos + (if kw { 1 } else { 0 }) + 2);
                }
                Op::Argary => { self.op_argary(base, a, b)?; }
                Op::Enter => { self.op_enter(a as u32)?; }
                Op::Karg => {
                    let k = Value::Sym(self.ireps[ci.irep].syms[b]);
                    let v = match self.kidx(&ci).and_then(|ki| self.hash_delete(self.stack[ki].get(), k)) {
                        Some(v) => v,
                        None => { let n = self.sym_name(self.ireps[ci.irep].syms[b]); return Err(self.raise_arg(&format!("missing keyword: {n}"))); }
                    };
                    setreg!(a, v);
                }
                Op::KeyP => {
                    let k = Value::Sym(self.ireps[ci.irep].syms[b]);
                    let has = match self.kidx(&ci) { Some(ki) => self.hash_get(self.stack[ki].get(), k).is_some(), None => false };
                    setreg!(a, Value::bool(has));
                }
                Op::Keyend => {
                    if let Some(ki) = self.kidx(&ci) {
                        let first = match self.stack[ki].get().obj().map(|o| &self.heap.get(o).kind) { Some(ObjKind::Hash(hd)) => hd.entries.first().map(|e| e.0.get()), _ => None };
                        if let Some(k) = first { let d = match k { Value::Sym(s) => self.sym_name(s), v => self.inspect_str(v)? }; return Err(self.raise_arg(&format!("unknown keyword: {d}"))); }
                    }
                }
                Op::Return => { let v = reg!(a); if let Some(r) = self.op_return(v, stop_depth, lc)? { return Ok(r); } }
                Op::ReturnBlk => {
                    let v = reg!(a);
                    if let Some(r) = self.op_return_blk(v, stop_depth, lc)? { return Ok(r); }
                }
                Op::Retself => { let v = reg!(0); if let Some(r) = self.op_return(v, stop_depth, lc)? { return Ok(r); } }
                Op::Retnil => { if let Some(r) = self.op_return(Value::Nil, stop_depth, lc)? { return Ok(r); } }
                Op::Rettrue => { if let Some(r) = self.op_return(Value::True, stop_depth, lc)? { return Ok(r); } }
                Op::Retfalse => { if let Some(r) = self.op_return(Value::False, stop_depth, lc)? { return Ok(r); } }
                Op::Break => {
                    let v = reg!(a);
                    if let Some(r) = self.op_break(v, stop_depth, lc)? { return Ok(r); }
                }
                Op::Blkpush => { let v = self.op_blkpush(base, b)?; setreg!(a, v); }
                Op::Add => { self.op_arith(base, a, self.s.plus)?; }
                Op::Sub => { self.op_arith(base, a, self.s.minus)?; }
                Op::Mul => { self.op_arith(base, a, self.s.mul)?; }
                Op::Div => { self.op_arith(base, a, self.s.div)?; }
                Op::Addi => { setreg!(a + 1, Value::Int(b as i64)); self.op_arith(base, a, self.s.plus)?; }
                Op::Subi => { setreg!(a + 1, Value::Int(b as i64)); self.op_arith(base, a, self.s.minus)?; }
                Op::Addilv | Op::Subilv => {
                    let mid = if matches!(op, Op::Addilv) { self.s.plus } else { self.s.minus };
                    match reg!(a) {
                        Value::Int(_) | Value::Float(_) => { setreg!(b, reg!(a)); setreg!(b + 1, Value::Int(c as i64)); self.op_arith(base, b, mid)?; let v = reg!(b); setreg!(a, v); }
                        recv => { let r = self.funcall(recv, mid, &[Value::Int(c as i64)], Value::Nil)?; setreg!(a, r); }
                    }
                }
                Op::Eq => { self.op_compare(base, a, self.s.eq)?; }
                Op::Lt => { self.op_compare(base, a, self.s.lt)?; }
                Op::Le => { self.op_compare(base, a, self.s.le)?; }
                Op::Gt => { self.op_compare(base, a, self.s.gt)?; }
                Op::Ge => { self.op_compare(base, a, self.s.ge)?; }
                Op::Array => { let v: Vec<Value> = values_of(&self.stack[base + a..base + a + b]); setreg!(a, self.ary_new(v)); }
                Op::Array2 => { let v: Vec<Value> = values_of(&self.stack[base + b..base + b + c]); setreg!(a, self.ary_new(v)); }
                Op::Arycat => {
                    // R[a] == nil means "start the argument accumulator": a fresh
                    // array (independent of R[a+1]) that later ARYPUSH/ARYCAT extend.
                    let (dst, src) = (reg!(a), reg!(a + 1));
                    let items = match src { Value::Nil => vec![], _ => self.to_array(src)? };
                    match dst {
                        Value::Nil => { setreg!(a, self.ary_new(items)); }
                        Value::Obj(o) if matches!(self.heap.get(o).kind, ObjKind::Array(_)) => { if let ObjKind::Array(v) = &mut self.heap.get_mut(o).kind { v.extend(slots_of(&items)); } }
                        _ => return Err(self.raise_type("not an array")),
                    }
                }
                Op::Arypush => {
                    let dst = reg!(a);
                    let items: Vec<Value> = values_of(&self.stack[base + a + 1..base + a + 1 + b]);
                    match dst { Value::Obj(o) => { if let ObjKind::Array(v) = &mut self.heap.get_mut(o).kind { v.extend(slots_of(&items)); } } _ => return Err(self.raise_type("not an array")) }
                }
                Op::Arysplat => { let v = reg!(a); let items = self.to_array(v)?; setreg!(a, self.ary_new(items)); }
                Op::Aref => {
                    let v = match reg!(b) { Value::Obj(o) => match self.heap.array(o) { Some(arr) => arr.get(c).map(|s| s.get()).unwrap_or(Value::Nil), None => reg!(b) }, other => if c == 0 { other } else { Value::Nil } };
                    setreg!(a, v);
                }
                Op::Aset => {
                    let v = reg!(a);
                    match reg!(b) { Value::Obj(o) => { if let ObjKind::Array(arr) = &mut self.heap.get_mut(o).kind { if arr.len() <= c { arr.resize(c + 1, Slot::NIL); } arr[c] = Slot::from(v); } } _ => return Err(self.raise_type("not an array")) }
                }
                Op::Apost => {
                    let src = reg!(a);
                    let items = match self.ary_vals(src) { Some(v) => v, None => vec![src] };
                    let pre = b; let post = c;
                    let len = items.len();
                    if len > pre + post {
                        let rest = items[pre..len - post].to_vec();
                        setreg!(a, self.ary_new(rest));
                        for i in 0..post { setreg!(a + 1 + i, items[len - post + i]); }
                    } else {
                        setreg!(a, self.ary_new(vec![]));
                        for i in 0..post { setreg!(a + 1 + i, items.get(pre + i).copied().unwrap_or(Value::Nil)); }
                    }
                }
                Op::Intern => { let v = reg!(a); let bytes = self.expect_str(v, "value")?; setreg!(a, Value::Sym(self.syms.intern(&bytes))); }
                Op::Symbol => {
                    let bytes = match &self.ireps[ci.irep].pool[b] { Pool::Str(s) => s.clone(), _ => return Err(VmError::Internal("SYMBOL pool".into())) };
                    setreg!(a, Value::Sym(self.syms.intern(&bytes)));
                }
                Op::String => {
                    let bytes = match &self.ireps[ci.irep].pool[b] { Pool::Str(s) => s.clone(), _ => return Err(VmError::Internal("STRING pool".into())) };
                    setreg!(a, self.str_new(&bytes));
                }
                Op::Strcat => {
                    let (dst, src) = (reg!(a), reg!(a + 1));
                    let bytes = self.as_string(src)?;
                    match dst { Value::Obj(o) => { if let ObjKind::String(s) = &mut self.heap.get_mut(o).kind { s.extend_from_slice(&bytes); } } _ => return Err(self.raise_type("not a string")) }
                }
                Op::Hash => {
                    let h = self.hash_new();
                    let pairs: Vec<(Value, Value)> = (0..b).map(|i| (self.stack[base + a + i * 2].get(), self.stack[base + a + i * 2 + 1].get())).collect();
                    for (k, v) in pairs { self.hash_set(h, k, v)?; }
                    setreg!(a, h);
                }
                Op::Hashadd => {
                    let h = reg!(a);
                    let pairs: Vec<(Value, Value)> = (0..b).map(|i| (self.stack[base + a + 1 + i * 2].get(), self.stack[base + a + 2 + i * 2].get())).collect();
                    for (k, v) in pairs { self.hash_set(h, k, v)?; }
                }
                Op::Hashcat => {
                    let (h, other) = (reg!(a), reg!(a + 1));
                    let entries = match other.obj().map(|o| &self.heap.get(o).kind) { Some(ObjKind::Hash(hd)) => hd.entries.clone(), _ => return Err(self.raise_type("not a hash")) };
                    for (k, v) in entries { self.hash_set(h, k.get(), v.get())?; }
                }
                Op::Lambda | Op::Block | Op::Method => {
                    let nirep = self.ireps[ci.irep].reps[b];
                    let capture = !matches!(op, Op::Method);
                    let strict = matches!(op, Op::Lambda | Op::Method);
                    let env = if capture { Some(self.frame_env()) } else { None };
                    let p = self.heap.alloc(self.core.proc_, ObjKind::Proc(ProcData {
                        irep: nirep, upper: Some(ci.proc_), env, target_class: Some(ci.target_class), strict, scope: matches!(op, Op::Method), orphan: false,
                    }));
                    setreg!(a, Value::Obj(p));
                }
                Op::RangeInc | Op::RangeExc => {
                    let (x, y) = (reg!(a), reg!(a + 1));
                    self.check_range_ends(x, y)?;
                    setreg!(a, self.range_new(x, y, matches!(op, Op::RangeExc)));
                }
                Op::Oclass => { setreg!(a, Value::Obj(self.core.object)); }
                Op::Class | Op::Module => {
                    let s = self.ireps[ci.irep].syms[b];
                    let base_v = reg!(a);
                    let outer = match base_v { Value::Nil => self.heap.proc_data(ci.proc_).target_class.unwrap_or(self.core.object), Value::Obj(o) if self.heap.is_class(o) => o, _ => return Err(self.raise_type("not a class/module")) };
                    let existing = self.heap.class(outer).consts.get(&s).map(|s| s.get());
                    let is_module = matches!(op, Op::Module);
                    let given_sup = match if is_module { Value::Nil } else { reg!(a + 1) } {
                        Value::Nil => None,
                        Value::Obj(o) if self.heap.is_class(o) && !self.heap.class(o).is_module && !self.heap.class(o).is_singleton => Some(o),
                        other => { if is_module { None } else { let d = self.inspect_str(other)?; return Err(self.raise_type(&format!("superclass must be a Class ({d} given)"))); } }
                    };
                    let cls = match existing {
                        Some(Value::Obj(o)) if self.heap.is_class(o) && self.heap.class(o).is_module == is_module => {
                            if let Some(sup) = given_sup {
                                let real_sup = self.real_superclass(o);
                                if real_sup != Some(sup) { let n = self.sym_name(s); return Err(self.raise_type(&format!("superclass mismatch for Class {n}"))); }
                            }
                            o
                        }
                        Some(v) if !v.is_nil() => { let d = self.inspect_str(v)?; return Err(self.raise_type(&format!("{d} is not a {}", if is_module { "module" } else { "class" }))); }
                        _ => {
                            let sup = if is_module { None } else { Some(given_sup.unwrap_or(self.core.object)) };
                            let meta = if is_module { self.core.module } else { self.core.class };
                            let ncls = self.heap.alloc(meta, ObjKind::Class(ClassData { name: Some(s), superclass: sup, is_module, outer: Some(outer), ..Default::default() }));
                            self.heap.class_mut(outer).consts.insert(s, Slot::from(Value::Obj(ncls)));
                            if !is_module { self.singleton_class(Value::Obj(ncls))?; }
                            if let Some(sup) = sup { self.call_inherited(sup, ncls)?; }
                            ncls
                        }
                    };
                    setreg!(a, Value::Obj(cls));
                }
                Op::Exec => {
                    let nirep = self.ireps[ci.irep].reps[b];
                    let cls = match reg!(a) { Value::Obj(o) if self.heap.is_class(o) => o, _ => return Err(self.raise_type("not a class/module")) };
                    let p = self.heap.alloc(self.core.proc_, ObjKind::Proc(ProcData { irep: nirep, upper: Some(ci.proc_), env: None, target_class: Some(cls), strict: false, scope: true, orphan: false }));
                    let nbase = base + a;
                    let nregs = self.ireps[nirep].nregs.max(4);
                    if self.stack.len() < nbase + nregs { self.stack.resize(nbase + nregs, Slot::NIL); }
                    for i in 1..nregs { self.stack[nbase + i] = Slot::NIL; }
                    self.ci.push(CallInfo { base: nbase, pc: 0, irep: nirep, proc_: p, n: 0, kw: false, mid: None, target_class: cls, env: None, cci: Cci::None, vis: Vis::Public, modfunc: false, vis_break: false });
                }
                Op::Def => {
                    let s = self.ireps[ci.irep].syms[b];
                    let target = match reg!(a) { Value::Obj(o) if self.heap.is_class(o) => o, _ => return Err(self.raise_type("not a class/module")) };
                    let p = match reg!(a + 1) { Value::Obj(o) if matches!(self.heap.get(o).kind, ObjKind::Proc(_)) => o, _ => return Err(self.raise_type("not a proc")) };
                    if let ObjKind::Proc(pd) = &mut self.heap.get_mut(p).kind { pd.target_class = Some(target); }
                    let (vis, modfunc) = if self.heap.class(target).is_singleton { (Vis::Public, false) } else { self.current_def_vis(target) };
                    self.def_method(target, s, Method::Ruby(p), if modfunc { Vis::Private } else { vis })?;
                    if modfunc {
                        let sc = self.singleton_class(Value::Obj(target))?;
                        self.def_method(sc, s, Method::Ruby(p), Vis::Public)?;
                    }
                    setreg!(a, Value::Sym(s));
                }
                Op::Tdef | Op::Sdef => {
                    let s = self.ireps[ci.irep].syms[b];
                    let nirep = self.ireps[ci.irep].reps[c];
                    let target = if matches!(op, Op::Tdef) { ci.target_class } else { let v = reg!(a); self.singleton_class(v)? };
                    let p = self.heap.alloc(self.core.proc_, ObjKind::Proc(ProcData { irep: nirep, upper: Some(ci.proc_), env: None, target_class: Some(target), strict: true, scope: true, orphan: false }));
                    let (vis, modfunc) = if matches!(op, Op::Tdef) && !self.heap.class(target).is_singleton { self.current_def_vis(target) } else { (Vis::Public, false) };
                    self.def_method(target, s, Method::Ruby(p), if modfunc { Vis::Private } else { vis })?;
                    if modfunc { let sc = self.singleton_class(Value::Obj(target))?; self.def_method(sc, s, Method::Ruby(p), Vis::Public)?; }
                    setreg!(a, Value::Sym(s));
                }
                Op::Alias => {
                    let (new, old) = (self.ireps[ci.irep].syms[a], self.ireps[ci.irep].syms[b]);
                    let tc = ci.target_class;
                    self.alias_method(tc, new, old)?;
                }
                Op::Undef => { let s = self.ireps[ci.irep].syms[a]; self.undef_method(ci.target_class, s)?; }
                Op::Sclass => { let v = reg!(a); setreg!(a, Value::Obj(self.singleton_class(v)?)); }
                Op::Tclass => { setreg!(a, Value::Obj(ci.target_class)); }
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
                    let v = self.stack.get(base + nlocals).map(|s| s.get()).unwrap_or(Value::Nil);
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
        // `const_missing` hook on the target class (default raises NameError)
        let cm = self.intern("const_missing");
        let tc = Value::Obj(ci.target_class);
        self.funcall(tc, cm, &[Value::Sym(s)], Value::Nil)
    }
    pub fn frozen_error(&mut self, v: Value) -> VmError {
        let c = self.describe_for_error(v);
        let i = self.inspect_str(v).unwrap_or_default();
        self.raise(self.core.frozen_error, &format!("can't modify frozen {c}: {i}"))
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
    /// The superclass as Ruby sees it (skipping include classes and singletons).
    pub fn real_superclass(&self, c: ObjId) -> Option<ObjId> {
        let mut s = self.heap.class(c).superclass;
        while let Some(x) = s {
            let cd = self.heap.class(x);
            if cd.iclass_of.is_none() && !cd.is_singleton && cd.origin_of.is_none() { return Some(x); }
            s = cd.superclass;
        }
        None
    }
    /// `Class#inherited` hook.
    pub fn call_inherited(&mut self, sup: ObjId, sub: ObjId) -> VmResult<()> {
        let inh = self.intern("inherited");
        self.funcall(Value::Obj(sup), inh, &[Value::Obj(sub)], Value::Nil)?;
        Ok(())
    }
    /// `mrb_mod_cv_get`: walks the whole chain and the *last* table that has the
    /// variable wins (an included module's value shadows the class's own).
    fn cvar_get(&self, class: ObjId, s: Sym) -> Option<Value> {
        let mut found = None;
        let mut c = Some(class);
        while let Some(x) = c {
            let cd = self.heap.class(x);
            let owner = cd.iclass_of.unwrap_or(x);
            if let Some(v) = self.heap.class(owner).cvars.get(&s) { found = Some(v.get()); }
            c = cd.superclass;
        }
        found
    }
    /// `mrb_mod_cv_set`: the first table (from the class up) that has the variable, else the class itself.
    fn cvar_set(&mut self, class: ObjId, s: Sym, v: Value) -> VmResult<()> {
        let mut c = Some(class);
        while let Some(x) = c {
            let owner = self.heap.class(x).iclass_of.unwrap_or(x);
            if self.heap.class(owner).cvars.contains_key(&s) {
                if self.heap.get(owner).frozen { return Err(self.frozen_error(Value::Obj(owner))); }
                self.heap.class_mut(owner).cvars.insert(s, Slot::from(v));
                return Ok(());
            }
            c = self.heap.class(x).superclass;
        }
        if self.heap.get(class).frozen { return Err(self.frozen_error(Value::Obj(class))); }
        self.heap.class_mut(class).cvars.insert(s, Slot::from(v));
        Ok(())
    }

    /// `mrb_ary_splat`: Array as is; `to_a` if it answers (nil means "no conversion");
    /// a non-Array `to_a` result is a TypeError.
    pub fn to_array(&mut self, v: Value) -> VmResult<Vec<Value>> {
        if let Some(a) = self.ary_vals(v) { return Ok(a); }
        let to_a = self.intern("to_a");
        if !self.respond_to(v, to_a) { return Ok(vec![v]); }
        let r = self.funcall(v, to_a, &[], Value::Nil)?;
        if r.is_nil() { return Ok(vec![v]); }
        match self.ary(r) {
            Some(a) => Ok(values_of(a)),
            None => { let c = self.describe_for_error(v); let rc = self.describe_for_error(r); Err(self.raise_type(&format!("can't convert {c} to Array ({c}#to_a gives {rc})"))) }
        }
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
    /// The hash code of a key: native for immediates and Strings, `hash` for other objects.
    pub fn key_hash(&mut self, k: Value) -> VmResult<i64> {
        match k {
            Value::Obj(o) if !matches!(self.heap.get(o).kind, ObjKind::String(_)) => {
                let hs = self.s.hash;
                // callers (HASH, keyword packing) hold the hash being built in a Rust local
                self.native_active += 1;
                let r = self.funcall(k, hs, &[], Value::Nil);
                self.native_active -= 1;
                match r? { Value::Int(i) => Ok(i), Value::Float(f) => Ok(f as i64), _ => Ok(self.value_hash(k)) }
            }
            _ => Ok(self.value_hash(k)),
        }
    }
    /// Key equality for lookups: `eql?` (natively for immediates and Strings).
    pub fn key_eql(&mut self, a: Value, b: Value) -> VmResult<bool> {
        match (a, b) {
            (Value::Obj(x), _) if !matches!(self.heap.get(x).kind, ObjKind::String(_)) => {
                let eql = self.s.eql;
                self.native_active += 1; // as in key_hash
                let r = self.funcall(a, eql, &[b], Value::Nil);
                self.native_active -= 1;
                Ok(r?.truthy())
            }
            _ => Ok(self.eql(a, b)),
        }
    }
    /// Makes the cached hashes match the entries (after wholesale edits of `entries`).
    fn hash_sync(&mut self, o: ObjId) -> VmResult<()> {
        let (need, keys): (bool, Vec<Value>) = match &self.heap.get(o).kind { ObjKind::Hash(hd) => (hd.hashes.len() != hd.entries.len(), hd.entries.iter().map(|e| e.0.get()).collect()), _ => (false, vec![]) };
        if !need { return Ok(()); }
        let mut hs = Vec::with_capacity(keys.len());
        for k in keys { hs.push(self.key_hash(k)?); }
        if let ObjKind::Hash(hd) = &mut self.heap.get_mut(o).kind { hd.hashes = hs; }
        Ok(())
    }
    /// Index of `k` in the hash (hash code first, then `eql?`).
    pub fn hash_index(&mut self, h: Value, k: Value) -> VmResult<Option<usize>> {
        let o = match h.obj() { Some(o) => o, None => return Ok(None) };
        if !matches!(self.heap.get(o).kind, ObjKind::Hash(_)) { return Ok(None); }
        self.hash_sync(o)?;
        let kh = self.key_hash(k)?;
        let mut i = 0;
        loop {
            let cand = match &self.heap.get(o).kind { ObjKind::Hash(hd) => { if i >= hd.entries.len() { break; } if hd.hashes.get(i) == Some(&kh) { Some(hd.entries[i].0.get()) } else { None } } _ => None };
            if let Some(ek) = cand { if self.key_eql(k, ek)? { return Ok(Some(i)); } }
            i += 1;
        }
        Ok(None)
    }
    pub fn hash_get(&mut self, h: Value, k: Value) -> Option<Value> {
        match self.hash_index(h, k) {
            Ok(Some(i)) => match &self.heap.get(h.obj().unwrap()).kind { ObjKind::Hash(hd) => hd.entries.get(i).map(|e| e.1.get()), _ => None },
            _ => None,
        }
    }
    pub fn hash_set(&mut self, h: Value, k: Value, v: Value) -> VmResult<()> {
        let o = match h.obj() { Some(o) => o, None => return Err(self.raise_type("not a hash")) };
        if self.heap.get(o).frozen { return Err(self.frozen_error(h)); }
        // an unfrozen String key is copied and the copy frozen; a frozen key is used as is
        let k = match k { Value::Obj(ko) if self.heap.string(ko).is_some() && !self.heap.get(ko).frozen => { let b = self.heap.string(ko).unwrap().to_vec(); let nk = self.str_new(&b); if let Some(no) = nk.obj() { self.heap.get_mut(no).frozen = true; } nk } _ => k };
        let pos = self.hash_index(h, k)?;
        let kh = self.key_hash(k)?;
        if let ObjKind::Hash(hd) = &mut self.heap.get_mut(o).kind {
            match pos { Some(i) => hd.entries[i].1 = Slot::from(v), None => { hd.entries.push((Slot::from(k), Slot::from(v))); hd.hashes.push(kh); } }
        }
        Ok(())
    }

    fn op_arith(&mut self, base: usize, a: usize, mid: Sym) -> VmResult<()> {
        let (x, y) = (self.stack[base + a].get(), self.stack[base + a + 1].get());
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
            Some(v) => { self.stack[base + a] = Slot::from(v); Ok(()) }
            None => {
                if let (Value::Int(_), Value::Int(_)) = (x, y) { return Err(self.raise(self.core.range_error, "integer overflow")); }
                self.op_send(base, a, mid, 1, false, false)
            }
        }
    }

    fn op_compare(&mut self, base: usize, a: usize, mid: Sym) -> VmResult<()> {
        let (x, y) = (self.stack[base + a].get(), self.stack[base + a + 1].get());
        let s = self.s;
        let num = |p: f64, q: f64| -> Value {
            Value::bool(if mid == s.eq { p == q } else if mid == s.lt { p < q } else if mid == s.le { p <= q } else if mid == s.gt { p > q } else { p >= q })
        };
        let ord = |o: Option<core::cmp::Ordering>| -> Value {
            Value::bool(match o { None => false, Some(o) => if mid == s.eq { o.is_eq() } else if mid == s.lt { o.is_lt() } else if mid == s.le { o.is_le() } else if mid == s.gt { o.is_gt() } else { o.is_ge() } })
        };
        let _ = &num;
        let r = match (x, y) {
            (Value::Int(p), Value::Int(q)) => Some(Value::bool(if mid == s.eq { p == q } else if mid == s.lt { p < q } else if mid == s.le { p <= q } else if mid == s.gt { p > q } else { p >= q })),
            (Value::Int(p), Value::Float(q)) => Some(ord(crate::builtins::numeric::int_float_cmp(p, q))),
            (Value::Float(p), Value::Int(q)) => Some(ord(crate::builtins::numeric::int_float_cmp(q, p).map(|o| o.reverse()))),
            (Value::Float(p), Value::Float(q)) => Some(ord(p.partial_cmp(&q))),
            _ if mid == s.eq => {
                // fast path (mruby `mrb_obj_eq` before the `==` send): identical
                // immediates or the same object; a NaN is never equal to itself
                if x == y { Some(Value::True) } else { None }
            }
            _ => None,
        };
        match r {
            Some(v) => { self.stack[base + a] = Slot::from(v); Ok(()) }
            None => self.op_send(base, a, mid, 1, false, false),
        }
    }

    /// Shared body of SEND/SSEND/SUPER: receiver at `R[a]`, args at `R[a+1..]`,
    /// block at `R[a+argc+1]` when `has_blk`.
    /// Lays out a call site (`c` = SEND's third operand): packs `nk` keyword pairs
    /// into one Hash right after the positional arguments, moves the block after
    /// it, and returns `(n, kw, blk)` for the new frame (`n` = 15 when packed).
    fn prepare_call(&mut self, nbase: usize, c: usize, has_blk: bool) -> VmResult<(usize, bool, Value)> {
        let n = c & 0xf;
        let nk = (c >> 4) & 0xf;
        let npos = if n == 15 { 1 } else { n };
        let bidx = nbase + npos + (if nk == 15 { 1 } else { nk * 2 }) + 1;
        if self.stack.len() <= bidx { self.stack.resize(bidx + 1, Slot::NIL); }
        let blk = if has_blk { let b = self.stack[bidx].get(); self.ensure_block(b)? } else { Value::Nil };
        let kw = nk > 0;
        if nk > 0 && nk != 15 {
            let kidx = nbase + npos + 1;
            let h = self.hash_new();
            for i in 0..nk { let (k, v) = (self.stack[kidx + i * 2].get(), self.stack[kidx + i * 2 + 1].get()); self.hash_set(h, k, v)?; }
            self.stack[kidx] = Slot::from(h);
        } else if nk == 15 {
            let h = self.stack[nbase + npos + 1].get();
            if !matches!(h.obj().map(|o| &self.heap.get(o).kind), Some(ObjKind::Hash(_))) { return Err(self.raise_type("keyword argument hash expected")); }
        }
        let new_bidx = nbase + npos + (if kw { 1 } else { 0 }) + 1;
        if self.stack.len() <= new_bidx { self.stack.resize(new_bidx + 1, Slot::NIL); }
        self.stack[new_bidx] = Slot::from(blk);
        Ok((n, kw, blk))
    }

    /// Positional arguments of a frame laid out by `prepare_call`, with the
    /// keyword Hash appended when present and non-empty (what a native method
    /// sees; mruby's `mrb_get_args` does the same for functions without `:`).
    fn native_args(&self, nbase: usize, n: usize, kw: bool) -> (Vec<Value>, Option<Value>) {
        let mut args = self.send_args(nbase, n);
        let npos = if n == 15 { 1 } else { n };
        let mut kd = None;
        if kw {
            let h = self.stack[nbase + npos + 1].get();
            let empty = match h.obj().map(|o| &self.heap.get(o).kind) { Some(ObjKind::Hash(hd)) => hd.entries.is_empty(), _ => true };
            if !empty { args.push(h); kd = Some(h); }
        }
        (args, kd)
    }

    fn op_send(&mut self, base: usize, a: usize, mid: Sym, c: usize, has_blk: bool, is_super: bool) -> VmResult<()> {
        self.op_send_vis(base, a, mid, c, has_blk, is_super, false)
    }
    /// `explicit`: the receiver was written (SEND/SEND0/SENDB), so private/protected are enforced.
    fn op_send_vis(&mut self, base: usize, a: usize, mid: Sym, c: usize, has_blk: bool, is_super: bool, explicit: bool) -> VmResult<()> {
        let recv = self.stack[base + a].get();
        let (argc, kw, blk) = self.prepare_call(base + a, c, has_blk)?;
        let start_class = if is_super {
            let owner = self.ci.last().unwrap().target_class;
            match self.heap.class(owner).superclass { Some(s) => s, None => return Err(self.raise(self.core.no_method_error, "super: no superclass method")) }
        } else { self.class_of(recv) };
        let found = self.find_method(start_class, mid);
        let (m, owner) = match found {
            Some((m, owner)) => {
                if explicit && !is_super {
                    match self.method_vis(owner, mid) {
                        Vis::Public => {}
                        Vis::Private => { let name = self.sym_name(mid); let desc = self.describe_for_error(recv); return Err(self.no_method_error(mid, recv, &format!("private method '{name}' called for {desc}"))); }
                        Vis::Protected => {
                            let caller_self = self.stack[base].get();
                            let home = self.heap.class(owner).iclass_of.unwrap_or(owner);
                            if !self.obj_is_kind_of(caller_self, home) { let name = self.sym_name(mid); let desc = self.describe_for_error(recv); return Err(self.no_method_error(mid, recv, &format!("protected method '{name}' called for {desc}"))); }
                        }
                    }
                }
                (m, owner)
            }
            None => {
                // method_missing (user-defined) takes the call; the basic one reports the error
                let mm = self.s.method_missing;
                let mm_class = if is_super { self.class_of(recv) } else { start_class };
                if let Some((m, owner)) = self.find_method(mm_class, mm) {
                    if !matches!(m, Method::Native(_)) {
                        // insert the method name as the first argument
                        let (args, kd) = self.native_args(base + a, argc, kw);
                        let mut nargs = vec![Value::Sym(mid)];
                        let pos = if kd.is_some() { &args[..args.len() - 1] } else { &args[..] };
                        nargs.extend_from_slice(pos);
                        let r = match m {
                            Method::Native(f) => { if let Some(k) = kd { nargs.push(k); } let (v, sw) = self.call_native_direct(f, recv, &nargs, blk, base + a)?; if sw { return Ok(()); } v }
                            Method::Ruby(p) => { let tc = if self.heap.proc_data(p).env.is_some() { None } else { Some(owner) }; self.call_proc_with(p, recv, &nargs, kd, blk, Some(mm), tc)? }
                            _ => Value::Nil,
                        };
                        self.stack[base + a] = Slot::from(r);
                        return Ok(());
                    }
                }
                let name = self.sym_name(mid);
                let desc = self.describe_for_error(recv);
                let args = self.native_args(base + a, argc, kw).0;
                let msg = if is_super { format!("no superclass method '{name}' for {desc}") } else { format!("undefined method '{name}' for {desc}") };
                let e = self.no_method_error(mid, recv, &msg);
                if let VmError::Raise(Value::Obj(o)) = e { let av = self.ary_new(args); let k = self.intern("@args"); self.heap.ivar_set(o, k, av); }
                return Err(e);
            }
        };
        match m {
            Method::Native(f) => {
                if core::ptr::fn_addr_eq(f, crate::builtins::object::send as crate::object::NativeFn) {
                    // `send`/`__send__` from bytecode re-dispatches in this frame
                    // (mruby `mrb_f_send` → `mrb_exec_irep`): no native boundary,
                    // so `Fiber.yield` and `break` inside the callee still work.
                    return self.op_send_redirect(base, a, argc, kw, has_blk, blk);
                }
                let (args, kd) = self.native_args(base + a, argc, kw);
                let saved = self.pending_kw.replace(kd.unwrap_or(Value::Nil));
                let r = self.call_native_direct(f, recv, &args, blk, base + a);
                self.pending_kw = saved;
                let (v, switched) = r?;
                if !switched { self.stack[base + a] = Slot::from(v); }
            }
            Method::AttrReader(iv) => {
                let (args, _) = self.native_args(base + a, argc, kw);
                if !args.is_empty() { return Err(self.argnum_error(args.len(), "0")); }
                self.stack[base + a] = Slot::from(recv.obj().map(|o| self.heap.ivar_get(o, iv)).unwrap_or(Value::Nil));
            }
            Method::AttrWriter(iv) => {
                let (args, _) = self.native_args(base + a, argc, kw);
                if args.len() != 1 { return Err(self.argnum_error(args.len(), "1")); }
                let v = args[0];
                match recv { Value::Obj(o) => self.heap.ivar_set(o, iv, v), _ => return Err(self.raise_type("can't set instance variable")) }
                self.stack[base + a] = Slot::from(v);
            }
            Method::Ruby(p) => {
                if self.ci.len() >= CALL_LEVEL_MAX { return Err(self.raise(self.core.system_stack_error, "stack level too deep")); }
                let nbase = base + a;
                let nirep = self.heap.proc_data(p).irep;
                let nregs = self.ireps[nirep].nregs.max(argc + 2).max(4);
                if self.stack.len() < nbase + nregs { self.stack.resize(nbase + nregs, Slot::NIL); }
                let used = (if argc == 15 { 1 } else { argc }) + (if kw { 1 } else { 0 }) + 2;
                for i in used..nregs { self.stack[nbase + i] = Slot::NIL; }
                self.ci.push(CallInfo { base: nbase, pc: 0, irep: nirep, proc_: p, n: argc as u8, kw, mid: Some(mid), target_class: owner, env: None, cci: Cci::None, vis: Vis::Public, modfunc: false, vis_break: false });
            }
            Method::Undef => unreachable!(),
        }
        Ok(())
    }

    /// `mrb_ci_bidx`: the register (relative to base) holding a frame's block.
    pub fn frame_bidx(ci: &CallInfo) -> usize {
        let n = ci.n as usize;
        (if n == 15 { 1 } else { n }) + (if ci.kw { 1 } else { 0 }) + 1
    }
    /// `mrb_ci_kidx`: the register holding the keyword Hash of a frame, if any.
    fn kidx(&self, ci: &CallInfo) -> Option<usize> {
        if !ci.kw { return None; }
        let n = ci.n as usize;
        Some(ci.base + (if n == 15 { 1 } else { n }) + 1)
    }
    pub fn hash_delete(&mut self, h: Value, k: Value) -> Option<Value> {
        let o = h.obj()?;
        let pos = self.hash_index(h, k).ok()??;
        match &mut self.heap.get_mut(o).kind { ObjKind::Hash(hd) => { if pos < hd.hashes.len() { hd.hashes.remove(pos); } Some(hd.entries.remove(pos).1.get()) } _ => None }
    }

    /// Positional arguments of a SEND at `nbase` (`argc == 15` = packed array).
    fn send_args(&self, nbase: usize, argc: usize) -> Vec<Value> {
        if argc == 15 { self.ary_vals(self.stack[nbase + 1].get()).unwrap_or_default() } else { values_of(&self.stack[nbase + 1..nbase + 1 + argc]) }
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
        if self.stack.len() < base + nregs { self.stack.resize(base + nregs, Slot::NIL); }
        for i in nargs..nregs { self.stack[base + i] = Slot::NIL; }
        let (mid, tc, self_) = match env {
            Some(e) => { let ed = self.heap.env(e); (ed.mid, ed.target_class.or(ptc).unwrap_or(self.core.object), self.env_get(e, 0)) }
            None => (self.ci[top].mid, ptc.unwrap_or(self.core.object), self.stack[base].get()),
        };
        let ci = &mut self.ci[top];
        ci.irep = irep; ci.proc_ = p; ci.pc = 0; ci.mid = mid; ci.target_class = tc; ci.env = None;
        self.stack[base] = Slot::from(self_);
    }

    fn op_enter(&mut self, spec: u32) -> VmResult<()> {
        let m1 = ((spec >> 18) & 0x1f) as usize;
        let o = ((spec >> 13) & 0x1f) as usize;
        let r = ((spec >> 12) & 1) as usize;
        let m2 = ((spec >> 7) & 0x1f) as usize;
        let k = ((spec >> 2) & 0x1f) as usize;
        let kdict_flag = (spec >> 1) & 1 == 1;
        let noblock = (spec >> 23) & 1 == 1;
        let kd = if k > 0 || kdict_flag { 1 } else { 0 };
        let top = self.ci.len() - 1;
        let (base, mut n, mut kw, proc_, irep) = { let ci = &self.ci[top]; (ci.base, ci.n as usize, ci.kw, ci.proc_, ci.irep) };
        let strict = self.heap.proc_data(proc_).strict;
        let nlocals = self.ireps[irep].nlocals;
        let len = m1 + o + r + m2;
        let nregs = self.ireps[irep].nregs.max(len + kd + 3);
        if self.stack.len() < base + nregs { self.stack.resize(base + nregs, Slot::NIL); }
        // fast path: only required parameters, no packed args
        if (spec & !0x7c0001) == 0 && n < 15 && strict {
            if n + (kw as usize) != m1 { return Err(self.argnum_error(n + kw as usize, &format!("{m1}"))); }
            for i in m1 + 2..nlocals { self.stack[base + i] = Slot::NIL; }
            self.ci[top].kw = false;
            return Ok(());
        }
        let npos = if n == 15 { 1 } else { n };
        let blk = self.stack[base + npos + (kw as usize) + 1].get();
        if noblock && !blk.is_nil() { return Err(self.raise_arg("no block accepted")); }
        let mut kdict = if kw { self.stack[base + npos + 1].get() } else { Value::Nil };
        if kd == 0 {
            let nonempty = match kdict.obj().map(|o| &self.heap.get(o).kind) { Some(ObjKind::Hash(hd)) => !hd.entries.is_empty(), _ => false };
            if nonempty {
                // the keyword Hash becomes the last positional argument
                if n < 14 { n += 1; }
                else if n == 14 { let all: Vec<Value> = values_of(&self.stack[base + 1..base + 16]); self.stack[base + 1] = Slot::from(self.ary_new(all)); n = 15; }
                else if let Some(o) = self.stack[base + 1].get().obj() { if let ObjKind::Array(v) = &mut self.heap.get_mut(o).kind { v.push(Slot::from(kdict)); } }
            }
            kdict = Value::Nil;
            kw = false;
        }
        let mut argv: Vec<Value> = if n == 15 { self.ary_vals(self.stack[base + 1].get()).unwrap_or_default() } else { values_of(&self.stack[base + 1..base + 1 + n]) };
        let mut argc = argv.len();
        if strict {
            if argc < m1 + m2 || (r == 0 && argc > len) {
                let exp = if r == 0 && o == 0 { format!("{}", m1 + m2) } else if r == 0 { format!("{}..{}", m1 + m2, len) } else { format!("{}+", m1 + m2) };
                return Err(self.argnum_error(argc, &exp));
            }
        } else if len > 1 && argc == 1 {
            if let Some(arr) = self.ary_vals(argv[0]) { argv = arr; argc = argv.len(); }
        }
        let mut pc = self.ci[top].pc;
        if argc < len {
            let mlen = if argc < m1 + m2 { if m1 < argc { argc - m1 } else { 0 } } else { m2 };
            for i in 0..argc.saturating_sub(mlen).min(m1 + o) { self.stack[base + 1 + i] = Slot::from(argv[i]); }
            for i in argc..m1 { self.stack[base + 1 + i] = Slot::NIL; }
            for i in 0..mlen { self.stack[base + len - m2 + 1 + i] = Slot::from(argv[argc - mlen + i]); }
            for i in mlen..m2 { self.stack[base + len - m2 + 1 + i] = Slot::NIL; }
            if r == 1 { let rest = self.ary_new(vec![]); self.stack[base + m1 + o + 1] = Slot::from(rest); }
            if o > 0 && argc > m1 + m2 { pc += (argc - m1 - m2) * 3; }
        } else {
            for i in 0..m1 + o { self.stack[base + 1 + i] = Slot::from(argv[i]); }
            let mut rnum = 0;
            if r == 1 { rnum = argc - m1 - o - m2; let rest = self.ary_new(argv[m1 + o..m1 + o + rnum].to_vec()); self.stack[base + m1 + o + 1] = Slot::from(rest); }
            if m2 > 0 { for i in 0..m2 { self.stack[base + m1 + o + r + 1 + i] = Slot::from(argv[m1 + o + rnum + i]); } }
            pc += o * 3;
        }
        let kw_pos = len + kd;
        let blk_pos = kw_pos + 1;
        self.stack[base + blk_pos] = Slot::from(blk);
        if kd == 1 {
            if kdict.is_nil() { kdict = self.hash_new(); }
            self.stack[base + kw_pos] = Slot::from(kdict);
            kw = true;
        }
        for i in blk_pos + 1..nlocals { if base + i < self.stack.len() { self.stack[base + i] = Slot::NIL; } }
        self.ci[top].n = len as u8;
        self.ci[top].kw = kw;
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
        let v = if lv == 0 { self.stack[base + 1 + offset].get() } else {
            match self.uvenv(lv - 1) { Some(e) if self.heap.env(e).len > offset + 1 => self.env_get(e, 1 + offset), _ => return Err(self.raise(self.core.local_jump_error, "unexpected yield")) }
        };
        if v.is_nil() { return Err(self.raise(self.core.local_jump_error, "unexpected yield")); }
        Ok(v)
    }

    /// Returns `Some(value)` when the loop should return to its caller.
    fn op_return(&mut self, v: Value, stop_depth: usize, lc: usize) -> VmResult<Option<Value>> {
        let top = self.ci.len() - 1;
        self.unwind_return(top, v, stop_depth, lc, UnwindBy::Return)
    }

    fn op_return_blk(&mut self, v: Value, stop_depth: usize, lc: usize) -> VmResult<Option<Value>> {
        let top = self.ci.len() - 1;
        let p = self.ci[top].proc_;
        let pd = self.heap.proc_data(p);
        if pd.env.is_none() || pd.strict { return self.op_return(v, stop_depth, lc); }
        // mruby `top_proc(proc, &env)`: walk `upper` until the method or lambda
        // that owns the locals. `env` ends as the environment captured by the
        // last block on the way, i.e. the environment *of the frame* running
        // that method/lambda (a frame's env is the one its blocks capture), so
        // it identifies the frame to return from.
        let mut cur = p;
        let mut env = pd.env;
        loop {
            let cd = self.heap.proc_data(cur);
            let up = match cd.upper { Some(u) => u, None => break };
            if cd.scope || cd.strict { break; }
            env = cd.env;
            cur = up;
        }
        let target_env = env;
        // a home frame in another fiber is not reachable (`dst->e.env->cxt == mrb->c`)
        if let Some(e) = target_env { let ed = self.heap.env(e); if ed.attached && ed.ctx != self.cur { return Err(self.raise(self.core.local_jump_error, "unexpected return")); } }
        let mut idx = None;
        for i in (0..self.ci.len()).rev() {
            if self.ci[i].env.is_some() && self.ci[i].env == target_env { idx = Some(i); break; }
        }
        match idx {
            Some(i) => self.unwind_return(i, v, stop_depth, lc, UnwindBy::Return),
            None => Err(self.raise(self.core.local_jump_error, "unexpected return")),
        }
    }

    fn op_break(&mut self, v: Value, stop_depth: usize, lc: usize) -> VmResult<Option<Value>> {
        let top = self.ci.len() - 1;
        let p = self.ci[top].proc_;
        let pd = self.heap.proc_data(p);
        if pd.strict { return self.op_return(v, stop_depth, lc); }
        let dst = match (pd.orphan, pd.env, pd.upper) { (false, Some(_), Some(u)) => u, _ => return Err(self.raise(self.core.local_jump_error, "break from proc-closure")) };
        // return from the frame whose *caller* runs `dst` (the method that received the block)
        let mut idx = None;
        for i in (1..self.ci.len()).rev() {
            if self.ci[i - 1].proc_ == dst { idx = Some(i); break; }
        }
        match idx {
            Some(i) => self.unwind_return(i, v, stop_depth, lc, UnwindBy::Break),
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
            if lv == 0 { Ok(vm.stack.get(base + 1 + i).map(|s| s.get()).unwrap_or(Value::Nil)) } else {
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
            if let Some(v) = self.ary_vals(rest) { args.extend(v); }
        }
        for i in 0..m2 { args.push(get(self, m1 + r + i).map_err(|_| fail(self))?); }
        let blk_or_kd = get(self, m1 + r + m2).map_err(|_| fail(self))?;
        let need = base + a + 3;
        if self.stack.len() < need { self.stack.resize(need, Slot::NIL); }
        self.stack[base + a] = Slot::from(self.ary_new(args));
        if kd == 1 {
            let blk = get(self, m1 + r + m2 + 1).map_err(|_| fail(self))?;
            self.stack[base + a + 1] = Slot::from(blk_or_kd);
            self.stack[base + a + 2] = Slot::from(blk);
        } else {
            self.stack[base + a + 1] = Slot::from(blk_or_kd);
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
                    let lineno = match ir.line_of(pc) { Some(l) => format!("{l:5} "), None => "      ".into() };
                    let mut line = format!("{lineno}{:03} {}", pc, op.name());
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
