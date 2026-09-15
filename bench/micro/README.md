# `bench/micro`: loops around one operation

These are *not* benchmarks in the sense of `bench/`. Nothing here is part of any published
total, no category owns them and the reference `mruby` is not run against them. They exist
because a change can be right and still invisible: a native that copies the whole array to
read one element of it costs O(n) where the answer is O(1), and none of the twenty-four
benchmarks calls it. `tools/ab_micro.sh` runs two SabiRuby binaries over these files
alternately, which is the same discipline as `tools/bench_ab.sh` (see
`docs/design/optimizations.md` §4 for why alternating and not best-of-5).

Each file is a `while` loop around a handful of calls, sized to run a few hundred
milliseconds. `m_loop_only.rb` is the same loop with nothing in it, so the loop's own share
can be subtracted when reading a percentage.

`bench/micro/src/*.rb` is the source; `bench/micro/*.mrb` is what the reference `mrbc`
makes of it (`tools/ab_micro.sh --compile`).
