# sabiruby-cli

The `sabiruby` command: runs Ruby source and mruby 4.1 bytecode on the
[SabiRuby](https://crates.io/crates/sabiruby) VM, and compiles like `mrbc` with the reference
compiler ([`sabiruby-compiler`](https://crates.io/crates/sabiruby-compiler), built as C, so a C
compiler is needed to install it).

```
cargo install sabiruby-cli

sabiruby run foo.rb [args...]         # compile and run (a .mrb file runs as is)
sabiruby -e 'p [1, 2].sum'            # code on the command line
sabiruby compile foo.rb -o foo.mrb    # like mrbc: -g, --remove-lv, --no-ext-ops, --no-optimize
sabiruby dump foo.rb                  # instruction listing (.rb or .mrb)
sabiruby run --stats foo.rb           # instructions, time and GC statistics to stderr
sabiruby --version
```

The bytecode is byte-identical to what the reference `mrbc` (mruby 4.1.0-rc) writes; compile
errors are printed as `FILE:LINE:COL: message`, as `mrbc` does. `SABIRUBY_GC_STRESS=1` makes
the VM collect garbage after every allocation (for testing).

The library crates are separate so that the VM stays pure Rust and `no_std`: use
[`sabiruby`](https://crates.io/crates/sabiruby) to embed the VM, and add `sabiruby-compiler`
only if the program itself compiles Ruby source.

MIT. See the [repository](https://github.com/kishima/sabiruby) for the design notes.
