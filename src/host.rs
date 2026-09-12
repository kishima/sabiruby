//! What the VM asks its host for: compiling a string (`eval`) and reading a file
//! (`require`, later).
//!
//! The VM is `no_std` and knows nothing about the compiler; a program that wants `eval`
//! installs a [`Host`] with [`Vm::set_host`](crate::Vm::set_host). The `sabiruby-compiler`
//! crate provides one (its feature `host`), so the dependency points compiler → VM and the
//! VM crate stays pure Rust. Without a host, `eval` raises NotImplementedError.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

/// What an `eval` string needs beyond its text.
pub struct EvalOptions<'a> {
    /// Name for the diagnostics and the debug info (`(eval)` unless the caller says otherwise).
    pub filename: &'a str,
    /// Line the string starts at (`eval(src, nil, file, line)`).
    pub line: u32,
    /// The local variable names the string can see. `scopes[0]` is the caller, each next one
    /// is further out; within a scope the names are in the order of the irep's local variable
    /// table, with an empty name for a hole (an unnamed parameter), so that a position is a
    /// register index.
    pub scopes: &'a [Vec<Vec<u8>>],
    /// Keep the line numbers (`mrbc -g`).
    pub debug_info: bool,
}

impl Default for EvalOptions<'_> {
    fn default() -> Self {
        EvalOptions { filename: "(eval)", line: 1, scopes: &[], debug_info: true }
    }
}

/// The services a host offers the VM.
pub trait Host {
    /// Ruby source to a RITE binary. `Err` is the compiler's message, which becomes the
    /// text of the SyntaxError.
    fn compile(&mut self, src: &[u8], opts: &EvalOptions) -> Result<Vec<u8>, String>;
    /// Contents of a file, for `require`/`load`; `None` when there is none.
    fn read_file(&mut self, path: &str) -> Option<Vec<u8>> {
        let _ = path;
        None
    }
    /// Whether a file is there (`File.file?`).
    fn file_exists(&mut self, path: &str) -> bool {
        self.read_file(path).is_some()
    }
}

/// The host a [`Vm`](crate::Vm) holds, if any.
pub type HostBox = Option<Box<dyn Host>>;
