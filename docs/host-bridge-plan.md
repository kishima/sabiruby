# Rust との接続を整える計画（実装指示書）

作成 2026-09-15。著者がまとめた設計議論「SabiRuby と Rust の接続に関する設計議論まとめ」（rubevy 側で議論。要点は 0 節）を受けて、
何をどの順に変えるかを決めたもの。各段階は独立に着手でき、終わるごとにこの文書の「状況」を更新する。

関連: `docs/performance.md`（値の表現と性能の見積もり）、`docs/gems.md`（mruby-task、Time limits）、`docs/host.rs` のコメント、
rubevy `docs/rust-bridge.ja.md`（今の接続の全容と C の mruby との比較）、`docs/outlook.ja.md`（何ができそうか）。

## 0. 方針（議論の要点と、コードと照らして直したところ）

議論の結論のうち採るもの:

* SabiRuby は「Rust で書いた mruby」ではなく「Cargo だけで組み込める Ruby 実行系」として整える。
* `unsafe` は増やさない（今 1 か所。段階 0 で 0 にする）。性能は安全な Rust の範囲で、本家比 2 倍未満を目標にする。約束ではなく目標。
* 順序は「手動の登録 API と型変換層を先に、マクロは後」。
* Rust の値は Rust 側が所有し、Ruby には**ハンドル**だけ持たせる（Host Object 方式）。Bevy の `World` は Ruby に見せず、
  今の `Rubevy.ask`（質問して system が答える）のままにする。

コードと照らして直したところ:

* 議論は遅さの原因に `Rc`／`RefCell`／`clone()` を挙げるが、VM の crate に `Rc` も `RefCell` も無い。`Value` は 16 バイトの Copy な enum、
  ヒープは `Vec<HeapObject>` を `ObjId(u32)` で引く。本当の候補は実行ループの中にある（段階 2）。
* 議論はネイティブを mruby/edge の `Rc<RObject>` 方式と同列に置くが、SabiRuby のネイティブは
  `fn(&mut Vm, Value, &[Value], Value) -> VmResult<Value>` で、値の受け渡しに Rc は無い。**本当の穴は関数ポインタであること**
  （環境を持てない）で、型付き登録もホスト状態への到達もここで詰まる。rubevy の `static Mutex<Vec<HostCommand>>` はその回避策。
* 「本家比 3 倍」は fib の数字。実際は 1.9 倍（mandelbrot）から 15.8 倍（`so_lists`）まで幅があり、平均ではなく極端な 1 つを追う。

守ること（全段階共通）:

* VM は `no_std + alloc`（`tools/check_no_std.sh`）。`Arc` は `alloc::sync`、`Any` は `core::any` から。
* `Vm: Send + Sync` を保つ（`tests/send_sync.rs`）。ネイティブのクロージャもホスト状態も `Send + Sync` を要求する。
* 本家テストの基準（`tests/mrbtest/baseline*.txt`）を下回らない。
* ベンチは変更前後で取り、`docs/bench.md` に残す。速くならない段階（3〜5）は「ぶれの範囲」を確かめる。
* gem が Ruby で定義するメソッドをネイティブに置き換えない（`docs/gems.md` の規則）。

## 状況

| 段階 | 内容 | 状態 |
|---|---|---|
| 0 | `unsafe` を 0 に | **済み**（2026-09-15、`354b6bb`）。ただし 2.5% 遅くなった。段階 2 で取り戻す（下の知見） |
| 1 | ベンチの分類 | **済み**（2026-09-15、`9837294`）。基準の計測は `bench/results/e9da768.tsv`、表は `docs/bench.md` |
| 2 | 実行ループの無駄取り | **済み**（2026-09-15、`5119773`〜`4a6826e`）。候補 5 つとも採用、合計で −13% |
| 2b | Hash と String | **一部済み**（2026-09-15、`05f6f28`）。`String#[]` と Hash の走査。`Array#shift` の O(1) 化と Hash のハッシュ表化は著者判断待ち（下の知見） |
| 3 | ネイティブのクロージャと型付きホスト状態 | **済み**（2026-09-15、`93824b1` `1472346`）。sabiruby 側のみ。rubevy 側の置き換えは未 |
| 4 | `FromRuby` / `IntoRuby` と `define_fn` | **済み**（2026-09-15、`ccc60de`） |
| 5 | Data オブジェクト（ハンドル方式）と解放フック | **済み**（2026-09-15、`c667a9d`）。rubevy 側（エンティティの Data 化、static の置き換え）は未 |
| 6 | マクロ、Future 連携、動的プロキシ | 方針だけ |

進め方（2026-09-15 から）: 計画は本体（Fable）が書き、実装は `implementer`（Opus のサブエージェント、
`/home/kishima/book/.claude/agents/implementer.md`）が worktree のブランチで行い、本体がレビューしてマージする。
段階 0・1 と 3 は 2 人が並行して行ったが、**両方がベンチを回して互いの数値を汚した**ので、ベンチを伴う作業は並行させない。

## 実装で分かったこと（段階ごと）

### 段階 0

* **`transmute` の除去は無料ではなかった。** 全体で 2.5%、ディスパッチ律速のベンチ（`loop_times` 7.7 ns/命令）で 6〜7%、
  命令の重いベンチ（`so_lists` 69 ns/命令）で 0%。`transmute` 版は範囲チェックだけで**読み出しが無い**（`u8` と `Op` は同じビット列）のに対し、
  表引きは毎命令 `OP_TABLE` から 1 バイト読む。計画書の「コストは今と同じ」は、変更前を 1 回多く数えていた。
* 受け入れて（著者判断 2026-09-15）、段階 2 で取り戻す。候補は、まず 119 本の `match`（判別子と値が同じなので無変換に落ちる公算が高い。未計測）、
  駄目なら実行ループが生のバイトで分岐する形（`from_u8` を呼ぶ熱い場所は `exec_frames` の 1 か所だけ。`rite.rs` と `inspect.rs` の利用は熱くない）。
* `tools/gen_opcode.py` は履歴にも本体が無い 253 バイトのスタブだった。`ops.h` から復元し、コミット済みの `opcode.rs` を
  `from_u8` と `OP_TABLE` 以外 1 バイトも変えずに再生成できることを確かめた。名前の規則は「`_` で分けて各語を capitalize、`LOADI__1` の空要素は `M`」。

### 段階 1

* 基準の計測（`e9da768`、本家 4.1.0-rc との倍率、P コア固定、best of 5）: データ構造 **10.9x**（`so_lists` 16.4x、`ds_hash` 12.9x、`ds_string` 11.4x、
  `ds_array` 4.6x）、命令ループ 5.7x（中央値 4.1x。`vm_optimization_bench` だけ 7.1x）、呼び出し 3.4x、実アプリ寄り 3.4x、メモリ 1.1x。
  **遅さの中心は Hash と String** で、Array が 4.6x なので Slot の出し入れではなく、Hash（挿入順の線形探索、`performance.md`）と String に固有の重さがある。
  段階 2 は「Hash / String の調査 → メソッドキャッシュ → 命令ループ」の順に組み替える価値がある。
* `mem_retained`（2 万個の長命オブジェクトを抱えたまま割り当て続ける）は SabiRuby の方が速い（0.5x）。本家の世代別 GC が live set を繰り返し mark するため。
  10 万個では本家 31 秒、SabiRuby 1.2 秒。ベンチは本家側が長くなりすぎるので 2 万に下げてある。
* この機械（i7-13700）は P コアと E コアが混在し、`taskset` 無しだと E コアに落ちて 20% 遅くなる。`tools/bench.sh --core 2` を必ず使う。
* 2 つの担当が同時にベンチを回すと ±20〜35% ぶれる。best と median の差が 0.5% 程度に収まっているかで、静かだったかを判断できる。
* `docs/bench.md` は `tools/bench.sh` が上書きしない（`BENCH_DOC` で明示したときだけ）。基準と段階ごとの結果は `bench/results/<sha>.tsv` に残す。

### 段階 2・2b（過程は `docs/worklog/2026-09-15-stage2-perf.md`）

* 基準 `440d4ba` → 最終で **全体 −22%、22 本すべてが速くなった**（本家比 4.40x → 3.50x）。段階ごと: `from_u8` を `match` に −1.7%、
  `find_method` の返り値を Copy に −2.1%、毎命令の `CallInfo` clone をやめて −2.8%、`op_counts` を既定でオフ + 固定長配列で −5.6%、
  メソッドキャッシュ −1.3%、`String#[]` と Hash の走査で −8.7%。
* **`op_counts` の常時カウントが最大の無駄だった**（計画書の見込み 1% に対し、密なループで 20〜25%）。`Vec` のポインタ再読み込みと、
  同じアドレスへの store→load 依存が毎命令に乗っていた。「`Vec` のままフラグで止める」は「固定長配列で数え続ける」より遅い。
  Playground は起動時に `Vm::set_op_counting(true)` を呼ぶ（`docs/playground.md`）。
* 候補 0 は `objdump` で確かめた: 119 本の `match` は範囲チェックだけに畳まれ、表引きのロードが消えた。
* 切り分けの結果: `ds_string` の 7 割は `String#[]`（全体複製 + UTF-8 の 1 バイト走査）で、直して 7.9 倍。`ds_hash` は線形探索が支配
  （固定費 90 ns + 1 要素 1.2 ns）で、走査の借用を 1 回にして −25〜32%。`bm_so_lists` の 78% は `Array#shift` 単独（`Vec::remove(0)`）。
* **著者判断待ち 2 件**（実装せず止めた）:
  (A) `Array#shift` を O(1) に。`ObjKind::Array` に開始オフセットを持たせる（案 1）か `VecDeque`（案 2）か、触らない（案 3。本家のベンチ以外に出ない）。
  (B) Hash のハッシュ表化。索引を足すのは簡単だが無効化の置き場所が問題で、`HashData` の `entries` を非公開にして書き込みを 1 本化する（案 2、37 か所）のが筋。
* **計測の教訓**: この機械（WSL2）は Linux VM の中が idle でもホスト側の都合で 10〜40% 動き、`taskset` では守れない。「A を 5 回 → B を 5 回」は同じ遅い窓に入って
  best of 5 でも当てにならず（同じ変更が +28% と −12% に出た）、**A と B を 1 回ずつ交互に回す** `tools/bench_ab.sh` を主たる計測にした。
  段階ごとの A/B の積（−20.4%）と通しの計測（−22.0%）が一致したので信用できる。荒れている日は min of 15。呼び出しを含まないベンチが候補ごとに ±4〜12% 動くのは、
  9000 行の `exec_frames` のループ本体がキャッシュラインのどこに乗るかの揺れ（コード配置）。

### 段階 4・5（過程は `docs/worklog/2026-09-15-stage4-5-bridge.md`）

* `define_fn` は `macro_rules!` 1 本で 42 impl。先頭の `&mut Vm` と `This<T>`、末尾の `Block` を「文脈」として読み、間を引数にする。
  `Method#arity` は引数の個数を答える（`ClosureBody` に arity を持たせた。段階 3 の `define_closure` は `-1` のまま）。
* `String` の `FromRuby` は `to_s` を呼ばず厳格（`nil` は `TypeError`）: 変換の途中で任意の Ruby が走って VM に再入するのを避ける。
  `Vec<u8>` は `Vec<T>`（Array）とコヒーレンスで重なるので `Bytes`。戻り値は `Result<T, VmError>` / `Result<T, String>` / `Result<T, &str>` の 3 つの具体型
  （境界で書くと重なる）。
* 解放フックは sweep の途中ではなく `gc_collect` の末尾で、`Heap::freed_data` に溜めた `(tag, handle)` を渡す（3 案を比べた。sweep はフリーリストを作り直している最中で、
  ホストのクロージャを走らせる場所ではない）。`&mut Vm` は渡さない。VM の `Drop` では呼ばない（回収されたものだけが届く）。
* Data の `dup`/`clone` は `TypeError`: ハンドルを写すとフックが 1 つの値に 2 回走る。複製の仕方を知っているのはホストだけ。

### 段階 3

* `Method` に `Arc` を持つ variant を足すと、`#[derive(Clone)]` が 16 バイトの memcpy から「判別子の分岐 + 片方の腕で atomic 増加」になる。
  `find_method` が呼び出しごとに `Method` を clone する今の作りでは、fib のような呼び出しだけのベンチがそれを拾う。静かな機械で測り直した結果
  （`docs/bench.md`）: fib +4.2%、`call_args` +4.0%、`app_tak` +3.8%、全体 +1.8%。データ構造は変わらず。`loop_while_add` の +6.6% は呼び出しが無いので
  この説明では足りず、`loop_times` が同時に −5.8% 動いていることから、コードの配置（アラインメント）の影響と見ている。段階 2 の候補 3（`find_method` の値返しをやめる）が本来の対処。代案は `Closure(u32)` で VM 側の表を引く形
  （`Method` が Copy に戻る。再定義したクロージャが VM の生存中残る）。
* `Method::Native` の分岐 16 か所のうち 7 か所を変更。変更不要としたもの: `respond_to?` の `notimpl_fns` 判定（関数アドレス比較。クロージャは該当しない）、
  `Class#new` の `default_allocate` 早道、`ext_struct.rs` の `initialize_is`、`native_arity` の表（クロージャは `-1`）。
* クロージャが捕まえた `Value` は GC の根にならない。ホストは `gc_register` を使う（rustdoc に明記）。段階 5 の `Data` が本来の答え。
* `Method#arity` はクロージャだと常に `-1`。段階 4 の `define_fn` は引数の個数を知っているので、そこで埋められる。
* `Hash#[]` の「再定義された `default` を尊重する」判定にも `Closure` を足した（ホストが `default` をクロージャで定義した場合のため）。


## 段階 0: `unsafe` を 0 に

**到達点**: `grep -rn unsafe src` が空。

**今**: `src/opcode.rs` の `Op::from_u8` が、範囲を確かめたうえで `u8` を `#[repr(u8)]` の `Op` に `transmute` している。毎命令呼ばれる。

**変更**:

* `tools/gen_opcode.py` に `const OP_TABLE: [Op; OP_COUNT] = [Op::Nop, Op::Move, …]` の生成を足し、
  `from_u8` を `OP_TABLE.get(b as usize).copied()` にする。`opcode.rs` は生成物なので手で直さない
  （生成器の本体はリポジトリの履歴にある、と冒頭のコメントにある。無ければ今の `opcode.rs` から表を作る小さな生成を足す）。
* ~~コストは今と同じ「範囲チェック 1 回 + 読み出し 1 回」。~~ 見込み違いだった: `transmute` 版に読み出しは無く、表引きで 2.5% 遅くなった（「実装で分かったこと」の段階 0）。

**確認**: `cargo test --workspace`、`tools/check_no_std.sh`、fib と `vm_optimization_bench` が変更前とぶれの範囲。

**大きさ**: 小。

## 段階 1: ベンチの分類

**到達点**: 「どの処理が何倍遅いか」が分かる表。`docs/bench.md` に載る。

**今**: `tools/bench.sh` は本家の `benchmark/*.rb` 5 本を回すだけ。`sabiruby run --stats` で命令数と ns/命令は取れる。

**変更**: `bench/src/` に分類ごとの小さなスクリプトを足す（各 1〜3 秒で終わる長さに調整）。

| 分類 | スクリプト | 見たいもの |
|---|---|---|
| 命令ループ | 整数加算の `while`、`if` の分岐、`times` | ディスパッチと算術の素の速さ |
| 呼び出し | 引数 0〜3 のメソッド呼び出し、ブロック付き、`yield`、`Fiber#resume`、キーワード引数 | `find_method`、`CallInfo`、環境の作成 |
| データ構造 | `Array#push`/`[]`/`each`、`Hash#[]=`/`[]`、`String#<<`/`+`/`[]` | 各 `ObjKind` の実装と Slot の出し入れ |
| メモリ | `Object.new` の大量生成（短命）、長命オブジェクトを持ったまま生成、GC の回数と時間 | 割り当てと `gc_collect` |
| 実アプリ寄り | fib、tak、SabiRuby Battle のロボット 1 フレーム相当（`radar`→`lead`→`act` を 1000 回）、JSON 風のハッシュ処理 | 全体の倍率 |

`tools/bench.sh` は今の形を保ち、分類ごとの小計を出す。本家との比較は同じ Docker イメージ（`kishima/mruby:4.1.0-rc`）で、
best of N と中央値の両方を出す（今は best of 3）。コンパイル時間は今も分離されている（`.mrb` を回す）。

**確認**: 表ができ、`so_lists` の 15.8 倍がどの分類に属するか言えること。

**大きさ**: 小〜中。VM には触らない。

## 段階 2: 実行ループの無駄取り

**到達点**: 段階 1 の表で、分類「命令ループ」と「呼び出し」の倍率が下がる。数値目標は置かない（測ってから決める）。

**順番の見直し（2026-09-15、段階 0・1・3 の結果を受けて）**: まず段階 0 と 3 で失った 4.3% を取り戻し（候補 0 と 3）、次に基準の計測が示した本丸である
Hash と String を調べる（段階 2b）。命令ループとメソッドキャッシュはその後。

**候補**（`src/vm.rs` `exec_frames` を読んで見つけたもの。1 つずつ変えて測る）:

0. **段階 0 の表引きを取り戻す。** `Op::from_u8` を 119 本の `match`（判別子と値が同じなので無変換に落ちる見込み）にして測る。
   駄目なら `exec_frames` だけ生のバイトで分岐する（`from_u8` を呼ぶ熱い場所はそこ 1 か所）。`unsafe` は使わない。
1. **毎命令 `CallInfo` を clone している**（`let ci = self.ci.last().unwrap().clone()`）。`base`・`irep`・`pc` だけ Copy で取り出す形にする。
2. **`op_counts[byte] += 1` が常時オン**。Playground の統計用。`Vm` にフラグを持たせて分岐するか、feature `stats` にする。
   分岐 1 回の方が安いか、feature で消す方が安いかは測る。
3. **`find_method` が `Method` を clone して返す**。`Method::Ruby(ObjId)` は Copy 相当だが、enum ごと clone している。
   参照を返す形、または `(kind, ObjId)` の Copy な組を返す形にする。
4. **メソッド探索にキャッシュが無い**。クラスチェーンを毎回 `HashMap` で引いている。本家のインラインキャッシュに相当するものは
   「選べる実装」（本の第 8 章）なので、形は自由。最初は `(class, mid) -> (Method, owner)` のグローバルな小さな表
   （メソッド定義・`include`・`prepend` で世代番号を上げて無効化）で十分。
5. `Slot::get` / `Slot::from` の往復、`self.stack[base + i]` の境界チェック。ここは `Slot` を 8 バイトにする実験
   （`docs/performance.md`）と合わせて後回し。

**やらないこと**: `unsafe` による境界チェックの省略、goto threading の模倣。

**確認**: 候補ごとに `bench/results/` に前後の TSV を残し、本体が `docs/bench.md` に表を足す。本家テストの基準を下回らない。

**大きさ**: 0〜3 は小、4 は中。

## 段階 2b: Hash と String

**到達点**: `ds_hash`（12.9x）と `ds_string`（11.4x）、`bm_so_lists`（16.4x）の倍率が下がる。数値目標は調べてから決める。

**調べ方**: まず 1 本ずつ、何に時間が行っているかを切り分ける（`perf` が使えれば `perf record` の release ビルド、無ければ
ベンチを操作ごとに分けた小さなスクリプトで）。仮説は `docs/performance.md` の「Known structural costs」にある:
Hash は挿入順の線形探索（本家の AR モードのみで、HT モードが無い）、ネイティブが配列を `items()` で複製している、
`Array#shift`/`unshift` が O(n)。String は `Vec<u8>` の複製と UTF-8 の文字数え直しが疑わしい。

**変更**: 切り分けの結果に従う。Hash は要素数が閾値を超えたらハッシュ表（`hashbrown`）に切り替える形が本家の AR/HT と同じ意味になる
（挿入順は保つ）。String は文字数のキャッシュか、ASCII だけの文字列の早道。どれも「選べる実装」の側で、本家テストが見ているのは振る舞いだけ。

**確認**: 段階 2 と同じ。加えて `tests/mrbtest` の hash / string / string-ext / regexp のファイルが基準どおり。

**大きさ**: 中。

## 段階 3: ネイティブのクロージャと型付きホスト状態

**到達点**: ホストが環境を持つ関数を Ruby のメソッドとして登録でき、ネイティブの中から自分の状態に型付きで届く。
rubevy の `static COMMANDS` が消える。

**今**: `object::NativeFn = fn(&mut Vm, Value, &[Value], Value) -> VmResult<Value>`。`Method::Native(NativeFn)`。
`Method` は `Clone` で、`find_method` が値で返す。`notimpl_fns` が `fn_addr_eq` で関数ポインタを比べている。

**変更**:

* `Method` に variant を**足す**（既存の `Native(fn)` は残す。組込みは今のまま関数ポインタで、コストを増やさない）:
  ```rust
  pub type NativeClosure = alloc::sync::Arc<dyn Fn(&mut Vm, Value, &[Value], Value) -> VmResult<Value> + Send + Sync>;
  Method::Closure(NativeClosure)
  ```
  `Arc` なのは `Method: Clone` のため。呼び出しコストは間接呼び出し 1 回で、関数ポインタと同じ。段階 2 の 3 が済んでいれば
  参照カウントの増減も消える。
* `Vm::define_closure(class, name, impl Fn(...) + Send + Sync + 'static)`。`define_method` はそのまま。
* `call_native` 相当の経路（`native_active` の増減、`native_ret_reg`、Time limits の `count_native`）を `Closure` にも通す。
  `Method::Native` を分岐しているところ（`vm.rs` の 6 か所程度、`respond_to?` の `notimpl_fns` 判定を含む）を洗う。
* **型付きホスト状態**: `Vm` に `host_state: Option<Box<dyn core::any::Any + Send + Sync>>` を 1 つ持たせ、
  `vm.set_host_state(T)`、`vm.host_state::<T>() -> Option<&T>`、`vm.host_state_mut::<T>() -> Option<&mut T>`。
  クロージャで環境を持てるので必須ではないが、「ネイティブから VM を経由してホストの状態に届く」通り道を 1 つに決めておく。
  既存の `Host` trait（compile / read_file）とは別物。`Host` は「VM がホストに頼むこと」、こちらは「ホストが VM に預けるもの」。

**rubevy 側**: `install_host_api` のネイティブを `define_closure` にし、コマンドのキューを `host_state` の中に移す。
`tests/replace.rs` が 3 本のテストを 1 本にまとめている理由（static の共有）が消えるので、分ける。

**確認**: `tests/native.rs` を新設。クロージャが環境（`Arc<Mutex<Vec<_>>>` など）に書けること、例外が `Err` で戻ること、
`respond_to?` と `method(:x).arity`、`Method#owner` が `Native` と同じに見えること、`Vm` が `Send + Sync` のままであること。
ベンチはぶれの範囲。

**大きさ**: 中。VM の変更は局所的だが、`Method::Native` の分岐を漏れなく洗う。

## 段階 4: `FromRuby` / `IntoRuby` と `define_fn`

**到達点**: Ruby の `Value` を意識せずに Rust の関数を登録できる。

```rust
vm.define_fn(class, "add", |a: i64, b: i64| a + b);
vm.define_fn(class, "greet", |vm: &mut Vm, name: String| format!("hi {name}"));   // vm が要る形も
```

**変更**（新しいモジュール `src/convert.rs`。VM のコアには触らない）:

* `pub trait FromRuby: Sized { fn from_ruby(vm: &mut Vm, v: Value) -> VmResult<Self>; }`
  impl: `Value`、`i64`、`i32`（範囲外は RangeError）、`f64`、`bool`、`String`（`as_string`）、`Vec<u8>`、`Option<T>`（nil → None）、
  `Vec<T>`（Array）、`Sym`。変換失敗は本家と同じ `TypeError` の文言（`expect_int` などが出す文言に揃える）。
* `pub trait IntoRuby { fn into_ruby(self, vm: &mut Vm) -> Value; }`
  impl: `Value`、整数・浮動小数・`bool`、`()`（nil）、`&str`／`String`、`Vec<T>`、`Option<T>`、`(A, B)`〜`(A, …, F)`（Array）、
  `Result<T, E: Into<String>>`（Err は `RuntimeError`。細かい例外クラスは呼び出し側で `vm.raise` を使う）。
* `define_fn` は引数の個数ごとに trait `RubyFn<Args>` を 0〜6 個分 impl する（タプルの impl をマクロで展開する、よくある形。
  手続きマクロは使わない）。引数の個数が違えば `ArgumentError`（本家の文言 `wrong number of arguments (given 1, expected 2)`）。
  受け手（self）が要る形と、`&mut Vm` が要る形の両方を用意する。
* ブロックは `Option<Value>` でそのまま受け、`vm.call_block` で呼ぶ。ブロックの型付きは後回し。

**確認**: `tests/convert.rs`。各 impl の往復、失敗時の例外クラスと文言、引数不足。段階 3 の `Closure` の上に載るのでベンチ不要。

**大きさ**: 中。書く量はあるが VM の意味は変えない。

## 段階 5: Data オブジェクト（ハンドル方式）と解放フック

**到達点**: Rust の値を Rust 側に置いたまま、Ruby のオブジェクトとして渡せる。回収されたらホストが知る。

**今**: `ObjKind::Data` は無い。rubevy はエンティティ番号を Integer で渡している（原始的なハンドル方式）。

**変更**:

* `ObjKind::Data { tag: u32, handle: u64 }`。`tag` はホストが決める種類（`TypeId` は `no_std` でも `core::any` にあるが
  64 ビットのままシリアライズしにくいので、ホストが登録した小さな番号にする）。
* `Vm::data_new(class, tag, handle) -> Value`、`Vm::data_of(v) -> Option<(tag, handle)>`、`FromRuby for DataRef { tag, handle }`。
* **解放フック**: `Vm` に `on_free: Option<Box<dyn Fn(u32, u64) + Send + Sync>>`。`sweep` で `Data` を解放するときに呼ぶ。
  GC の途中でホストのクロージャが走るので、フックの中で VM に触ることは禁止（型で `&mut Vm` を渡さないことで守る）。
  ホストはそこでハンドルを slab から外す。
* `inspect` は `#<Player:0x…>` 相当の形。`==` は同じ `(tag, handle)` なら真（`equal?` はオブジェクトの同一性のまま）。
* Marshal 相当は無いので直列化の考慮は不要。

**rubevy 側**: `Rubevy.entity` と `ask` の答えに入るエンティティ番号を `Data` に置き換える案を検討する
（`f64` で渡している今の形は 2^53 を超えると壊れる。`docs/rust-bridge.ja.md` 6 章）。ECS の橋の 1 段目。

**確認**: `tests/data.rs`。作る・比べる・回収でフックが呼ばれる（`GC.start` とストレスモード）、ハンドルを Rust 側の slab に戻す往復。
本家テストは無関係だが基準を確認。

**大きさ**: 中。GC の sweep に 1 か所足す。

## 段階 6: その先（方針だけ。着手は段階 5 の後に決める）

* **マクロ**: 別 crate `sabiruby-macros`。`#[ruby_methods] impl Player { … }` が段階 4 の `define_fn` と段階 5 の `Data` を呼ぶコードを
  生成するだけにし、コアに新しい機構を足さない。
* **Future 連携**: Rust の Future が完了したら `task_queue_push` する薄い層。`Rubevy.ask` がすでにこの形なので、
  汎用にするだけ（`Vm::task_queue_new` を返す `spawn_future` 相当）。時計は段階 3 の後の rubevy の仕事。
* **動的プロキシ**: `method_missing` は VM が対応済み。Ruby 側のライブラリ（prelude）で書けるので VM の変更は要らない。
  議論のとおり、明示登録が基本で、外部オブジェクトにだけ使う。

## 記録

* 各段階が終わったら、この文書の「状況」と、性能に触れた段階は `docs/bench.md` を更新する。
* 設計の理由で本文（書籍）に関わるものは、書籍側の `docs/notes/sabiruby-findings.md` に書く（Rust の名前は本文には出さない規則）。
* rubevy 側の変更は rubevy の `docs/host-api.md` と `docs/rust-bridge.ja.md` の該当節を直す。
