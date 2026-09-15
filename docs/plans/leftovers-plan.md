# 小さな残り（実装指示書）

作成 2026-09-16。各段階が「判断待ち」「範囲外」として残した小さな項目を 1 か所に集めたもの。どれも半日以内。
著者の判断: `from-mrubyedge-plan.md` と `perf3-plan.md` の間か、implementer の手が空いたときに。1 つずつ別コミット、worklog は 1 本（`YYYY-MM-DD-leftovers.md`）。

## 状況

| # | 内容 | 出どころ | 状態 |
|---|---|---|---|
| 1 | `foo(**{})`（空のキーワード Hash）を `native_args` が落とす。本家は届ける | 6c | 未着手 |
| 2 | マクロの生成する `register` に残る `expect` 1 つ → `VmResult` を返す形に | 6a | 未着手 |
| 3 | `Player.allocate` で作った素のオブジェクトにメソッドを呼んだときの文言（`wrong argument type Player (expected Player)`）を読めるものに | 6a | 未着手 |
| 4 | `#[ruby_methods]` でブロックを取るメソッド（`define_fn` の `Block` を通す） | 6a | 未着手 |
| 5 | `Kernel#printf` / `#putc` が無く、本家のベンチ `bm_ao_render` と `bm_mandel_term` が動かない | 段階 1 から | 未着手 |
| 6 | `items()` が配列を複製する箇所の残り（中身を読むだけのもの。ベンチには出ないが実コードで効く） | 2d | 未着手 |
| 7 | rubevy の `Rubevy.ask` の `Arg` に Hash/Array を運べない（`Arg::Value`。今は数値と文字列と Entity だけ） | ECS の橋 A | 未着手 |
| 8 | SabiRuby の公開 API で足りなかったもの: Hash のキー列挙、`Task::Queue` の長さと非ブロッキング pop（rubevy が `funcall` で代用している） | ECS の橋 B | 未着手 |

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
