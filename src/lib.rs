//! SabiRuby — an mruby 4.1.0 bytecode-compatible virtual machine in Rust.
//!
//! The VM executes RITE 0400 binaries produced by mruby's `mrbc`. It follows the
//! semantics documented in "Deep dive into mruby" (register layout, callinfo,
//! catch handlers, environments) while replacing mruby's C-side implementation
//! choices (boxing, tricolor GC, setjmp) with Rust-native ones.
//!
//! Status: early. See `README.md` for the supported subset.
//!
//! The crate is `no_std` + `alloc`. Rule: no `std::` paths in `src/` outside
//! `src/bin/`; `tools/check_no_std.sh` builds for a bare-metal target to enforce it.
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod error;
pub mod object;
pub mod opcode;
pub mod rite;
pub mod symbol;
pub mod value;
pub mod vm;
pub mod builtins;
pub mod mrbtest;

pub use error::VmError;
pub use value::Value;
pub use vm::{Step, Vm};

/// mruby's core library written in Ruby (`mrblib/*.rb` of 4.1.0-rc), compiled
/// with the reference `mrbc`. Loaded by [`Vm::with_mrblib`].
pub const MRBLIB_MRB: &[u8] = include_bytes!("mrblib.mrb");
/// `mrbgems/mruby-enumerator/mrblib/enumerator.rb` (pure Ruby, needs Fiber), loaded after the core mrblib.
pub const MRBLIB_ENUMERATOR_MRB: &[u8] = include_bytes!("mrblib_enumerator.mrb");
