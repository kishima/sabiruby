//! Heap objects. Every non-immediate value is an entry in [`Heap`], addressed
//! by [`ObjId`]. There is no garbage collector yet: the heap only grows
//! (a mark & sweep collector driven by a step budget is planned; see the
//! design notes in the book repository).

use alloc::vec::Vec;

use hashbrown::HashMap;

use crate::error::VmResult;
use crate::symbol::Sym;
use crate::value::{ObjId, Value};
use crate::vm::Vm;

/// Native (Rust) method: `fn(vm, self, args, block)`.
pub type NativeFn = fn(&mut Vm, Value, &[Value], Value) -> VmResult<Value>;

/// Index of a loaded irep in [`Vm`].
pub type IrepId = usize;

#[derive(Clone)]
pub enum Method {
    /// A method written in Ruby: the Proc (irep + target class) to run.
    Ruby(ObjId),
    Native(NativeFn),
    AttrReader(Sym),
    AttrWriter(Sym),
    /// `undef_method`: stops the lookup.
    Undef,
}

#[derive(Default)]
pub struct ClassData {
    pub name: Option<Sym>,
    pub superclass: Option<ObjId>,
    pub methods: HashMap<Sym, Method>,
    pub consts: HashMap<Sym, Value>,
    pub cvars: HashMap<Sym, Value>,
    pub is_module: bool,
    pub is_singleton: bool,
    /// For a singleton class: the object it belongs to.
    pub attached: Option<Value>,
    /// For an include class (mruby `MRB_TT_ICLASS`): the module whose
    /// method table and constants are shared.
    pub iclass_of: Option<ObjId>,
}

pub struct ProcData {
    pub irep: IrepId,
    /// The proc that was running when this one was created (`upper`).
    pub upper: Option<ObjId>,
    /// Captured environment (`REnv`), for blocks and lambdas.
    pub env: Option<ObjId>,
    pub target_class: Option<ObjId>,
    /// `MRB_PROC_STRICT`: methods and lambdas check arity; `return` returns from here.
    pub strict: bool,
    /// `MRB_PROC_SCOPE`: a method/class body (new scope for `return`).
    pub scope: bool,
    /// `MRB_PROC_ORPHAN`: the frame that created the block has returned.
    pub orphan: bool,
}

/// `REnv`: the locals a block can see. While the creating frame is alive the
/// values live on the VM stack (`attached`); when the frame is popped they are
/// copied into `values` (mruby's `mrb_env_detach`).
pub struct EnvData {
    pub base: usize,
    pub len: usize,
    pub attached: bool,
    pub values: Vec<Value>,
    pub mid: Option<Sym>,
    pub target_class: Option<ObjId>,
}

#[derive(Default)]
pub struct HashData {
    /// Insertion-ordered entries; lookup is linear with `eql?` semantics
    /// (like mruby's AR mode, without the hash-table switch yet).
    pub entries: Vec<(Value, Value)>,
    pub default: Value,
}

impl Default for Value {
    fn default() -> Self {
        Value::Nil
    }
}

/// Why a `Break` object is unwinding the stack (mruby `RBREAK_TAG_*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BreakTag {
    /// `return`/`break`: unwind to frame `ci_index` and return `value` from it.
    Break,
    /// `OP_JMPUW`: after the ensure body, jump to pc `value` (an Integer).
    Jump,
}

pub enum ObjKind {
    Object,
    /// mruby `RBreak`: a non-local exit that must first run `ensure` bodies.
    /// Never visible to Ruby code (only passes through EXCEPT/RAISEIF).
    Break { tag: BreakTag, ci_index: usize, value: Value },
    Class(ClassData),
    String(Vec<u8>),
    Array(Vec<Value>),
    Hash(HashData),
    Range { begin: Value, end: Value, excl: bool },
    Proc(ProcData),
    Env(EnvData),
    Exception,
}

pub struct HeapObject {
    pub class: ObjId,
    pub ivars: Vec<(Sym, Value)>,
    pub frozen: bool,
    pub kind: ObjKind,
}

#[derive(Default)]
pub struct Heap {
    objs: Vec<HeapObject>,
}

impl Heap {
    pub fn alloc(&mut self, class: ObjId, kind: ObjKind) -> ObjId {
        let id = ObjId(self.objs.len() as u32);
        self.objs.push(HeapObject { class, ivars: Vec::new(), frozen: false, kind });
        id
    }
    /// Allocates a class object whose own class is filled in later.
    pub fn alloc_raw(&mut self, kind: ObjKind) -> ObjId {
        self.alloc(ObjId(u32::MAX), kind)
    }
    #[inline]
    pub fn get(&self, id: ObjId) -> &HeapObject {
        &self.objs[id.0 as usize]
    }
    #[inline]
    pub fn get_mut(&mut self, id: ObjId) -> &mut HeapObject {
        &mut self.objs[id.0 as usize]
    }
    pub fn len(&self) -> usize {
        self.objs.len()
    }
    pub fn is_empty(&self) -> bool {
        self.objs.is_empty()
    }
    pub fn class(&self, id: ObjId) -> &ClassData {
        match &self.get(id).kind {
            ObjKind::Class(c) => c,
            _ => panic!("object {:?} is not a class", id),
        }
    }
    pub fn class_mut(&mut self, id: ObjId) -> &mut ClassData {
        match &mut self.get_mut(id).kind {
            ObjKind::Class(c) => c,
            _ => panic!("object {:?} is not a class", id),
        }
    }
    pub fn is_class(&self, id: ObjId) -> bool {
        matches!(self.get(id).kind, ObjKind::Class(_))
    }
    pub fn ivar_get(&self, id: ObjId, name: Sym) -> Value {
        self.get(id).ivars.iter().find(|(n, _)| *n == name).map(|(_, v)| *v).unwrap_or(Value::Nil)
    }
    pub fn ivar_set(&mut self, id: ObjId, name: Sym, v: Value) {
        let o = self.get_mut(id);
        if let Some(e) = o.ivars.iter_mut().find(|(n, _)| *n == name) {
            e.1 = v;
        } else {
            o.ivars.push((name, v));
        }
    }
    pub fn string(&self, id: ObjId) -> Option<&[u8]> {
        match &self.get(id).kind {
            ObjKind::String(s) => Some(s),
            _ => None,
        }
    }
    pub fn array(&self, id: ObjId) -> Option<&Vec<Value>> {
        match &self.get(id).kind {
            ObjKind::Array(a) => Some(a),
            _ => None,
        }
    }
    pub fn proc_data(&self, id: ObjId) -> &ProcData {
        match &self.get(id).kind {
            ObjKind::Proc(p) => p,
            _ => panic!("object {:?} is not a proc", id),
        }
    }
    pub fn env(&self, id: ObjId) -> &EnvData {
        match &self.get(id).kind {
            ObjKind::Env(e) => e,
            _ => panic!("object {:?} is not an env", id),
        }
    }
    pub fn env_mut(&mut self, id: ObjId) -> &mut EnvData {
        match &mut self.get_mut(id).kind {
            ObjKind::Env(e) => e,
            _ => panic!("object {:?} is not an env", id),
        }
    }
}
