# 性能の第 3 弾（実装指示書）

作成 2026-09-16。`host-bridge-plan.md` の段階 2〜2d で本家比 4.3x → 2.8x にした続き。残りの重さは `docs/design/optimizations.md` 5 節。
著者の判断: ECS の橋（rubevy `ecs-bridge-plan.md`）→ `from-mrubyedge-plan.md` の後に着手。目標は「本家比 2 倍台前半」だが、数値目標より
**切り分けを先に、1 つずつ交互 A/B で**（`optimizations.md` 4 節）を守る。

## 状況

| 段階 | 内容 | 状態 |
|---|---|---|
| 3a | String（4.6x）: `<<`/`+` の割り当て、文字数え | 未着手 |
| 3b | 命令ループの素の速さ（`vmo_dispatch`/`vmo_arith` 3.7x） | 未着手 |
| 3c | 呼び出し（`call_args` 2.9x、`vmo_calls` 3.5x）: `CallInfo` の積み下ろし、`OP_ENTER` | 未着手 |
| 3d | `Slot` 8 バイト化の実験（値の表現） | 未着手（3b の結果を見てから） |
| 3e | `bm_so_lists` の残り（4.4x）: `push`、`reverse!`、`==` | 未着手 |

## 3a. String

**切り分けから**: `ds_string` を `<<`、`+`、`[]`（済み）、`==`、`size`、`each_char` に割り、多バイトと ASCII の両方で測る。仮説:
`<<` と `+` が毎回 `Vec<u8>` を作り直している（`+` は本家も新しい文字列を作るが、`<<` は伸ばすだけのはず）、`size` が UTF-8 を数え直している
（ASCII 判定の早道は `[]` にだけ入れた）、`str_new` の割り当てとヒープの `HeapObject` の大きさ。

**候補**: `<<` を in-place の `extend`（借用を持ち越さない形で）、ASCII だけの文字列に旗を立てて文字数え・添字を O(1) に（本家の `MRB_STR_ASCII` 相当。
`utf8` feature のときだけ意味がある）、`HeapObject` の `ivars: Vec` を空のときに割り当てない。

## 3b. 命令ループ

**切り分けから**: `vmo_dispatch`（命令の分岐だけ）、`vmo_arith`（整数演算）、`loop_*` の 3 本で、1 命令あたりのナノ秒を出す（`--stats`）。本家は同じ命令列で何 ns か。
仮説: `exec_frames` が 9000 行の 1 つの関数で、レジスタ割り当てが悪い（コード配置の揺れ ±4〜12% がその証拠）、オペランドのデコード（`read_b`/`read_s`）の分岐、
`self.stack[base + i]` の境界チェック、`Slot::get`/`from` の往復。

**候補**（`unsafe` なしで）: 命令本体を種類ごとの関数に分けて `exec_frames` を小さくする（インライン化の判断をコンパイラに戻す）、
`ADD`/`SUB`/`LT` などの整数の早道を `match` の先頭に、オペランドのデコードをテーブル駆動から命令ごとの固定長読みに、
レジスタの窓を `&mut [Slot]` のスライスとして 1 回借りて命令中は添字だけにする（境界チェックが 1 回に）。
効かなければ `objdump` で何が出ているかを見てから次を決める。

## 3c. 呼び出し

**切り分けから**: `call_args`（引数 0〜3）と `vmo_calls`、`call_block_yield`、`call_kwargs`。1 回の呼び出しの ns と、そのうち `CallInfo` の push/pop、
`find_method`（キャッシュあり）、`OP_ENTER` の引数の並べ替え、`RETURN` の後始末の内訳。

**候補**: `CallInfo` の 13 フィールドのうち呼び出しごとに要らないものを外す（大きさを減らす）、`OP_ENTER` の単純な形（必須引数だけ）の早道、
`RETURN` で `stack.truncate` を呼ばない（窓を残して次の呼び出しで上書き）。

## 3d. `Slot` 8 バイト化の実験

`Slot` は `Value` の透明な包みで、この実験のために用意してある（`docs/design/performance.md`）。`Value` は 16 バイト（enum の判別子 + 8 バイトの payload）。
8 バイトにする案: NaN boxing（Float を主にし、他をタグ付きで NaN の中に入れる）か、ポインタタグ（下位ビットに種類。Float は箱に入れる）。
`unsafe` なしで書けるか（`f64::to_bits`/`from_bits` と整数演算だけで組める。ヒープの参照は `ObjId(u32)` なので生ポインタは不要）。
**先に見積もる**: レジスタ・配列・ハッシュのメモリ帯域が半分になる効果と、Float の読み書きにビット操作が増える代償。`vmo_arith`（整数）と
`bm_so_mandelbrot`（浮動小数）で逆に出るはず。3b の結果で「帯域が効いている」と分かってから着手。1 つの実験ブランチで全ベンチを取り、
効かなければ捨てる（記録は残す）。

## 3e. `bm_so_lists` の残り

`shift` を O(1) にした後の 999 ms の内訳（`push`、`reverse!`、`==`、`dup`）を切り分けてから。`==` は要素ごとに `funcall("==")` している可能性。

## 守ること

`host-bridge-plan.md` と同じ: `unsafe` を増やさない、no_std、`Vm: Send + Sync`、本家テストの基準、1 つずつ交互 A/B、効かないものは取り込まず数値だけ残す、
worklog に過程を書く。終わったら `docs/design/optimizations.md` に節を足し、`docs/verification/bench.md` に表を足す（本体）。
