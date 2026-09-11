//! `sabiruby` command line: run, compile or dump Ruby scripts and mruby RITE binaries (.mrb).
//! Ruby source is compiled by the embedded reference compiler (feature `compiler`).
use std::io::Write;
use std::process::ExitCode;

/// `SABIRUBY_GC_STRESS=1`: collect at every instruction boundary after an allocation.
fn gc_stress() -> bool {
    std::env::var("SABIRUBY_GC_STRESS").map(|v| !v.is_empty() && v != "0").unwrap_or(false)
}

/// Nanoseconds since the first call (the library has no clock of its own).
fn clock_ns() -> u64 {
    static T0: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    T0.get_or_init(std::time::Instant::now).elapsed().as_nanos() as u64
}

fn usage() -> ExitCode {
    eprintln!("usage: sabiruby run [--stats] <file> [args...]   run a script: Ruby source or a RITE binary (.mrb)");
    eprintln!("                                      (--stats: instructions, time and GC to stderr)");
    eprintln!("       sabiruby -e '<code>' [args...]  run Ruby code given on the command line");
    eprintln!("       sabiruby compile <file.rb> [-o <out.mrb>] [-g] [--remove-lv] [--no-ext-ops] [--no-optimize]");
    eprintln!("                                      compile like mrbc (default output: <file>.mrb)");
    eprintln!("       sabiruby dump <file>           print the instruction sequence (.rb or .mrb)");
    eprintln!("       sabiruby mrbtest [-v] <assert.mrb> <test.mrb>...");
    eprintln!("                                      run mruby's test suite, print a Markdown report");
    eprintln!("       sabiruby --version");
    eprintln!("SABIRUBY_GC_STRESS=1: collect at every instruction boundary after an allocation");
    ExitCode::from(2)
}

/// Compiles Ruby source with the embedded reference compiler (crate `sabiruby-compiler`).
/// Errors are printed as `FILE:LINE:COL: message`, like mrbc.
#[cfg(feature = "compiler")]
fn compile_source(src: &[u8], opts: &sabiruby_compiler::Options) -> Result<Vec<u8>, ExitCode> {
    sabiruby_compiler::compile(src, opts).map_err(|e| { eprintln!("{e}"); ExitCode::from(1) })
}

/// A program file: a RITE binary as is, anything else compiled as Ruby source
/// (`debug_info` keeps line numbers and local variable names, as `mrbc -g`).
fn load_program(path: &str, debug_info: bool) -> Result<Vec<u8>, ExitCode> {
    let bytes = std::fs::read(path).map_err(|e| { eprintln!("{path}: {e}"); ExitCode::from(1) })?;
    if bytes.starts_with(b"RITE") { return Ok(bytes); }
    #[cfg(feature = "compiler")]
    { compile_source(&bytes, &sabiruby_compiler::Options { filename: path.to_string(), debug_info, ..Default::default() }) }
    #[cfg(not(feature = "compiler"))]
    { let _ = debug_info; eprintln!("{path}: not a RITE binary (this sabiruby is built without the `compiler` feature)"); Err(ExitCode::from(1)) }
}

/// `sabiruby compile FILE [-o OUT] [-g] [--remove-lv] [--no-ext-ops] [--no-optimize]`.
#[cfg(feature = "compiler")]
fn compile_cmd(args: &[String]) -> ExitCode {
    let mut opts = sabiruby_compiler::Options::default();
    let (mut input, mut output) = (None, None);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-o" => match it.next() { Some(o) => output = Some(o.clone()), None => return usage() },
            "-g" => opts.debug_info = true,
            "--remove-lv" => opts.remove_lv = true,
            "--no-ext-ops" => opts.no_ext_ops = true,
            "--no-optimize" => opts.no_optimize = true,
            f if !f.starts_with('-') && input.is_none() => input = Some(f.to_string()),
            _ => return usage(),
        }
    }
    let Some(input) = input else { return usage() };
    // mrbc replaces the extension; a name without one gets `.mrb` added here
    // (mrbc would reuse the input name and overwrite the source)
    let output = output.unwrap_or_else(|| std::path::Path::new(&input).with_extension("mrb").to_string_lossy().into_owned());
    let src = match std::fs::read(&input) { Ok(b) => b, Err(e) => { eprintln!("{input}: {e}"); return ExitCode::from(1); } };
    opts.filename = input;
    let bin = match compile_source(&src, &opts) { Ok(b) => b, Err(code) => return code };
    let written = if output == "-" { std::io::stdout().write_all(&bin) } else { std::fs::write(&output, &bin) };
    match written {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => { eprintln!("{output}: {e}"); ExitCode::from(1) }
    }
}

#[cfg(not(feature = "compiler"))]
fn compile_cmd(_args: &[String]) -> ExitCode {
    eprintln!("sabiruby compile: this sabiruby is built without the `compiler` feature");
    ExitCode::from(1)
}

/// Runs a RITE binary with `ARGV` = `argv`.
fn run(bin: &[u8], argv: &[String], stats: bool) -> ExitCode {
    let mut vm = sabiruby::Vm::new();
    vm.set_gc_stress(gc_stress());
    if stats { vm.gc_clock = Some(clock_ns); }
    if let Err(e) = vm.load_mrblib() { eprintln!("failed to initialize VM (mrblib): {}", vm.describe_error(&e)); return ExitCode::from(1); }
    // like the `mruby` command: ARGV holds the arguments after the script
    let argv: Vec<sabiruby::Value> = argv.iter().map(|a| vm.str_new(a.as_bytes())).collect();
    let argv = vm.ary_new(argv);
    let n = vm.intern("ARGV");
    vm.heap.class_mut(vm.core.object).consts.insert(n, sabiruby::value::Slot::from(argv));
    let (gc0, gct0) = (vm.gc_count, vm.gc_time_ns);
    let started = std::time::Instant::now();
    let result = vm.load_and_run(bin);
    if stats {
        let e = started.elapsed();
        // before the instructions line: tools/bench.sh reads the last line
        eprintln!("gc: {} collections, {:.3} ms, live: {}, heap slots: {}", vm.gc_count - gc0, (vm.gc_time_ns - gct0) as f64 / 1e6, vm.heap.live_count(), vm.heap.len());
        eprintln!("instructions: {} elapsed_ms: {:.3} ns_per_instruction: {:.1}", vm.instructions, e.as_secs_f64() * 1000.0, e.as_nanos() as f64 / vm.instructions.max(1) as f64);
    }
    let out = std::io::stdout();
    let mut lock = out.lock();
    lock.write_all(vm.take_output().as_slice()).ok();
    lock.flush().ok();
    match result {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{}", vm.describe_error(&e));
            ExitCode::from(1)
        }
    }
}

/// `sabiruby mrbtest [-v] assert.mrb t/*.mrb`: one fresh VM per file, a
/// Markdown table of the `report` counts, and the opcodes never executed.
fn mrbtest(args: &[String]) -> ExitCode {
    let verbose = args.first().map(|a| a == "-v").unwrap_or(false);
    let files: Vec<&String> = args.iter().filter(|a| *a != "-v").collect();
    if files.len() < 2 {
        return usage();
    }
    let assert_mrb = match std::fs::read(files[0]) { Ok(b) => b, Err(e) => { eprintln!("{}: {}", files[0], e); return ExitCode::from(1); } };
    let cap: u64 = 300_000_000;
    let mut rows = Vec::new();
    let mut counts = vec![0u64; sabiruby::opcode::OP_COUNT];
    let mut tot = sabiruby::mrbtest::Summary::default();
    for f in &files[1..] {
        let bin = match std::fs::read(f) { Ok(b) => b, Err(e) => { eprintln!("{f}: {e}"); return ExitCode::from(1); } };
        let name = std::path::Path::new(f).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let sum = match sabiruby::mrbtest::run_file_cfg(&assert_mrb, &bin, cap, verbose, gc_stress()) {
            Ok(s) => s,
            Err(e) => { eprintln!("{name}: {e}"); return ExitCode::from(1); }
        };
        for (i, c) in sum.op_counts.iter().enumerate() { counts[i] += c; }
        tot.total += sum.total; tot.ok += sum.ok; tot.ko += sum.ko; tot.crash += sum.crash; tot.warn += sum.warn; tot.skip += sum.skip;
        if verbose {
            eprintln!("=== {name}");
            eprintln!("{}", String::from_utf8_lossy(&sum.output));
        }
        let note = match (&sum.aborted, sum.timeout) {
            (Some(a), _) => format!("aborted: {}", a.lines().next().unwrap_or("")),
            (None, true) => "timeout".to_string(),
            _ => String::new(),
        };
        rows.push((name, sum, note));
    }
    println!("| file | total | ok | ko | crash | warn | skip | note |");
    println!("|---|---:|---:|---:|---:|---:|---:|---|");
    for (name, s, note) in &rows {
        println!("| {} | {} | {} | {} | {} | {} | {} | {} |", name, s.total, s.ok, s.ko, s.crash, s.warn, s.skip, note);
    }
    println!("| **all** | {} | {} | {} | {} | {} | {} | |", tot.total, tot.ok, tot.ko, tot.crash, tot.warn, tot.skip);
    let never = sabiruby::mrbtest::unexecuted_opcodes(&counts);
    println!();
    println!("Opcodes executed: {} / {} (never: {})", sabiruby::opcode::OP_COUNT - never.len(), sabiruby::opcode::OP_COUNT, never.join(", "));
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    // Native -> VM re-entry recurses on the host stack; give it room.
    let t = std::thread::Builder::new().stack_size(256 << 20).spawn(real_main).expect("spawn");
    t.join().unwrap_or(ExitCode::from(1))
}

fn real_main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("mrbtest") => return mrbtest(&args[2..]),
        Some("compile") => return compile_cmd(&args[2..]),
        Some("--version" | "-v") => {
            println!("sabiruby {}", env!("CARGO_PKG_VERSION"));
            #[cfg(feature = "compiler")]
            println!("compiler: {}", sabiruby_compiler::version());
            return ExitCode::SUCCESS;
        }
        _ => {}
    }
    let stats = args.iter().any(|a| a == "--stats");
    args.retain(|a| a != "--stats");
    if args.len() < 3 {
        return usage();
    }
    match args[1].as_str() {
        "-e" => {
            #[cfg(feature = "compiler")]
            {
                let opts = sabiruby_compiler::Options { filename: "-e".into(), debug_info: true, ..Default::default() };
                match compile_source(args[2].as_bytes(), &opts) { Ok(bin) => run(&bin, &args[3..], stats), Err(code) => code }
            }
            #[cfg(not(feature = "compiler"))]
            { eprintln!("sabiruby -e: this sabiruby is built without the `compiler` feature"); ExitCode::from(1) }
        }
        "run" => match load_program(&args[2], true) {
            Ok(bin) => run(&bin, &args[3..], stats),
            Err(code) => code,
        },
        "dump" => {
            let bin = match load_program(&args[2], false) { Ok(b) => b, Err(code) => return code };
            match sabiruby::rite::parse(&bin) {
                Ok(rite) => { print!("{}", sabiruby::vm::dump(&rite)); ExitCode::SUCCESS }
                Err(e) => { eprintln!("{e}"); ExitCode::from(1) }
            }
        }
        _ => usage(),
    }
}
