# 小さな残り（実装指示書）

作成 2026-09-16。各段階が「判断待ち」「範囲外」として残した小さな項目を 1 か所に集めたもの。どれも半日以内。
著者の判断: `from-mrubyedge-plan.md` と `perf3-plan.md` の間か、implementer の手が空いたときに。1 つずつ別コミット、worklog は 1 本（`YYYY-MM-DD-leftovers.md`）。

## 状況

| # | 内容 | 出どころ | 状態 |
|---|---|---|---|
| 1 | `foo(**{})`（空のキーワード Hash）を `native_args` が落とす。本家は届ける | 6c | **済み**（2026-09-16、`ce3a134`） |
| 2 | マクロの生成する `register` に残る `expect` 1 つ → `VmResult` を返す形に | 6a | **済み**（2026-09-16、`758e1a2`） |
| 3 | `Player.allocate` で作った素のオブジェクトにメソッドを呼んだときの文言（`wrong argument type Player (expected Player)`）を読めるものに | 6a | **済み**（2026-09-16、`8d7bd9d`） |
| 4 | `#[ruby_methods]` でブロックを取るメソッド（`define_fn` の `Block` を通す） | 6a | **済み**（2026-09-16、`a9461d8`） |
| 5 | `Kernel#printf` / `#putc` が無く、本家のベンチ `bm_ao_render` と `bm_mandel_term` が動かない | 段階 1 から | 未着手 |
| 6 | `items()` が配列を複製する箇所の残り（中身を読むだけのもの。ベンチには出ないが実コードで効く） | 2d | 未着手 |
| 7 | rubevy の `Rubevy.ask` の `Arg` に Hash/Array を運べない（`Arg::Value`。今は数値と文字列と Entity だけ） | ECS の橋 A | 未着手 |
| 8 | SabiRuby の公開 API で足りなかったもの: Hash のキー列挙、`Task::Queue` の長さと非ブロッキング pop（rubevy が `funcall` で代用している） | ECS の橋 B | **済み**（2026-09-16、`6af8276`）。rubevy 側の置き換えは未（`reflect.rs:136`、`lib.rs:782`・`789`） |
| 9 | coverage が見つけた「本家にあって本当に無い」もの: `Hash#default_proc=`、`Numeric#fdiv`、`Module#const_added`/`#method_undefined`、`BasicObject#singleton_method_added`/`_removed`/`_undefined` | coverage | **済み**（2026-09-16、`ef4611f`）。`coverage.md` の「本家だけ」22 → 15 |
| 10 | 可視性の食い違い 50 件（本家が private、SabiRuby が public。`Module#private`/`module_function`/`included`/`method_added` の類）と、トップレベルの `def` が private にならない件 | coverage | **著者判断待ち**（直すか「意図した差分」にするか）。項目 9 のフック 5 件が加わって 45 → 50 |

## 各項目

1. **`foo(**{})`**: `native_args` がキーワード Hash を「空なら無かったことに」している箇所を本家（`mrb_get_args` の `:` と `OP_ENTER` の kw の扱い）と突き合わせ、
   空でも届ける。`tests/custom/` に本家の出力で期待値を置く。`send` と `method_missing` の再ディスパッチも同じ経路なので一緒に直る。
2. **`register` の `expect`**: `singleton_class(...)` が失敗しうるのは到達不能のはずだが、生成コードに `expect` を残さない。`T::register(&mut Vm) -> VmResult<ObjId>` にする
   （破壊的変更だが利用者はまだ無い）。`macros/tests/expand.rs` の固定を更新。
3. **`allocate` の文言**: `This<DataRef>` の変換失敗を「`Player` のインスタンスが Rust の実体を持っていない（`allocate` で作られた）」と分かる文言に。
4. **ブロックを取るメソッド**: `#[ruby_methods]` の `fn f(&self, vm: &mut Vm, blk: Block)` を通す（`define_fn` に形はある。5 行程度）。テストを `player.rs` に 1 本。
5. **`printf` / `putc`**: 本家の `mruby-print`/`mruby-sprintf` を見て、`Kernel#printf`（`sprintf` の上に）と `#putc` を足す。`bm_ao_render` と `bm_mandel_term` が動くようになるので、
   `bench/categories.tsv` に分類を足し、基準を取り直す（`docs/verification/bench.md` に「23・24 本目」として）。
6. **`items()` の残り**: `grep -n "items(vm" src/builtins/` で一覧を作り、中身を読むだけのものから `Heap::array` の借用に。ブロックを呼ぶものは添字で。
   micro（`ab_micro.sh` の流儀）で 1 つずつ前後を取る。
7. **`Arg::Value`**: `Rubevy.ask` の引数に Hash/Array を許す。`Value` を `gc_register` して運び、答えた後に解除する（`Request` が drop されたときに解除する形にすると漏れない）。
   `Request::value(i)` と、`FromRuby` で取り出す補助。
8. **公開 API の穴埋め**: `Vm::hash_keys(h) -> Vec<Value>`、`Vm::task_queue_len(q)`、`Vm::task_queue_try_pop(q) -> Option<Value>`。rubevy の `funcall` 代用を置き換える。

## 守ること

`host-bridge-plan.md` と同じ。5 と 6 は性能に触るので交互 A/B（5 はベンチの本数が変わるので基準の取り直し）。
9. **coverage の穴**: `docs/verification/coverage.md` の「本家だけ」22 件のうち効くもの。`Hash#default_proc=` は本家 `mrb_hash_default_proc_set`（`hash.c`）、
   `Numeric#fdiv` は `mruby-numeric-ext`、フックは `mrb_method_added` 系の呼び出し箇所（`class.c`）と突き合わせる。各 1 本 `tests/custom/`。終わったら `tools/coverage.sh` で生成し直す。
10. **可視性**: 50 件の一覧は `coverage.md` の「両方にあるが可視性が違う」。直すなら `define_methods` に可視性を渡す形（本家の `MRB_METHOD_PRIVATE_FL`）と、
    `OP_DEF` がトップレベル（`self` が main）で private にする本家の規則（`vm.c` の `OP_DEF` → `mrb_define_method_raw` の可視性）。`respond_to?` と `send` の差にだけ効くので、
    mrbtest には出ていない。著者が「意図した差分」と決めるなら `docs/design/gems.md` の Deviations kept に 1 項。



## 実装で分かったこと（2026-09-16、項目 1・2・3・4・8・9）

作業の記録は [`../worklog/2026-09-16-leftovers.md`](../worklog/2026-09-16-leftovers.md)。
確認: `cargo test --workspace` 全通過、`tools/check_no_std.sh` 通過、
`tools/mrbtest.sh` の 3 ビルドとも `tests/mrbtest/baseline*.txt` と一致（2507 中 2344 通過、変化なし）、
VM の crate の `unsafe` 0、`cargo doc` の rustdoc 警告 0。

1. **空のキーワード Hash は「畳み込まない」であって「無い」ではない。** 本家の `mrb_get_args` も
   `vm_op_enter` も、キーワード Hash を位置引数に畳み込むのは中身があるときだけだが、**空のときは `ci->kw` を立てたまま**残す。
   `OP_ENTER` の速い経路が `argc + ci->kw != m1` と数えるので、本家では `def one(x); end; one(**{})` の `x` が `{}` になる
   （CRuby は ArgumentError なので mruby の方言）。SabiRuby はバイトコードからの直接の呼び出しだけこの規則を写していて、
   フレームを書き換えて配り直す `send` と `method_missing` で落としていた。
   * **`send` と `method_missing` はフレームの作り方が違う。** `send_method` は呼ばれた形を保つ（`n == 15` は 15 のまま
     `mrb_ary_subseq`）が、`prepare_missing` は `mrb_args_pack_positional` を通るので**常に `n = 15`**。
     `2026-09-15-stage6c-method-missing.md` の「詰めるのとずらすのは `OP_ENTER` から見れば同じ」は、
     キーワード Hash があるときだけ成り立たない（速い経路は `argc < 15` でしか通らない）。
     `Enumerator#__enumerator_block_call` の `@obj.__send__ @meth, *@args, **@kwd, &block` が、ほどくと落ちる実例。
   * **残った差**: `public_send` はネイティブから `vm.funcall` へ落ちるので空のキーワード Hash を運べない。
     本家の `public_send` は `send_method(mrb, self, TRUE)` で `send` と同じ経路なので、
     `op_send_redirect` に可視性の判定を足せば揃う（今回の範囲外）。
2. **`register` の `expect`** は消えた（`T::register(&mut Vm) -> VmResult<ObjId>`）。rubevy はまだ `#[ruby_methods]` を
   使っていない（クラス登録は手書き）ので、**rubevy に直す呼び出しは無い**。`rubevy/src/lib.rs:1215` の
   `vm.singleton_class(...).expect("Rubevy singleton")` は rubevy 自身が書いたもので、これとは別。
3. **本家に同じ文言がある。** `mrb_data_check_type`（`src/etc.c`）は `DATA_TYPE` が NULL のとき
   `uninitialized %t (expected %s)` を投げ、`%t` はオブジェクト自身のクラス名。`Time.allocate.to_i` が
   `uninitialized Time (expected Time)`、部分クラスなら `uninitialized MyTime (expected Time)`。それに寄せた。
   `convert.rs` の `DataRef::from_ruby` は期待するクラスを持たないので触っていない（本家も引数側は
   `wrong argument type Object (expected Data)` で、SabiRuby の今の文言と一致している）。
4. **`define_fn` には最初から形があった**（`MVmBlock`、`MVmThisBlock`）。マクロが読んでいなかっただけで 5 行。
   `Block` 無しの形が `define_fn` に無いので、`Block` を取るメソッドは `&mut Vm` も取る（ブロックを呼ぶのに要る）。
   ブロックは arity に数えない。`&mut Vm` を取るメソッドは受け手を店から出したまま走るので、
   **ブロックの中から同じオブジェクトに触ると弾かれる**（`Player is already in use by a call on the same object`）。
5. **rubevy が `funcall` で代用していたのは 3 か所**（項目 8）: `reflect.rs:136`（`:keys`）、
   `lib.rs:782`（`:size`）、`lib.rs:789`（`:__pop_try(true)`）。後ろの 2 つは `ScriptWorld::make_room` の中で毎フレーム回りうる。
   `hash_keys` は `hash_entries` と同じ形（挿入順、Hash でなければ `None`、Ruby を走らせない）に揃えた。
   `task_queue_try_pop` は空なら `None`（ホストはタスクではないので park するものが無く、
   `Task::Error` を上げるのも違う）。閉じたキューも `None`（区別が要るなら `closed?`）。
6. **`Numeric#fdiv` は `mruby-numeric-ext` ではなくコア**（`src/numeric.c` の `numeric_rom_entries`）。
   計画書の出どころが違っていた。`const_added` も読み違えた: `mrb_vm_define_class` は直接 `mrb_const_set` を
   呼ばないので `class Foo; end` では発火しないと思ったが、`setup_class` の中身が `mrb_const_set` そのもので、
   **クラス定義でも発火する**。本家で期待値を作って初めて分かった。
7. **`singleton_method_added` の既定が `Object` にあったせいで、判定が一度も当たっていなかった。**
   `Vm::method_added` の「既定の no-op なら呼ばない」は `owner == Module || owner == BasicObject` を見る。
   本家は `bob_rom_entries`、つまり `BasicObject`。`BasicObject` へ移して初めてその判定が効く
   （それまでは特異メソッドを定義するたびに `funcall` が 1 本走っていた）。
   `ext_metaprog.rs` の `remove_method` は既定を確かめずに無条件で `funcall` していたので、
   `singleton_method_removed` の既定が無いまま特異メソッドを `remove_method` すると NoMethodError になっていたはず。
8. **見つけたが直していない差**（どれも今回の項目の外）:
   * `7.fdiv(0)`: 本家は ZeroDivisionError（`int_fdiv` の `if (y == 0) mrb_int_zerodiv`）、SabiRuby は `Infinity`。
   * `1.0.fdiv("2")`: 本家 `String cannot be converted to Float`、SabiRuby `String can't be coerced into Float`。
   * `public_send(**{})`（上の 1 を参照）。
