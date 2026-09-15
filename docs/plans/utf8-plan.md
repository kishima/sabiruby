# UTF-8 strings (`MRB_UTF8_STRING`) — plan

**実装状況 (2026-09-12): 完了。** Option B の通り Cargo feature `utf8`（既定で有効）として実装し、
文字としての String は `docs/design/utf8.md` にまとめた。本家テストは UTF-8 ビルドで 1877/1916、
バイトビルドで 1819/1866（それぞれ `tests/mrbtest/baseline.txt` と `baseline-bytes.txt`）。
以下は着手時の計画で、記録として残す。

Requirement (author, 2026-09-12): Japanese text must work, because the VM is meant for
games built on a game engine later. Today SabiRuby is a byte-string build, like the
reference image `kishima/mruby:4.1.0-rc` (`__ENCODING__` is `"ASCII-8BIT"`,
`"日本語".length` is 9 there). This is a build-configuration milestone, not a gem; it
sits before the string-heavy gems (regexp) and after the small ones.

## What the reference does

`MRB_UTF8_STRING` (with `MRB_USE_ASCII_CTYPE` optional) switches `src/string.c` to
character indexing: 22 `#ifdef` sites (`mrb_str_char_len`, `char_to_byte`,
`byte_to_char`, `str_subseq`/`str_substr`, `mrb_str_index_m`/`rindex_m`, `aref`/`aset`,
`reverse`, `split`, `inspect` escaping of code points, Unicode case mapping in
`upcase`/`downcase`/`capitalize`/`swapcase` unless ASCII_CTYPE), 20 in
`mruby-string-ext` (`each_char`/`chars`, `center`/`ljust`/`rjust`, `tr`/`delete`/
`squeeze`/`count` on characters, `ord`/`chr`, `scrub`, `casecmp?`, `slice!`,
`byteslice` stays bytes) and `%c` in `mruby-sprintf`. Symbols follow strings
(`Symbol#length` in symbol-ext). `byteindex`/`bytesize`/`byteslice`/`getbyte`/`setbyte`
are byte-based in both builds. 76 assertions in the reference tests are guarded by
`UTF8STRING` (`__ENCODING__ == "UTF-8"`), currently skipped here (see
`docs/verification/mrbtest-notes.md`: gem_string 9, gem_sprintf 3, plus core string tests).

## Decision

Option B, decided by the author on 2026-09-12 (there may be uses without Japanese).
One crate serves both: crates.io ships the source with both code paths, and the
feature is chosen by whoever builds (`default = ["std", "utf8"]`; a byte-string build
is `sabiruby = { version = "...", default-features = false, features = ["std"] }`).
Features are additive: `utf8` adds character semantics, so opting out is done by
dropping the default, never by a "bytes" feature. Cargo unifies features per build, so
one program has one mode, as with mruby's compile-time flag.

## The two options that were considered

* **A: UTF-8 only.** Drop the byte build. Simplest code and one verification baseline,
  but the baseline moves: a second reference image `kishima/mruby:4.1.0-rc-utf8` (built
  with `MRB_UTF8_STRING`, `../ref/mruby_containers/build_image.sh` with a changed
  build_config) replaces the current one for fixtures and `tools/mrbtest.sh`, and every
  byte-mode expectation in `docs/verification/mrbtest-notes.md` is redone.
* **B: Cargo feature `utf8`, default on** (recommended). Mirrors the reference's
  compile-time switch. The byte build keeps the existing image and baseline (must not
  move); the UTF-8 build gets its own image and its own `tools/mrbtest.sh` baseline.
  CI runs both. rubevy and the playground use the default (UTF-8). Cost: two baselines
  and `#[cfg(feature = "utf8")]` in the string natives.

## Work (option B)

1. Build the UTF-8 reference image locally (`4.1.0-rc-utf8`), record what changes in
   `mruby`'s own test results (the reference's own KO/skip list).
2. `src/string.rs`: character helpers (`char_len`, `char_to_byte`, `byte_to_char`,
   valid-encoding check as in `mrb_str_valid_encoding_p`), then the 78 core natives
   reviewed one by one for "index means character" vs "index means byte".
3. `src/builtins/ext_string.rs` (55 natives): same review; `tr` family on characters;
   `scrub` (currently absent); Unicode case mapping only if the reference image is
   built without `MRB_USE_ASCII_CTYPE` (decide with the image).
4. `inspect`/`dump` escaping (`\u{...}` vs `\x..`), `%c`, `Symbol#length`,
   `__ENCODING__` → `"UTF-8"`, `String#encoding`? (not in mruby without
   mruby-encoding; keep absent).
5. Fixtures: add `tests/fixtures/utf8.rb` (Japanese: length, index, slice, reverse,
   each_char, tr, center, inspect, split, upcase of ASCII in a Japanese string) with the
   `.out` from the UTF-8 image; run under both features (byte mode compares with the
   byte image's output — two `.out` files, or skip in byte mode).
6. `tools/mrbtest.sh` takes the image tag; the UTF-8 run un-skips the 76 guarded
   assertions and its baseline is recorded separately in `docs/verification/mrbtest.md`.
7. Playground: UTF-8 text in the editor already arrives as UTF-8 bytes; only the
   output rendering (`take_output` → `TextDecoder`) needs to be checked.

Estimate: 2–3 days including the image. Regexp (`mruby-regexp`) must be done on top of
this (its matcher is UTF-8 aware under the same flag), so UTF-8 comes first.

## Not covered

Encodings other than UTF-8 and `String#force_encoding` (mruby-encoding gem, "poor
man's" encoding, needs `MRB_UTF8_STRING`); could follow as a gem once this is in.
