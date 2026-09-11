use crate::value::Value;

/// Errors that stop or divert execution.
#[derive(Debug)]
pub enum VmError {
    /// A Ruby exception object (`Exception` or subclass) is being raised.
    Raise(Value),
    /// A `return`/`break` is unwinding through native code (mruby throws the
    /// `RBreak` object to the outer `mrb_vm_exec`). Handled by `Vm::run_loop`.
    Break(crate::value::ObjId),
    /// A malformed RITE binary.
    Rite(String),
    /// The VM hit something it does not implement yet.
    Unimplemented(String),
    /// An internal invariant was violated (bug in the VM).
    Internal(String),
}

impl std::fmt::Display for VmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VmError::Raise(v) => write!(f, "uncaught exception {v:?}"),
            VmError::Break(_) => write!(f, "break/return unwinding escaped the VM"),
            VmError::Rite(s) => write!(f, "invalid RITE binary: {s}"),
            VmError::Unimplemented(s) => write!(f, "not implemented: {s}"),
            VmError::Internal(s) => write!(f, "internal error: {s}"),
        }
    }
}

impl std::error::Error for VmError {}

pub type VmResult<T> = Result<T, VmError>;
