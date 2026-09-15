# Rust との接続を整える計画（実装指示書）

作成 2026-09-15。著者がまとめた設計議論「SabiRuby と Rust の接続に関する設計議論まとめ」（rubevy 側で議論。要点は 0 節）を受けて、
何をどの順に変えるかを決めたもの。各段階は独立に着手でき、終わるごとにこの文書の「状況」を更新する。

関連: `docs/design/performance.md`（値の表現と性能の見積もり）、`docs/design/gems.md`（mruby-task、Time limits）、`docs/host.rs` のコメント、
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
* ベンチは変更前後で取り、`docs/verification/bench.md` に残す。速くならない段階（3〜5）は「ぶれの範囲」を確かめる。
* gem が Ruby で定義するメソッドをネイティブに置き換えない（`docs/design/gems.md` の規則）。

## 状況

| 段階 | 内容 | 状態 |
|---|---|---|
| 0 | `unsafe` を 0 に | **済み**（2026-09-15、`354b6bb`）。ただし 2.5% 遅くなった。段階 2 で取り戻す（下の知見） |
| 1 | ベンチの分類 | **済み**（2026-09-15、`9837294`）。基準の計測は `bench/results/e9da768.tsv`、表は `docs/verification/bench.md` |
| 2 | 実行ループの無駄取り | **済み**（2026-09-15、`5119773`〜`4a6826e`）。候補 5 つとも採用、合計で −13% |
| 2b | Hash と String | **一部済み**（2026-09-15、`05f6f28`）。`String#[]` と Hash の走査 |
| 2c | `Array#shift` の O(1) 化（開始オフセット）、Hash のハッシュ表化（入口の 1 本化 → 索引） | **済み**（2026-09-15、`35d58a5` `8ba2336` `e2b026b`）。3 つで −12.8%、`bm_so_lists` −73% |
| 3 | ネイティブのクロージャと型付きホスト状態 | **済み**（2026-09-15、`93824b1` `1472346`）。sabiruby 側のみ。rubevy 側の置き換えは未 |
| 4 | `FromRuby` / `IntoRuby` と `define_fn` | **済み**（2026-09-15、`ccc60de`） |
| 5 | Data オブジェクト（ハンドル方式）と解放フック | **済み**（2026-09-15、`c667a9d`）。rubevy 側も済み（2026-09-15、rubevy `68d80ed` `7b71c16`: static のキューを `host_state` へ、`Rubevy::Entity`） |
| 6a | `sabiruby-macros`（`#[derive(RubyClass)]`、`#[ruby_methods]`、`HostStore`） | **済み**（2026-09-15、`e7ea20e`〜`bd7fcb7`） |
| 6b | rubevy: Future 連携（`answer_with`） | **済み**（2026-09-15、rubevy `995cd5d`〜`5007337`） |
| 6c | rubevy: 動的プロキシ（`proxy.rb`） | **済み**（2026-09-15、著者判断で案 A: sabiruby `6314b84` で Ruby の `method_missing` を呼ぶ側のフレームで再ディスパッチ（本家と同じ形）、rubevy `684d3a8` で `Rubevy::Proxy`） |
| 2d | 性能の第 2 弾: Hash の固定費と `eql?`、`items()` の複製、`vm_optimization_bench` の分類分け | **済み**（2026-09-15、`07659aa` `2521b79` `f222934` `17ab5dc`）。通しで −15.8%（元の 20 本で −11.1%）、`ds_hash` −63% |
| 3b | VM の公開 API の穴埋め: rubevy が内部フィールドに触る 4 か所に入口を足し、rubevy を移す | **済み**（2026-09-15、sabiruby `4e5b590`、rubevy `1afb91c`）。rubevy の `src/` に `vm.heap` / `vm.task` / `vm.globals` への直接アクセスは 0 |

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
  **遅さの中心は Hash と String** で、Array が 4.6x なので Slot の出し入れではなく、Hash（挿入順の線形探索、`../design/performance.md`）と String に固有の重さがある。
  段階 2 は「Hash / String の調査 → メソッドキャッシュ → 命令ループ」の順に組み替える価値がある。
* `mem_retained`（2 万個の長命オブジェクトを抱えたまま割り当て続ける）は SabiRuby の方が速い（0.5x）。本家の世代別 GC が live set を繰り返し mark するため。
  10 万個では本家 31 秒、SabiRuby 1.2 秒。ベンチは本家側が長くなりすぎるので 2 万に下げてある。
* この機械（i7-13700）は P コアと E コアが混在し、`taskset` 無しだと E コアに落ちて 20% 遅くなる。`tools/bench.sh --core 2` を必ず使う。
* 2 つの担当が同時にベンチを回すと ±20〜35% ぶれる。best と median の差が 0.5% 程度に収まっているかで、静かだったかを判断できる。
* `docs/verification/bench.md` は `tools/bench.sh` が上書きしない（`BENCH_DOC` で明示したときだけ）。基準と段階ごとの結果は `bench/results/<sha>.tsv` に残す。

### 段階 2・2b（過程は `docs/worklog/2026-09-15-stage2-perf.md`）

* 基準 `440d4ba` → 段階 2・2b で **全体 −22%、22 本すべてが速くなった**（本家比 4.40x → 3.50x）。段階 2c を足して、計画の出発点 `e9da768` からは **−31%、本家比 4.32x → 3.01x**（`docs/verification/bench.md`）。段階ごと: `from_u8` を `match` に −1.7%、
  `find_method` の返り値を Copy に −2.1%、毎命令の `CallInfo` clone をやめて −2.8%、`op_counts` を既定でオフ + 固定長配列で −5.6%、
  メソッドキャッシュ −1.3%、`String#[]` と Hash の走査で −8.7%。
* **`op_counts` の常時カウントが最大の無駄だった**（計画書の見込み 1% に対し、密なループで 20〜25%）。`Vec` のポインタ再読み込みと、
  同じアドレスへの store→load 依存が毎命令に乗っていた。「`Vec` のままフラグで止める」は「固定長配列で数え続ける」より遅い。
  Playground は起動時に `Vm::set_op_counting(true)` を呼ぶ（`docs/design/playground.md`）。
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

### 段階 2c（過程は `docs/worklog/2026-09-15-stage2c-array-hash.md`）

* `ArrayData { buf, start }`（非公開フィールド、`Deref<[Slot]>`）で `shift`/`unshift`/`insert(0)` が O(1)。`bm_so_lists` 3699 → 999 ms（−73%）、全体 −6.1%。
  `pack`/`Struct` はスライスしか使っていないので無変更で通った。
* **借用を変えると隠れていた O(n) が表に出る。** `Heap::array` が `&Vec` を返していた間は、`items(vm, s).len()`（配列を丸ごと複製して長さだけ見る）の割り当てを
  LLVM が消していた。`&[Slot]` に変えたら消えなくなり `ds_array` が +155%。長さだけ要る 8 か所を `ary_len` に替えて解消。「最適化されていたから遅くなかった」コードは、
  周りを変えると突然遅くなる。
* Hash は先に**書き込みの入口を 1 本化**（`HashData` のフィールドを非公開に、54 行。前の担当の「37 か所」に 1 か所の数え漏れがあり、コンパイラが見つけた）、
  性能はぶれの範囲（−0.3%）。そのうえで 16 要素を超えたら索引（`hash -> 添字`、バケットごとに先頭挿入の 1 本のチェイン、削除で作り直し）。
  `ds_hash` −19.8%、`vm_optimization_bench` −14.4%（中に 5 万要素の Hash 操作がある）、全体 −7.1%。捨てた案: バケットごとの `Vec`、削除で索引を捨てるだけ
  （削除と探索が交互だと二度と作られない）、tombstone（読み手全員が穴を飛ばす必要がある）。キーが `eql?` で一意なので候補の順序が問題にならず、本家より単純にできた。
* 索引ありと索引なし（B1 のバイナリ）で境界（15/16/17/40 要素、閾値を下回る削除、`shift`、同一 `hash` で非 `eql?` の 25 キー、`rehash`）の出力が一致し、本家とも一致
  （`tests/custom/hash_index_boundary`）。
* 計測中に自分で `cargo build` を回して数値が荒れ、取り直した（best/median が 44% 差）。自分の道具も外乱になる、の再現。

### 段階 2d（過程は `docs/worklog/2026-09-15-stage2d-perf.md`）

* 「固定費 90 ns ＝ ネイティブ呼び出しと引数の受け渡し」は間違いだった。呼び出しの実費は 28 ns（`Array#[]` で実測）で、残りは Hash 自身の仕事。
* 計画の「鍵が Integer/Symbol/String なら Rust で答える早道」は**すでに入っていた**（`Vm::key_hash`/`key_eql`、本家の `mrb_hash_ht_hash_func` と同じ形）。着手不要。
* **隠れていた O(n) がもう 1 つ**: `hash_sync` が「ハッシュ値が古いか」を聞く前に全鍵を `Vec` に複製していた。Hash を 1 回引くたびに全要素の複製。
  順番を入れ替えただけで全体 −11.7%、`ds_hash` −63%（本家比 7.4x → 2.7x）、`vmo_objects`（5 万要素）−85%、`call_kwargs` −15%（キーワード引数は Hash で渡る）。
  段階 2c で索引を入れても `ds_hash` が −20% しか動かなかった理由。
* `hash_set` は `key_hash` を 2 回計算し、既存の鍵でも String 鍵を毎回複製・凍結していた。1 回に、挿入時だけ複製に（ベンチではぶれの範囲、micro では効く）。
* `items()` の複製は、段階 2c の 8 か所修正でベンチからは消えていた。残っていたのは `include?`/`member?`/`count` がループ内で `items()` を呼ぶ O(n²)。
  100 要素の `count` で −86%。ベンチ 22 本は呼ばないので合計は動かない。
* `vm_optimization_bench` を 5 本（`vmo_dispatch`/`arith`/`calls`/`index`/`objects`）に分けて分類し直した。元は「実アプリ寄り」へ。

### 段階 6a（過程は `docs/worklog/2026-09-15-stage6a-macros.md`）

* `HostStore<T>` は VM が `TypeId` で型ごとに持つ（案 B）。決め手は解放フック: `set_on_free` は 1 つしか持てず `&mut Vm` も受け取らないので、
  型ごとにフックを張る設計は 2 型目が 1 型目を潰す。VM が自分の store を回収時に解放する形にしたので、生成コードはフックを使わず、
  フックはホスト用に空いたまま。
* クラスメソッドの判定は「`self` を取らない `fn`」。`&mut Vm` を取るインスタンスメソッドのために、番号を予約したまま実体を貸し出す
  `take`/`restore` を用意し、同じオブジェクトへの再入は `RuntimeError`。枠は `Empty`/`Full`/`Out` の 3 状態。
* 引数の型が `FromRuby` 非対応のときのエラーが読めなかった（`{closure@…}: RubyFn<_>`）ので、生成コードが引数の型の span で `FromRuby` を要求し直す。
* 判断待ち: 生成された `register` に残る `expect` 1 つ（`VmResult` を返す形にするか）、`allocate` で作った素のオブジェクトへの呼び出しの文言、ブロックを取るメソッド。

### 段階 6b・6c（過程は rubevy の `docs/worklog/2026-09-15-stage6bc-futures-proxy.md`）

* `answer_with` の system は `tick_scripts` の**前**: 前フレームの終わりから今フレームの頭までに終わった future を今フレームで拾える。
* rubevy の Bevy に `multi_threaded` は付いていなかった。無いとタスクプールはメインスレッドで進めるだけ進めてから帰り、計算だけの future はフレームを止める。
  ライブラリは要求せず利用側が足す（README）。
* テストは `sleep` との競争を避け、テストが自分で開ける門（`bool` + `Waker`）に future を待たせる形。
* **6c の壁**: `method_missing` の中で `ask(...).pop` すると `blocking pop cannot be called from within a C function boundary`。
  VM が `method_missing` を `call_proc_with`（入れ子の実行ループ）で呼ぶため。`initialize` の中も同じ（`Class#new` がネイティブ）。
  `define_method` で作った本物のメソッドなら止まれる。案 A（VM 側で `send` の `op_send_redirect` と同じ再ディスパッチにする。本家も `mrb_exec_irep` で
  同じフレームに入る）、B（キューを返して呼ぶ側で `.pop`）、C（名前の一覧から `define_method`）。

### 段階 6c（過程は `docs/worklog/2026-09-15-stage6c-method-missing.md`、rubevy の `2026-09-15-stage6c-proxy.md`）

* Ruby で定義された `method_missing` は、`send` の再ディスパッチ（`op_send_redirect`）と同じ経路で呼ぶ側のフレームに入る。引数の書き戻しを
  `relay_args` に切り出して共有。可視性の検査は通さない（本家は private でも呼ぶ）。`funcall` 側は入れ子のまま（本家の `mrb_funcall` も入れ子）。
* 本家は `prepare_missing` で常に配列 1 本に詰める（`n` は必ず 15）が、`OP_ENTER` から見れば同じもの。引数 0/1/14/15/20・キーワード・ブロック・
  `super`・private・例外・バックトレース・`Fiber.yield` の 29 行が本家の出力と一致（`tests/custom/method_missing_dispatch`）。
  変更前でも 29 行中 22 行は合っていて、変えたのは「本体の中で止まれるか」だけ。
* 空のキーワード Hash（`foo(**{})`）は `native_args` が落とすので本体に届かない。本家は届ける。変更前からで、`send` も同じ。直すなら `native_args` の側。
* 通常の呼び出しへの影響: 交互 A/B で `call_args` +1.1%、`bm_fib` +0.7%（15 ラウンド）。同じ変更で `bm_fib` が −0.1% ↔ +0.7% を動くので ±1% は地の揺れ。
* **cargo の増分ビルドが古い成果物を残す**ことが今日 2 回あった（テストが「メソッドが無い」と言う、マージ後だけ 1 件落ちる）。
  `cargo clean -p sabiruby` で解消。テストが変更内容と矛盾するときは、まずこれを疑う。

### 段階 3b（過程は `docs/worklog/2026-09-15-stage3b-host-entry-points.md`、rubevy 側は rubevy の worklog）

* 6 本の入口（`task_running`、`ivar_get`/`ivar_set`、`global_get`/`global_set`、`is_exception`）は既存の内部関数を包むだけ。
  読む側の 2 本は `&self` のまま: `intern` は `&mut self` が要るが、intern されていない名前はどの ivar/global の名前でもありえない（どちらも nil）ので、
  intern せずに引くだけの `Interner::lookup_str`（3 行）を足した。読むだけの入口が `&mut Vm` を要ると `&Vm` しか持たない場所から呼べない。
* rubevy の `Slot` と `ObjKind` の import が消え、VM の内部表現を知っているのは VM だけになった。
* `pgrep -f` で自分の待ちシェルを見つけ続ける失敗（`[b]ench` の形が要る）。

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
  （`docs/verification/bench.md`）: fib +4.2%、`call_args` +4.0%、`app_tak` +3.8%、全体 +1.8%。データ構造は変わらず。`loop_while_add` の +6.6% は呼び出しが無いので
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

**到達点**: 「どの処理が何倍遅いか」が分かる表。`docs/verification/bench.md` に載る。

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
   （`docs/design/performance.md`）と合わせて後回し。

**やらないこと**: `unsafe` による境界チェックの省略、goto threading の模倣。

**確認**: 候補ごとに `bench/results/` に前後の TSV を残し、本体が `docs/verification/bench.md` に表を足す。本家テストの基準を下回らない。

**大きさ**: 0〜3 は小、4 は中。

## 段階 2b: Hash と String

**到達点**: `ds_hash`（12.9x）と `ds_string`（11.4x）、`bm_so_lists`（16.4x）の倍率が下がる。数値目標は調べてから決める。

**調べ方**: まず 1 本ずつ、何に時間が行っているかを切り分ける（`perf` が使えれば `perf record` の release ビルド、無ければ
ベンチを操作ごとに分けた小さなスクリプトで）。仮説は `docs/design/performance.md` の「Known structural costs」にある:
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

## 段階 2d: 性能の第 2 弾（Hash の固定費、`items()` の複製、ベンチの分類）

**到達点**: `ds_hash`（7.7x）と `app_json_hash`（3.0x）が下がる。`items()` が中身を読むためだけに複製している箇所が減る。
`vm_optimization_bench` の中身が分類に分かれる。数値目標は切り分けてから決める。

**手がかり**（`docs/design/optimizations.md` 5 節、worklog の切り分け）:
* Hash の 16 要素以下は線形探索で、1 回 90 ns の固定費（ネイティブ呼び出し、引数の受け渡し、`hash` の計算）が支配。
  `Integer`/`Symbol`/`String` の鍵は `hash` と `eql?` を Ruby に戻さず Rust で答えられる（本家も `mrb_hash_ht_hash_func` で同じ早道を持つ）。
  `hash_index` の呼び出し経路（`hash_get`/`hash_set` → `hash_index` → `key_eql`）の中で何が 90 ns を占めるかを先に測る。
* `items(vm, s)` は配列を `Vec<Value>` に複製する。中身を**読むだけ**の呼び出し（`each`、`inject`、`join`、`inspect`、`==`、`pack`、…）は
  `Heap::array` の `&[Slot]` を借りて済むが、ブロックを呼ぶものは借用を持ち越せない（`eql?` と同じ問題）。添字で回して 1 要素ずつ取る形にする。
  どの呼び出しが多いかは `ds_array` と `bm_so_lists` の切り分けで。
* `vm_optimization_bench` は本家のベンチのまま分類「命令ループ」に置かれているが、中に 5 万要素の Hash 操作がある。
  中身を見て、いくつかの小さなベンチに分けて分類し直す（元のファイルは残す）。

**やり方**: 段階 2 と同じ。1 つずつ別コミット、交互 A/B、効かないものは取り込まず数値だけ残す。切り分けを先に。

**やらないこと**: `Slot` の 8 バイト化（別の段階）。`unsafe`。

## 段階 3b: VM の公開 API の穴埋め

**到達点**: rubevy が `Vm` の内部フィールドに直接触らない。段階 6（マクロ）の前提。

**今**（rubevy `src/lib.rs`）: `vm.heap.ivar_set(task, k, …)`（タスクにエンティティを持たせる）、`vm.heap.ivar_get(task, k)`（`Rubevy.entity`）、
`vm.task.running`（いま走っているタスク）、`vm.globals.insert(…)`（`$rubevy`）、`vm.heap.get(o).kind` が `Exception` か（終わったタスクの結果が例外か）。

**変更**（VM 側、`src/vm.rs` に小さな公開関数。名前は既存の `task_*` / `str_new` の流儀に合わせる）:
* `task_running(&self) -> Option<ObjId>`。
* `ivar_get(&self, obj, name: &str) -> Value` / `ivar_set(&mut self, obj, name, value)`（`intern` 込み。既存の内部関数を包む）。
* `global_get(&self, name: &str) -> Value` / `global_set(&mut self, name, value)`。
* `is_exception(&self, v: Value) -> bool`。
* それぞれ rustdoc に「ホストが使う入口」と書く。`tests/` に短い確認（`tests/host_api.rs`）。
* rubevy 側: 4 か所を置き換え、`sabiruby::value::Slot` と `sabiruby::object::ObjKind` の import が消えること。worklog に「残った内部アクセス」が 0 になったことを書く。

**確認**: sabiruby の `cargo test --workspace`、no_std、本家テストの基準。rubevy の `cargo test`、examples。ベンチは不要（ホットパスに触らない）。

## 段階 6: マクロ、Future 連携、動的プロキシ（2026-09-15 に具体化。6a と 6b は並行、6c は 6b の担当）

### 6a: `sabiruby-macros`（属性マクロ）

**到達点**: Rust の構造体とその `impl` を、手書きの `define_fn`/`data_new` なしで Ruby のクラスにできる。

```rust
#[derive(RubyClass)]            // Data のタグと、Ruby 側のクラス名（既定は型名）
struct Player { hp: i64 }

#[ruby_methods]                  // impl 内の pub fn を、そのままメソッドとして登録
impl Player {
    fn new(hp: i64) -> Self { Player { hp } }          // `Player.new(100)`
    fn damage(&mut self, n: i64) { self.hp -= n; }     // `player.damage(10)`
    fn hp(&self) -> i64 { self.hp }                    // `player.hp`
}
// 登録: sabiruby_macros で生成された `Player::register(&mut vm, store)`
```

**設計の要点**（段階 4・5 の上に載せるだけ。コアに新しい機構を足さない）:
* 新しい crate `macros/`（`sabiruby-macros`、`proc-macro = true`、`syn`/`quote`）。ワークスペースの `members` に足す。`sabiruby` からは依存しない
  （利用側が `sabiruby` と `sabiruby-macros` の両方を書く。将来 `sabiruby` の feature `macros` で re-export してもよい）。
* **実体は Rust 側が所有する**（Host Object 方式）。生成コードは、`Vm::host_state` の中に置いた `HostStore<T>`（slab: `Vec<Option<T>>` と空き番号）に
  実体を入れ、Ruby には `data_new(class, TAG, handle)` を渡す。`&self`/`&mut self` を取るメソッドは、`This<DataRef>` で受けたハンドルから
  `host_state_mut::<HostStore<T>>()` を引いて実体を借りる。解放フック（`set_on_free`）で slab から外す。
* `self` を取らない `fn new(...) -> Self` はクラスメソッド `new` に。`&self` は読み、`&mut self` は書き。引数と戻り値は段階 4 の `FromRuby`/`IntoRuby`
  （対応外の型はコンパイルエラーにし、エラーメッセージに型名を出す）。`&mut Vm` を先頭に取る形も許す。
* `HostStore<T>` と `TAG` の割り当て（型ごとに一意な `u32`。`TypeId` は `no_std` で `const` にできないので、登録時に VM から採番する `Vm::next_data_tag()` を足す）
  は `sabiruby` 側の小さな追加（`src/convert.rs` か新しい `src/host_store.rs`）。ここだけがコアの変更。
* マクロの出力が読めること: `cargo expand` 相当を `macros/tests/expand.rs` に固定（生成コードの意図をテストで示す）。

**確認**: `macros/tests/` に上の `Player` を含む結合テスト（`new`、読み書き、`Method#arity`、解放フックで slab から消える、`dup` は `TypeError`、
2 つの型を同時に登録して tag が衝突しない）。`cargo test --workspace`、`cargo doc --no-deps` 警告なし、`tools/check_no_std.sh`（`sabiruby` 本体は
`no_std` のまま。`macros` は proc-macro なので対象外）、本家テストの基準、`cargo publish --dry-run --workspace`（`macros` の `Cargo.toml` に
`version`/`license`/`description`/`repository` を書く。公開はしない）。

**大きさ**: 中。

### 6b: Future 連携（rubevy）

**到達点**: ゲームが「答えを非同期に作る」仕事を Bevy のタスクプールに投げ、完了したら待っているスクリプトが起きる。

* `ScriptWorld::answer_with(request, future)`: `Future<Output = Answer> + Send + 'static` を `AsyncComputeTaskPool` に投げ、
  完了を毎フレームの system（`drain_commands` の隣）が拾って `answer` する。`Request` は今のまま（queue を持つ）。
  スクリプト側は何も変わらない（`Rubevy.ask(...).pop` で待つだけ）。
* `examples/` に 1 本（`headless` の隣。数フレームかかる計算を future で答える）。`tests/` に 1 本（answer が数フレーム後に届き、
  その間スクリプトが止まっていて他のスクリプトは動く）。
* `docs/host-api.md` に節を足す。

**大きさ**: 小。

### 6c: 動的プロキシ（rubevy の Ruby 側）

**到達点**: ホストのオブジェクトに、Ruby 側で `method_missing` を使った代理を薄く作れる。

* rubevy の `assets/scripts/` に `proxy.rb`（`require` できる Ruby のライブラリ）: `Rubevy::Proxy.new(kind)` が、未定義メソッド `name(*args)` を
  `Rubevy.ask("#{kind}.#{name}", *args).pop` に変え、`respond_to_missing?` も答える。VM の変更は要らない。
* 使い道の例を `examples/` の 1 本に足す（6b の例と同じでよい: `robot = Rubevy::Proxy.new("robot"); robot.move_to(1, 2)`）。
* 議論の結論どおり、**明示登録が基本で、外部オブジェクトにだけ使う**、と `docs/host-api.md` に書く。

**大きさ**: 小。

## 記録

* 各段階が終わったら、この文書の「状況」と、性能に触れた段階は `docs/verification/bench.md` を更新する。
* 設計の理由で本文（書籍）に関わるものは、書籍側の `docs/notes/sabiruby-findings.md` に書く（Rust の名前は本文には出さない規則）。
* rubevy 側の変更は rubevy の `docs/host-api.md` と `docs/rust-bridge.ja.md` の該当節を直す。
