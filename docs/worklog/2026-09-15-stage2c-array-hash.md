# 作業記録: 段階 2c（`Array#shift` を O(1) に、Hash に索引を持たせる）

`docs/worklog/2026-09-15-stage2-perf.md` の末尾で「止まって報告するもの」として残した 2 件、
(A) `Array#shift` の O(n) と (B) Hash のハッシュ表化を、著者の判断（2026-09-15）に従って実装した記録。
判断は (A) 案 1（`ObjKind::Array` に開始オフセットを持たせる）、(B) 案 2（`HashData` の書き込みを 1 本化してから索引を足す）。

作業: worktree `sabiruby-wt-perf`、ブランチ `bridge-perf3`、基準は main の `38c78f1`。
機械は i7-13700（P コア 2 に `taskset` 固定）の WSL2。本家は Docker の `kishima/mruby:4.1.0-rc`（`docker info` は通った）。

## 0. 基準を先に確かめた

`38c78f1` をそのまま `cargo build --release --workspace` してバイナリを退避し
（`scratchpad/bin/base-38c78f1`）、本家テストの全ファイルを `tests/mrbtest/baseline.txt` と突き合わせた。
`tools/mrbtest.sh` は Docker で本家の `.rb` を集めて再コンパイルするところから始まるので、
基準だけ確かめたいときには重すぎる。`baseline.txt` の各行（`<name> <ok 数>`）を読んで
`sabiruby mrbtest tests/mrbtest/assert.mrb tests/mrbtest/prelude.mrb tests/mrbtest/<name>.mrb < /dev/null`
を 1 本ずつ回し、出力の markdown 表の `| **all** |` 行の `ok` 列を比べる小さなスクリプトを
スクラッチパッドに置いた（`gem_ascii_*` は対象外）。基準は `ALL OK`。

---

# (A) `Array#shift` を O(1) に

## 何が O(n) だったか

`ObjKind::Array(Vec<Slot>)` なので `Array#shift` は `Vec::remove(0)`（`src/builtins/array.rs:154`）。
前の担当の切り分けでは `bm_so_lists` 3685 ms のうち 2870 ms がここ 1 つだった。
本家は `RArray` が共有バッファの途中を指せるので、`shift` は先頭ポインタを 1 つ進めるだけで済む。

## 何を作ったか

共有までは真似しない。共有を入れると「いつ共有をほどくか」（本家の `ARY_SHARED`、`mrb_ary_modify`）を
どの書き込み口でも判断しなければならず、`ObjKind` の外に染み出す。
**オフセットだけ**を配列自身に持たせた（`src/object.rs` の `ArrayData`）:

```rust
pub struct ArrayData {
    buf: Vec<Slot>,
    start: usize,
}
```

`buf[start..]` が要素、`buf[..start]` が `shift` が前に残した空き。これは本の言う「選べる実装」の側で、
Ruby から見える振る舞い（`shift`、`shift(n)`、空配列の `shift` は nil）は本家と同じままにできる。

**読む側には見せない。** `ArrayData` は `Deref<Target = [Slot]>` / `DerefMut` を持ち、フィールドは非公開。
配列を読む場所（`items()`、`ary_vals`、GC の mark、`inspect`、`ext_pack.rs`、`ext_struct.rs`、
`vm.rs` の ARRAY 系 opcode）は今までどおりスライスを受け取り、オフセットの存在を知らない。
`Vm::ary` と `Heap::array` の戻り値は `Option<&Vec<Slot>>` から `Option<&[Slot]>` に変えた
（呼び手 20 か所はすべて `len` / `iter` / `get` / `first` / `last` / 添字なので、スライスで足りる）。
**書く側だけ**が `ArrayData` のメソッドを通る: `push` / `pop` / `clear` / `truncate` / `resize` /
`extend`（`Extend` 実装）/ `split_off` / `insert` / `remove` / `shift` / `shift_n` / `unshift` /
`drain_range` / `splice_range`。`Vec` の `drain`/`splice` はレンジを取るので、
オフセットを足し込む必要から `drain_range(from, to)` / `splice_range(from, to, items)` という
2 引数の形に置き換えた（呼び手は `array.rs` の 3 か所だけ）。

**前詰め（`compact`）。** `shift` のたびに `start > len && start > 16` なら `buf.drain(..start)` で前詰めして
`start = 0` に戻す。`start` がそこまで育つには少なくとも `len` 回の `shift` が要り、詰め直しの費用も
`len` なので、`shift` 1 回あたりの償却費用は定数に収まる。閾値 16 は「小さい配列では
どうせコピーが安い」ことと、「`shift` だけで空にしていく配列（`start == buf.len()`、`len == 0`）が
毎回コピーしてしまう」ことの両方を避けるために置いた。

**`unshift` も O(1) にした。** 前に空きがある（`items.len() <= start`）ならそこへ書いて `start` を戻すだけ。
足りなければ従来どおり `splice`。`insert(0, v)` も同じ早道を通る。

**GC。** 空きになったスロットは `Slot::NIL` で潰してから `start` を進める。mark が見るのは
`buf[start..]`（`Deref` 越し）なので前の分は元々たどられないが、配列が手放した値を
オブジェクトが握ったままにしないため。

## 直した場所

コンパイラに数えさせた（`ObjKind::Array` を書き換えたあと `cargo build` のエラーが 11 個）。
`ObjKind::Array(Vec::new())` → `ObjKind::Array(Default::default())` が 4 か所
（`builtins/mod.rs`、`ext_struct.rs`、`ext_data.rs` × 2）、`*arr = slots_of(&x)` → `.into()` が 12 か所、
`slots_of(&v)` を `ObjKind::Array` に渡す 2 か所、`with_mut` のクロージャ引数の型、
`Vm::ary` と `Heap::array` の戻り値、GC の mark の `for v in a` → `for v in a.iter()`。
`Marshal` は無い。`pack` と `Struct`（配列形）はスライスしか使っていないので無変更で通った。

## 確かめたこと

まず本家との差を直接見た。`shift` / `shift(n)` / 空配列 / `unshift` / `insert(0)` / `[]=` / `slice!` /
`delete_at` / `concat` / `clear` / `sort` / `join` / `inspect` / `pack` / `Struct` / `dup` / `clone` /
「前詰めをまたぐ 30 回の push+shift」を 1 本のスクリプトにして、
本家（`kishima/mruby:4.1.0-rc`）と SabiRuby の出力を `diff` した ―― **完全一致**。

- `cargo test --workspace`: 失敗 0。
- 本家テスト全ファイル（utf8 ビルド、バイト文字列ビルドの両方）: `ALL OK`（`baseline.txt` / `baseline-bytes.txt`）。
- `tools/check_no_std.sh`: `no_std OK`。`grep -rn unsafe src` は 0 のまま。`cargo doc --no-deps` 警告なし。

## 外れた最初の計測 ―― `ds_array` が 2.5 倍遅くなった

最初の交互 A/B で `bm_so_lists` は **−73.2%** になったが、同時に **`ds_array` が +154.8%**（898 ms → 2288 ms）だった。
`ds_array` は `push` / `[]` / `[]=` / `each` だけで、`shift` も `unshift` も出てこない。オフセットが 1 段増えたくらいで
2.5 倍になるはずがないので、ベンチを 3 つに割って（`push` だけ、`a[j] = a[j] + 1` だけ、`each` だけ）測り直した:

| 切り分け | A（`38c78f1`） | B（オフセット版） |
|---|---:|---:|
| `push` 2000 × 1300 | 449 ms | 456 ms |
| `a[j] = a[j] + 1` 2000 × 1300 | 442 ms | 3043 ms |
| `each` 2000 × 1300 | 891 ms | 896 ms |

`[]=` だけが 7 倍。`ary_aset`（`src/builtins/array.rs:268`）の冒頭はこうだった:

```rust
let len = items(vm, s).len();
```

`items` は `vm.ary_vals(v).unwrap_or_default()`、つまり**配列全体を `Vec<Value>` に複製してから長さだけを見る**。
2000 要素の複製を 260 万回やれば 3 秒かかるのは当たり前で、おかしいのは**変更前が 442 ms で済んでいたこと**のほうだった。

理由は LLVM の割り当て除去だと考えている。`values_of(slots).len()` は、確保した `Vec` の中身を誰も読まないので、
Rust の `alloc`/`dealloc` は除去可能な呼び出しとしてマークされており、`ary_vals` と `values_of` が
呼び出し側にインライン展開されれば `collect` ごと消えて `slots.len()` だけが残る。
`Heap::array` の戻り値を `&Vec<Slot>` から `&[Slot]`（中身は `&buf[start..]`）に変えたことで、
この連鎖のどこかでインライン展開が起きなくなり、**隠れていた O(n) が表に出た**。

念のため `as_slice` を `&self.buf[self.start..]` から
`match self.buf.get(self.start..) { Some(s) => s, None => &[] }`（パニック経路の無い形）に変えて測ったが
3094 / 3109 / 3157 ms で変わらなかったので、境界検査やパニック経路のせいではない。元の形に戻した。

**直し方**: 長さしか要らないところで配列を複製しない。`ary_len(vm, v) = vm.ary(v).map(|l| l.len()).unwrap_or(0)`
を `array.rs` と `ext_array.rs` に置き、`items(vm, s).len()` の 8 か所（`ary_aset`、`insert`、`compact!`、
`ext_array.rs` の `__aset_range` ほか 5 つ）を差し替えた。`ds_split2` は A 443 / B 440 に戻った。

これは本の素材になる話だと思う。**「最適化が消してくれていた無駄」は、無駄が消えたのではなく見えなくなっていただけで、
表現をひとつ変えた拍子に戻ってくる。** 元のコードは最初から「長さを知るために配列を複製する」と書いてあった。

## 数値（交互 A/B、P コア 2 固定、5 ラウンド、`bench/results/ab-38c78f1-aryshift.tsv`）

| 分類 | A（`38c78f1`）ms | B（オフセット版）ms | 変化 |
|---|---:|---:|---:|
| 実アプリ寄り | 8895 | 8935 | +0.5% |
| データ構造 | 5841 | 3163 | **−45.8%** |
| 命令ループ | 21524 | 21587 | +0.3% |
| 呼び出し | 3785 | 3765 | −0.5% |
| メモリ | 2037 | 2044 | +0.4% |
| **全体** | **42082** | **39495** | **−6.1%** |

個別で動いたのは `bm_so_lists` の **3698.6 → 998.9 ms（−73.0%）**だけで、残り 19 本は −1.3%〜+2.5% に収まっている。
`shift` を使わないベンチでは何も変わらない、という予想どおりの形。`ds_array` は +0.1%、`ds_hash` +2.3%、
`app_robot` +2.5%、`call_kwargs` −1.3%。前の担当が書いたとおり、呼び出しを含まないベンチが候補ごとに
数パーセント動くのは 9000 行の `exec_frames` のコード配置の揺れなので、この範囲は変化と見ていない。

`bm_so_lists` の本家比は、段階 2 終了時点の **15.34x から 4.1x** 前後まで下がる見込み
（本家込みの通し計測は本体のレビュー後に取る想定。ここでは交互 A/B の比だけを報告する）。

`bm_ao_render` と `bm_mandel_term` は A・B どちらでも `fail` になる。基準のバイナリ（`38c78f1`）でも
`undefined method 'printf' for Object` / `undefined method 'putc' for Object` で落ちるので、
この変更とは無関係の既存の穴（`sabiruby run` に `printf` / `putc` が無い）。報告に残す。

---

# (B) Hash

著者の判断は案 2、「まず `HashData` の `entries`/`hashes` を非公開にして書き込みを 1 本化し、
そのうえで索引を持たせる」。**2 つのコミットに分けた**: B1 は非公開化だけで振る舞いも性能も変えない、
B2 で索引を足す。分ける意味は、B2 で何か壊れたときに「索引のせい」と「入口を変えたせい」を
切り分けられることと、B1 の時点で「ぶれの範囲」を測っておけば B2 の数字がそのまま索引の効果になること。

## B1: 書き込みを 1 本化する

### 何が問題だったか

`HashData` は `entries: Vec<(Slot, Slot)>` と `hashes: Vec<i64>`（キーのハッシュ値、`entries` と平行）を
`pub` で持っていた。前の担当が数えたとおり、`entries` を書き換える場所は 11 か所あり、
そのうち削除（`Hash#delete`、`#shift`）は**位置をずらす**。索引は「ハッシュ値 → `entries` の添字」なので、
位置がずれた瞬間に丸ごと嘘になる。

いまのコードには「`hashes.len() != entries.len()` ならハッシュ値を作り直す」という遅延同期の合図があるが、
削除は `entries` と `hashes` を**両方**縮めるので長さが一致したままで、この合図では捕まえられない。
つまり索引の無効化は、**書き込み側が自分で覚えておく**しかない。覚えられるようにするのが B1。

### 何を作ったか

`entries` と `hashes` を非公開にして、読みと書きの入口を `HashData` のメソッドに集めた（`src/object.rs`）。

読み: `entries()`（スライス）、`len()`、`is_empty()`、`hash_at(i)`、`hashes_stale()`、
そして**探索の入口 2 つ** `first_candidate(kh)` と `next_candidate(p, kh)`。
探索を `HashData` の中に入れたのは、B2 で「走査」を「索引引き」に差し替える場所をここ 1 か所にするため。
探索が 2 つに割れているのは、候補を確かめるには `eql?` を呼ばねばならず、`eql?` は Ruby でありうるので
**借用をまたげない**から。`Vm::hash_index` は「候補をもらう → 借用を手放して `eql?` → 次の候補をもらう」
という形になった:

```rust
let mut cand = match &self.heap.get(o).kind { ObjKind::Hash(hd) => hd.first_candidate(kh), _ => None };
while let Some((p, ek)) = cand {
    if self.key_eql(k, ek.get())? { return Ok(Some(p)); }
    cand = match &self.heap.get(o).kind { ObjKind::Hash(hd) => hd.next_candidate(p, kh), _ => None };
}
```

書き: `push_entry(k, v, kh)` / `set_value_at(i, v)` / `remove_entry(i)` / `clear()` /
`set_entries(entries)` / `set_entries_with_hashes(entries, hashes)` / `set_hashes(hashes)` /
`from_entries(entries, default)`。`set_entries` と `set_entries_with_hashes` が別なのは、
呼び手が「キーをハッシュ済みか」で分かれるから ―― `replace` や `merge` は他所から来たキーを渡すので
ハッシュ値を計算できず（計算には VM が要る）空にして次の探索に任せる、`rehash` と `compact!` は
自分で計算した値を持っているので渡す。

書き換えたのは `object.rs` を除いて **11 ファイル・54 行**（`.entries` / `.hashes` の参照は 42 か所。
前の担当の見積もりは 37 か所だった）。`ext_objectspace.rs` の
`GC.stat` がハッシュを `clear` していたのはコンパイラに教えてもらった（数え漏れ）。
`Hash#dup` が `HashData { .. }` を直接組み立てていた 1 か所は `from_entries` に、
`Object#clone` の `HashData { entries: .., hashes: .., default: .. }` は `#[derive(Clone)]` を足して
`hd.clone()` にした。

### 確かめたこと

`cargo test --workspace` 失敗 0、本家テスト全ファイル `ALL OK`、`check_no_std.sh` `no_std OK`、
`unsafe` 0、`cargo doc --no-deps` 警告なし。

### 数値（B1 = 非公開化だけ。交互 A/B、`bench/results/ab-aryshift-hashencap.tsv`）

| 分類 | A（(A) の版）ms | B（非公開化）ms | 変化 |
|---|---:|---:|---:|
| 実アプリ寄り | 9596 | 9450 | −1.5% |
| データ構造 | 3154 | 3170 | +0.5% |
| 命令ループ | 21553 | 21539 | −0.1% |
| 呼び出し | 3763 | 3791 | +0.7% |
| メモリ | 2057 | 2043 | −0.7% |
| **全体** | **40124** | **39994** | **−0.3%** |

狙いどおり「ぶれの範囲」。個別は −6.7%（`app_robot`）〜 +2.8%（`app_json_hash`）で、
どちらも Hash の書き込み方を変えていない（`app_robot` は Hash をほとんど使わない）ので、
コード配置の揺れ。`ds_hash` は −0.4%、`app_json_hash` の +2.8% は次の B2 の計測では +0.1% に戻っている。

**この計測は 1 回やり直した。** 最初に取った回は `app_tak` の best と median が 1138 / 1643（44% 差）、
`vm_optimization_bench` が 18302 / 19615（7% 差）と荒れていて、全体 +1.4% という値が出た。
荒れた原因ははっきりしていて、**計測中に自分が `cargo build` を 1 回走らせ、進捗を何度も `tail` で見ていた**。
前の担当が「自分の道具も外乱になる」と書いていたとおりで、静かにして取り直したら best と median の差は
どのベンチも 1% 以内に収まり、全体 −0.3% になった。荒れた回の TSV は残していない（`bench/results/` に
入っているのは取り直した方）。

## B2: 索引

### 形

`HashData` に 2 つ足した:

```rust
index: Option<HashMap<i64, u32>>,   // ハッシュ値 -> その値を持つ 1 つの位置
chain: Vec<u32>,                    // chain[i] = i と同じハッシュ値を持つ別の位置（NO_ENTRY で終わり）
```

`entries` の `Vec` が挿入順を保つので、**索引は順序を一切持たない**。本家の HT モードは
テーブル側に順序の情報を持たせる必要があるが、こちらは `entries` が最初からそれなので、索引は
「ハッシュ値 → 位置」だけでよい。閾値は本家の AR/HT の切り替えと同じ **16**
（`HASH_INDEX_THRESHOLD`）。

**衝突の扱いを本家より簡単にできた理由**を書いておく。当初は「ハッシュ値 → 位置の `Vec`」にしようとしたが、
それだと異なるキーごとに小さな `Vec` を確保することになり、1000 要素のハッシュで 1000 回の確保になる。
実際には**チェインを 1 本の `Vec<u32>` に畳める**。ハッシュの中のキーは `eql?` で一意なので
（`hash_set` がそれを保証している）、**同じハッシュ値を持つ候補のうち一致するのは高々 1 つ**、
つまり候補を返す順番はどうでもよい。だから新しい要素をチェインの先頭に挿し込めばよく、末尾を覚えておく必要がない:

```rust
chain[at] = ix.insert(kh, at).unwrap_or(NO_ENTRY);   // O(1)
```

B1 で探索の入口を `first_candidate` / `next_candidate` の 2 つに割っておいたのがここで効いた。
索引があるときは前者が `index.get(&kh)`、後者が `chain[p]` を読むだけになり、`Vm::hash_index` は 1 行も変わっていない。

### 無効化

* `push_entry`: 索引があればチェインに足す（O(1)）。無ければ `reindex()` を呼び、
  閾値を越えていてハッシュ値が揃っていれば作る（越えた瞬間に 1 回だけ O(n)）。
* `remove_entry`: 削除した位置より後ろが全部 1 つずれるので、**索引を作り直す**。O(n) だが、
  直前の `Vec::remove` がすでに O(n) を払っているので、次数は変わらない（定数は増える）。
* `clear` / `set_entries`: 索引を捨てる。`set_entries` はハッシュ値も捨てるので、
  どのみち次の `hash_sync` まで索引は作れない。
* `set_entries_with_hashes` / `set_hashes`: `reindex()`。後者は `hash_sync` が
  「ハッシュ値が揃った」と知らせてくる唯一の場所なので、索引が作られるのは実質ここか `push_entry`。

**「索引を持つ / 持たない」で振る舞いが変わらないことの確かめ方**が肝で、B1 で入口を 1 本にしたので
無効化の置き場所を探す必要がなくなった ―― `entries` を書き換えられる場所が 7 つしかなく、そのすべてが
同じファイルの隣り合った行にある。

### 確かめたこと

`tests/custom/hash_index_boundary.rb` を足した（本家の `mrbc` でコンパイルし、
`.expected` は本家 mruby 4.1.0-rc の出力）。境界の両側（15・16・17・40 要素）、
閾値を下回るまでの削除、途中の削除で位置がずれる場合、`Hash#shift`、
**同じ `hash` を返すが `eql?` ではないキー 25 個**（衝突チェインが本当に歩かれる）、
キーを壊してからの `rehash`、`replace` / `dup` / `merge` / `clear` / `compact!`、
凍結された文字列キー。

* 本家の出力と **1 バイトも違わない**。
* 同じ `.mrb` を**索引なしのビルド（B1）でも回して、同じ出力**になることを確かめた。
  「索引を持つ Hash と持たない Hash で振る舞いが変わらない」の直接の証拠。
* 本家テスト全ファイル（utf8 / バイト文字列の両方）`ALL OK`、`cargo test --workspace` 失敗 0、
  `no_std OK`、`unsafe` 0、`cargo doc --no-deps` 警告なし。

### 数値（交互 A/B、`bench/results/ab-hashencap-hashindex.tsv`）

| 分類 | A（B1）ms | B（索引）ms | 変化 |
|---|---:|---:|---:|
| 実アプリ寄り | 8895 | 8818 | −0.9% |
| データ構造 | 3155 | 2986 | −5.4% |
| 命令ループ | 21534 | 18997 | **−11.8%** |
| 呼び出し | 3771 | 3777 | +0.1% |
| メモリ | 2025 | 2005 | −1.0% |
| **全体** | **39380** | **36582** | **−7.1%** |

大きいのは 2 本:

* **`ds_hash` −19.8%**（803 → 644 ms）。1000 要素のハッシュに 1000 回入れて 1000 回引く形なので、
  挿入も探索も線形だった。
* **`vm_optimization_bench` −14.4%**（17649 → 15106 ms）。「命令ループ」に分類されているが、
  中身に `hash_read`（1000 要素のハッシュを 100 万回引く）と `hash_ops`（**50000 要素**のハッシュを
  作って走査する）が入っている。後者は完全な O(n²) で、索引が効くのはここ。
  分類の名前と中身がずれているので、`docs/bench.md` を更新するときは注意がいる。

小さいハッシュしか使わない `app_json_hash` は +0.1%、`app_robot` −0.2% で、
**閾値の下では何も変わらない**（索引を作らないので `Option` の分岐 1 つだけ増える）。
`call_kwargs` の +3.7% はキーワード引数のハッシュが 1〜3 要素で索引に届かないので、コード配置の揺れと見ている。

### 捨てた案

* **`HashMap<i64, Vec<u32>>`**（バケットごとに位置のリスト）。素直だが、異なるキーの数だけ
  `Vec` を確保する。キーが `eql?` で一意という性質を使えば 1 本の `Vec<u32>` で足りると気づいて捨てた。
* **削除のときに索引を「捨てるだけ」にする**（次の探索から線形に戻る）。実装は 1 行で済むが、
  削除と探索が交互に来ると索引が二度と作られないまま O(n) に戻り続ける。
  作り直しも O(n) で、`Vec::remove` がすでに O(n) を払っている以上、次数は同じ。作り直す方にした。
* **`entries` の削除を tombstone にする**（本家の HT が近い）。位置がずれなくなるので索引を保てるが、
  `entries()` を返しているすべての読み手（`each`、`keys`、`inspect`、`to_a`、gem の 10 か所以上）が
  穴を飛ばさなければならなくなる。B1 で「読みもメソッド越し」にしたので不可能ではないが、
  この段階の目的（索引を入れる）に対して代償が大きすぎる。

---

# まとめと、本体に渡すもの

## 3 コミットの積み上げ（すべて交互 A/B、P コア 2 固定、5 ラウンド）

| コミット | 変えたもの | 全体 | 目立つもの |
|---|---|---:|---|
| (A) `35d58a5` | `Array` に開始オフセット | **−6.1%** | `bm_so_lists` −73.0% |
| (B1) `8ba2336` | `HashData` の書き込みを 1 本化 | −0.3% | （ぶれの範囲） |
| (B2) | Hash の索引（16 要素超） | **−7.1%** | `ds_hash` −19.8%、`vm_optimization_bench` −14.4% |

掛け合わせると `38c78f1` から **−12.8%**。段階 2・2b の −22.0% と合わせると、
`440d4ba` からは −32% 前後になる計算（本家込みの通し計測は本体が取る）。

## 気づいたこと（本体の判断が要るもの）

1. `docs/performance.md` の「Known structural costs」の 2 行 ――
   「Native methods copy arrays (`items()`) ...; `Array#shift`/`unshift` are O(n).」と
   「`Hash` is insertion-ordered linear search ...; no hash table for large hashes yet.」――
   はこの 3 コミットで古くなった。`items()` が配列を複製するのは**まだ本当**（直したのは
   長さだけが欲しい 8 か所）なので、書き換えるなら分けたほうがよい。
2. `bm_ao_render` と `bm_mandel_term` が `sabiruby run` で落ちる（`printf` / `putc` が無い）。
   `38c78f1` でも落ちるので新しい穴ではないが、`tools/bench_ab.sh` はこの 2 本を `fail` として
   飛ばすので、**ベンチ 22 本のうち 20 本しか測れていない**。
3. `vm_optimization_bench` は分類が「命令ループ」だが、中身の `hash_ops` は 50000 要素の
   Hash を作る。索引の効果がこの分類に乗ってしまう。
