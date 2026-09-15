# mruby/edge から取り込むもの（実装指示書）

作成 2026-09-16。mruby/edge（`../ref/mrubyedge`、`c7dd9ae`、2026-04-14 のクローン）の実装を読んで、SabiRuby に取り込む価値があると判断した 5 件。
著者の判断: `docs/plans/ecs-bridge-plan.md` の残り（`e[:Transform]` の読み）が終わってから、この順で。比較の記録は書籍側の
`docs/notes/related-implementations.md`（本には書かない）。

取り込まないもの（理由は同じ記録に）: `SharedMemory`、`Rc` の値表現、`insn-limit`、CLI の wasm 用 crate 生成。

## 状況

| # | 内容 | 状態 |
|---|---|---|
| 1 | `sabiruby-serde`: `Value` と serde をつなぐ（JSON ほか） | **済み**（2026-09-16、`bd71d31`）。記録は `docs/worklog/2026-09-16-serde.md`、設計は `docs/design/serde.md` |
| 2 | RBS で境界を宣言する（検討→設計） | **済み（設計文書）**（2026-09-16、`584d42c`）。記録は `docs/worklog/2026-09-16-rbs-study.md`、設計は `docs/design/rbs.md`。実装は次の段階 |
| 3 | 対応メソッド一覧の生成（`docs/verification/coverage.md`） | 未着手 |
| 4 | Cargo feature で gem を落とせるようにする（まず regexp） | 未着手 |
| 5 | `RUBY_ENGINE` をどう答えるか決める | 未着手 |

## 1. `sabiruby-serde`

**到達点**: Rust の serde 対応の型と Ruby の値を相互に変換でき、その上に `JSON` が載る。

```rust
let v: Value = sabiruby_serde::to_value(&mut vm, &config)?;      // Serialize → Ruby（Hash/Array/String/…）
let config: Config = sabiruby_serde::from_value(&mut vm, v)?;    // Ruby → Deserialize
```
```ruby
JSON.parse('{"a": [1, 2]}')   #=> {"a" => [1, 2]}
JSON.generate(h)              # / h.to_json
```

**設計**:
* 新しい crate `serde/`（`sabiruby-serde`）。`sabiruby` に依存し、`serde` に依存する。`sabiruby` 本体は serde に依存しない（`no_std` と依存の小ささを保つ）。
  ワークスペースの `members` に足し、`cargo publish --dry-run --workspace` が通る形に（`version`/`license`/`description`/`repository`）。
* `Serializer` の実装: serde のデータモデル（struct → Hash（キーは文字列。シンボルにするかは option）、seq → Array、map → Hash、
  数値 → Integer/Float、`Option` → nil、enum の unit variant → シンボル、newtype/tuple variant → `{name: value}`）。
  `Deserializer` はその逆。`Value` を借りるのに `&mut Vm` が要るので、`to_value(&mut Vm, &T)` / `from_value(&mut Vm, Value)` の形。
* **`FromRuby`/`IntoRuby` との関係**: `T: Serialize` に対する `IntoRuby` の blanket impl は、既存の impl（`i64`、`String`…）と重なるので書けない。
  代わりに `Serde<T>` のような包み型で `define_fn` から使えるようにする（`|cfg: Serde<Config>| …`）。理由を rustdoc に。
* `JSON` は `mruby-serde-json` と同じく **Ruby 側の `JSON` クラスを Rust で**（`JSON.parse`/`generate`/`pretty_generate`、`Object#to_json`）。
  `serde_json` の `Value` を経由するのが最短。本家には mruby-json（非公式 gem）があるが互換は目標にしない（「選べる実装」）。
* テスト: 往復（Rust → Ruby → Rust）が同一、CRuby の `JSON` と出力を突き合わせる `tests/custom/` の case（`json_roundtrip.rb`。期待値は CRuby）。
  `tests/serde.rs`（struct/enum/Option/nested、数値の境界、非 UTF-8 の String は `Bytes` で）。

**大きさ**: 中。

## 2. RBS で境界を宣言する（まず検討）

**到達点（この段階では設計文書）**: ホストが Ruby に出す関数・クラスを RBS で書き、それをどう使うかを決める。`docs/design/rbs.md`。

**検討すること**:
* 何に使うか: (a) `#[ruby_methods]` の生成コードに、RBS の型と Rust の型の一致を検査させる、(b) Playground とゲーム内エディタの補完・ホバーの文書、
  (c) rubevy の `Rubevy.ask` の `kind` ごとの引数と答えの文書（今は文字列で、何を渡せるかがコードを読まないと分からない）。
* RBS のパーサ: mruby/edge の `rbs_parser`（nom、`def name: (T, T) -> T` の 1 形だけ）程度の小ささで足りるか、`rbs` crate の有無。
* 逆向き（Rust の `#[ruby_methods]` から RBS を**生成**する）のほうが、二重管理が無く筋がよい可能性。まずこちらを試す。

**大きさ**: 小（文書）。実装は別の段階に。

## 3. 対応メソッド一覧の生成

**到達点**: `docs/verification/coverage.md` に、SabiRuby が持つ組込みクラスとメソッドの一覧（クラスごと、`.class_method` / `#instance_method`、
どの gem 由来か）。生成物で、`tools/coverage.sh` が作る。

**設計**: VM を `with_mrblib` で起こし、`ObjectSpace` と `Module#instance_methods` / `singleton_methods` / `ancestors` を Ruby のスクリプトで回して
Markdown を吐く（`sabiruby run tools/coverage.rb`）。gem 由来かどうかは `docs/design/gems.md` の一覧と突き合わせ（手で持たず、`Method#owner` と
定義元のファイル名で判定できる範囲で）。本家 4.1.0-rc の同じ一覧も Docker で取り、**差分**（本家にあって無いもの、その逆）を出す。
これは `mrbtest` の通過率とは別の「何があるか」の答えになる。

**大きさ**: 小。

## 4. Cargo feature で gem を落とせるようにする

**到達点**: `sabiruby` を `default-features = false, features = ["utf8"]` のように regexp なしでビルドでき、thumbv7em のサイズが減る。

**設計**:
* `regexp` feature（既定オン）。オフのとき `ext_regexp` と `regex-automata` を外し、`Regexp` 定数を定義しない。`String` の regexp を取る形
  （`sub`/`gsub`/`scan`/`match`/`=~`/`split` の Regexp 引数）は `TypeError` か `NotImplementedError`（本家で regexp gem を外したときの振る舞いに合わせる）。
* `mrblib` の `.mrb` のうち regexp 依存のもの（`regexp.mrb`）を読まない。`tools/mrbtest.sh` に `--no-regexp` を足し、regexp の test ファイルを除いた基準を持つ
  （`baseline-noregexp.txt`）。CI に「regexp なしで thumbv7em がビルドできる」を 1 行。
* サイズを `docs/verification/` に記録（`.text` の大きさ、thumbv7em、release、`opt-level = "z"`）: regexp あり／なし。
* 同じ形で `random`、`time`、`pack` も外せるようにするかは、regexp の結果を見て決める。

**大きさ**: 中。

## 5. `RUBY_ENGINE`

**到達点**: 何を答えるか決めて、`docs/design/gems.md` の「Intended differences」か `corelib` の節に理由を書く。

**今**: `RUBY_ENGINE = "mruby"`、`RUBY_ENGINE_VERSION = "4.1.0"`（`src/builtins/object.rs`）。本家テストは `RUBY_ENGINE` を見て分岐する箇所がある
（`grep` で数える）。

**選択肢**: (a) `"mruby"` のまま（互換）。(b) `"sabiruby"` にし、`RUBY_ENGINE_VERSION` に SabiRuby の版、`MRUBY_VERSION` に本家の版。
本家テストの分岐が壊れる件数と、「SabiRuby で動いているか」をスクリプトが知る手段（mruby/edge の `wasm?` に当たるもの）の両方を見て決める。
(b) なら `SABIRUBY_VERSION` 定数を足す案も。

**大きさ**: 小。

## 実装で分かったこと

### 1. `sabiruby-serde`

* **エラー型は分けるしかない。** serde の `ser::Error` / `de::Error` は `custom(msg)` を要求するが、
  `VmError::Raise` は例外オブジェクトを持つので `&mut Vm` 無しには作れない。`sabiruby_serde::Error`
  （`Message` / `Vm`）を置き、`to_value` / `from_value` の境界で `VmError` に変える形にした。
  計画書の `VmResult` を返す形はそのまま保てる。`Message` は `TypeError`（`FromRuby` と同じクラス）。
* **`Serde<T>` は `IntoRuby` と `IntoRubyRet` の**どちらか一方**しか実装できない。**
  `impl<T: IntoRuby> IntoRubyRet<RetValue> for T` がすでにあるので、両方あると `define_fn` の
  マーカ推論が曖昧になる。指示どおり `IntoRuby` を採り、`into_ruby` が raise できない以上、
  シリアライズ失敗時は**例外オブジェクトそのもの**を返すことにした（`nil` だと `None` と区別がつかない）。
  この経路に入るのは手書きの `Serialize` が `Error::custom` を呼んだときだけ。
* **`serde_json` の `Map` は既定で `BTreeMap`** なので、`JSON.generate` の鍵が整列されてしまう。
  CRuby の期待値と 1 行だけ食い違って気づいた。`preserve_order` で直るがその feature は `std` を含む
  （`preserve_order = ["indexmap", "std"]`）。`json` feature（既定オン）に分け、変換層だけなら
  `no_std` + alloc のままにした（`thumbv7em-none-eabi` でビルド確認）。
* **`JSON.generate` は `from_value` を通していない。** 鍵は `to_s`（CRuby は `{1 => 2}` を `{"1":2}` にする）、
  他クラスのオブジェクトは `to_s`（`(1..3).to_json` は `"1..3"`）という規則が serde のデータモデルに無いため。
  `deserialize_string` を緩めて通すと、普通の struct の `String` フィールドに Integer が黙って入るようになる。
  `parse` の向きは計画書どおり `serde_json::Value` → `to_value` の一本で済む。
* **VM に足りなかった入口は 2 本。** `Vm::hash_entries`（`ary_vals` の相方。Hash を読む手段が `hash_get` しか無く、
  鍵の一覧を得る術が無かった）と `Vm::define_class_under`（`define_class` は必ず `Object` の定数にするので、
  `JSON::ParserError` が置けない）。段階 3b と同じ形の穴埋め。
* **`BigInt` → `i128` は `to_i64` / `to_u64` では足りない**（`2**64` がどちらにも入らない）。
  `to_string_radix(10)` を経由する。`to_f64` は丸めるので使えない。
* **単体での公開はまだできない。** CI の `cargo publish --dry-run --workspace` は通る（未公開の依存は
  ワークスペース自身のパッケージで検証されるため）が、`cargo publish --dry-run -p sabiruby-serde` は
  verify のビルドで落ちる。crates.io の `sabiruby 0.4.0` には `src/convert.rs` が無い（段階 4 より前の公開）。
  実際に上げるときは `sabiruby` を先に公開する。依存は `version = "0.4"` のままにしてある。

### 2. RBS（検討の段階）

* **推奨は「パーサを入れない、生成だけやる」。** 計画書が挙げた 3 用途のうちパーサが要るのは
  (a) マクロ時の型検査だけで、それは落とした。理由は二重管理より前に、**マクロが見ているのが
  解決済みの型ではなく型の綴りだから**。`type Hp = i64;` の `n: Hp` は展開時には `Hp` でしかない。
  生成側はこれを `untyped` に倒せば嘘をつかずに済むが、検査側は通すか落とすかしかなく、
  通せば検査にならず、落とせば型エイリアスを書いた人が理由の分からないエラーを見る。
  同じ情報不足が、片方では許容できる劣化になり、もう片方では機能を壊す。
* **`ruby-rbs` は本家（ruby/rbs）が出しているが、この機械ではビルドできなかった。**
  0.3.0（2026-04-03、BSD-2、約 33k DL）は rbs の C パーサを同梱して `cc` + `bindgen` で束ねる作りで、
  `cargo build` が `Unable to find libclang` で落ちる。依存 42 個。VM の crate（依存 4 つ、
  `no_std`、CI で thumbv7em と wasm32）には入れられず、入れるならホスト側の開発ツールに限る。
  crates.io の `rbs 4.8.4` は rbatis の ORM で無関係。`tree-sitter-rbs` は CST だけ。
* **mruby/edge の `.rbs` は RBS の部分集合ですらない。** `rbs` gem 3.6.1 に `def hello: () -> void` を
  食わせると ``Syntax error: cannot start a declaration, token=`def` ``。RBS のトップレベルに書けるのは
  宣言だけで、`def` は `class … end` の中にしか置けない。あれは RBS の見た目をした wasm の IDL。
  確かめる前に「小さい部分集合」と書きかけていた。
* **生成に足りないのは 2 つだけ。** `MethodSpec` が `sig.output` を読んでいないこと（1 フィールド）と、
  Ruby のクラス名が struct の `#[derive(RubyClass)]` 側にあって `impl` ブロックから見えないこと。
  後者が出力先を決めた: 実行時の定数（`<T as RubyClass>::NAME` が使える）が案 A、マクロがファイルを
  書く案 B は `OUT_DIR` が無い・`cargo check` でも走る・キャッシュを壊すのに加えてクラス名が取れない。
* **拾い物は引数名。** Rust の `fn damage(&mut self, n: i64)` の `n` が `(Integer n)` としてそのまま
  正しい RBS になる。エディタのホバーで効くのはここ。
* **Playground の補完は今はできない。** 効くのは組込みクラスだが、そこに型が無い。`src/` の
  `define_fn` の出現 16 件はすべて `convert.rs` の rustdoc の中で、実際の定義は 0 件。組込みは
  `define_closure` / `define_method`（171 箇所）で、型は本体の `expect_int` などの中にしかない。
  計画書 3 番の `coverage.md` と組んで「名前だけの補完」が先。
* **rubevy の `ask` に無いのは書式ではなく宣言する場所。** `Arg` 3 つ・`Answer` 7 つで語彙は
  閉じている——と思ったが `answer_value` / `publish_value` があるので閉じていない（予約 kind の
  `component.get` が Hash を返す）。宣言さえあれば実行時の検査も、知らない kind の即時エラーも、
  `Rubevy::Proxy#respond_to_missing?` が本当を言うことも落ちてくる。RBS はその文書化の出力形式。
  rubevy の repo の仕事。
* **試作は `macros/tests/rbs.rs`。** `src/` は 1 行も変えていない。proc-macro crate はマクロ以外を
  公開できないので、`src/` に置いても外から呼べず dead_code になる、という事情もある。
  出力は `rbs parse` と `rbs -I . validate`（rbs 3.6.1）を通った。
