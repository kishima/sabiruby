# 2026-09-16 `leftovers-plan.md` の項目 5 と 6（`printf`/`putc` とベンチの取り直し、`items()` の残り）

ブランチ `leftovers-perf`（main `287826e` から）。作業場所は git の worktree
`kishima/sabiruby-wt-bench`。指示書は [`../plans/leftovers-plan.md`](../plans/leftovers-plan.md) の
項目 5（`Kernel#printf` / `#putc` を足して本家のベンチ `bm_ao_render` と `bm_mandel_term` を動かし、基準を取り直す）と
項目 6（`items()` が配列を複製する箇所の残りを借用に）。

## 項目 5: `printf` と `putc`

### 出どころは `mruby-print` ではなく `mruby-io` だった

計画書は「本家の `mruby-print`/`mruby-sprintf` を見て」と書いているが、4.1.0-rc の `mrbgems/` に
`mruby-print` は無い。`grep -rn putc` で当たるのは `mrbgems/mruby-io`（`mrblib/kernel.rb:95` の
`module_function def putc(c); $stdout.putc(c); nil; end` と `src/io.c:1112` の `io_putc`）で、
`printf` も同じファイルの 122 行目（`module_function def printf(...); $stdout.printf(...); end`、
`IO#printf` は `mrblib/io.rb:280` の `write sprintf(*args)`）。
`mruby-sprintf` が持っているのは `sprintf`/`format` だけで、`printf` は持っていない。
SabiRuby には IO が無いので、両方とも `print` と同じ出口（`Vm::write_out`）に書く
ネイティブとして `src/builtins/kernel.rs` に置いた。`print`/`puts`/`p` の隣で、
同じ `module_function` の輪（Kernel の特異メソッドにも入れ、インスタンス側は private）に加えた。

### 振る舞いは本家で実測して決めた

仕様は doc コメントを読むだけでは決まらなかったので、`kishima/mruby:4.1.0-rc` と
`…-rc-utf8` の中で本家を走らせて確かめた（スクラッチの `probe.rb`〜`probe3.rb`）。分かったこと:

* **`printf` は必ず nil を返す。** `IO#printf` は `write sprintf(*args)` なので書けたバイト数が
  返りそうに見えるが、実測は `nil`。返り値まで doc（`printf(...) -> nil`）どおりだった。
* **`printf` の引数の誤りは全部 `sprintf` の例外**。`printf` 単独は
  `ArgumentError: too few arguments`、`printf(1)` は `TypeError: Integer cannot be converted to String`。
  つまり `printf` 自身は引数を数えない。
* **`putc` は nil を返す**（`IO#putc` は引数を返すが、`Kernel#putc` は `; nil` で潰している）。
  引数はちょうど 1 個（`mrb_get_arg1`）。
* **Integer は `c & 0xff` の 1 バイト**。`putc(0x141)` が `"A"`、`putc(0x2603)` が `0x03`。
  UTF-8 のビルドでもコードポイントとしては扱わない。
* **Integer 以外は `mrb_obj_as_string` を通して「最初の 1 文字」**。`putc("hello")` は `"h"`、
  `putc(nil)` は空文字列なので何も書かず、`putc(1.5)` は `"1"`、`to_s` を持つ独自クラスも同じ。
* **「1 文字」はビルドの都合で、文字列の都合ではない。** `io_putc` は `#ifdef MRB_UTF8_STRING` で
  `mrb_utf8len` を使い、その文字列が binary（`String#b`）かどうかは見ない。実測でも
  `-rc-utf8` の `putc("→".b)` が 3 バイト書く。だから SabiRuby も
  `char_mode(vm, v)`（= `UTF8 && !str_binary(v)`）ではなく `string::UTF8` を渡している。
  `-rc`（非 UTF-8）は同じ入力に 1 バイト（`0xe2`）だけ書く。
* 正しい列を成さないバイト（`"\xff\xfe"`）は `mrb_utf8len_table` が 1 と答えるので 1 バイト。
  SabiRuby の `string::utf8len` は同じ表を写してあるので、そのまま合う。

### `sprintf` との共有

`printf` は `sprintf` の書式化をそのまま使いたいので、`ext_sprintf.rs` の `sprintf` から
`sprintf_bytes(vm, a) -> VmResult<(Vec<u8>, bool)>`（バイト列と、それがバイト読みかどうか）を
切り出して両方から呼ぶ形にした。**残る差**: 本家の `printf` は Ruby から `sprintf` を呼ぶので
`Kernel#sprintf` を再定義すればその影響を受けるが、こちらは書式化器を直接呼ぶので受けない。
ベンチの `bm_mandel_term` は内側のループで `putc` を呼ぶ（1 ピクセル 1 回）ので、
`putc` は `funcall` を経由しない素のネイティブにしてある。

### テスト

`tests/custom/printf_putc.rb`（ASCII だけ。両方のビルドで同じ）と
`tests/custom/printf_putc_utf8.rb`（`# utf8-only:`、多バイトの「最初の 1 文字」）。
期待値は `tools/custom.sh` が本家の出力から作ったもので、両方とも `.rc.out` と一致する。

ひとつだけ本家と違うものが出た。最初に書いた `p Object.new.respond_to?(:printf)` が
本家 `true`、SabiRuby `false` になる。`printf` のせいではなく、**private なメソッドに対する
`respond_to?` の答えが元から違う**（`Object.new.respond_to?(:puts)` も本家 `true`、SabiRuby `false`、
`respond_to?(:puts, true)` はどちらも `true`）。計画書の項目 10（可視性の食い違い、著者判断待ち）の
範囲なのでここでは直さず、テストは `respond_to?(:printf, true)` を聞く形にして、
理由を `.rb` のヘッダに書いた。

### ベンチが動くようになったことの確認

`bench/bm_ao_render.mrb` と `bench/bm_mandel_term.mrb` は**すでにリポジトリにあり**、
`bench/categories.tsv` にも「whole program」として載っていた（動かないので `fail:` の行が
結果に残っていただけ）。だから項目 5 で足すものは無く、確かめるのは出力である。
標準出力を本家と 1 バイトずつ比べて、両方とも完全に一致した
（`bm_mandel_term` 3900 バイト、`bm_ao_render` 12301 バイト = PPM 画像）。
ベンチの計測は `tools/bench.sh` が `> /dev/null` で捨てるので、端末への書き出しは時間に入らない
（`--stats` の行は stderr なので残る）。`bm_so_*` も同じ扱い。

なお `bench/src/bm_app_lc_fizzbuzz.rb` は `bench/*.mrb` に対応が無い。`tools/bench.sh` は
`benchmark/bm_*.rb` を全部 `bench/src/` に写して本家の `mrbc` にかけるが、この 1 本だけは
`mrbc` が受け付けない（λ 計算の入れ子が深い）ので `.mrb` ができず、ベンチの本数には入らない。
