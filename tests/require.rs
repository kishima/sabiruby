//! `require`/`load` (`docs/plans/eval-require-plan.md` 5). mruby has none of it, so there is no
//! reference output to compare with: every expectation below was measured with CRuby 3.2
//! (`ruby`) on the same files, and the comment says where the two differ on purpose.
//!
//! The library files live in a temporary directory, which becomes `$LOAD_PATH`.

use std::path::{Path, PathBuf};

struct Dir(PathBuf);
impl Dir {
    fn new(name: &str) -> Dir {
        let p = std::env::temp_dir().join(format!("sabiruby-require-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).expect("mkdir");
        Dir(p)
    }
    fn file(&self, name: &str, content: &str) -> &Dir {
        std::fs::write(self.0.join(name), content).expect("write");
        self
    }
    fn path(&self) -> &str { self.0.to_str().unwrap() }
}
impl Drop for Dir {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

/// Runs `src` with `dir` as the only entry of `$LOAD_PATH` and answers what it printed,
/// with `<error: …>` appended where it raised.
fn run(dir: &Dir, src: &str) -> String {
    let bin = sabiruby_compiler::compile(src.as_bytes(), &sabiruby_compiler::Options {
        filename: "(test)".into(), debug_info: true, ..Default::default()
    }).expect("compile");
    let mut vm = sabiruby::Vm::new();
    vm.set_host(Box::new(sabiruby_compiler::Compiler::new()));
    vm.load_mrblib().expect("mrblib");
    vm.set_load_path(&[dir.path()]);
    let r = vm.load_and_run(&bin);
    let mut out = String::from_utf8_lossy(&vm.take_output()).into_owned();
    if let Err(e) = r { out.push_str(&format!("<error: {}>\n", vm.describe_error(&e))); }
    out
}

#[test]
fn require_loads_once() {
    let d = Dir::new("once");
    d.file("lib1.rb", "$loaded = ($loaded || 0) + 1\nVALUE = 42\n");
    let out = run(&d, r#"
      n = $LOADED_FEATURES.size
      p require("lib1")
      p $LOADED_FEATURES.size - n
      p require("lib1")
      p [VALUE, $loaded]
    "#);
    assert_eq!(out, "true\n1\nfalse\n[42, 1]\n");
}

#[test]
fn load_runs_every_time() {
    let d = Dir::new("load");
    d.file("lib1.rb", "$loaded = ($loaded || 0) + 1\n");
    // `load` answers true each time and does not touch $LOADED_FEATURES, as in CRuby
    let out = run(&d, r#"
      n = $LOADED_FEATURES.size
      p load("lib1.rb")
      p load("lib1.rb")
      p $loaded
      p $LOADED_FEATURES.size - n
    "#);
    assert_eq!(out, "true\ntrue\n2\n0\n");
}

#[test]
fn a_missing_file_is_a_load_error() {
    let d = Dir::new("missing");
    let out = run(&d, r#"
      begin; require "nope"; rescue LoadError => e; p [e.class, e.message]; end
      begin; load "nope.rb"; rescue LoadError => e; p e.message; end
      begin; require "./nosuch.rb"; rescue LoadError => e; p e.message; end
    "#);
    assert_eq!(out, "[LoadError, \"cannot load such file -- nope\"]\n\
                     \"cannot load such file -- nope.rb\"\n\
                     \"cannot load such file -- ./nosuch.rb\"\n");
}

#[test]
fn a_cycle_stops() {
    let d = Dir::new("cycle");
    d.file("cyc_a.rb", "$order << \"a-start\"\nrequire \"cyc_b\"\n$order << \"a-end\"\n");
    d.file("cyc_b.rb", "$order << \"b-start\"\nrequire \"cyc_a\"\n$order << \"b-end\"\n");
    // the feature is recorded before the file runs, so the second `require "cyc_a"` answers
    // false instead of looping (CRuby answers the same order)
    let out = run(&d, "$order = []\nrequire \"cyc_a\"\np $order\n");
    assert_eq!(out, "[\"a-start\", \"b-start\", \"b-end\", \"a-end\"]\n");
}

#[test]
fn a_raise_inside_the_library_propagates_and_unrecords_it() {
    let d = Dir::new("raise");
    d.file("boom.rb", "raise \"boom from the library\"\n");
    let out = run(&d, r#"
      begin; require "boom"; rescue RuntimeError => e; p e.message; end
      p $LOADED_FEATURES.any? { |f| f.end_with?("boom.rb") }
      begin; require "boom"; rescue RuntimeError => e; p e.message; end
    "#);
    assert_eq!(out, "\"boom from the library\"\nfalse\n\"boom from the library\"\n");
}

#[test]
fn bytecode_is_preferred_and_run_as_it_stands() {
    let d = Dir::new("mrb");
    d.file("both.rb", "p :from_rb\n");
    let bin = sabiruby_compiler::compile(b"p :from_mrb\n", &sabiruby_compiler::Options {
        filename: "both.rb".into(), debug_info: true, ..Default::default()
    }).expect("compile");
    std::fs::write(Path::new(d.path()).join("both.mrb"), &bin).expect("write");
    // a bare name tries `.mrb` first, an explicit extension is taken as written
    let out = run(&d, "p require(\"both\")\np require(\"both.rb\")\n");
    assert_eq!(out, ":from_mrb\ntrue\n:from_rb\ntrue\n");
}

#[test]
fn a_broken_file_says_which_it_was() {
    let d = Dir::new("broken");
    d.file("bad.rb", "def (\n");
    d.file("badbin.mrb", "RITE0300xxxxxxxx");
    let out = run(&d, r#"
      begin; require "bad"; rescue SyntaxError => e; p e.message; end
      begin; require "badbin"; rescue LoadError => e; p e.message; end
    "#);
    assert!(out.contains("bad.rb line 1:"), "{out}");
    assert!(out.contains("invalid RITE version 0300"), "{out}");
}

#[test]
fn the_built_in_gems_are_already_features() {
    let d = Dir::new("gems");
    // a build that linked the gem answers false for `require` of it; here they are all linked
    let out = run(&d, "p require(\"fiber\")\np require(\"regexp\")\np require(\"require\")\n");
    assert_eq!(out, "false\nfalse\nfalse\n");
}

#[test]
fn the_argument_is_converted_the_way_ruby_does() {
    let d = Dir::new("arg");
    d.file("lib1.rb", "");
    let out = run(&d, r#"
      class P; def to_str; "lib1"; end; end
      p require(P.new)
      begin; require 1; rescue TypeError => e; p e.message; end
    "#);
    assert_eq!(out, "true\n\"no implicit conversion of Integer into String\"\n");
}

#[test]
fn the_load_path_is_searched_in_order() {
    let d = Dir::new("order");
    std::fs::create_dir_all(Path::new(d.path()).join("a")).expect("mkdir");
    std::fs::create_dir_all(Path::new(d.path()).join("b")).expect("mkdir");
    std::fs::write(Path::new(d.path()).join("a/dup.rb"), "WHICH = :a\n").expect("write");
    std::fs::write(Path::new(d.path()).join("b/dup.rb"), "WHICH = :b\n").expect("write");
    // the first entry that has the file wins, and the list is an ordinary Array the program may
    // change while it runs
    let out = run(&d, &format!(r#"
      $LOAD_PATH.clear
      $LOAD_PATH << "{0}/a" << "{0}/b"
      p require("dup")
      p WHICH
      p $LOAD_PATH.size
    "#, d.path()));
    assert_eq!(out, "true\n:a\n2\n");
}

#[test]
fn a_library_may_require_another() {
    let d = Dir::new("nested");
    d.file("outer.rb", "$outer = 1\nrequire \"inner\"\n");
    d.file("inner.rb", "$inner = 1\n");
    // both are recorded, and a name below a directory is a path like any other
    std::fs::create_dir_all(Path::new(d.path()).join("sub")).expect("mkdir");
    std::fs::write(Path::new(d.path()).join("sub/deep.rb"), "SUB = 1\n").expect("write");
    let out = run(&d, r#"
      n = $LOADED_FEATURES.size
      require "outer"
      p [$outer, $inner]
      p $LOADED_FEATURES.size - n
      p require("sub/deep")
      p SUB
    "#);
    assert_eq!(out, "[1, 1]\n2\ntrue\n1\n");
}

#[test]
fn a_file_runs_in_a_top_level_scope_of_its_own() {
    let d = Dir::new("scope");
    d.file("deflib.rb", "def helper; :from_lib; end\nLOCAL_ONLY = (x = 5)\n");
    // what the file defines is visible; the local variables it used are not, and `require` works
    // from inside a method as well as at the top level
    let out = run(&d, r#"
      require "deflib"
      p helper
      p defined?(x)
      p LOCAL_ONLY
      def in_method; require("deflib"); end
      p in_method
    "#);
    assert_eq!(out, ":from_lib\nnil\n5\nfalse\n");
}

#[test]
fn an_absolute_path_is_taken_as_written() {
    let d = Dir::new("abs");
    d.file("lib1.rb", "ABS = 1\n");
    // a name that says where it lives is not looked for in $LOAD_PATH
    let out = run(&d, &format!(r#"
      $LOAD_PATH.clear
      p require("{0}/lib1.rb")
      p ABS
      p require("{0}/lib1.rb")
      begin; require "lib1"; rescue LoadError => e; p e.message; end
    "#, d.path()));
    assert_eq!(out, "true\n1\nfalse\n\"cannot load such file -- lib1\"\n");
}

#[test]
fn load_takes_bytecode_too() {
    let d = Dir::new("loadmrb");
    let bin = sabiruby_compiler::compile(b"$n = ($n || 0) + 1\n", &sabiruby_compiler::Options {
        filename: "counter.rb".into(), debug_info: true, ..Default::default()
    }).expect("compile");
    std::fs::write(Path::new(d.path()).join("counter.mrb"), &bin).expect("write");
    let out = run(&d, "p load(\"counter.mrb\")\np load(\"counter.mrb\")\np $n\n");
    assert_eq!(out, "true\ntrue\n2\n");
}

#[test]
fn without_a_host_there_is_no_require() {
    let d = Dir::new("nohost");
    d.file("lib1.rb", "");
    let bin = sabiruby_compiler::compile(b"require 'lib1'\n", &sabiruby_compiler::Options {
        filename: "(test)".into(), debug_info: true, ..Default::default()
    }).expect("compile");
    let mut vm = sabiruby::Vm::with_mrblib().expect("vm");
    vm.set_load_path(&[d.path()]);
    // `__file_exist?` answers false without a host, so the search ends in a LoadError
    let e = vm.load_and_run(&bin).expect_err("no host");
    assert!(vm.describe_error(&e).contains("cannot load such file -- lib1"), "{}", vm.describe_error(&e));
}
