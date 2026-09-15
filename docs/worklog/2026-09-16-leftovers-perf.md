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
`mrbc` が `src/bm_app_lc_fizzbuzz.rb:11: syntax error, unexpected ']'` で受け付けず（λ 計算だけで書かれた
1 行 1700 桁の式）、`.mrb` ができないのでベンチの本数には入らない。元からそうである。

### `printf`/`putc` がベンチを動かしていないことの確認

メソッド表が 2 つ増えただけでも（キャッシュの行、クラスの `HashMap` の中身）命令ループの数値は動きうるので、
`287826e`（main、`printf`/`putc` 無し）と `519eb17` のバイナリを作って交互 A/B を 4 本回した（7 ラウンド、P コア固定）:

| ベンチ | A（287826e） | B（519eb17） | 変化 |
|---|---:|---:|---:|
| bm_fib | 5394.221 | 5361.431 | −0.6% |
| call_args | 874.067 | 881.907 | +0.9% |
| ds_hash | 230.328 | 228.920 | −0.6% |
| loop_while_add | 838.556 | 840.219 | +0.2% |

全部 ±1% 以内。`bench/results/ab-287826e-printf.tsv`。

### 基準の取り直しで分かったこと

**1 回目の計測は捨てた。** ベンチ自体は通ったが、best と median の差が平均 3.3%（`loop_while_add` で 7.7%）あり、
過去の基準（`2aa4f13` 0.9%、`9da4724` 0.8%）と比べて明らかに荒れていた。`docs/design/optimizations.md` 4 節の
「best と median の差が 1% を超えたら取り直す」に当たる。取り直した 2 回目は平均 0.8%、最大 2.7% で、
こちらを `bench/results/519eb17.tsv` にした。絶対値は 1 回目より 3% ほど遅い窓に入っているが、
本家も同じ分だけ遅く出ているので比は動かない（共通 25 本で本家 14529 → 15343 ms）。

**取り直しのときに踏んだ落とし穴**: `tools/bench.sh` は `SABIRUBY_BIN` が無ければ `cargo build --release` を
自分で走らせる。項目 6 の編集を作業ツリーに置いたまま走らせたので、2 回目は**コミットされていない木**を測り始めていた
（ビルドが通ってしまったので気づきにくい）。止めて、`git stash` してから `519eb17` のバイナリを作り、
`SABIRUBY_BIN` で名指しして測り直した。以後の A/B もすべて、スクラッチに置いた名前付きのバイナリ
（`sabiruby-287826e`、`sabiruby-519eb17`、…）を指している。

## 項目 6: `items()` の残り

`grep -rn "items(vm" src/` は 101 か所。うち `src/builtins/ext_task.rs` の 13 か所は別物で
（`q_items` は `Task::Queue` の中の Array オブジェクトを返すだけで複製しない）、本当の対象は
`src/builtins/array.rs`（52 か所）と `src/builtins/ext_array.rs`（36 か所）の 2 ファイル。
`items(vm, v)` は `vm.ary_vals(v).unwrap_or_default()`、つまり `Vec<Slot>` を `Vec<Value>` に
**丸ごと複製**する。

一覧を作って読んでみると、「複製している」だけでは直す理由にならないことが分かった。**答えが配列全体なら、
複製は 1 回はどのみち要る**（`vm.ary_new` が `Vec<Value>` を取り、その中でもう 1 回 `Vec<Slot>` にする）。
`reverse`・`rotate`・`compact`・`+`・`*` を借用に書き換えても、割り当ての回数は 2 のままで何も変わらない。
効くのは次の 2 通りだけだった。

### 第 1 群: 答えが 1 要素か数要素なのに、配列全体を複製していたもの（`e27d9a4`）

`fetch`、`at`、`__fetch`、`first(n)`、`last(n)`、`take`、`drop`、`values_at`、`__svalue`、
`count`（引数もブロックも無いとき、長さを聞くためだけに複製していた）、`slice!`（長さを聞くためだけ）、
`Array#[]` の Range／2 引数の経路、`__combination_next`（k 要素）、`__product_next` の `product_fetch`（1 要素）、
`state_ints`（状態の Array）。O(n) が O(1) か O(k) になる。

両ファイルに `fn slots(vm: &Vm, v: Value) -> &[Slot]`（`vm.ary(v).unwrap_or(&[])`）を置き、
引数の変換（`expect_int`）を**借用の前に**済ませてから読む形にした。`expect_int` は `to_int` を呼びうる、
つまり Ruby を走らせて配列を動かしうるので、借用を跨がせない。`Array#[]` の遅い経路だけは
`index_args` を挟むが、これは Integer と Integer の Range しか読まず（Bignum は即例外）Ruby を走らせないので、
その後に借りても安全である、とコメントに書いた。

micro（`bench/micro`、`tools/ab_micro.sh`、7 ラウンドの交互 A/B）:

| micro | A（`519eb17`） | B（第 1 群） | 変化 |
|---|---:|---:|---:|
| m_ary_read（1000 要素から `fetch`/`at` で 1 個） | 181.128 | 86.131 | **−52.4%** |
| m_ary_ends（1000 要素から `first(3)`/`last(3)`/`take`/`drop`） | 114.194 | 46.655 | **−59.1%** |
| m_ary_copy（`dup`/`compact`/`reverse`/`rotate`/`+`） | 104.269 | 106.355 | +2.0% |
| m_ary_set（`-`/`|`/`&`/`difference`） | 272.511 | 280.039 | +2.8% |
| m_ary_walk（`index`/`count {}`/`assoc`） | 1807.566 | 1820.042 | +0.7% |
| m_loop_only（空のループ） | 7.218 | 7.380 | +2.2% |

**空のループが +2.2% 動いている**のが、この対の雑音の下限である。触っていない経路の
+0.7〜+2.8% はコード配置の揺れで、速くなったのは触った 2 本だけ、というのが正しい読み方。

公開ベンチ 8 本の交互 A/B（`bench/results/ab-519eb17-items-g1.tsv`）は
`ds_array` −1.2%、`ds_hash` −1.3%、`ds_string` +2.0%、`app_json_hash` +2.9%、`app_robot` −0.0%、
`app_tak` +0.6%、`bm_so_lists` +0.3%、`vmo_index` +0.1% で、合計はほぼ ±0。
`ds_string` と `app_json_hash` は変えたメソッドを 1 つも呼ばないので、これも配置の揺れである。
ベンチは動かないが micro で 2 倍、というのが項目 6 に期待されていたとおりの形なので、これは採る。

### 第 2 群: 要素を 2 回複製していた 3 つ（`dup`・`initialize_copy`・`replace`、`87ad19b`）

`("dup", … let v = items(vm, s); … ObjKind::Array(slots_of(&v).into()))` は、
`Vec<Slot>` → `Vec<Value>`（`items`）→ `Vec<Slot>`（`slots_of`）と 200 要素を 2 回写していた。
借りたスライスを `to_vec()` するだけで 1 回になる。`initialize_copy`（`clone` と `Array.new(other)` が通る）と
`replace` も同じ形。

| micro | A（第 1 群） | B（第 2 群） | 変化 |
|---|---:|---:|---:|
| m_ary_dup（`dup`/`clone`/`replace`、200 要素） | 44.614 | 37.209 | **−16.6%** |
| m_ary_copy（中に `dup` がある） | 105.313 | 102.922 | −2.3% |
| m_ary_ends | 47.105 | 46.982 | −0.3% |
| m_ary_read | 86.982 | 86.652 | −0.4% |
| m_ary_set | 276.815 | 277.196 | +0.1% |
| m_ary_walk | 1823.994 | 1822.100 | −0.1% |
| m_loop_only | 7.280 | 7.303 | +0.3% |

この対は空のループが +0.3% しか動いておらず、静かな回である。公開ベンチは
`ds_array` −0.1%、`ds_hash` −0.1%、`ds_string` −0.7%、`bm_so_lists` +0.3%、`app_json_hash` −0.1%
（`bench/results/ab-items-g1-g2.tsv`）。採る。

### 採らなかったもの（数値だけ残す）

**(a) 配列全体を答えるものを借用に書き換える**（`reverse`・`rotate`・`compact`・`compact!`・`+`・`*`・`uniq`）。
`vm.ary_new` が `Vec<Value>` を取る以上、借用から `Vec<Value>` を組み立てるのと `items` で作るのは同じ 1 回の割り当てで、
その後 `ary_new` の中でもう 1 回写るのも同じ。**書き換える理由が無い**ので手を付けなかった
（第 2 群の 3 つだけは、答えが `Vec<Slot>` のまま済むので別）。`Vm::ary_new` に `Vec<Slot>` を取る入口を足せば
この群も 1 回にできるが、公開 API が増えるうえ効き先は micro だけなので、指示書の範囲外として置いた。

**(b) 要素ごとに Ruby を呼ぶ走査を添字の借り直しに**（`Array#-`・`&`・`|`・`uniq`・`to_h`・
`delete_if`・`select!`・`sort_by`・`sum`、ext_array の集合演算）。これらは `items` を**ループの外で 1 回**呼ぶので、
すでに O(n) であって O(n²) ではない（O(n²) だった `include?`/`member?`/`count` は 2d の `f222934` で直っている）。
1 要素あたりのコストは `vm.equal`／`memb_in`／`call_block`、つまり Ruby のディスパッチが支配していて、
複製 1 回はその中に埋もれる。第 3 群（下）で実測したとおり効き目は雑音の底あたりなので、
**結果を捨てる `select!`/`delete_if` の類と、snapshot を配ることに意味がある `sort_by`/`sum` には手を付けない**。

**(c) `flatten_internal`・`transpose`・`product`・`intersection` の `items`**: どれも中身を全部要るので、
複製は仕事そのもの。

### 第 3 群: `assoc`・`rassoc`・`__ary_index` を本家と同じ「毎回読み直す」形に（`e877d17`）

第 2 群まで書いてから、(b) のうち **`assoc`/`rassoc`/`__ary_index` だけは別**だと分かった。本家の
`ary_assoc`（`mrbgems/mruby-array-ext/src/array.c:86`）は

```c
for (mrb_int i = 0; i < RARRAY_LEN(ary); i++) {
  mrb_value v = mrb_check_array_type(mrb, RARRAY_PTR(ary)[i]);
  mrb_gc_protect(mrb, v); // v may be removed from ary by mrb_equal()
```

と、**長さも要素も毎回読み直す**（コメントまで「`mrb_equal` が消すかもしれない」と言っている）。
SabiRuby の `index`/`include?`/`member?`/`__count` はすでにこの形（2d の `f222934`）で、
`assoc`/`rassoc`/`__ary_index` だけが snapshot を配る形で残っていた。つまりこれは速度の話ではなく
**取り残し**である。直したうえで数値も取った:

| micro | A（第 2 群） | B（第 3 群） | 変化 |
|---|---:|---:|---:|
| m_ary_walk（`index`/`count {}`/`assoc`） | 1818.802 | 1766.830 | −2.9% |
| m_loop_only（空のループ） | 7.337 | 7.191 | **−2.0%** |
| m_ary_read | 87.428 | 86.199 | −1.4% |
| m_ary_dup | 37.169 | 36.619 | −1.5% |
| m_ary_set | 274.748 | 271.545 | −1.2% |
| m_ary_copy | 101.520 | 101.061 | −0.5% |
| m_ary_ends | 46.420 | 46.251 | −0.4% |

**この回は B のバイナリが全体に速く出ている**（空のループが −2.0%、触っていない micro も −0.4〜−1.5%）。
差し引くと `m_ary_walk` の正味は **−1% 前後、雑音の底と同じ大きさ**である。`assoc`/`rassoc` だけを変えた
1 回目（`rassoc` を入れる前）は `m_ary_walk` −1.5%、空のループ −0.0% だったので、そちらの読みでも −1.5%。
**速さを理由に採ったのではなく、本家と同じ走査にするために採った**（1 要素あたり `Vec` を 1 本作らなくなるのは
そのついで）。数値は `bench/results/micro-items-g3.tsv`。
