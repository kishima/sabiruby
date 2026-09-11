//! `sabiruby` command line: run or dump an mruby RITE binary (.mrb).
use std::io::Write;
use std::process::ExitCode;

fn usage() -> ExitCode {
    eprintln!("usage: sabiruby run <file.mrb>        run a compiled script");
    eprintln!("       sabiruby dump <file.mrb>       print the instruction sequence");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
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
            let mut vm = match sabiruby::Vm::with_mrblib() {
                Ok(vm) => vm,
                Err(e) => {
                    eprintln!("failed to initialize VM: {e}");
                    return ExitCode::from(1);
                }
            };
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
