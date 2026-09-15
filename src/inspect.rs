//! Looking into a running VM, for debuggers and visualisers (the browser playground).
//!
//! Two things: [`Vm::snapshot`] describes the VM as it stands (contexts, frames, registers,
//! environments, heap), and [`TraceEvent`] records what the interpreter did between two
//! snapshots (an environment created or detached, an exception looking for a handler, a fiber
//! switch, a collection) — the things `docs/fibers.md`, `docs/exceptions.md` and `docs/gc.md`
//! describe, which are otherwise invisible from Ruby.
//!
//! **Values are rendered here, in Rust.** Calling Ruby's `inspect` would run the program being
//! inspected (and could raise or allocate), so [`render`](Vm::render) formats values directly
//! and never calls a method.
//!
//! Recording is off by default (`Vm::set_trace`); the interpreter loop itself is untouched, so
//! nothing is added per instruction (see `docs/inspect.md`).

use alloc::{format, string::{String, ToString}, vec::Vec};

use crate::object::{IrepId, ObjKind};
use crate::symbol::Sym;
use crate::value::{ObjId, Value};
use crate::vm::{Cci, FiberState, Vm};

/// Why an environment left the stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DetachReason {
    /// The frame that owned it returned (`cipop`).
    FrameReturn,
    /// Its fiber was collected; the values were copied out before the context went away.
    ContextSwept,
}

/// What removed a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnwindBy { Raise, Break, Return }

/// Why the running context changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwitchKind { Resume, Yield, Transfer, Terminate, Reset }

/// A catch handler that matched (`rescue` or `ensure` of the frame's irep).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CatchHandlerInfo {
    pub ensure: bool,
    pub begin: u32,
    pub end: u32,
    pub target: u32,
}

/// Something the interpreter did. Recorded only while `Vm::set_trace(true)`.
#[derive(Clone, Debug, PartialEq)]
pub enum TraceEvent {
    /// A frame got an environment (`REnv`), so a block can capture its locals.
    EnvCreate { env: ObjId, ctx: usize, frame: usize, base: usize, len: usize, mid: Option<Sym> },
    /// The environment's values moved from the stack into the object.
    EnvDetach { env: ObjId, len: usize, reason: DetachReason },
    /// An exception started propagating.
    Raise { exc: Option<ObjId>, class: String, frame: usize, irep: IrepId, pc: usize, line: Option<u32> },
    /// One frame's catch table was consulted; `None` means nothing matched and the frame goes.
    CatchLook { frame: usize, irep: IrepId, pc: usize, line: Option<u32>, matched: Option<CatchHandlerInfo> },
    /// A frame was removed while unwinding.
    FrameUnwound { frame: usize, mid: Option<Sym>, by: UnwindBy },
    /// The running context changed.
    FiberSwitch { from: usize, to: usize, kind: SwitchKind },
    /// A collection finished.
    GcCollect { before_live: usize, after_live: usize, swept: usize, allocated_since: usize },
}

/// The VM as it stands (`Vm::snapshot`).
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub cur: usize,
    pub contexts: Vec<ContextView>,
    /// The exception or `RBreak` being propagated, if any.
    pub pending_exc: Option<ValueView>,
    pub heap: HeapView,
    pub instructions: u64,
    /// Environments reachable from the frames (their own, and those of their procs).
    pub envs: Vec<EnvView>,
}

#[derive(Clone, Debug)]
pub struct ContextView {
    pub index: usize,
    pub status: FiberState,
    pub fiber: Option<ObjId>,
    pub is_current: bool,
    /// `prev` of `docs/fibers.md`: the context a `Fiber.yield` returns to.
    pub prev: Option<usize>,
    pub frames: Vec<FrameView>,
}

#[derive(Clone, Debug)]
pub struct FrameView {
    pub index: usize,
    pub irep: IrepId,
    pub pc: usize,
    pub line: Option<u32>,
    pub mid: Option<String>,
    pub target_class: String,
    pub base: usize,
    pub nregs: usize,
    pub nlocals: usize,
    /// `Cci::Skip`: the frame was pushed by native code, which waits for it.
    pub native_boundary: bool,
    pub env: Option<ObjId>,
    pub proc_: ProcView,
    /// `R0..nregs` of this frame, for the innermost frames only (`regs_frames`).
    pub regs: Vec<RegView>,
}

#[derive(Clone, Debug)]
pub struct ProcView {
    pub id: ObjId,
    pub irep: IrepId,
    /// The proc that created this one; `GETUPVAR` climbs this chain.
    pub upper: Option<ObjId>,
    pub env: Option<ObjId>,
}

#[derive(Clone, Debug)]
pub struct RegView {
    pub index: usize,
    /// Local variable name from the LVAR section (`R1..`), when the program was compiled with `-g`.
    pub name: Option<String>,
    pub value: ValueView,
}

#[derive(Clone, Debug)]
pub struct EnvView {
    pub id: ObjId,
    /// While attached the values live on the context's stack; detached they are in the object.
    pub attached: bool,
    pub len: usize,
    pub ctx: usize,
    pub base: usize,
    pub mid: Option<String>,
    pub values: Vec<ValueView>,
}

#[derive(Clone, Debug)]
pub struct HeapView {
    pub len: usize,
    pub live: usize,
    pub free: usize,
    pub allocated_since_gc: usize,
    pub alloc_threshold: usize,
    pub gc_count: u64,
    pub live_after_gc: usize,
    pub stress: bool,
    pub disabled: bool,
}

/// A value as text, without running Ruby.
#[derive(Clone, Debug, PartialEq)]
pub struct ValueView {
    pub text: String,
    pub class: String,
    /// Heap objects carry their id, so that a register and an environment slot holding the
    /// same object can be linked in a view.
    pub id: Option<ObjId>,
}

impl Vm {
    /// Starts or stops recording [`TraceEvent`]s (off by default).
    pub fn set_trace(&mut self, on: bool) {
        self.trace = if on { Some(Vec::new()) } else { None };
    }
    pub fn tracing(&self) -> bool {
        self.trace.is_some()
    }
    /// Takes what was recorded since the last call (recording stays on).
    pub fn take_trace(&mut self) -> Vec<TraceEvent> {
        match &mut self.trace {
            Some(t) => core::mem::take(t),
            None => Vec::new(),
        }
    }

    /// The VM as it stands. `regs_frames` is how many innermost frames of each context get
    /// their registers (a deep recursion would otherwise make the snapshot huge).
    pub fn snapshot(&self, regs_frames: usize) -> Snapshot {
        let mut contexts = Vec::with_capacity(self.contexts.len());
        let mut envs: Vec<EnvView> = Vec::new();
        for (i, ctx) in self.contexts.iter().enumerate() {
            let is_current = i == self.cur;
            // the running context's registers and frames live in the Vm (swapped on every switch)
            let (stack, ci) = if is_current { (&self.stack, &self.ci) } else { (&ctx.stack, &ctx.ci) };
            let with_regs = ci.len().saturating_sub(regs_frames);
            let frames = ci.iter().enumerate().map(|(f, c)| {
                let irep = &self.ireps[c.irep];
                let regs = if f >= with_regs {
                    (0..irep.nregs).map(|r| RegView {
                        index: r,
                        name: if r >= 1 { irep.lv.get(r - 1).and_then(|n| n.map(|s| self.sym_name(s))) } else { None },
                        value: self.render(stack.get(c.base + r).map(|s| s.get()).unwrap_or(Value::Nil), 2),
                    }).collect()
                } else { Vec::new() };
                let pd = self.heap.proc_data(c.proc_);
                FrameView {
                    index: f,
                    irep: c.irep,
                    pc: c.pc,
                    line: irep.line_of(c.pc),
                    mid: c.mid.map(|m| self.sym_name(m)),
                    target_class: self.class_name(c.target_class),
                    base: c.base,
                    nregs: irep.nregs,
                    nlocals: irep.nlocals,
                    native_boundary: c.cci == Cci::Skip,
                    env: c.env,
                    proc_: ProcView { id: c.proc_, irep: pd.irep, upper: pd.upper, env: pd.env },
                    regs,
                }
            }).collect::<Vec<_>>();
            // environments of these frames, and those their procs captured
            for f in &frames {
                for e in [f.env, f.proc_.env].into_iter().flatten() { self.collect_env(e, &mut envs); }
                let mut up = f.proc_.upper;
                while let Some(p) = up {
                    if !matches!(self.heap.get(p).kind, ObjKind::Proc(_)) { break; }
                    let pd = self.heap.proc_data(p);
                    if let Some(e) = pd.env { self.collect_env(e, &mut envs); }
                    up = pd.upper;
                }
            }
            contexts.push(ContextView { index: i, status: ctx.status, fiber: ctx.fib, is_current, prev: ctx.prev, frames });
        }
        Snapshot {
            cur: self.cur,
            contexts,
            pending_exc: self.exc.map(|e| self.render(e, 2)),
            heap: HeapView {
                len: self.heap.len(),
                live: self.heap.live_count(),
                free: self.heap.len() - self.heap.live_count(),
                allocated_since_gc: self.heap.allocated_since_gc,
                alloc_threshold: self.heap.alloc_threshold,
                gc_count: self.gc_count,
                live_after_gc: self.live_after_gc,
                stress: self.gc_stress,
                disabled: self.gc_disabled,
            },
            instructions: self.instructions,
            envs,
        }
    }

    fn collect_env(&self, id: ObjId, out: &mut Vec<EnvView>) {
        if out.iter().any(|e| e.id == id) { return; }
        let ObjKind::Env(e) = &self.heap.get(id).kind else { return };
        let values = if e.attached {
            let stack = if e.ctx == self.cur { &self.stack } else { &self.contexts[e.ctx].stack };
            (0..e.len).map(|i| self.render(stack.get(e.base + i).map(|s| s.get()).unwrap_or(Value::Nil), 1)).collect()
        } else {
            e.values.iter().map(|v| self.render(v.get(), 1)).collect()
        };
        out.push(EnvView { id, attached: e.attached, len: e.len, ctx: e.ctx, base: e.base, mid: e.mid.map(|m| self.sym_name(m)), values });
    }

    /// A value as text, without calling any Ruby method (`inspect` would run the program).
    pub fn render(&self, v: Value, depth: u8) -> ValueView {
        let class = self.class_name(self.real_class_of(v));
        let id = v.obj();
        let text = self.render_text(v, depth);
        ValueView { text, class, id }
    }

    fn render_text(&self, v: Value, depth: u8) -> String {
        match v {
            Value::Nil => "nil".into(),
            Value::True => "true".into(),
            Value::False => "false".into(),
            Value::Int(i) => i.to_string(),
            Value::Float(f) => crate::builtins::numeric::float_to_s(f),
            Value::Sym(s) => format!(":{}", self.sym_name(s)),
            Value::Obj(o) => {
                if self.heap.is_free(o) { return format!("#<freed {}>", o.0); }
                match &self.heap.get(o).kind {
                    ObjKind::Regexp(_) => format!("#<Regexp {}>", o.0),
                    ObjKind::Data { tag, handle } => format!("#<Data tag={tag} handle={handle}>"),
                    ObjKind::Task(t) => format!("#<Task {} ctx={}>", o.0, t.ctx),
                    ObjKind::MatchData { .. } => format!("#<MatchData {}>", o.0),
                    ObjKind::String(b) => render_string(b),
                    ObjKind::Array(a) => {
                        if depth == 0 { return format!("#<Array len={}>", a.len()); }
                        let shown = a.len().min(8);
                        let mut s = String::from("[");
                        for (i, e) in a[..shown].iter().enumerate() {
                            if i > 0 { s.push_str(", "); }
                            s.push_str(&self.render_text(e.get(), depth - 1));
                        }
                        if a.len() > shown { s.push_str(&format!(", ...(+{})", a.len() - shown)); }
                        s.push(']');
                        s
                    }
                    ObjKind::Hash(h) => {
                        if depth == 0 { return format!("#<Hash size={}>", h.entries.len()); }
                        let shown = h.entries.len().min(4);
                        let mut s = String::from("{");
                        for (i, (k, val)) in h.entries[..shown].iter().enumerate() {
                            if i > 0 { s.push_str(", "); }
                            s.push_str(&format!("{} => {}", self.render_text(k.get(), depth - 1), self.render_text(val.get(), depth - 1)));
                        }
                        if h.entries.len() > shown { s.push_str(&format!(", ...(+{})", h.entries.len() - shown)); }
                        s.push('}');
                        s
                    }
                    ObjKind::Range { begin, end, excl } =>
                        format!("{}{}{}", self.render_text(begin.get(), 0), if *excl { "..." } else { ".." }, self.render_text(end.get(), 0)),
                    ObjKind::Proc(p) => format!("#<Proc irep={}{}{}>", p.irep,
                        if p.strict { " lambda" } else { "" },
                        match p.env { Some(e) => format!(" env=#{}", e.0), None => String::new() }),
                    ObjKind::Env(e) => format!("#<Env len={} {}>", e.len, if e.attached { "attached" } else { "detached" }),
                    ObjKind::Class(c) => {
                        let n = self.class_name(o);
                        if c.is_module { format!("{n} (module)") } else { n }
                    }
                    ObjKind::Exception => {
                        let m = self.heap.ivar_get(o, self.s.mesg);
                        let msg = match m.obj().and_then(|mo| self.heap.string(mo)) { Some(b) => String::from_utf8_lossy(b).into_owned(), None => String::new() };
                        format!("#<{}: {}>", self.class_name(self.real_class_of(v)), msg)
                    }
                    ObjKind::Fiber(ctx) => match self.contexts.get(*ctx) {
                        Some(c) => format!("#<Fiber {:?}>", c.status),
                        None => "#<Fiber uninitialized>".into(),
                    },
                    ObjKind::Break { tag, ci_index, .. } => format!("#<Break {tag:?} frame={ci_index}>"),
                    ObjKind::BigInt(b) => b.to_string_radix(10),
                    ObjKind::Object if o == self.top_self => "main".into(),
                    ObjKind::Object => {
                        let ivars = self.heap.get(o).ivars.len();
                        format!("#<{}:0x{:012x}{}>", self.class_name(self.real_class_of(v)), (o.0 as usize + 1) * 0x40,
                                if ivars > 0 { format!(" ivars={ivars}") } else { String::new() })
                    }
                }
            }
        }
    }

    /// Executions per opcode so far, by name, leaving out the ones never executed.
    pub fn op_histogram(&self) -> Vec<(&'static str, u64)> {
        self.op_counts.iter().enumerate()
            .filter(|(_, c)| **c > 0)
            .filter_map(|(i, c)| crate::opcode::Op::from_u8(i as u8).map(|op| (op.name(), *c)))
            .collect()
    }
}

/// `"..."` with the escapes of `String#inspect`, cut at 64 bytes.
fn render_string(b: &[u8]) -> String {
    let mut s = String::from("\"");
    for &c in b.iter().take(64) {
        match c {
            b'"' => s.push_str("\\\""),
            b'\\' => s.push_str("\\\\"),
            b'\n' => s.push_str("\\n"),
            b'\t' => s.push_str("\\t"),
            b'\r' => s.push_str("\\r"),
            0x20..=0x7e => s.push(c as char),
            _ => s.push_str(&format!("\\x{c:02X}")),
        }
    }
    s.push('"');
    if b.len() > 64 { s.push_str(&format!(" ...(+{} bytes)", b.len() - 64)); }
    s
}
