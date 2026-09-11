# GC 実装指示書（SabiRuby）

対象: この文書だけを読んで、別セッションの実装者（AI）が SabiRuby に到達可能性 GC を入れられること。
作業前に `README.md`（Rules、Verification）、`docs/performance.md`、`docs/fibers.md` を読むこと。
設計判断はここに書いたとおりにし、変えたい場合は理由を `docs/gc.md` に残す。

## 0. 前提と現状

* SabiRuby は no_std + alloc の Rust 製 mruby VM。ヒープは `src/object.rs` の `Heap { objs: Vec<HeapObject> }` で、
  オブジェクトは `ObjId(u32)`（`objs` の添字）で参照する。回収は無く、ヒープは伸びる一方。
* 値は `Value` enum（16 バイト）、格納場所は `Slot`（`Value` の透明な包み。`src/value.rs`）。**GC が走査すべき場所は `Slot` の並びそのもの**。
  演算は `Value` で行い、`slot.get()` / `Slot::from(v)` だけが出入り口。
* `GC` モジュールのインタフェース（`start`、`enable`、`disable`、`stat`、`interval_ratio`、`step_ratio`、`step_limit`、`malloc_threshold`、`generational_mode`）は
  `src/builtins/object.rs` に**形だけ**ある（`start` は nil を返すだけ、`stat[:live]` は `heap.len()`）。
* 本家 mruby 4.1.0-rc のテスト `test/t/gc.rb` は 24 件中 18 が通り、5 件が落ちている。落ちている 5 件はすべて
  「`GC.start` → `GC.stat[:live]` を記録 → 2 万回の操作 → `GC.start` → `live` の増分が 100 未満」という形（`OP_GETIDX ... in the GC arena` など）。
  つまり**本物の回収が入れば通る**。アリーナの数え方を本家に寄せる必要は無い。
* 本家との対応: `src/gc.c`（三色マーク＆スイープ、世代別、負債駆動、ヒープページ）は**実装都合**であり写さない。
  再現するのは「到達できないオブジェクトが回収される」「`GC.*` の意味」「拡張（ネイティブ）から見た約束」の 3 つ（本『Deep dive into mruby』第 4 章と、移植章 段階 8）。

## 1. 設計（決定事項）

### 1.1 方式

* **停止型（stop-the-world）の非移動マーク＆スイープ**。オブジェクトは動かさない（`ObjId` は添字なので動かせない）。
* 空きスロットは**フリーリスト**で再利用する。`Heap` に `free: Vec<u32>` を足し、`alloc` はまず `free.pop()` を使う。
* 印は `HeapObject` に `marked: bool` を足すか、`Heap` 側に `mark: Vec<bool>`（`objs` と同じ長さ）を持つ。どちらでもよいが、後者の方が `HeapObject` を触らずに済む。
* 解放したスロットは `kind = ObjKind::Object`、`ivars.clear()`、`class = ObjId(0)`、`frozen = false` にして中身を落とす（`ObjKind` に `Free` を足さない。
  足すと `match` を持つ全ファイルが変わる）。解放済みスロットへのアクセスはバグなので `debug_assert!` で検出できるよう `Heap` に `is_free(id)` を用意する。
* 増分（インクリメンタル）にはしない。したがって**ライトバリアは不要**。将来増分にするなら `docs/performance.md` の「格納場所の窓口」を使う（`Slot::set` に入れる）。
* 世代別にしない。`GC.generational_mode=` は値を覚えて返すだけでよい（現状のまま）。

### 1.2 いつ走るか

* **命令の境界だけ**。`src/vm.rs` の `exec_frames` のループ先頭（`step_left` の判定の直後）で `if self.gc_pending { self.gc_maybe(); }` とする。
  1 命令あたり `bool` 判定 1 回で済ませること（`gc_pending` は `Heap::alloc` が閾値到達時に立てる）。
* **ネイティブ実行中は走らせない**。`Vm` に `native_active: u32` を足し、`call_native_direct`（SEND からのネイティブ呼び出し）と
  `funcall` の `Method::Native` 分岐で `f(...)` の前後に増減する。`gc_maybe` は `native_active == 0` のときだけ回収する。
  理由: ネイティブは Rust のローカル変数に `Value`/`ObjId` を持つ（例: `let h = vm.hash_new(); ... vm.hash_set(h, k, v)?` の途中で `key_hash` が Ruby を呼び、
  そこで割り当てが起きる）。本家はアリーナで守るが、SabiRuby はネイティブにフレームを積まないので「ネイティブが生きている間は回収しない」が最も単純で安全。
  ネイティブから Ruby へ再入している間（`Array#sort { }` のブロック中など）は回収が延びるが、mrblib の `each`/`map`/`times`/`loop` は Ruby なので普段の反復は影響しない。
  この制約を `docs/gc.md` に「拡張から見た約束」として書くこと。
* `GC.start` は `native_active` に関係なく**即時**に回収してよい？ → **いいえ**。`GC.start` 自体がネイティブなので、`GC.start` の中で回収すると呼び出し元ネイティブ（無い）…
  正確には `GC.start` を呼んだ SEND 命令のレジスタ状態は `stack` にあるので、`GC.start` の中で回収しても VM 側の根は揃っている。
  **ただし `GC.start` が `funcall` 経由（ネイティブ → ネイティブ）で呼ばれた場合は危険**なので、`GC.start` は `native_active == 1`（自分だけ）のときに限り即時回収し、
  それ以外は `gc_pending = true` にして次の命令境界に回す。テスト（`GC.start; base = GC.stat[:live]`）はバイトコードからの直接呼び出しなので即時になる。
* `GC.disable` 中は回収しない（`gc_pending` は立ててよいが `gc_maybe` が何もしない）。`GC.enable` で次の境界に回収される。
* 閾値: 直前の回収後の生存数 `live_after_gc` に対して、`allocated_since_gc >= max(live_after_gc, 4096)` で `gc_pending`。
  本家の `interval_ratio`（既定 200% ）に相当する。`GC.interval_ratio=` は値を覚え、閾値の係数にする（`ratio/100`）。
* `GC.malloc_threshold`: 割り当てバイト数の概算 `malloc_increase` を持ち、非 0 の閾値を超えたら `gc_pending`。概算は
  `HeapObject` 1 個 = 64 バイト、String は `+ len`、Array は `+ 16 * len`、Hash は `+ 40 * entries` 程度でよい（文字列の `set`、配列の `with_mut` でも増分を足す必要は**無い**。
  割り当て時だけでよい。テストは `GC.start` 後に `malloc_increase` が減ることを見る）。
  `GC.stat[:malloc_increase]` はこの値、回収で 0 に戻す。

### 1.3 根（ルート）

`Vm` の次をすべて辿る。漏れは「しばらく後で別の場所が壊れる」形で出るので、§4 のストレスモードで必ず確かめる。

1. `self.stack`（実行中コンテキストのレジスタ列。`Vec<Slot>` 全部。使っていない上位も含めて全部でよい）
2. `self.ci` の各 `CallInfo`: `proc_`、`target_class`、`env`（`Option<ObjId>`）
3. `self.contexts` の各 `Context`: `stack`、`ci`（同上）、`fib`、`proc_`。**終了した文脈**（`status == Terminated`）は空になっているので走査量は少ない。
   実行中の文脈のぶんは `self.stack`/`self.ci` に入っている（swap 済み）ので二重に見ても害は無い
4. `self.globals`（`HashMap<Sym, Slot>`）
5. `self.exc`（伝播中の例外または `RBreak`）
6. `self.core`（クラス群の `ObjId`）、`self.top_self`、`self.call_proc`
7. `self.inspect_guard`（`Vec<ObjId>`）、`self.eq_guard`（`Vec<(ObjId, ObjId)>`）、`self.pending_kw`（`Option<Value>`）
8. `self.gc_registered: Vec<ObjId>`（新設。`Vm::gc_register(id)` / `gc_unregister(id)`。ホスト（rubevy）が複数フレームにまたがって持つオブジェクト用。本家 `mrb_gc_register`）
9. `self.ireps` は辿らなくてよい（リテラルは `Pool` に Rust 値として入っていて `ObjId` を持たない。確認: `grep -n "enum Pool" src/`）。
   `Method::Ruby(ObjId)` は各クラスの `methods` 表から辿る（下記）。
10. シンボル表（`self.syms`）は回収対象外。

オブジェクトから辿る参照（`HeapObject`）: `class`、`ivars`（`Vec<(Sym, Slot)>`）、そして `kind` ごとに:

| `ObjKind` | 辿るもの |
|---|---|
| `Object`、`String`、`Exception` | なし（ivars と class のみ） |
| `Array(Vec<Slot>)` | 全要素 |
| `Hash(HashData)` | `entries` の key と value、`default` |
| `Range { begin, end, .. }` | begin、end |
| `Proc(ProcData)` | `upper`、`env`、`target_class` |
| `Env(EnvData)` | `values`（切り離し後の値）、`target_class`。付いたまま（`attached`）の環境は値がスタック側にあるので `values` は空。`ctx` が指す文脈のスタックは根 3 で辿られる |
| `Class(ClassData)` | `superclass`、`methods` の `Method::Ruby(ObjId)`、`consts`、`cvars`、`attached`、`iclass_of`、`origin`、`origin_of`、`outer` |
| `Break { value, .. }` | value |
| `Fiber(ctx)` | `contexts[ctx]` の `stack`、`ci`、`proc_`（未初期化 `usize::MAX` は無視） |

マークは**明示的なスタック（`Vec<ObjId>`）を使った反復**で行い、Rust の再帰は使わない（深い配列の入れ子でホストのスタックを溢れさせない。no_std でも同じ）。

### 1.4 スイープ後

* `heap.len()` を `live` として使っている箇所（`GC.stat`、`object_id` の表示など）を `live = objs.len() - free.len()` に直す。
* `ObjId` は再利用されるので `object_id` と `inspect` の `0x...` は再利用される。本家もアドレスを再利用するので差異ではない。`docs/gc.md` に書く。
* Fiber の文脈: `Fiber` オブジェクトが回収されたら `contexts[ctx]` の `stack`/`ci`/`proc_` を空にして（`Context::new(Terminated)` に置き換え）、
  番号は再利用しない（`contexts` は伸びるが、中身は空）。これで十分。番号の再利用は将来の課題として `docs/gc.md` に書く。

### 1.5 触るファイルと関数

* `src/object.rs`: `Heap` に `free`、`mark`（または `HeapObject.marked`）、`allocated_since_gc`、`malloc_increase`、`gc_pending` を追加。`alloc`/`alloc_raw` を変更。
  `sweep(&mut self)`、`is_free`、`live_count` を追加。**`HeapObject` の drop で payload を落とす**（`kind` を差し替えれば `Vec` は drop される）。
* `src/vm.rs`: `native_active`、`gc_registered`、`gc_interval_ratio`、`live_after_gc` を `Vm` に追加。`gc_maybe`、`gc_collect`（マーク＋スイープ本体）、`gc_mark_roots`、
  `gc_register`/`gc_unregister` を追加。`exec_frames` のループ先頭に判定を 1 つ。`call_native_direct` と `funcall` に `native_active` の増減。
* `src/builtins/object.rs`: `GC.start`（§1.2 の規則で `gc_collect` を呼ぶ）、`GC.stat`（`live`、`malloc_increase`、`malloc_threshold`、`interval_ratio` を実値に）、
  `GC.interval_ratio=`（値を保持）。他は現状維持。
* `src/builtins/fiber.rs`: 変更不要（回収は `Fiber` オブジェクトのスイープで §1.4）。
* `src/bin/sabiruby.rs`: `run --stats` に `gc: N collections, live: M` を足す。環境変数 `SABIRUBY_GC_STRESS=1` で §4 のストレスモード。

## 2. 手順

1. `Heap` にフリーリストとマーク領域を入れ、`alloc` で空きを再利用する（まだ回収しない）。`cargo test` と `tools/mrbtest.sh` が変わらないことを確認。
2. `gc_mark_roots` と `gc_collect` を書く。最初は `GC.start` からだけ呼ぶ。`tools/mrbtest.sh -v gc` で 5 件の `arena` テストが通ることを確認。
3. ストレスモード（§4）で全テストを回し、根の漏れを潰す。**ここが本番**。落ちた箇所から「誰が参照を持っていたか」を辿り、根の表に足す。
4. 自動起動（`gc_pending`、閾値）を入れる。`tools/mrbtest.sh` と `cargo test` と `tools/check_no_std.sh`。
5. 計測（§5）。
6. 文書（§6）。

## 3. 注意点（踏みやすい点）

* `Heap::alloc` の途中で GC を走らせない（`alloc` は `gc_pending` を立てるだけ）。
* `hash_sync`／`hash_index` は `key_hash` で Ruby の `hash` を呼ぶ。その間に GC が走っても、呼び出しは SEND 経由なら `native_active > 0`… **ではない**。
  `hash_set` はネイティブ（例: `Hash#[]=`）の中から呼ばれるので `native_active > 0` で守られる。バイトコード側の `HASH`／`HASHADD` 命令から直接 `hash_set` を呼ぶ経路がある場合、
  その命令の実行中は命令境界ではないので GC は走らない（境界判定はループ先頭だけ）。確認: `grep -n "hash_set\|hash_new" src/vm.rs`。
* `Op::Send` でネイティブを呼ぶ `call_native_direct`: 戻り値を `self.stack[base + a]` に書く**前**に GC は走らない（境界は次のループ先頭）。OK。
* Fiber 切り替え直後: `switch_context` で `self.stack`/`self.ci` が入れ替わるが、根 1〜3 で両方辿るので問題ない。
* `EnvData.attached == true` の環境の値は `contexts[ctx].stack`（または `self.stack`）にある。根 1/3 で辿られる。`ctx` の文脈が回収済み（§1.4 で空にした）なら、
  その環境は既に `pop_frame` で切り離されているはず（Fiber 終了時にフレームは全部外れる）。`debug_assert` を置く。
* `Vm::step`（rubevy）: 予算切れの一時停止は `run_loop` から戻るだけで、状態は全部 `Vm` の中にある。GC はその途中でも走ってよい。
  rubevy が `ObjId` を Bevy 側に保持するようになったら `gc_register` を使う（現在は保持していない。`grep -rn ObjId ../rubevy/src`）。
* `inspect_guard`／`eq_guard` は「実行中の `inspect`／`==` の対」で、ネイティブ実行中にしか要素が無い。根に入れておけば安全。
* `notimpl_fns`、`native_arity`、`op_counts`、`out` は参照を持たない。
* `ObjKind::Class` の `methods` に `Method::Native`／`AttrReader(Sym)`／`AttrWriter(Sym)`／`Undef` があるが、辿るのは `Ruby(ObjId)` だけ。
* `core.fiber` など `Core` の全フィールドを根にする（`Core` は `Copy` の `ObjId` 集合）。
* 解放したスロットの `class` は `ObjId(0)` にしておき、`class_of` が万一触っても panic しないようにする（`objs[0]` は `BasicObject` 以外か確認して適宜）。

## 4. 検証

* **ストレスモード**（必須）: 環境変数 `SABIRUBY_GC_STRESS=1`（std 有効時のみ読む。`lib` 側は `Vm::gc_stress: bool` を持ち、`bin` と `mrbtest` サブコマンドが設定する）
  で「割り当てが 1 回でもあれば次の命令境界で回収」にする。本家の `MRB_GC_STRESS` に相当。
  `SABIRUBY_GC_STRESS=1 tools/mrbtest.sh` と `SABIRUBY_GC_STRESS=1 cargo test --release` が**通常時と同じ結果**になること。遅いので時間がかかるが、根の漏れはこれでしか見つからない。
* 通常: `tools/mrbtest.sh`（`tests/mrbtest/baseline.txt` と比べて回帰ゼロ。`gc` ファイルは 23 か 24 に上がるはず。残る 1 skip は `requires mruby-bigint`）、
  `cargo test --release`（照合スクリプト 16 本）、`tools/check_no_std.sh`。
* メモリ: `bench/gc_churn.rb`（新設。`i = 0; while i < 1_000_000; a = [i, "x" * 10, {k: i}]; i += 1; end; p GC.stat[:live]`）を `sabiruby run --stats` で走らせ、
  `live` が数千以下で安定すること。`/usr/bin/time -v` の Maximum resident set size が回収なし版（git の直前コミット）より桁で小さいこと。
* 速度: `tools/bench.sh` の `bm_fib`、`bm_so_lists`、`bm_so_mandelbrot` が導入前（`docs/bench.md`: 3.5x、15.9x、1.9x 前後）から誤差の範囲であること。
  1 命令あたりの追加コストは `bool` 判定 1 回なので、悪化が 5% を超えたら原因を調べる（`alloc` に重い処理を入れていないか）。
* テスト結果の理由の表 `docs/mrbtest-notes.md` と `tests/mrbtest/notes.tsv` の `gc` 行を更新する（5 KO が消える）。`tools/mrbtest.sh --update` で baseline を更新。

## 5. 計測して記録するもの

`docs/performance.md` の Measured に追記:

* ベンチ 3 本の導入前後。
* `gc_churn.rb` の実行時間と回収回数、`live` の推移。
* 1 回の回収にかかる時間の目安（`--stats` に総 GC 時間を足してよい。std のときだけ計る）。

## 6. 文書（必須）

* `docs/gc.md`（新設、英語で可）: 方式、根の表（§1.3 をそのまま）、ネイティブから見た約束（「ネイティブ実行中は回収しない」「複数フレームにまたがって持つなら `gc_register`」）、
  `GC.*` の意味、意図した差異（`GC.stat` の項目のうち実値でないもの、`object_id` の再利用、世代別なし）、将来の課題（増分化にはライトバリアが要る、`contexts` の番号再利用）。
* `README.md`: Status の「garbage collection (the heap only grows)」を消し、`docs/gc.md` へのリンク。
* `docs/mrbtest-notes.md` / `tests/mrbtest/notes.tsv`: `gc` の行。
* 本『Deep dive into mruby』のリポジトリ（`../../book_mruby3`）の `docs/notes/sabiruby-findings.md` に節を足す（日本語。「§13 GC」。決めたこと、踏んだ罠、本家との対応、本の第 4 章・段階 8 との対応と、本文に足すべき点）。
  本文（`.re`）は触らない。著者が後で反映する。

## 7. 完了条件

1. `tools/mrbtest.sh` で `gc` が 23/24 以上、他のファイルに回帰なし。
2. ストレスモードで全テストと照合スクリプトが通常時と同じ結果。
3. `gc_churn.rb` で `live` が安定し、RSS が桁で減る。
4. ベンチ 3 本が誤差の範囲。
5. `tools/check_no_std.sh` が通る。
6. §6 の文書がある。
7. コミットは 2 段階以上（フリーリスト＋マーク＆スイープ／自動起動と文書）に分け、メッセージに結果の数字を入れる。push まで。
