//! The reference mruby 4.1.0 compiler as a Rust library: Ruby source in, RITE bytecode out.
//!
//! This crate does not reimplement the compiler. It builds mruby 4.1.0-rc's own
//! `mruby-compiler` (the Prism parser plus mruby's code generator) as C, standalone like
//! the reference `mrbc`, and calls it through a small C shim. The output is byte-for-byte
//! what `mrbc` writes for the same source and options (checked by the golden tests of the
//! repository). The bytecode runs on the [SabiRuby](https://crates.io/crates/sabiruby) VM,
//! or on mruby itself.
//!
//! ```
//! let bin = sabiruby_compiler::compile(b"p 1 + 2", &Default::default()).unwrap();
//! assert_eq!(&bin[..4], b"RITE");
//!
//! let err = sabiruby_compiler::compile(b"def", &Default::default()).unwrap_err();
//! assert_eq!(err.diagnostics[0].kind, sabiruby_compiler::Kind::ParserError);
//! ```
//!
//! Needs a C compiler at build time (the `cc` crate). For `wasm32-wasip1`, use wasi-sdk's clang
//! (`CC_wasm32_wasip1`); the module then needs WebAssembly exception handling (see the README).
//! The vendored sources and their licences are listed in `vendor/VENDOR.md`.

use std::ffi::CString;
use std::fmt;
use std::sync::Mutex;

mod ffi;

/// Compiler options; the fields mirror `mrbc`'s.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// Name recorded in the debug info and used in diagnostics (`mrbc` uses the path given).
    pub filename: String,
    /// `-g`: include the DBG section (line numbers).
    pub debug_info: bool,
    /// `--remove-lv`: drop the LVAR section (local variable names).
    pub remove_lv: bool,
    /// `--no-ext-ops`: do not emit `OP_EXT1..3`.
    pub no_ext_ops: bool,
    /// `--no-optimize`: disable peephole optimisation.
    pub no_optimize: bool,
}

impl Default for Options {
    fn default() -> Options {
        Options { filename: "-e".into(), debug_info: false, remove_lv: false, no_ext_ops: false, no_optimize: false }
    }
}

/// Kind of a diagnostic (`mrc_diagnostic_code`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    ParserWarning,
    ParserError,
    GeneratorWarning,
    GeneratorError,
}

impl Kind {
    pub fn is_error(self) -> bool {
        matches!(self, Kind::ParserError | Kind::GeneratorError)
    }
}

/// A message from the parser or the code generator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub kind: Kind,
    pub message: String,
    pub filename: String,
    pub line: u32,
    pub column: u32,
}

impl fmt::Display for Diagnostic {
    /// `FILE:LINE:COL: message`, as `mrbc` prints it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}: {}", self.filename, self.line, self.column, self.message)
    }
}

/// Compilation failed. `diagnostics` holds the errors, and the warnings reported with them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileError {
    pub diagnostics: Vec<Diagnostic>,
}

impl fmt::Display for CompileError {
    /// The error diagnostics, one per line (warnings are left out, as `mrbc` does).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for d in self.diagnostics.iter().filter(|d| d.kind.is_error()) {
            if !first { writeln!(f)?; }
            write!(f, "{d}")?;
            first = false;
        }
        if first { write!(f, "compile error")?; }
        Ok(())
    }
}

impl std::error::Error for CompileError {}

/// `mrc_presym.c` keeps a global that every parse writes, so compilations are serialised.
static LOCK: Mutex<()> = Mutex::new(());

/// Compiles Ruby source to a RITE binary (what `mrbc` would write to the `.mrb` file).
pub fn compile(src: &[u8], opts: &Options) -> Result<Vec<u8>, CompileError> {
    let internal = |message: &str| CompileError { diagnostics: vec![Diagnostic { kind: Kind::GeneratorError, message: message.into(), filename: opts.filename.clone(), line: 0, column: 0 }] };
    let filename = CString::new(opts.filename.as_str()).map_err(|_| internal("file name contains a NUL byte"))?;
    let mut flags = 0;
    if opts.debug_info { flags |= ffi::DEBUG_INFO; }
    if opts.remove_lv { flags |= ffi::REMOVE_LV; }
    if opts.no_ext_ops { flags |= ffi::NO_EXT_OPS; }
    if opts.no_optimize { flags |= ffi::NO_OPTIMIZE; }
    let (code, bin, diag) = {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        ffi::compile(src, &filename, flags)
    };
    match code {
        ffi::OK => Ok(bin),
        ffi::COMPILE_ERROR => {
            let diagnostics = parse_diagnostics(&diag);
            if diagnostics.is_empty() { Err(internal("compile error")) } else { Err(CompileError { diagnostics }) }
        }
        ffi::DUMP_ERROR => Err(internal("could not write the RITE binary")),
        ffi::NO_MEMORY => Err(internal("out of memory")),
        _ => Err(internal("unexpected result from the compiler")),
    }
}

fn parse_diagnostics(text: &str) -> Vec<Diagnostic> {
    text.split('\u{1e}')
        .filter(|r| !r.is_empty())
        .filter_map(|r| {
            let mut f = r.splitn(5, '\u{1f}');
            let kind = match f.next()? { "0" => Kind::ParserWarning, "1" => Kind::ParserError, "2" => Kind::GeneratorWarning, _ => Kind::GeneratorError };
            let line = f.next()?.parse().unwrap_or(0);
            let column = f.next()?.parse().unwrap_or(0);
            let filename = f.next()?.to_string();
            let message = f.next().unwrap_or("").to_string();
            Some(Diagnostic { kind, message, filename, line, column })
        })
        .collect()
}

/// The compiler this crate embeds, e.g. `"mruby 4.1.0-rc (3cf73ee), Prism 1.9.0"`.
pub fn version() -> &'static str {
    ffi::version()
}
