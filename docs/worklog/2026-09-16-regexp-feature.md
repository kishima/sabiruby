# 2026-09-16 Cargo feature で gem を落とす（`from-mrubyedge-plan.md` の 4）

`docs/plans/from-mrubyedge-plan.md` の項目 4。`regexp` feature（既定オン）を足し、
`--no-default-features --features utf8` で mruby-regexp を丸ごと落とせるようにした。
ブランチ `regexp`（main `eec1d23` から）。守るものは `docs/plans/host-bridge-plan.md` の
0 節と同じ: `unsafe` を増やさない、`no_std + alloc`、本家テストの基準を下回らない。

## まず読んだところ ── regexp がどこに触れているか

`grep -rln "regexp\|Regexp" src --include=*.rs` で 12 ファイル。読んでみると、
**本体は 2 つに集まっていて、残りは細い**という形だった。

* `src/regexp/mod.rs`（908 行）: パターン文字列を `regex-automata` の構文に訳す層。
* `src/builtins/ext_regexp.rs`（1882 行）: `Regexp` と `MatchData` のクラス、String と Symbol の
  「パターンを取る形」。入口は `pub fn init(vm)` の 1 本だけ（`src/vm.rs:702` から呼ばれる）。
* 細いほう: `ObjKind::Regexp` / `ObjKind::MatchData`（`src/object.rs:592`）と、それを見る
  `match` の腕が 5 か所（GC のマーク 2 か所、`initialize_copy`、`ObjectSpace` の型分類、`inspect` 2 か所）、
  `Core::regexp` / `match_data`（`src/vm.rs:314`）、`Syms::backref` と `OP_GETGV`/`OP_SETGV` の
  `$~` 分岐（`src/vm.rs:3241`）、`MRBLIB_REGEXP_MRB`（`src/lib.rs:131`）。

入口が `ext_regexp::init` 1 本というのが効いた。`#[cfg]` を撒く場所は、モジュール宣言 2 つ、
`load_mrblib` の 2 行、`match` の腕 5 つ、`Cargo.toml` の依存 1 つで済んでいる。

## 「gem 無しのとき何を答えるか」は本家のテストが決めていた

計画書は「`String` の regexp を取る形は `TypeError` か `NotImplementedError`（本家で regexp gem を
外したときの振る舞いに合わせる）」と書いている。本家 `ref/mruby` を読むと、**そもそも前提が違った**。

`mrbgems/mruby-regexp/src/regexp.c:3594` からの `mrb_define_method` の並びを見ると、
`match` / `match?` / `=~` / `scan` は **この gem でしか定義されていない**。一方
`sub` / `gsub` / `split` / `index` / `partition` / `start_with?` / `[]` / `[]=` は、
本家の `mrblib/string.rb:52,112` などに Ruby 版があり、gem はそれを C で**置き換えて**いる。
`grep -rn '"sub"' --include=*.c` の結果が `mruby-regexp/src/regexp.c` の 1 行だけ、というのが証拠。

だから gem 無しの本家は:

* `"a".match(/x/)` → そもそも `/x/` が書けない。コンパイラはリテラルを `Regexp.new(…)` に落とす
  （`codegen.c`）ので、**`Regexp` という定数が無い**ところで NameError になる。
* `"a".match("x")` → NoMethodError（メソッド自体が無い）。
* `"aa".sub("a", "b")` → 動く。mrblib の Ruby 版が残っている。

つまり「Regexp を渡したら TypeError」という場面は**存在しない**。Regexp を作れないからだ。
計画書の想定（TypeError か NotImplementedError）は採らず、「クラスを定義しない」を採った。
決め手は本家の `test/t/codegen.rb` にあった一行で、`/static/` が NoMethodError を上げることを
assert している。本家のテストドライバは regexp gem をリンクしないので、本家ではこれが通る。
SabiRuby では今まで 1 件 ko だった（`tests/mrbtest/notes.tsv` にその旨の注記が既にあった）。
**feature を切るとこの 1 件だけ増える**（codegen 14 → 15）。他の全ファイルは同数。
「本家に寄せた」ことの、いちばん気持ちのよい確認になった。

## `String#sub`/`gsub` は、ずっと死んだコードだった

feature を切って `cargo test` を回したら、`tests/fixtures/utf8` が落ちた。

```
left:  ... "true\n<error: offset 19 does not land on character boundary (IndexError)>\n"
right: ... "true\n\"日本語文字\"\n ...
```

`"日本語テキスト".sub("テキスト", "文字")` のところ。犯人は本家 `mrblib/string.rb` の `sub`:

```ruby
found = self.index(pattern)          # utf8 ビルドでは「文字」単位
result << self.byteslice(0, found)   # こちらは「バイト」単位
```

文字とバイトが同じ幅のときしか合わない書き方で、本家では mruby-regexp の C 版が上書きするので
表に出てこない。SabiRuby にも `src/builtins/string.rs` に `str_sub`（String パターン専用の
ネイティブ）があり、その doc コメントには「`post_mrblib` が mrblib の後に登録する」と書いてある。
ところが **`post_mrblib` という関数はリポジトリのどこにも無かった**（`grep -rn post_mrblib src/` の
ヒットはそのコメント 1 行だけ）。`str_sub` は `string::init` の表に載っているだけなので、
mrblib の Ruby 版に上書きされ、さらに `ext_regexp::init` に上書きされる。つまり**一度も呼ばれていない**。
その証拠に、`#[cfg(not(feature = "regexp"))]` を付けて regexp ありでビルドすると
`warning: function str_sub is never used` が出る。

直し方は 2 つ考えた。

* (A) mrblib の Ruby 版のままにする。「gem 無しの本家」に最も忠実で、本家のバグもそのまま再現する。
* (B) コメントが約束していた `post_mrblib` を実際に作り、feature が無いときだけ
  `sub`/`gsub` をネイティブに戻す。

(B) を採った。この段階の目的は「Regexp を外す」であって「多バイト文字列で `sub` を壊す」ことではない。
`utf8` と `regexp` は直交した feature なので、片方を切ったらもう片方が壊れるのは筋が悪い。
本家が C で同じ穴を塞いでいる以上、SabiRuby がネイティブで塞ぐのは「gem のメソッドをネイティブで
置き換えない」規則（`docs/design/gems.md`）にも反しない——置き換えているのは gem のメソッドではなく、
コア mrblib の壊れたメソッドだからだ。`src/builtins/string.rs` に
`#[cfg(not(feature = "regexp"))] pub fn post_mrblib(vm)` を足し、`load_mrblib` の
mrblib ループ直後に呼ぶ。regexp ありのときは、そのあと `ext_regexp::init` が同じ名前を取るので
既定ビルドの振る舞いは 1 ビットも変わらない（`tests/mrbtest/baseline.txt` と完全一致で確認）。
stale だったコメントも書き直した。

## `$LOADED_FEATURES` は Ruby 側のリテラル

`src/mrblib/require.rb:102` の `$LOADED_FEATURES = [...]` に `"regexp"` が並んでいる。
これは**コンパイル済みの `.mrb`** なので、feature で出し分けられない。放っておくと
`require "regexp"` が `false`（＝リンク済み）を返し、嘘をつく。`Vm::load_mrblib` の最後に
`#[cfg(not(feature = "regexp"))]` のブロックを 1 つ置き、`global_get("$LOADED_FEATURES")` から
`"regexp"` を除いた配列を `global_set` し直す形にした。これで `LoadError` になる。
`tests/require.rs` の `the_built_in_gems_are_already_features` を両方の場合に分けた。

## 落とし穴: `cli` が既定で VM を `default-features = false` で取っている

`cli/Cargo.toml` は `sabiruby = { …, default-features = false, features = ["std"] }`。
VM に `regexp` を足しただけだと、**`cargo build -p sabiruby-cli` が regexp 無しの VM を作る**。
`cli` にも既定オンの `regexp = ["sabiruby/regexp"]` を足した。
同じ理由で `tools/mrbtest.sh --bytes` の `--no-default-features` も
`--no-default-features --features regexp` に直した（byte 文字列ビルドは regexp を持ったままでないと
`baseline-bytes.txt` を割る）。ここは feature を足す作業でいちばん間違えやすいところだと思う。
**「既定オン」は、その crate を直接ビルドしたときの話でしかない。**

## 依存を optional にする書き方

`utf8 = ["regex-automata/unicode"]` のままだと、`regex-automata` を `optional = true` にした
途端に `utf8` が暗黙に `regexp` を有効にしてしまう。weak dependency feature
（`regex-automata?/unicode`、Cargo 1.60 以降）にして、「`regex-automata` が入っているなら
その `unicode` も入れる、入っていないなら何もしない」に直した。確認は `cargo tree`:

```
$ cargo tree -p sabiruby --no-default-features --features utf8 -e normal
sabiruby v0.4.0
├── hashbrown v0.15.5 └── foldhash v0.1.5
└── libm v0.2.16
$ cargo tree -p sabiruby -e normal
… └── regex-automata v0.4.18 └── regex-syntax v0.8.11
```

## テストの回し方

`tools/mrbtest.sh --no-regexp` を足した。`--bytes` と同じ形で、出力は
`docs/verification/mrbtest-noregexp.md`、基準は `tests/mrbtest/baseline-noregexp.txt`。
除くファイルは **mruby-regexp 自身の test/*.rb だけ**（`gem_regexp*`、`gem_match_data`、
`gem_string_regexp`、`gem_symbol_regexp`、`gem_string_index`、`gem_backref_scope`、
`gem_backtracking_stack`、`gem_unicode_*`、`gem_ascii_*` の 12 本）。

最初は本家コアの `regexperror.rb` も除いていたが、読み直したら `RegexpError` は 15.2.27 で
**コアのクラス**（本家も `src/error.c` 側で定義し、gem はそれを raise するだけ）であり、
ファイルの唯一の assert は本家で `# TODO broken ATM` とコメントアウトされている。
実際に feature 無しで回しても 0 assertion で素通りするので、除外リストから戻した。
結果、2 つの基準の差は「gem の 12 ファイルが無い」と「codegen が 14 → 15」だけになった。

`tests/mrbtest.rs`（コンパイル時に基準を選ぶ回帰テスト）は `(utf8, regexp)` の組で選ぶように
した。`(false, false)` は誰もビルドしないので基準を持たず、その旨を出して素通りする。

`tests/custom/` には `# utf8-only:` という仕組みが既にあったので、同じ形の `# regexp-only:` を
足した。使うのは `getidx_dispatch.rb` 1 件だけ（`"hello"[/l+/]` を送る節がある）。
ヘッダを 1 行足したので `tools/custom.sh getidx_dispatch` で `.mrb` / `.dump` を取り直した
（`.expected` に行番号は入っていないので内容は不変）。

`tests/regexp_engine.rs` は `#![cfg(feature = "regexp")]` を 1 行。

副産物として `tests/mrbtest/notes.tsv` の 3 か所が `docs/gems.md` / `docs/gc.md` という
2026-09-15 の docs 移動前のパスのままだったのを直した（生成される `mrbtest.md` は手で直された
あとだったらしく、再生成すると古いパスに戻ってしまう状態だった）。`codegen` の注記も、
両方のビルドで読める文に書き直した。

## 数字

`docs/verification/size.md` に置いた。要点だけ:

| | regexp あり | regexp なし | 差 |
|---|---:|---:|---:|
| thumbv7em `opt-level="z"` `.text`（依存込み） | 1,391,312 | 742,467 | −648,845（−46.6%） |
| 同、sabiruby の rlib だけ | 713,398 | 659,400 | −53,998（−7.6%） |
| thumbv7em `opt-level=3` `.text`（依存込み） | 1,952,800 | 1,140,446 | −812,354（−41.6%） |
| x86_64 `sabiruby` コマンド（リンク済み） | 5,604,312 | 3,862,280 | −1,742,032（−31.1%） |

道具は `arm-none-eabi-size` 2.38（GNU binutils。システムに入っていたものを使った。
`rustup component add llvm-tools` の `llvm-size` でも同じ列が読める）。ライブラリにはリンクが
無いので、これは `.rlib` を `ar` の中身ごと足した**上限**である——リンカが到達しないコードを捨てるぶん、
実際のファームウェアはこれより小さい。**2 行の差**は上限同士の差なので、そこは素直に読める。
唯一リンカを通っているのは x86_64 のコマンドで、こちらは −31.1% が実測値。

**削れたバイトの 91.7% は `regex-automata` + `regex-syntax`** で、SabiRuby 自身のコードは
53,998 バイトしかない。これが `random` / `time` / `pack` の判断をそのまま決めた。

## `random` / `time` / `pack` を同じ扱いにするか → しない

判断材料として、`arm-none-eabi-nm --print-size --demangle` を rlib にかけ、逆マングルした
シンボル名の先頭のモジュールで `.text` を足し上げた（ジェネリクスの実体化は型を名付けたほうの
モジュールに載るので、厳密な分割ではなく帰属）。

| モジュール | `.text` |
|---|---:|
| `builtins::ext_regexp` | 35,144 |
| `Vm::exec_frames`（命令ループ） | 17,420 |
| `regexp`（訳す層） | 9,442 |
| `builtins::ext_pack` | 9,020 |
| `builtins::ext_random` | 5,168 |
| `builtins::ext_time` | 2,900 |
| `builtins::ext_strftime` | 1,308 |

`pack` + `random` + `time`（+ `strftime`）＝ 約 18 KB。regexp 無しの最小構成 742 KB に対して
2.4%。作業（`#[cfg]` の散布、基準ファイル 3 本、CI 3 行）と、feature の組み合わせが
2 倍ずつ増えることに見合わない。

**規則として「外部 crate を連れてくる gem だけ feature にする」を `docs/design/gems.md` に書いた。**
今それに当たるのは mruby-regexp だけ。もし本当に最後の 2% が要る用途が出てきたら、3 つ個別ではなく
1 つの粗い feature（たとえば `extras`）にまとめるほうがよい。`ext_strftime` は `ext_time` の
`gmtime` / `seconds_of` / `MON_NAMES` / `WDAY_NAMES` を使っているので、`time` を落とすなら
`strftime` も落ちる（`random` と `pack` は他から参照されていない）。

## 確認したこと

```
cargo test --workspace                                   … 全 30 バイナリ ok、失敗 0
cargo test -p sabiruby --no-default-features --features "std utf8"  … 全 17 バイナリ ok、失敗 0
tools/check_no_std.sh                                    … no_std OK
tools/check_no_std.sh --features utf8                    … no_std OK
tools/check_no_std.sh --features regexp                  … no_std OK
tools/mrbtest.sh                 … all 2507/2344、baseline.txt と完全一致
tools/mrbtest.sh --no-regexp     … all 2011/1979、baseline-noregexp.txt（96 ファイル）
grep -rn unsafe src --include=*.rs | grep -v '^[^:]*:[0-9]*: *//' | wc -l  … 0
```

CI（`.github/workflows/ci.yml`）には 3 行足した: regexp 無しの `cargo test`、
`tools/check_no_std.sh --features utf8`（thumbv7em、regexp 無し）、
`tools/check_no_std.sh --features regexp`（今まで素の `--no-default-features` が
regexp を含んでいたぶんの穴埋め）。`tools/mrbtest.sh --no-regexp` は本家 `mrbc` の Docker が
要るので CI では回せず、代わりに `tests/mrbtest.rs` が同じ assertion を
`baseline-noregexp.txt` に対して回す。

ベンチは取っていない。この段階はコードを**足していない**（`#[cfg]` と、feature オフのときだけ
効く `post_mrblib`）ので、既定ビルドの命令列は変わらない。`baseline.txt` の完全一致が
その確認になっている。
