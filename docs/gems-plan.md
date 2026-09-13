# gem 移植 実装指示書（順序 4 以降）

> 2026-09-13 実装済み（順序 4〜7 と UTF-8 文字列、require／load）。実装の説明と、この指示書から変えた点（理由付き）は
> [`gems.md`](gems.md)。移植後の作業は [`after-gems-plan.md`](after-gems-plan.md)。

対象: この文書だけを読んで、別セッションの実装者（AI）が SabiRuby に残りの本家 gem を移植できること。
作業前に `README.md`（Rules、Verification）、`docs/gems.md`（手順と、移植済み gem が教えたこと）、
`docs/gc.md`（ネイティブから見た GC の約束）を読むこと。設計判断はここに書いたとおりにし、
変えたい場合は理由を `docs/gems.md` に残す。

## 0. 前提と現状（着手時 2026-09-12、完了時の値を併記）

* 本家は mruby 4.1.0-rc（`../../ref/mruby`）。参照バイナリは Docker イメージ `kishima/mruby:4.1.0-rc`
  （バイト列ビルド、bigint 無し）。本家テストは `tools/mrbtest.sh` で走らせ、`docs/mrbtest.md` に表を書く。
  文字としての String は `kishima/mruby:4.1.0-rc-utf8`（同じ木を `MRB_UTF8_STRING` で建てたもの）と
  突き合わせる（`tools/mrbtest.sh` は既定が文字、`--bytes` がバイト）。
  着手時は文字ビルド 1916 件中 1877 件／バイトビルド 1866 件中 1819 件。
  **完了時は文字ビルド 2484 件中 2313 件／バイトビルド 2429 件中 2236 件**（`docs/mrbtest.md`、`docs/mrbtest-bytes.md`）。
  落ちる件の理由は `docs/mrbtest-notes.md` と `tests/mrbtest/notes.tsv`。
* 移植済み（完了時）: `default.gembox` の 33 gem のうち POSIX 依存の 7 つ（io／socket／errno／dir／env／signal／process）を
  除く全部と、gembox 外の cmath、pack、eval、binding、proc-binding、regexp、task。
  加えて UTF-8 文字列（ビルド構成、`docs/utf8.md`）と require／load（`docs/eval-require-plan.md` 5 節）。
* 残り: 無い。POSIX 依存の 7 gem は対象外（著者決定）。
* 本家テストを 1 件でも落とす gem を「移植済み」と呼ばない。落ちる件は理由を `notes.tsv` に書き、
  意図した差異なら `docs/gems.md` の「Deviations kept」にも書く。

## 1. 変えてはいけない約束（移植済み gem と同じ）

1. **gem が Ruby（mrblib）で定義するメソッドをネイティブで置かない**。gem の `mrblib/*.rb` は `tools/mrbtest.sh` が
   本家 `mrbc` で `src/mrblib/<gem>.mrb` に固め、`Vm::load_mrblib`（`src/vm.rs`）が gembox 順に読む。
   `lib.rs` に `MRBLIB_<GEM>_MRB` 定数を足し、`load_mrblib` の配列の正しい位置に入れる。
2. ネイティブは `src/builtins/ext_<gem>.rs` に置き、`src/builtins/mod.rs` の `init` で core の後に登録する
   （同名の core ネイティブを gem が置き換える順序）。ネイティブを書く前に「本家はそれを C で定義しているか」を確かめる。
3. テストは `tools/mrbtest.sh` の `GEMS` に gem 名を足すだけ。`test/*.rb` は `gem_<file>.rb` に、
   同名衝突は `gem_<gem>_<file>.rb` に複製される。C の補助コード（`test/*.c`）は `src/mrbtest.rs` の `install` に Rust で書く。
4. `-g` でコンパイルされる（LVAR と DBG が要る前提）。`Vm::backtrace`／`caller` は DBG の行番号を使う。
5. **no_std + alloc**（`tools/check_no_std.sh`）。`std` を使う箇所は `cli/` だけ。ホストから受け取るものは
   `Vm` の差し込み口（`gc_clock`、`wall_clock`、将来の `Host` trait）にする。
6. ネイティブの引数規約: 呼び出し側のキーワードは末尾の Hash で来て、`vm.pending_kw == Some(最後の引数)` のときだけ
   「キーワードとして渡された」と分かる（`ext_random.rs` の `random_kw`、`ext_struct.rs` の `init_body` が例）。
   ブロックへ転送するなら `Vm::call_block_with_self_kw` か `funcall` の pending 規則を使う。
7. ネイティブは呼ばれた名前を `vm.native_mid` で読める（入口でだけ有効。`ext_struct.rs` のアクセサが例）。
8. ネイティブ実行中は GC が走らない（`native_active`）。ネイティブが Ruby を呼び返す（`funcall`、`call_block`）間に
   保持する `ObjId` は、レジスタか `gc_register` で守る。長いループで大量に作るなら `docs/gc.md` の注意を読む。
9. 完了の定義: `tools/mrbtest.sh --update` で baseline 更新、`cargo test --release`、`SABIRUBY_GC_STRESS=1` で
   その gem のテストファイル、`tools/check_no_std.sh`、警告ゼロ。`docs/gems.md` に「What each gem needed」の項と
   残り表の更新。本の repo（`/home/kishima/book/book_mruby3`）の `docs/notes/sabiruby-findings.md` に節を足し、
   踏んだ罠は `data/porting_stages.yml`（段階 7）と `porting.re`（同じ文面）へ。コミットは gem ごと。push は著者。

## 2. 順序（決定）

```
bigint → rational → complex（+ cmath）→ pack → eval／binding／proc-binding → UTF-8（utf8-plan.md）→ regexp → task
```

bigint は著者決定（2026-09-12「入れましょう」）。bigint、rational、complex は core の Integer／Float 演算の同じ場所に
「相手の型で分岐する」コードを足す作業なので、本家 `numeric.c` の `#ifdef` の並び（BIGINT → RATIONAL → COMPLEX）と同じ順で
入れる。pack は依存が無いが `Q`／`J`（符号なし 64bit）の読み出しが bigint を要るので bigint の後。
eval はコンパイラ側の C パッチを伴うので独立した節にする。regexp は UTF-8 の後。

## 3. 各 gem の指示

### 3.1 mruby-pack（2133 C / 0 Ruby / 278 test）— **済み（2026-09-12）**

* 入口は 3 つ: `Array#pack(fmt)`、`String#unpack(fmt)`、`String#unpack1(fmt)`（`src/pack.c` 末尾）。
* `src/pack.c` の指示子の表（`a A Z b B h H c C s S l L q Q j J n N v V U w m M u f d e E g G x X @` と
  修飾 `_ ! < >`、`*`、数）を**そのまま**移植する。本家の `PACK_DIR_*`／`PACK_TYPE_*` の enum を Rust の enum にし、
  `pack_*`／`unpack_*` の各関数を 1 対 1 で書く。エンディアンは `to_le_bytes`／`to_be_bytes`、
  float は `f32::to_bits`／`f64::from_bits`。`m`（Base64）、`M`（quoted-printable）、`u`（uuencode）、
  `w`（BER）、`U`（UTF-8）は自前実装（依存を足さない）。
* `U` はバイト列ビルドでも UTF-8 を書き出す（本家も `MRB_UTF8_STRING` に依らない）。
* エラーメッセージ（`ArgumentError: unknown pack directive 'x'` など）は本家の文言で。
* 検証: `test/pack.rb` 全件。本家イメージとの照合スクリプトを `docs/gems.md` に残す
  （`[1.5].pack("e").bytes` のような境界値、負数の `w`、`Z*` の終端）。

### 3.2 mruby-rational（1512 C / 72 Ruby / 742 test）と mruby-complex（1087 C / 295 Ruby / 325 test）— **済み（2026-09-12）**

実装で決めたこと・分かったことは `docs/gems.md` の項と「Deviations kept」に書いた。以下は着手時の指示。

* 本家は `MRB_TT_RATIONAL`／`MRB_TT_COMPLEX` という専用の格納形（`struct RRational { mrb_int numerator, denominator }`、
  `struct RComplex { mrb_float real, imaginary }`）。SabiRuby では **`ObjKind::Object` に隠し ivar**
  （`__num`／`__den`、`__real`／`__imag`）で持つ（Random、Time と同じ流儀。`ObjKind` を増やさない）。
  `count_objects` では `T_OBJECT` に数えられる差異を `docs/gems.md` に書く。
* リテラル `3r`／`2i` はコンパイラが `Kernel#Rational(n, d)`／`Kernel#Complex(0, x)` への `OP_SSEND` に落とす
  （`mrbgems/mruby-compiler/src/codegen.c` の `PM_RATIONAL_NODE`／`PM_IMAGINARY_NODE`）。
  つまり VM 側に新しい命令は要らず、`Kernel#Rational`／`Complex` を gem が定義すれば動く。
* **core 側の分岐**: 本家 `src/numeric.c` は `MRB_USE_RATIONAL`／`MRB_USE_COMPLEX` で Integer／Float の
  `+ - * / ** <=> ==` に Rational／Complex の相手を受ける分岐を持つ（`mrb_rational_div`、`mrb_complex_div` など。
  `int_quo` は Rational があれば `Rational(x, y)` を返す）。SabiRuby は gem を常に組み込む前提なので、
  `src/builtins/numeric.rs` の該当演算に「相手が Rational／Complex なら gem の関数へ」の分岐を足す。
  分岐は gem の `ext_rational.rs`／`ext_complex.rs` の `pub(crate) fn` を呼ぶ形にし、numeric.rs には判定だけ置く。
* rational の `mrblib`（`Rational#to_s` などの Ruby）は読むだけ。complex の Ruby 部分（295 行）も同じ。
* 依存: complex は math（済）。rational のテストは complex を要求する（`add_test_dependency`）ので、
  **rational と complex は同じコミットでテストを通す**。cmath（425 C / 41 test、`default.gembox` 外）は complex の直後に。
* 検証: 本家テストに加え、`Rational(1, 3) + 1`、`1 / 3r`、`(1 + 2i) * (3 - 1i)`、`Complex(1, 2).abs`、
  `2 ** -1`（rational があると本家は Rational を返す。`gem_numeric_ext_numeric` の `Integer#pow` の期待が変わらないか確認）を
  本家イメージと照合。参照イメージには rational が入っている（`docker run --rm kishima/mruby:4.1.0-rc mruby -e 'p 1r'`
  が `(1/1)` を返す。2026-09-12 確認）。complex と cmath も同様に `p 2i` で確かめてから照合する。

### 3.3 mruby-eval、mruby-binding、mruby-proc-binding — **済み（2026-09-12、require を除く）**

実装で決めたこと・分かったことは `docs/gems.md` の項と `docs/eval-require-plan.md`。以下は着手時の指示。

* 設計は `docs/eval-require-plan.md` に決めてある。要点:
  VM に `Host` trait（`compile(source, scopes) -> Result<irep bytes>`、`read_file`）の差し込み口を置き、
  `sabiruby-compiler` が feature で `Host` を実装する（依存の向きは compiler → VM）。
  コンパイラ側は vendoring した C に「外側の変数名の表を受け取る」分岐（`SABIRUBY_EVAL_SCOPES`、3 か所、50 行前後）。
  既存の `sabiruby_mrc_compile` と黄金テストは不変。
* 段階: (1) eval（外側の変数なし）→ (2) eval（外側の変数あり。`upvar` の段数は `lv - 1`）→ (3) binding
  （`Kernel#binding`、`Binding#local_variable_get/set/defined?`、`local_variables`、`receiver`、`eval`）→
  (4) `Proc#binding` → (5) require（PicoRuby の `require.rb` を土台、`run_irep` と同じ入れ子ループ）。
* 本家 rc の `instance_eval`／`class_eval`（文字列）はターゲットクラスを漏らす問題があり master で修正済み。
  **rc の挙動に合わせない**（`tests/custom/eval_class_eval_scope`）。この差で落ちる本家テストは理由を `notes.tsv` に。
* `tests/custom/` に `# pending: eval`／`# pending: binding` の 4 件が置いてある。実装後は pending を外し、
  `tools/custom.sh` で通す（`.expected` は手で決めた期待、`.rc.out` は本家 rc の出力）。
* eval の VM 側は `src/builtins/ext_eval.rs`（設計 4.3）。フレームの環境（`REnv`）を Proc に付けて
  `Vm::run_irep` 相当で呼び出し元の `self`／ターゲットクラスで走らせる。`Vm::call_proc_with` の `override_tc` と
  `env` の扱い（`docs/fibers.md`、`vm.rs` の `call_proc_inner`）を先に読む。
* `mrb_f_eval` の `file`／`line` 引数はデバッグ情報にだけ効く。`__FILE__` の期待があれば DBG のファイル名に入れる。

### 3.4 mruby-bigint（6409 C / 0 Ruby / 529 test）— **済み（2026-09-12）**

実装で決めたこと・分かったことは `docs/gems.md` の「mruby-bigint」の項と「Deviations kept」に書いた。
以下は着手時の指示（記録として残す）。踏んだ罠のうち本家の不具合と見て合わせなかった 5 件
（`~x`、負数の `>>`、`div(Float)`、`% Float`、`dup`）は `tests/custom/bigint_reference_bugs.rb`。

**根拠**: 参照イメージ `kishima/mruby:4.1.0-rc` は bigint 入り（`2**63` → `9223372036854775808`、`2**64 * 2**64`、
`to_s(36)`、`pow(3, 1000)`、`divmod(-7)` の床除算まで答える。2026-09-12 確認）。SabiRuby の「溢れは RangeError」は
参照イメージとの差異で、本家テストの skip 4 件（array、gc、integer、literals）はそのために skip している。
rational、pack の照合も bigint がある前提の方が楽。

**表現（決定）**
* `ObjKind::BigInt(BigInt)` を足す。`BigInt { neg: bool, mag: Vec<u32> }`（絶対値を 32bit limb の little-endian、先頭ゼロなし、
  ゼロは `mag` 空）。本家の `mpz_t`（32bit limb、`sn`、`sz`）と同じ粒度。埋め込み最適化（`RBIGINT_EMBED_SIZE_MAX`）は写さない。
* クラスは `Integer`（`Value::Obj` だが `class_of` は `core.integer`）。`frozen?` は本家どおり **false**（RBigint は凍結されない。
  `(2**62).frozen?` は本家では RInteger で true だが SabiRuby は即値なので true。既存の差異のまま）。
* **正規化**: 多倍長の演算結果が `i64` に収まれば必ず `Value::Int` に戻す（本家 `bint_norm`）。`Value::Int` の範囲の値が
  `BigInt` として存在することは無い（`==`／`eql?`／`hash`／Hash のキーがこれに依存する）。
* 隠し ivar 方式は使わない（演算のたびに ivar を引くのは遅い）。`ObjKind` を増やすので `dup`（object.rs）、`inspect.rs`、
  `object.rs` の mark（辺は無い）、`ext_objectspace.rs` の `T_BIGINT`（`MRB_TT_BIGINT` = 27 番目、`T_RATIONAL` の後）、
  `count_objects` の表を更新する。

**手順**
1. **ローダー**（`src/rite.rs`）: pool 型 7 は「長さ 1 バイト、基数 1 バイト、数字列（長さバイト分）」。本家 `load.c` の
   `pool_data_len = len + 2` は長さバイト自身を含む。今のコードは長さバイトを読んだ後に `len + 2` バイト読むので 1 バイト多く、
   次の pool の型バイトを数字 `'8'`（56）として読んで `bad pool type 56` になる。長さバイトの後は `len + 1` バイトが正しい。
   `Pool::BigInt` は `(base: u8, digits: Vec<u8>)` にし、`OP_LOADL` で `BigInt::from_str(digits, base)` を作る（毎回作る。
   本家も pool から `mrb_bint_new_str` で作る）。`tools/mrbtest.sh` の literals テストで確かめる。
2. **`src/bigint.rs`**（VM 側、no_std）: 本家 `core/bigint.c` の順で写す。加減算（limb 単位の桁上がり）、乗算（本家は
   schoolbook + 大きい時 Karatsuba。schoolbook だけで始め、`docs/bench.md` に測って足りなければ Karatsuba）、
   除算（Knuth D、`mrb_bint_divmod` の床除算と `rem` の切り捨て）、`pow`、`powm`、`sqrt`（Newton）、シフト、
   ビット演算（負数は 2 の補数の意味。本家 `mrb_bint_2comp` の手順）、比較、`to_s(base)`／`from_str(base)`
   （2〜36。本家は `mrb_bint_to_s` で基数ごとの分割）、`as_float`（丸めは本家 `mrb_bint_as_float` の方法に合わせ、
   `2**64 == 18446744073709551616.0` が true になること）、`hash`（`mrb_bint_hash`: limb 列のバイトハッシュ。値が同じなら同じ）、
   `gcd`／`lcm`。関数名は `mrb_bint_*` に対応させて `bint_add` などとし、`docs/gems.md` に対応表を書く。
3. **core の分岐**（`src/builtins/numeric.rs`、`vm.rs` の算術命令）: 本家 `numeric.c` の `#ifdef MRB_USE_BIGINT` 62 か所に相当。
   * `+ - *`: `checked_*` が None なら多倍長へ昇格して計算（今の RangeError を置き換える）。相手が BigInt なら多倍長。
   * `/ % divmod`: `MRB_INT_MIN / -1` は多倍長。`Float` が相手なら Float。
   * `**`: 負の指数は Float（変更なし）、溢れは多倍長（`int_pow` の RangeError を置き換え）。
   * `<=> == eql? hash`: BigInt と Int の比較（正規化により同値なら必ず同じ表現なので、Int と BigInt が等しくなることは無い）。
     Float との比較は本家 `mrb_bint_cmp`（`integer.rb` の「mrb_int より広い整数と NaN」テスト）。
   * `& | ^ ~ << >>`: 溢れる左シフトは多倍長。`~x` は `-x-1`。
   * `to_s(base)`、`inspect`、`to_f`、`to_i`（Float → Integer で `1e30.to_i` は多倍長。`expect_int` の RangeError 経路を確認）、
     `Integer#size`（本家は limb 数×4。`docs/gems.md` に差異があれば記録）、`digits`、`bit_length`、`pow(b, m)`、
     `gcd`、`lcm`、`even?`／`odd?`、`abs`、`-@`、`succ`／`pred`、`times`／`upto`（mrblib は `+` で回るので分岐不要）、
     `Integer.sqrt`、`chr`（多倍長は RangeError）、`Integer()`／`String#to_i`（`str_to_integer` の overflow 経路を多倍長へ。
     ただし本家は `Integer("18446744073709551616")` を ArgumentError にする（badcheck 経路の癖）。同じにする）、
     `Comparable`、`Array#[]`（添字が多倍長なら RangeError、本家 `[][hi]` の挙動）、`Array#first(bigint)`。
   * `vm.rs` の `OP_ADD`／`OP_SUB`／`OP_MUL` などの高速経路は今の `checked_*` のまま。None のときだけ `numeric.rs` の関数へ
     落とす（本家と同じ形。高速経路は変えない）。
   * 他 gem: `ext_random.rs`（`rand(bigint)`: 本家は `mrb_bint_from_bytes` で乱数バイト列から作って `mod`）、`ext_time.rs`
     （`Time.at(bigint)` は `mrb_bint_as_int64` で RangeError）、`ext_kernel.rs`（`Integer()`）、`ext_numeric.rs`（`pow`、
     `digits`、`bit_length`、`gcd`／`lcm`、`Integer.sqrt` の bigint 分岐は本家 `numeric_ext.c` にそのままある）。
4. **テスト**: `tools/mrbtest.sh` の `GEMS` に `mruby-bigint`（`test/bigint.rb` 529 行）。今 skip している 4 件が通ることを確認
   （array、gc「OP_ADD does not retain an overflowed Integer」、integer、literals）。`tests/mrbtest/notes.tsv` の
   「needs mruby-bigint」を消す。`SABIRUBY_GC_STRESS=1` で bigint のファイル。
5. **照合**: 参照イメージと次を比べるスクリプトを `docs/gems.md` に残す。`2**63`、`2**64 * 2**64`、`-(2**64) / 7` と
   `divmod(-7)`、`(2**64).to_s(2).size`、`to_s(36)`、`pow(3, 1000)`、`(1 << 64) >> 63`、`~(2**64)`、`(2**64) & (2**63)`、
   `2**64 == 18446744073709551616.0`、`(2**64).to_f.to_i == 2**64`、`{2**64 => 1}[2**64]`、`"%d" % 2**64`、
   `Integer.sqrt(10**40)`、`(10**40).digits.size`、`(2**63).frozen?`（false）、`"18446744073709551616".to_i`。
6. **性能**: `tools/bench.sh` で fib など整数ベンチが変わらないこと（高速経路に触らない）。

**差異として書くもの**: 埋め込み最適化なし。`Integer#size` の多倍長の答え（本家は確保サイズ）。`(2**62).equal?(2**62)`
（本家 false、SabiRuby true。即値の幅の違い）。`(2**62).dup` が本家で 0 になるのは本家の不具合と見て合わせない
（`docs/gems.md` と本の corelib 章のメモに記録する）。

### 3.5 UTF-8 文字列 — 済み（2026-09-12）

案 B のまま実装した。feature `utf8` が既定で on、`--no-default-features` でバイト列ビルド。
文字か バイトかは「読み方」を引数で渡す形にまとめ（`string.rs` の `char_mode`／`char_len`／
`char_to_byte`…）、`String#b` の byte-read 文字列（`MRB_STR_ENCODING_BINARY`）も持つ。
本家が `mrb_str_substr` にバイト長を渡している 5 メソッド（`delete_prefix`／`delete_suffix`／
`strip`／`lstrip`／`rstrip`）は本家の取りこぼしなので CRuby の意味に合わせ、
`tests/custom/utf8_reference_bugs.rb` と `docs/utf8.md` に記録した。詳細は `docs/utf8.md`。

### 3.6 mruby-regexp — エンジンは `regex-automata`、表面は本家を移植（著者決定 2026-09-13）— **済み（2026-09-13）**

結果は `docs/gems.md`（「What each gem needed」の mruby-regexp と「Deviations kept」）、
テストの内訳は `docs/mrbtest-notes.md`、wasm のサイズは `docs/playground.md`。
本家テスト 501 件のうち、有限オートマトンに無い構文を使う 82 件と、両エンジンの答えが違う 39 件が意図した差異。
`backref_scope.rb`（64 件、`$~` のスコープ）と `match_data.rb`（53 件）は全件通る。

**決定**: 本家のエンジン（`re_compile.c`、`re_exec.c`、`re_utf8.c` 7,263 行と `re_ctype.h`／`re_cased.h` の表 3,611 行）は
移植せず、Rust の `regex-automata` を使う。Ruby 側の表面（`src/regexp.c` 3,677 行相当）は本家から移植する。
正規表現の仕様は Rust のエンジンに寄せる（著者「正規表現の仕様は Rust 製のものによせてもよい」）。
本家テスト（14 ファイル 501 件）のうち、有限オートマトンに無い構文を使う件は**意図した差異**として数え、直さない。

**参考: mruby/edge** は `regex` クレート（`std` 付き）を Cargo feature で任意にし、`Regexp.new`／`=~`／`!~`／`match`／
`MatchData#[]` だけを 307 行で提供している（`ref/mrubyedge/mrubyedge/src/yamrb/prelude/regexp.rs`）。SabiRuby はそこまで
削らず、`String#gsub` など表面は本家どおりにする。

**クレート（決定）**
* `regex-automata = { version = "0.4", default-features = false, features = ["alloc", "syntax", "meta", "nfa-pikevm", "nfa-backtrack"] }`。
  `regex` クレート本体は `std` 前提なので使わない。`regex-automata` は no_std + alloc で動く（0.4.18 の Cargo.toml で確認済み）。
  Unicode の表（`unicode-*`）は `utf8` feature のときだけ有効にし、バイト列ビルドでは `unicode(false)`・`utf8(false)`。
  wasm のサイズは Playground で両方測って `docs/playground.md` に書く（未計測）。
* 使う API は `meta::Regex`（`Config` で `case_insensitive`、`multi_line`、`dot_matches_new_line`、`ignore_whitespace`、
  `unicode`、`utf8`）と `Captures`。`&[u8]` を検索するので String の格納形（バイト列）にそのまま合う。
* 依存が増えるので `tools/check_no_std.sh` と CI で no_std ビルドが通ることを最初に確かめる。

**翻訳層**（`src/builtins/ext_regexp.rs` の前段。Ruby の構文 → `regex-syntax` の構文）
* 常時: `^`／`$` は行頭行末（`multi_line(true)` を常に on）。`/m` は `dot_matches_new_line`。`/x` は `ignore_whitespace`。`/i` は `case_insensitive`。
* 書き換えるもの: `\h`→`[0-9a-fA-F]`、`\H`、`\d`→`[0-9]`、`\w`→`[0-9A-Za-z_]`、`\s`→`[ \t\r\n\f\v]`（Ruby は ASCII、Rust は Unicode 既定。
  否定形も）、`\Z`→`(?:\n?\z)` は先読みが無いと書けないので `\z` に落として差異に数える、クラス内の `\b`（バックスペース）、
  8 進 `\NNN`、`(?imx-imx)` と `(?imx-imx:…)`（Rust も持つが `x` の扱いを確認）、`(?#…)` コメント、`(?'name'…)`→`(?<name>…)`。
* そのまま通るもの: `[[:alpha:]]`、`[a-z&&[^aeiou]]`、`(?<name>…)`、`(?:…)`、`\p{…}`（utf8 ビルド）、`\A`、`\z`、`\b`、`\B`、
  非貪欲 `*?`、`{n,m}`。
* **対応しない構文**は `RegexpError` にして、文言に構文名を入れる（例: `backreference \1 is not supported by this engine`）:
  後方参照 `\N`／`\k<…>`、先読み `(?=` `(?!`、後読み `(?<=` `(?<!`、アトミック `(?>`、絶対最大量指定子 `*+ ++ ?+`、
  部分式呼び出し `\g<…>`、条件 `(?(…)…)`、不在演算子 `(?~…)`。`regex-syntax` が `UnsupportedBackreference`／`UnsupportedLookAround`
  で拒む構文はこの表と一致する。
* 本家テストでの出現回数（差異の見積もり）: 後方参照 182、先読み 120、後読み 147、アトミック＋絶対最大 110、`\g` 128、条件＋不在 113。
  `regexp_syntax.rb`（148 件）、`backref_scope.rb`（64 件）、`regexp_call.rb`（15 件）はほぼ落ちる。
  表面のテスト（`string_regexp` 74、`string_index` 41、`match_data` 53、`regexp` 40、`symbol_regexp` 11、`regexp_utf8` 40）は
  単純なパターンが多く、大半が通るはず。バックトラックの段数上限（`MRB_REGEXP_STACK_LIMIT`）や step 上限のテストも落ちる（エンジンに上限が無い）。

**表面（本家 `regexp.c` を移植）**
* `Regexp`: `new`／`compile`（文字列と `Regexp`、オプションは整数・文字列・true）、`escape`／`quote`、`union`（0 個は `/(?!)/`。
  先読みが無いので「決してマッチしない」パターンは `\b\B` などで代用し差異に書く）、`last_match`、`source`、`options`、`casefold?`、
  `names`、`named_captures`、`==`／`eql?`／`hash`、`=~`、`===`、`match`、`match?`、`~`、`to_s`、`inspect`、`freeze`、定数 `IGNORECASE`／
  `EXTENDED`／`MULTILINE`。リテラル `/…/` はコンパイラが `Regexp.new` 相当（`OP_STRING` + `Regexp.compile` の SEND）に落とすので VM 命令は不要。
* `MatchData`: `[]`（番号、名前、範囲、負）、`begin`／`end`／`offset`／`byteoffset`、`captures`、`named_captures`、`names`、`pre_match`／`post_match`、
  `regexp`、`string`（凍結複製）、`size`／`length`、`to_a`、`to_s`、`values_at`、`inspect`、`deconstruct`／`deconstruct_keys`。
* `String`（本家が `__split` などに退避してから置き換えるもの）: `=~`、`match`、`match?`、`scan`、`split`、`sub`／`sub!`／`gsub`／`gsub!`
  （置換文字列の `\1`、`\k<name>`、`\0`、`\&`、`` \` ``、`\'`、Hash、ブロック内の `$~`）、`[]`／`slice`／`slice!`、`index`／`rindex`、
  `partition`／`rpartition`、`start_with?`、`each_line` は対象外。`Symbol#=~`／`match`／`match?`／`[]`（symbol-ext の `Symbol#slice` は
  `String#slice` に委ねる設計なので自動で効く）。
* **`$~` と `$1`〜`$9`、`$&`、`` $` ``、`$'`**: 本家はメソッドフレームごとの特別変数（`OP_GETSV`／`OP_SETSV` → `mrb_vm_special_get/set`、
  `src/variable.c`）。SabiRuby ではこの 2 命令がまだ一度も実行されていない（`docs/mrbtest.md` の never 一覧）。
  `CallInfo` に `last_match: Option<Slot>` を持たせ、`=~`／`match`／`scan`／`gsub` が**呼び出し元のフレーム**（ネイティブは
  フレームを積まないので `vm.ci.last()`）に書く。ブロック内では作られたフレームの環境をたどる本家の規則（`mrb_vm_special_get` を読む）に合わせる。
  GC のルートに足す（`docs/gc.md` の表を更新）。
* 大小文字: Ruby の `/i` は ASCII と Unicode の単純折り畳み、Rust は Unicode の simple case folding。差が出る文字（`ſ`、`K` など）は差異に書く。

**手順**
1. `regex-automata` を依存に足し、no_std ビルドと wasm ビルド（`unicode-*` 有無で 2 通り）のサイズを測る。
2. `$~` の特別変数を VM に入れる（`OP_GETSV`／`OP_SETSV`、フレームの欄、GC ルート）。regexp 無しでも `$~` が nil を答えることを確かめる。
3. 翻訳層と `Regexp`／`MatchData`。
4. `String`／`Symbol` のメソッド。string-ext の `Symbol#slice` と core の `String#split`／`index` が Regexp を受ける分岐。
5. `tools/mrbtest.sh` の `GEMS` に `mruby-regexp`。`backref_scope.c` は C 補助なので `src/mrbtest.rs` へ。
   落ちる件は `notes.tsv` に「engine: no backreference」のように**構文ごとに**分類して数える。`regexperror.rb`（本家テスト）が空でなくなり、
   `superclass.rb` の skip も外れる。
6. `docs/gems.md` に「Deviations kept」として構文の表と件数を書く。本の移植章には「正規表現エンジンは置き換えてよい実装都合だが、
   後方参照・先読み・アトミックは仕様の差になる。mruby/edge は同じ選択で表面を最小にしている」と書く（`extension.re` の
   「mruby-regexp は NFA エンジン」は本家の説明なので変えない）。

### 3.7 mruby-task（2390 C / 46 Ruby / 860 test、`default.gembox` 外）— **済み（2026-09-13）**

実装は `src/builtins/ext_task.rs`（スケジューラの状態は `Vm::task`）、Ruby 側は本家の `mrblib/queue.rb` を
`src/mrblib/task.mrb` に、C のテスト補助（`test/tasktest.c`）は `src/mrbtest.rs` に。
本家テストは `task` 43/43、`queue` 23/23、`gc_task` 4/6（`GC.generational_mode` が常に false）、
`proc_set_stack` は 0 件（`respond_to?` で自分を飛ばす）。詳細と差異は `docs/gems.md`。
計画との違いは tick の出どころ: タイマー割り込みが無いので命令数で数える（`TaskState::tick_every`）。


* 目的は rubevy の「毎フレーム少し進める」ループ。`Vm::step` と Fiber のコンテキストの上に、
  優先度付きの実行キュー、`Task.pass`／`sleep`／`join`／`Task::Queue`、tick による横取りを載せる。
* HAL（`hal_tick`、`sleep_us`、idle）はホストの差し込み口（`Vm::wall_clock` と同じ流儀の関数ポインタ）。
  本家の `mrb_task_run` はブロックするので、rubevy には `run_once`（1 tick 分だけ進める）の形を用意する。
* `docs/gc.md` の「スケジューラ駆動の GC」（idle で回収）はここで入れる。
* 本家 `mrbgems/mruby-task/test/gc_task.rb` は GC の細部（`GC_STEP_SIZE`）に依存する。落ちる件は理由を書く。

## 4. 落とし穴（今日の 17 gem で踏んだもの。`docs/gems.md` に詳細）

* core のネイティブが gem の Ruby 定義を隠す（`clamp`、`tap`、`zero?` など）。gem を入れたら core 側を消す。
* 本家が C で定義するメソッドが core の段階で既にあるなら `ext_<gem>.rs` へ移す（`Integer#gcd` など）。
* テストの同名ファイル衝突（3 参照）。
* `Vm::inspect` の再帰の印はクラスで決まる。新しい格納形を既存の `ObjKind` に相乗りさせるときは、
  `Kernel#Array`／splat／`inspect` など「格納形で判断する経路」が意図どおりか確かめ、差異を書く。
* `Range` は凍結されていない、alias は別 Proc、Integer に特異クラスは無い、backtrace は DBG の無いフレームを飛ばす
  （いずれも `porting_stages.yml` 段階 7 に記載）。
* 本家 C ソースと参照イメージが食い違うことがある（`Integer("-9223372036854775808")`）。ソースを正とし、イメージとの差は記録。

## 5. 記録

* 実装で分かったことは `docs/gems.md`（gem ごとの項）、本の repo の `docs/notes/sabiruby-findings.md`（節 18 以降）、
  `data/porting_stages.yml` 段階 7 と `porting.re` の「踏みやすい点」へ。本文（`.re`）に Rust の名前は書かない。
* この文書は着手時に「実装済み」の注記を頭に足し、変えた判断は `docs/gems.md` に理由付きで書く（`gc-plan.md` と同じ運用）。
