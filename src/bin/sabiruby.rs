//! `sabiruby` command line: run or dump an mruby RITE binary (.mrb).
use std::io::Write;
use std::process::ExitCode;

fn usage() -> ExitCode {
    eprintln!("usage: sabiruby run <file.mrb>        run a compiled script");
    eprintln!("       sabiruby dump <file.mrb>       print the instruction sequence");
    eprintln!("       sabiruby mrbtest [-v] <assert.mrb> <test.mrb>...");
    eprintln!("                                      run mruby's test suite, print a Markdown report");
    ExitCode::from(2)
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
        let sum = match sabiruby::mrbtest::run_file(&assert_mrb, &bin, cap) {
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
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 2 && args[1] == "mrbtest" {
        return mrbtest(&args[2..]);
    }
    if args.len() < 3 {
        return usage();
    }
    let bin = match std::fs::read(&args[2]) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{}: {}", args[2], e);
            return ExitCode::from(1);
        }
    };
    match args[1].as_str() {
        "dump" => {
            let rite = match sabiruby::rite::parse(&bin) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("{e}");
                    return ExitCode::from(1);
                }
            };
            print!("{}", sabiruby::vm::dump(&rite));
            ExitCode::SUCCESS
        }
        "run" => {
            let mut vm = sabiruby::Vm::new();
            if let Err(e) = vm.load_and_run(sabiruby::MRBLIB_MRB) {
                eprintln!("failed to initialize VM (mrblib): {}", vm.describe_error(&e));
                return ExitCode::from(1);
            }
            let result = vm.load_and_run(&bin);
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
        _ => usage(),
    }
}
