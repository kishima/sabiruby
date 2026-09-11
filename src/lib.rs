//! SabiRuby — an mruby 4.1.0 bytecode-compatible virtual machine in Rust.
//!
//! The VM executes RITE 0400 binaries (`.mrb` files) produced by mruby 4.1's `mrbc`. This
//! crate is pure Rust and `no_std`; to compile Ruby source in the same program, add the
//! companion crate [`sabiruby-compiler`](https://crates.io/crates/sabiruby-compiler) (the
//! reference compiler built as C). The `sabiruby` command is the crate
//! [`sabiruby-cli`](https://crates.io/crates/sabiruby-cli). Behaviour is checked against the
//! reference mruby 4.1.0-rc (its own test suite passes 1185 of 1227 assertions; see the
//! repository's README for what is missing). The design follows the book *Deep dive into
//! mruby* (register layout, callinfo, catch handlers, environments) and replaces mruby's
//! C-side choices (boxing, tricolor GC, setjmp) with Rust-native ones.
//!
//! # Usage
//!
//! ```
//! # fn run(bytes: &[u8]) -> Result<(), sabiruby::VmError> {
//! let mut vm = sabiruby::Vm::with_mrblib()?; // core classes + mruby's mrblib
//! vm.load_and_run(bytes)?;                   // a RITE binary, run to completion
//! let out = vm.take_output();                // what puts/p/print wrote
//! # let _ = out; Ok(()) }
//! ```
//!
//! Stepped execution, e.g. once per frame of a game loop:
//!
//! ```
//! # fn run(bytes: &[u8]) -> Result<(), sabiruby::VmError> {
//! use sabiruby::Step;
//! let mut vm = sabiruby::Vm::with_mrblib()?;
//! let irep = vm.load(bytes)?;
//! vm.start(irep);
//! loop {
//!     match vm.step(10_000)? {        // at most 10 000 instructions
//!         Step::Paused => { /* next frame */ }
//!         Step::Finished(_) => break,
//!     }
//! }
//! # Ok(()) }
//! ```
//!
//! Native methods are `fn(&mut Vm, self, args, block) -> VmResult<Value>` registered with
//! [`Vm::define_method`]. A native may keep values in Rust locals while it runs; objects kept
//! across calls into the VM must be registered with [`Vm::gc_register`] (see the repository's
//! `docs/gc.md`).
//!
//! # Features
//!
//! The library is `no_std` + `alloc` (it builds for bare-metal targets and
//! `wasm32-unknown-unknown`). The default `std` feature only adds `std::error::Error` for
//! [`VmError`]; `default-features = false` gives the `no_std` library.
//!
//! # Stability
//!
//! 0.x: the API follows the VM's internals and will change between minor versions. Items
//! hidden from these docs are internal even where they are `pub`.
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod error;
pub mod object;
pub mod opcode;
pub mod rite;
pub mod symbol;
pub mod value;
pub mod vm;
#[doc(hidden)]
pub mod builtins;
/// Runner for mruby's own test suite (used by `sabiruby mrbtest` and the crate's tests).
#[doc(hidden)]
pub mod mrbtest;

pub use error::VmError;
pub use value::Value;
pub use vm::{Step, Vm};

/// mruby's core library written in Ruby (`mrblib/*.rb` of 4.1.0-rc), compiled
/// with the reference `mrbc`. Loaded by [`Vm::with_mrblib`].
pub const MRBLIB_MRB: &[u8] = include_bytes!("mrblib.mrb");
/// `mrbgems/mruby-enumerator/mrblib/enumerator.rb` (pure Ruby, needs Fiber), loaded after the core mrblib.
pub const MRBLIB_ENUMERATOR_MRB: &[u8] = include_bytes!("mrblib_enumerator.mrb");
/// The Ruby parts of the *-ext gems, in the reference's gembox order (`mrbgems/default.gembox`).
pub const MRBLIB_SPRINTF_MRB: &[u8] = include_bytes!("mrblib_sprintf.mrb");
pub const MRBLIB_ENUM_EXT_MRB: &[u8] = include_bytes!("mrblib_enum-ext.mrb");
pub const MRBLIB_STRING_EXT_MRB: &[u8] = include_bytes!("mrblib_string-ext.mrb");
pub const MRBLIB_ARRAY_EXT_MRB: &[u8] = include_bytes!("mrblib_array-ext.mrb");
pub const MRBLIB_HASH_EXT_MRB: &[u8] = include_bytes!("mrblib_hash-ext.mrb");
pub const MRBLIB_RANGE_EXT_MRB: &[u8] = include_bytes!("mrblib_range-ext.mrb");
pub const MRBLIB_PROC_EXT_MRB: &[u8] = include_bytes!("mrblib_proc-ext.mrb");
pub const MRBLIB_METHOD_MRB: &[u8] = include_bytes!("mrblib_method.mrb");
