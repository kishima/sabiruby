# gem 移植 実装指示書（順序 4 以降）

対象: この文書だけを読んで、別セッションの実装者（AI）が SabiRuby に残りの本家 gem を移植できること。
作業前に `README.md`（Rules、Verification）、`docs/gems.md`（手順と、移植済み 28 gem が教えたこと）、
`docs/gc.md`（ネイティブから見た GC の約束）を読むこと。設計判断はここに書いたとおりにし、
変えたい場合は理由を `docs/gems.md` に残す。

## 0. 前提と現状（2026-09-12）

* 本家は mruby 4.1.0-rc（`../../ref/mruby`）。参照バイナリは Docker イメージ `kishima/mruby:4.1.0-rc`
  （バイト列ビルド、bigint 無し）。本家テストは `tools/mrbtest.sh` で走らせ、`docs/mrbtest.md` に表を書く。
  現在 **1505 件中 1461 件**。落ちる 44 件の理由は `docs/mrbtest-notes.md` と `tests/mrbtest/notes.tsv`。
* 移植済み: `default.gembox` の 33 gem のうち 28（fiber、enumerator、*-ext 5 種、sprintf、metaprog、proc-ext、
  method、compar-ext、toplevel-ext、enum-chain、enum-lazy、object-ext、symbol-ext、kernel-ext、class-ext、
  numeric-ext、catch、objectspace、math、random、struct、data、set、time）。
* 残り: eval、binding、proc-binding（順序 4）、pack、bigint、rational、complex、cmath（順序 5）、
  UTF-8 文字列（ビルド構成のマイルストーン、`docs/utf8-plan.md`）、regexp（順序 6）、task（順序 7）。
  io／socket／errno／dir／env／signal／process は POSIX 依存で対象外（著者決定）。
* 本家テストを 1 件でも落とす gem を「移植済み」と呼ばない。落ちる件は理由を `notes.tsv` に書き、
  意図した差異なら `docs/gems.md` の「Deviations kept」にも書く。

## 1. 変えてはいけない約束（移植済み gem と同じ）

1. **gem が Ruby（mrblib）で定義するメソッドをネイティブで置かない**。gem の `mrblib/*.rb` は `tools/mrbtest.sh` が
   本家 `mrbc` で `src/mrblib_<gem>.mrb` に固め、`Vm::load_mrblib`（`src/vm.rs`）が gembox 順に読む。
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
pack → rational → complex（+ cmath）→ eval／binding／proc-binding → UTF-8（utf8-plan.md）→ regexp → task
bigint は著者判断（4.3）
```

pack は依存が無く自己完結で、着手に最適。rational／complex は core の Integer／Float 演算に分岐を足す作業で、
eval より小さい。eval はコンパイラ側の C パッチを伴うので独立した節にする。regexp は UTF-8 の後。

## 3. 各 gem の指示

### 3.1 mruby-pack（2133 C / 0 Ruby / 278 test）

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

### 3.2 mruby-rational（1512 C / 72 Ruby / 742 test）と mruby-complex（1087 C / 295 Ruby / 325 test）

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
  本家イメージと照合。**参照イメージには rational／complex が入っていない可能性がある**（`default.gembox` に
  無い gem は入っていない。`docker run --rm kishima/mruby:4.1.0-rc mruby -e 'p 1r'` で確かめる）。
  入っていなければ照合は本家のテストと C ソースの読みだけで行い、その旨を `docs/gems.md` に書く。

### 3.3 mruby-eval、mruby-binding、mruby-proc-binding

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

### 3.4 mruby-bigint（著者判断）

* `MRB_USE_BIGINT` は Integer の溢れの意味を変える（RangeError → bigint に昇格）。参照イメージは bigint 無しで、
  本家テストの `skip: needs mruby-bigint` はそのままの方が一致する。**入れるなら UTF-8 と同じく別の参照イメージ
  （`4.1.0-rc-bigint`）と別の baseline を持つ**（`docs/utf8-plan.md` の方式）。著者に確認してから着手。
* 入れる場合の形: `Value::Int` の溢れを `ObjKind::Object` + 隠し ivar ではなく **`ObjKind::BigInt(Vec<u32>)`** にする
  （演算のたびに ivar を読むのは遅すぎる）。`core/` の多倍長演算（6409 行）は写す。

### 3.5 UTF-8 文字列

`docs/utf8-plan.md`（案 B: feature `utf8` 既定 on、バイト列モードも残す、参照イメージ `4.1.0-rc-utf8` と baseline を別に持つ）。
regexp の前に終える。string-ext の `tr`／`delete` などバイト単位の関数はここで文字単位の分岐を足す。

### 3.6 mruby-regexp（10940 C / 42 Ruby / 10213 test）

* 4.0.0 の NFA エンジン。最大の単体。UTF-8 の後。`test/` に 13 ファイル（`backref_scope.c` は C 補助）。
* `String#slice`／`[]`／`=~`／`sub`／`gsub`／`scan`／`split` など、string-ext と symbol-ext（`Symbol#slice` は
  `String#slice` に委ねる設計）が regexp の有無で分岐する箇所を `grep -n "REGEXP\|mrb_regexp" src/string.c mrbgems/mruby-string-ext` で洗い出してから。
* `regexperror.rb`（本家テスト）は regexp が入ると空でなくなる。`superclass.rb` の skip も外れる。
* 自前のエンジンを書かず、本家 `src/*.c` の構造（コンパイル → NFA → 実行、backtracking stack の上限）を写す。

### 3.7 mruby-task（2390 C / 46 Ruby / 860 test、`default.gembox` 外）

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
