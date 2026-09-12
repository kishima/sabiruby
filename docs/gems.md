# Ported gems

How a gem of the reference tree becomes part of SabiRuby, and what each ported one taught.

## Recipe

1. **Natives**: the gem's `src/*.c` methods go into `src/builtins/ext_<gem>.rs` (or
   `fiber.rs`), registered after the core natives in `builtins::init` so they replace core
   natives of the same name, as the gem's `mrb_..._gem_init` does in the reference.
2. **Ruby part**: `tools/mrbtest.sh` concatenates the gem's `mrblib/*.rb`, compiles it with the
   reference `mrbc` into `src/mrblib_<gem>.mrb`, and `Vm::with_mrblib` loads it after the core
   mrblib **in the order of `mrbgems/default.gembox`** (`lib.rs`, `vm.rs`). The order matters
   when two gems define the same method: mruby-enumerator's `Enumerable#zip` replaces
   mruby-enum-ext's because it is initialised later.
3. **Tests**: add the gem to `GEMS` in `tools/mrbtest.sh`; its `test/*.rb` become
   `tests/mrbtest/gem_<file>.mrb` and run like the core files. C test helpers (`test/*.c`)
   are re-implemented in `src/mrbtest.rs`.
4. **Core natives that only exist because of a gem must go.** SabiRuby's core stage had
   `Array#zip`, `Kernel#to_enum` (a NotImplementedError stub), `Range#count`, `Hash#count`
   as natives; in the reference those are gem mrblib Ruby methods (or `Enumerable`'s). A
   native on the class shadows the Ruby definition inherited through `Enumerable`, so the
   gem's method never runs. Rule: before writing a native, check whether the reference
   defines the method in C at all.

## What each gem needed

* **mruby-fiber** — see `docs/fibers.md`.
* **mruby-enumerator** — pure Ruby; needed the `send`/`__send__` in-place dispatch,
  `initialize_copy` on `dup`/`clone`, and the removal of the `to_enum` stub.
* **mruby-array-ext** — 34 natives (`ext_array.rs`). The set operations (`-`, `|`, `&`,
  `difference`, `union`, `intersection`, `intersect?`, `__uniq`) compare with `eql?`, not
  `==` (the reference uses a khash set keyed by `hash`/`eql?`; here a linear walk with
  `eql?`). `__combination_init/next` and `__product_generate/next` keep their state in a
  plain Array instead of an RData. `insert` and `__fill_exec` refuse sizes above
  `ARY_MAX_SIZE` (2^28 here) with the reference's "array size too big".
* **mruby-enum-ext** — `__minmax` and `__count` on Array. `Kernel#<=>` (`mrb_obj_cmp`:
  0 for the same object or `==`, else nil) was missing and is needed by `min_by`/`max_by`.
  `OP_EQ` must answer true for the same object *before* sending `==` (`mrb_obj_eq`);
  SabiRuby only did that for immediates, which broke `count(obj)` on an object whose `==`
  is always false.
* **mruby-hash-ext** — `values_at`, `slice`, `slice!`, `except`, `key`, `__merge`,
  `Hash.[]`, plus the core `__compact` the gem's Ruby `compact!` calls.
* **mruby-range-ext** — `cover?` (range-in-range rules), `size` (Float-aware, with the
  reference's epsilon), `__empty_range?`. Its Ruby `first`/`last`/`min`/`max` replace the
  core natives. Note: with this gem loaded, `(1..).last` raises RangeError, and the core
  test `Range#last` (`assert_nil (1..).last`) fails on the reference `mruby` as well.
* **mruby-string-ext** — 56 natives (`ext_string.rs`) for the byte-string build. The
  `tr`/`delete`/`squeeze`/`count` pattern parser is ported byte for byte, including two
  quirks of the reference: the bitmap used by `delete`/`squeeze`/`count` treats a range
  `a-c` as *exclusive* of `c` (`tr_compile_pattern` loops `i < ch[1]`), while `tr` itself
  is inclusive; and replacement bytes are read as signed `char`, so a byte ≥ 0x80 in the
  replacement deletes. `String#succ` follows `str_succ_bang`: the rightmost alphanumeric
  steps and carries across non-alphanumerics, but a letter never carries into a digit nor a
  digit into a letter (`"1-z".succ == "1-aa"`, `"1.9".succ == "2.0"`). `Integer#chr` accepts
  only `ASCII-8BIT`/`BINARY` (a UTF-8 name is an ArgumentError in this build).

## Deviations kept

* NaN identity: every NaN is one immediate here, the reference allocates one object per NaN
  (`[nan].uniq`, `[nan].count(nan)`, `[nan] - [nan]`).
* `String#slice!`, `tr` and friends work on bytes; the multibyte tests skip themselves.
* Wide integers (mruby-bigint): five answers of the reference are slips of its own bigint code,
  not decisions, and SabiRuby keeps the meaning the same at both widths
  (`tests/custom/bigint_reference_bugs.rb` holds them with CRuby's answers):
  `~x` is `-x-1` (`mrb_bint_rev` takes one off the *magnitude*, answering `-(x-1)` for `x > 0`);
  `x >> n` floors for a negative `x` (`mpz_div_2exp` shifts the magnitude, truncating toward
  zero); `x.div(f)` divides by a Float (`mrb_bint_div` multiplies by it); `x % f` floors like
  `Integer#%` (`mrb_bint_mod` takes `fmod`, which truncates); and `x.dup` is the number (the
  reference's `mrb_obj_dup` copies an RInteger as an empty object and answers 0 — visible there
  for `(2**62).dup`, which is wide in its build and immediate here). The bit operations work over
  one limb more than the longer operand, which the reference does not, so a result needing the
  extra limb (`-1 ^ 0xffffffff`) is not truncated.
* `Integer#quo` on a wide integer answers a Float; the reference answers a Rational (its
  `int_quo` uses mruby-rational, which is not ported yet — and answers `(0/1)` for
  `(2**64).quo(2)`, which is wrong there too).
* `ObjectSpace.count_objects` has a `T_BIGINT` entry, counted after `T_BREAK` as in the
  reference's type enum.
* **mruby-sprintf** — `Kernel#sprintf`/`format` (`ext_sprintf.rs`): the reference's state machine
  for flags, `n$`, `<name>`/`{name}` and `*`, with its error messages; integers follow
  `mrb_int_to_cstr`/`mrb_uint_to_cstr` including the `..f` two's-complement form for negative
  `%x`/`%o`/`%b`; floats are rendered with `core::fmt` (correct rounding) and reshaped to C's
  `%f`/`%e`/`%g` (exponent of at least two digits, `%g` trailing-zero removal, `#` keeps them).
  `String#%` is the gem's Ruby part. The core-stage `String#%` stub was removed.
* **mruby-metaprog** (`ext_metaprog.rs`) — instance/class variable name checks (`'@0' is not
  allowed as an instance variable name`), `methods`/`instance_methods` families with the
  `regular`/`inherit` argument and the reference's listing rule (first table wins, `undef`
  hides), `singleton_methods(recur)`, `local_variables` from the caller's irep names,
  `included_modules`, `constants(inherit)`, `Module.constants`, `Module.nesting`,
  `remove_method` (+ `method_removed` hook), `undefined_instance_methods`, `public_send` with
  `method_missing` fallback. Two structural fixes came with it: the natives SabiRuby's core stage
  had defined on **Object** are moved to **Kernel** at init (the reference defines them in
  `mrb_init_kernel`; owners, listings and `Kernel.instance_method(:inspect)` depend on it), and
  `Vm::funcall` now dispatches a user-defined `method_missing` like a SEND does.
* **mruby-proc-ext** (`ext_proc.rs`) — `Proc#inspect` (`#<Proc:0x... -:-> (lambda)`),
  `lambda?`, `parameters` (from the ENTER operand and the irep's local names),
  `source_location` (nil), `Kernel#proc`; `curry`, `<<`, `>>`, `===`, `yield` are Ruby.
  `proc { break }.call` raising LocalJumpError needed the orphan rule for natives: when a native
  returns, a non-strict block made by the calling frame and passed to it is marked orphan
  (`Vm::orphan_block_of_native`; the reference does it in `cipop` of the C frame).
* **mruby-method** (`ext_method.rs`) — `Method`/`UnboundMethod` as plain objects with the
  reference's ivars `_owner`, `_recv`, `_name`, `_proc`, `_klass` (plus `_missing` for
  `respond_to_missing?` methods). A native method has no Proc object here: it is looked up again
  by owner and name when called or compared (two natives are `==` when they are the same
  function). Native arity comes from a small table keyed by function (`Vm::native_arity`),
  `-1` otherwise, because SabiRuby natives carry no `MRB_ARGS_*` spec.

* **mruby-compar-ext**, **mruby-toplevel-ext**, **mruby-enum-lazy** — pure Ruby, loaded as is. The
  core stage had a native `clamp` on Comparable and on Integer; in the reference the only `clamp` is
  this gem's Ruby method, so both natives went (rule 4: the native took two arguments and shadowed the
  one-argument range form).
* **mruby-enum-chain** — pure Ruby. Its `rewind` test defines a singleton method on a Range literal,
  which uncovered a deviation: SabiRuby marked every Range **frozen** and used that flag as the
  reference's `RANGE_INITIALIZED` flag (`'initialize' called twice`). In the reference `(1..2).frozen?`
  is false, and since the singleton class of a frozen object is frozen too (`sc->frozen = o->frozen`),
  `define_method` on the range's singleton class raised FrozenError here. Now an uninitialised Range is
  a plain object until `initialize`/`initialize_copy` gives it its ends, and no Range is frozen.
  `Enumerator::Chain#size` with a Range fails on the reference as well (mruby-range-ext defines
  `Range#size`, the test expects nil).

* **mruby-object-ext** (`ext_object.rs`) — `NilClass#to_a/to_h/to_i/to_f`, `Kernel#itself`,
  `BasicObject#instance_exec`; `tap`, `then`/`yield_self` are its Ruby (the core-stage natives
  `tap`/`then` went). `instance_exec` on an Integer uncovered a rule of `mrb_singleton_class_ptr`:
  an Integer/Float/Symbol has no singleton class, the frame then has no target class of its own
  and `class B` inside the block goes to the block's lexical class (`Vm::call_block_with_self_kw`
  passes no override). Keywords given to `instance_exec` stay keywords for the block (the
  pending-kdict rule of `send`).
* **mruby-symbol-ext** (`ext_symbol.rs`) — `length`/`size` (bytes in this build), `slice`/`[]`,
  which hand the name to `String#slice` as the reference does; `Comparable`, `capitalize`,
  `casecmp`, `empty?`, `intern` are its Ruby (the core-stage native `empty?` went).
* **mruby-kernel-ext** (`ext_kernel.rs`) — `Integer()`/`Float()` are the reference's scanners
  ported byte for byte (`mrb_str_len_to_integer`, `mrb_str_len_to_dbl`, `mrb_read_float`; the
  float's value is parsed by `core` from the exact span, so rounding is right). `caller` is the
  reference's arithmetic over `Vm::backtrace`, new here: one `file:line:in method` entry per Ruby
  frame with debug info, the native itself first, located at the frame that called it (the
  reference locates a C frame at the nearest Ruby frame below it the same way). `__method__`
  reads the frame: its `mid`, else the environment's (a block answers the method it was written
  in). Two things came with it: **an alias is a proc of its own** (`ProcData::mid`, the
  reference's `MRB_PROC_ALIAS` with `body.mid`) so a frame of `alias m3 m1` has `mid` `:m1`,
  which `__method__`, `__callee__` and `super` see; and `raise`/`fail` is one function. The
  core-stage `Integer()`, `Float()`, `String()`, `Array()`, `__method__` moved here.
* **mruby-class-ext** (`ext_class.rs`) — `Module#<`, `<=`, `<=>`, `>`, `>=` with the reference's
  three answers (true, false, nil when unrelated; a TypeError for a non-module), `class_exec`/
  `module_exec` (keywords kept), `name` (frozen), `singleton_class?`; `Class#attached_object`,
  `subclasses` (a heap walk over `Heap::ids`). The core-stage `Module#<`, `<=`, `name` moved here.
* **mruby-numeric-ext** (`ext_numeric.rs`) — `remainder`, `pow(b, m)` with the reference's
  overflow rule, `digits`, `size`, `bit_length`, `odd?`, `even?`, `gcd`, `lcm`, `modulo`,
  `Integer.sqrt`, `Float#remainder`/`modulo`, `Float::EPSILON`.. constants. `zero?`, `nonzero?`,
  `positive?`, `negative?`, `integer?`, `allbits?`, `ceildiv` are its Ruby: the core-stage natives
  of those went, and `even?`, `odd?`, `size`, `bit_length`, `gcd` moved here. The core `Float#div`
  (`flo_idiv`) was missing and is in `numeric.rs` now.
* **mruby-objectspace** (`ext_objectspace.rs`) — `count_objects` (TOTAL = every slot, FREE = the
  swept ones, then the live `T_*` counts in the reference's type order), `each_object`. Its test
  counts hashes across a `GC.start`, which exposed **the stack root**: SabiRuby marked the whole
  register vector, so registers left over from returned frames kept garbage alive. Now the
  running frame's window is the limit (`base + nregs`, or the arguments still to be packed),
  as `mark_context_stack` marks `ci->stack + nregs`. The test's C helper `__gc_root_survivors`
  (`mrb_gc_register` counting) is in `mrbtest.rs`; it found that `Heap::is_free` said "live" for
  a slot the sweep had truncated away.
* **mruby-catch** (`ext_catch.rs`) — `catch` records its tag and the depth of the block's frame
  in `Vm::catch_tags`; `throw` finds the innermost entry with the same object and returns a
  `Break` to that frame, the mechanism a `return` from a nested block already uses, so `ensure`
  bodies run and `rescue Exception` does not see it (the reference raises an `RBreak` aimed at
  the frame of its bytecode `catch`). An unmatched tag raises the gem's `UncaughtThrowError`.

* **mruby-math** (`ext_math.rs`) — the `Math` module over `libm` (already a dependency), the
  reference's domain checks as `Math::DomainError`; module functions. Output is identical to
  the reference for the values tried (`frexp`, `ldexp`, `cbrt`, `log(x, base)`, ...).
* **mruby-random** (`ext_random.rs`) — PCG-XSH-RR with the reference's seeding, so
  `Random.new(123)` gives the reference's sequence (checked: `rand(1000)`, `rand`, `bytes`,
  `shuffle`, `sample` with the same seeds). The state lives in two hidden instance variables
  (`__state`, `__seed`) rather than an `MRB_TT_ISTRUCT` payload. The default generator is
  seeded with a constant (the VM has no clock; the reference uses `time(NULL)`), so its
  sequence repeats across runs until `srand`; `srand` without a seed mixes the host's
  `gc_clock` when there is one.

* **mruby-struct** (`ext_struct.rs`) — a struct instance is the reference's `MRB_TT_STRUCT`, an
  array-shaped object whose class is the struct class: here an `ObjKind::Array` with that class,
  so `mrb_ary_set`/`mrb_ary_replace` are the array operations on it (VM paths keyed on the array
  kind, such as a splat, see the members, as the reference's `to_a` gives them anyway). The member
  accessors are one native each way that reads the name it was called by (`Vm::native_mid`, set
  at every native dispatch) instead of the reference's per-member C procs carrying an index. The
  constructor logic (`struct_init_body`, the overridden-`initialize` bridge `__struct_init_fwd`,
  `keyword_init`) is the reference's; "keywords given" is the pending-kdict rule. A struct's
  recursive `inspect` uncovered a shortcut in `Vm::inspect`: the recursion mark was chosen by
  storage kind (`[...]` for anything array-shaped); it now goes by class, so a Struct or Set
  answers through its own `inspect`.
* **mruby-data** (`ext_data.rs`) — the same shape, frozen once built; `Data.define`, keyword
  construction (`missing keyword`/`unknown keyword` are ArgumentErrors), `with`, the bridge
  `__init_with_kw` for an overridden `initialize`.
* **mruby-set** (`ext_set.rs`) — a Set is a Hash-shaped object (`ObjKind::Hash`, element => true)
  with the class `Set` (`instance_kind`), so membership follows the Hash's `hash`/`eql?` rule and
  elements come out in insertion order (the reference's khash walks its buckets). Deviations: an
  unfrozen String element is stored as a frozen copy (the Hash key rule); the reference's
  "uninitialized Set" state and its rebuild-during-`eql?` RuntimeError (GHSA-4jw6-mq65-g3c8,
  one failing assertion) do not exist here, the lookup simply finishes. `Set#hash` is the
  reference's xor fold over 32-bit element hashes.

* **mruby-time** (`ext_time.rs`) — `sec`/`nsec`/zone in hidden instance variables (`__sec`,
  `__nsec`, `__utc`); the calendar (`gmtime`, `timegm`) is computed in the VM (days-from-civil
  and back). There is no `localtime`: the local zone is UTC with the offset `+0000` (no zone
  database in a no_std VM), so `Time.local` and `Time.utc` differ only in `utc?`, `zone` and the
  `to_s` suffix; `utc_offset` is 0 and `dst?` false. `Time.now` reads the host's `Vm::wall_clock`
  (the CLI sets it from `SystemTime`); without one it is the epoch. Checked against the reference:
  negative times, leap years, day overflow (`Time.gm(2024, 2, 30)`), the float/usec rounding of
  `Time.at`, `-` between Times.

* **mruby-bigint** (`src/bigint.rs`, the branches in `src/builtins/numeric.rs`) — the only
  gem that changes what a core type *is*: an Integer that leaves the `i64` range stops being an
  immediate and becomes a heap object, so every arithmetic, comparison, conversion and hash path
  of Integer has a second width to answer for. What it needed:

  * **Representation** — `ObjKind::BigInt(BigInt)`, a sign and the absolute value in 32-bit
    limbs (the reference's `mpz_t`: `sn`, `p[0..sz]`), with the class `Integer`, so `class_of`,
    `is_a?` and `inspect` need no special case. The reference's embedded-limb optimization
    (`RBIGINT_EMBED_SIZE_MAX`) is not copied. **Normalization is the invariant the rest relies
    on**: a result that fits in an `i64` is always turned back into `Value::Int`
    (`Vm::bint_value`, the reference's `bint_norm`), so one number never has two shapes and
    `==`, `eql?`, `hash` and Hash keys stay simple.
  * **The loader** — pool type 7 (`IREP_TT_BIGINT`) is `len, base, digits[len]`, and
    `load.c`'s `pool_data_len = len + 2` counts the length byte itself. `src/rite.rs` read
    `len + 2` bytes *after* the length byte, one too many, and the next pool entry's type byte
    was then read as a digit (`bad pool type 56`, the `'8'` of a literal). A negative base means
    a negative number (`mrb_bint_new_str`). This is why `test/t/literals.rb` skipped.
  * **The arithmetic** (`src/bigint.rs`) — schoolbook multiplication and Knuth's algorithm D for
    division, not the reference's Karatsuba/Barrett/Montgomery: `tools/bench.sh` shows the
    integer benchmarks unchanged (the VM's `checked_*` fast path is untouched and only its
    overflow exit reaches here), and no test is slow. `tests/bigint.rs` checks the layer against
    `i128` for everything that fits and against known values (`3**1000`, `Integer.sqrt(10**40)`,
    base 2..36 round trips) for what does not.
  * **The core branches** — `+ - * / div % divmod ** pow -@ ~ & | ^ << >> <=> == eql? hash to_s
    inspect to_f succ pred chr floor ceil round truncate quo fdiv size bit_length digits gcd lcm
    even? odd? remainder pow(b,m) Integer.sqrt`, `Float#to_i`/`floor`/`ceil`/`round`/`divmod`
    (a Float beyond the `i64` range now converts instead of raising), `String#to_i`/`hex`/`oct`,
    `Integer()`, `sprintf` (`%d`/`%x`/`%o`/`%b` and the `..f` form for a negative wide value),
    `rand(big)`, `Time.at(big)`, and `Vm::expect_int`, which answers the reference's RangeError
    `integer out of range` wherever a wide integer reaches something that needs a machine
    integer (an Array index, a shift width, an exponent).
  * **What stopped raising** — `RangeError: integer overflow` is gone from `+`, `-`, `*`, `**`,
    `<<`, `MRB_INT_MIN / -1` and `-@`; those now promote, in `op_arith`'s overflow exit as well
    as in the methods.
  * **`Integer#hash`** — a wide integer hashes its limbs and sign (`mrb_bint_hash`), so
    `{2**64 => 1}[2**64]` finds the entry although the two objects are different.

  Checked against the reference image with this script (`docker run --rm -v "$PWD:/w" -w /w
  kishima/mruby:4.1.0-rc mruby probe.rb` against `sabiruby probe.rb`; everything agrees except
  the six lines listed under "Deviations kept"):

  ```ruby
  def t(l); print l, " => "; begin; p yield; rescue => e; p e; end; end
  t("2**64"){ 2**64 };                t("square"){ (2**64) * (2**64) }
  t("div"){ -(2**64) / 7 };           t("divmod"){ (2**64).divmod(-7) }
  t("to_s(2)"){ (2**64).to_s(2).size }; t("to_s(36)"){ (2**64).to_s(36) }
  t("pow"){ 3.pow(1000).to_s.size };  t("powm"){ (2**64).pow(2, 7) }
  t("shift"){ (1 << 64) >> 63 };      t("and"){ (2**64) & (2**63) }
  t("float =="){ 2**64 == 18446744073709551616.0 }
  t("to_f.to_i"){ (2**64).to_f.to_i == 2**64 }
  t("hash key"){ ({2**64 => 1})[2**64] }
  t("%d"){ "%d" % 2**64 };            t("%b"){ "%b" % -(2**64) }
  t("sqrt"){ Integer.sqrt(10**40) };  t("digits"){ (10**40).digits.size }
  t("frozen?"){ (2**64).frozen? };    t("size"){ (2**64).size }
  t("to_i"){ "18446744073709551616".to_i }
  t("Integer()"){ Integer("18446744073709551616") }
  t("1e30.to_i"){ 1e30.to_i };        t("MIN/-1"){ (-9223372036854775807-1) / -1 }
  t("index"){ [1,2,3][2**64] };       t("chr"){ (2**64).chr }
  t("Time.at"){ Time.at(2**64) };     t("exp"){ 2 ** (2**64) }
  ```

## Compiling the tests

`tools/mrbtest.sh` copies a gem's `test/<file>.rb` as `gem_<file>.rb`; a name an earlier gem
already used is qualified as `gem_<gem>_<file>.rb` (`numeric.rb` of string-ext and numeric-ext,
`range.rb` of range-ext and string-ext, whose second copy had silently replaced the first
until 2026-09-12).

`tools/mrbtest.sh` compiles the test files with `mrbc -g`, which keeps the LVAR section:
`local_variables` and `Proc#parameters` need the names, and the reference driver compiles the
tests from source so it has them too. Reading LVAR uncovered a bug of the loader: the symbol
count of the section is 32-bit (`write_lv_sym_table`), not 16-bit.


## Remaining gems (plan as of 2026-09-12)

Order and instructions for the rest: `docs/gems-plan.md`. mruby-bigint is done (order 5 started
with it, because rational and complex branch in the same places of `numeric.rs`).

The reference `mruby` command is built from `default.gembox` = stdlib, stdlib-ext,
stdlib-io, math, metaprog (33 gems). Ported: fiber, enumerator, array-ext,
enum-ext, hash-ext, range-ext, string-ext, sprintf, metaprog, proc-ext, method,
compar-ext, toplevel-ext, enum-chain, enum-lazy, object-ext, symbol-ext, kernel-ext,
class-ext, numeric-ext, catch, objectspace, math, random, struct, data, set, time, bigint (29).
Sizes are lines of the reference C / mrblib Ruby / test.

| order | gem | C / Ruby / test | depends on | notes |
|---|---|---|---|---|
| 4 | mruby-eval | 417 / 0 / 333 | binding, compiler | `docs/eval-require-plan.md`; `tests/custom` cases wait for it |
| 4 | mruby-binding | 523 / 0 / 102 | – (tests: proc-ext) | with eval |
| 4 | mruby-proc-binding | 75 / 0 / 22 | binding, proc-ext | `Proc#binding` |
| 5 | mruby-pack | 2133 / 0 / 278 | – | `Array#pack`/`String#unpack`; large but self-contained |
| 5 | mruby-rational | 1512 / 72 / 742 | – (tests: complex) | `Rational`; the lexer literals (`1r`) already compile |
| 5 | mruby-complex | 1087 / 295 / 325 | math | `Complex` |
| 5 | mruby-cmath | 425 / 0 / 41 | complex | not in default.gembox |
| 6 | mruby-regexp | 10940 / 42 / 10213 | enumerator, symbol-ext, string-ext | the NFA engine (4.0.0); the largest single piece, its own milestone |
| – | mruby-io, mruby-socket, mruby-errno, mruby-dir, mruby-env, mruby-signal, mruby-process | 3868+1429+334+530+223+108+1320 | POSIX | not planned: the VM is no_std; a host `Host` trait may offer `puts`-level output only. mruby-error and mruby-exit are C API helpers, not needed |
| 7 | mruby-task | 2390 / 46 / 860 | – | not in default.gembox but planned: `Task` (priority queues, `Task.pass`/`sleep`/`join`/`Task::Queue`, tick-based preemption) on top of the Fiber contexts and `Vm::step`; the HAL (timer tick, `sleep_us`, idle) comes from the host, like the compiler hook; the scheduler-driven GC of `docs/gc.md` is part of it. Its `mrb_task_run` blocks, so the host-loop form (`run_once`) is the one rubevy needs |

Order: 1 (pure Ruby, done 2026-09-12) → 2 (small natives, done 2026-09-12) → 3 (data structures and host
clocks, done 2026-09-12) → 4 (eval, with the compiler hook) → 5 (numeric tower, pack) → UTF-8 strings
(`docs/utf8-plan.md`, a build-configuration milestone required for Japanese text) →
6 (regexp, on top of UTF-8) → 7 (task). regexp is by far the heaviest and can be moved after task.
Each gem: natives in `src/builtins/ext_<gem>.rs`, mrblib into `src/mrblib_<gem>.mrb`,
tests into `tools/mrbtest.sh` `GEMS`, reasons for what does not pass into
`docs/mrbtest-notes.md`, and the `Vm::with_mrblib` load order stays the gembox order.

### Core gems outside default.gembox

Candidates, with the reference sizes (C / Ruby / test):

| gem | size | worth it? |
|---|---|---|
| mruby-sleep | 186 / 0 / 29 | yes, with task: `Kernel#sleep`/`usleep` via the host clock hook |
| mruby-strftime | 118 / 0 / 152 | yes, with time: `Time#strftime` |
| mruby-string-bitops | 581 / 0 / 210 | maybe: `String#&`, `|`, `^`, `~` on bytes; small, self-contained |
| mruby-os-memsize | 283 / 0 / 63 | maybe: `ObjectSpace.memsize_of`; needs per-object sizes from our heap, answers will differ from the reference (deviation) |
| mruby-encoding | 109 / 0 / 921 | no for now: only meaningful with `MRB_UTF8_STRING`, which the byte-string build does not have |
| mruby-cmath | 425 / 0 / 41 | with complex (order 5) |
| mruby-benchmark | 0 / 130 / 283 | no: pure Ruby but depends on io and process |
| mruby-error, mruby-exit | 143, 82 | no: C API helpers (`mrb_protect`), `exit` is a host decision |
| mruby-test-inline-struct, mruby-test, mruby-bin-* | – | build/test infrastructure, not runtime |

Third-party gems are out of scope until the core list is done; the ones worth a look then
are pure-Ruby or small-C libraries used by PicoRuby (`picoruby-json`, `picoruby-yaml`,
`picoruby-base64`, `picoruby-crc`, `picoruby-markdown`) since their mrblib compiles as is.
