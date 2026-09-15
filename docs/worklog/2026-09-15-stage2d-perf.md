# 作業記録: 段階 2d（Hash の固定費、`items()` の複製、ベンチの分類）

`docs/plans/host-bridge-plan.md` の段階 2d。基準は main の `b627d0a`、worktree `sabiruby-wt-perf`、
ブランチ `bridge-perf4`。機械は i7-13700（P コア 2 に `taskset` 固定）の WSL2、本家は Docker の
`kishima/mruby:4.1.0-rc`（`docker info` は通った）。段階 3b を別の担当が別の worktree で進めているので、
`src/vm.rs` の公開 API（`task_running` / `ivar_get` / `ivar_set` / `global_get` / `global_set` / `is_exception`）と
`tests/host_api.rs` には触らない。ベンチを回すのはこちらだけ。

## 0. 基準の計測

worktree を作った直後に `cargo build --release --workspace` して、そのバイナリを退避
（スクラッチパッドの `bin/base-b627d0a`）、`tools/bench.sh --core 2 --runs 5` で
`bench/results/b627d0a.tsv` を取った。best と median の差はほとんどのベンチで 1% 以内
（例外は `vm_optimization_bench` 2.3%、`loop_while_add` 2.3%）なので、静かな窓で取れている。
`9da4724`（段階 2c 終了時点）の通し計測とも 1% 以内で一致した。

計測の前に 1 つだけ細工をした。`tools/bench.sh` は Docker があると `bench/src/*.rb` を本家の `mrbc` で
コンパイルし直すが、`bench/src/bm_app_lc_fizzbuzz.rb` だけは `.mrb` が repo に無い（分類表にも無い）。
そのまま回すと 23 本目のベンチが増えて過去の結果と比べられなくなるので、この 1 本を計測の間だけ
どかし、`MRUBY_SRC` を存在しないパスにして本家ツリーからの取り込みも止めた。
残りの 22 本の `.mrb` は再生成しても 1 バイトも変わらなかった（`git status` が空）。
`bm_ao_render` と `bm_mandel_term` は `printf` / `putc` が無くて基準でも落ちる（段階 2c からの既知の穴）。

## 1. 切り分け

`perf` はこの環境に無いので、これまでと同じく操作ごとの小さなスクリプトで測る
（スクラッチパッドの `micro/*.rb`、min of 5、`taskset -c 2`）。
Hash 系はすべて同じ形にした ―― 外 50,000 × 内 10 の二重ループで 50 万回、
本体だけを差し替え、ループだけの版（`h00_loop` 22.7 ms）を引いて 1 回あたりに直す。

### 1.1 Hash の 16 要素以下の固定費は何でできているか

| 切り分け | ms | ループを引いて 1 回あたり |
|---|---:|---:|
| `h00_loop` ループだけ（`s += j`） | 22.7 | — |
| `h12_ary_get10` `s += a[j]`（10 要素の Array） | 36.5 | **27.6 ns** |
| `h13_ary_size` `s += a.size` | 40.9 | 36.4 ns |
| `h14_ruby_call` `s += r.get(j)`（Ruby のメソッド） | 43.3 | 41.2 ns |
| `h01_get_int10` `s += h[j]`（10 要素、Integer 鍵） | 44.6 | **43.9 ns** |
| `h04_set_int10` `h[j] = j`（既存の鍵） | 38.5 | 31.7 ns |
| `h06_key_int10` `s += 1 if h.key?(j)` | 54.6 | 63.9 ns |
| `h08_aref_only` `s += 1 if ks[j]`（下の 2 つの土台） | 36.3 | 27.3 ns |
| `h07_get_sym10` `s += h[ks[j]]`（Symbol 鍵） | 57.4 | 42.1 ns（土台を引く） |
| `h09_get_str10` `s += h[ks[j]]`（14 バイトの String 鍵） | 62.4 | **52.2 ns**（同上） |
| `h10_set_str10` `h[ks[j]] = j`（String 鍵、既存） | 111.9 | **151.2 ns**（同上） |
| `h11_get_obj10` `s += h[ks[j]]`（`hash`/`eql?` が Ruby） | 179.3 | **285.9 ns**（同上） |

読み方はこうなる。

* **ネイティブ呼び出しと引数の受け渡しだけで 28 ns**（`Array#[]`、引数 1 個、中身はほぼ添字 1 回）。
  Ruby のメソッド呼び出し（41 ns）と大差ない。`docs/design/optimizations.md` 5 節が
  「固定費 90 ns（ネイティブ呼び出しと引数の受け渡し）」と書いていたが、**その 90 ns の中で
  呼び出しが占めるのは 3 分の 1 弱**で、残りは Hash 自身の仕事だった（下の 1.2）。
* **Hash に固有の仕事は Integer 鍵で 16 ns**（43.9 − 27.6）。内訳は `hash_sync`、`key_hash`、
  候補探し、`key_eql`。String 鍵では 24 ns（52.2 − 27.6）で、差の 8 ns が 14 バイトの FNV。
* **鍵の型で Ruby に戻るかどうかが桁を変える**。`hash` と `eql?` を Ruby で定義した鍵は 286 ns で、
  Integer の 6.5 倍。逆に言えば、計画書が段階 2d でやろうとしていた
  「`Integer`/`Symbol`/`String` の鍵を Rust で答える早道」は **すでに入っていた**（`Vm::key_hash` と
  `Vm::key_eql`、`src/vm.rs:3529`〜`3556`）。`Value::Obj` でかつ `String` でないものだけが Ruby に戻る。
  本家の `mrb_hash_ht_hash_func` も同じ形（`MRB_TT_STRING` / `SYMBOL` / `INTEGER` / `FLOAT` だけ
  native、それ以外は `mrb_funcall_id(hash)`）なので、**この項目は着手不要**と判断した。
  `String#eql?` の再定義を見に行かない点も本家と同じ振る舞いになっている。
* **`h[k] = v` は String 鍵で 151 ns と、読みの 3 倍**。差の 99 ns は
  `Vm::hash_set`（`src/vm.rs:3591`）が毎回やっていること ―― 鍵の文字列を `to_vec()` で複製し、
  それを `str_new` でもう一度複製してヒープに置き、凍結し、さらに `hash_index` の中と
  そのあとで `key_hash` を **2 回**計算する。

### 1.2 隠れていた O(n): `hash_sync` が毎回すべての鍵を複製する

上の形のまま、**引く鍵は 0〜9 のまま**、ハッシュの要素数だけを変えて測った。
鍵 0〜9 は必ず `entries` の先頭 10 個に居るので、線形探索の距離は要素数によらず一定で、
16 要素を超えれば索引が引かれて O(1) になる。つまり**要素数に比例する分が残っていれば、
それは探索ではない**。

| 要素数 | ms | 1 回あたり | 10 要素との差 |
|---:|---:|---:|---:|
| 10 | 44.6 | 43.9 ns | — |
| 100 | 55.9 | 66.4 ns | +22 ns |
| 1000 | 206.2 | 366.9 ns | **+323 ns** |

1 要素あたり 0.32 ns できれいに伸びている。犯人は `Vm::hash_sync`（`src/vm.rs:3558`）:

```rust
let (need, keys): (bool, Vec<Value>) = match &self.heap.get(o).kind {
    ObjKind::Hash(hd) => (hd.hashes_stale(), hd.entries().iter().map(|e| e.0.get()).collect()),
    _ => (false, vec![]) };
if !need { return Ok(()); }
```

`need` が偽でも `keys` を作ってしまう。つまり **Hash の探索 1 回ごとに、全要素の鍵を `Vec` に複製している**。
段階 2c で索引を入れて探索を O(1) にしたのに `ds_hash` が −19.8% しか動かなかったのは、
探索と並んでこの O(n) が残っていたからだった。`vm_optimization_bench` の中の
`hash_ops`（5 万要素）はこれで 1 回の引きにつき 5 万要素の複製をしていることになる。

### 1.3 `items()` の複製は、この 3 本ではもう効いていない

| 切り分け | ms | 中身 |
|---|---:|---|
| `a00_loop` ループだけ（1300 × 2000） | 104.6 | — |
| `a01_push` `a.push(j)` | 220.0 | 44 ns/回 |
| `a02_aset` `a[j] = a[j] + 1` | 213.2 | 42 ns/回（ネイティブ 2 回） |
| `a03_each` `a.each { |x| s += x }` | 443.9 | 131 ns/要素 |
| `a05_dup` `a.dup`（10000 要素 × 300） | 5.0 | 16.7 µs/回 |
| `a06_eq` `a != b`（10000 要素 × 300） | 6.3 | 21 µs/回 |
| `l01_build` `(1..10000).to_a` + `dup` × 300 | 7.6 | |
| `l02_shift_push` `while !li2.empty? ; li3.push(li2.shift) ; end` | 564.7 | 188 ns/回（ネイティブ 3 回＋ループ） |
| `l03_pop_push` `until li3.empty? ; li2.push(li3.pop) ; end` | 444.7 | 148 ns/回 |
| `l04_tail` `reverse!` / `!=` / `length` × 300 | 4.8 | |

`ds_array`（899 ms）は `push` 220 + `[]=` 213 + `each` 444 でほぼ全部で、**`items()` の複製は 1 つも通らない**
（`Array#[]` は添字の早道、`[]=` は段階 2c で `ary_len` に直した、`each` は mrblib の Ruby で `self[i]`）。
`bm_so_lists`（1005 ms）も `l02` + `l03` で 1010 ms、つまり `empty?`・`push`・`shift`・`pop` の
**ネイティブ呼び出しの回数そのもの**で、複製する `dup` と `!=` は合わせて 11 ms（1%）しかない。
`app_json_hash` も配列は `["a","b","c"]` の生成と `.size` だけ。

つまり計画書の「`ds_array` と `bm_so_lists` と `app_json_hash` で `items()` の複製がどれだけ効いているか」への
答えは **「この 3 本ではもう効いていない」**。段階 2c で長さだけを見ていた 8 か所を直した時点で、
ベンチに出る複製は無くなっていた。

残っているのは**ベンチに出ない側**で、こちらには質の悪いものがある。`ext_array.rs` の
`include?` / `member?` / `__count` は、ループの**中**で `items(vm, s)` を呼び直しているので O(n²) になる:

| 切り分け | ms | 1 回あたり |
|---|---:|---:|
| `a04_include` `a.include?(j)`（10 要素、50 万回） | 79.9 | **114.4 ns** |
| （比較）`h01_get_int10` `h[j]`（10 要素） | 44.6 | 43.9 ns |

10 要素で 114 ns、100 要素なら 10 倍以上になる形。ベンチには出ないが、ホストから渡された配列を
`include?` で引くのは実アプリでは普通なので、直す価値はある（下の 3 節）。

### 1.4 `app_json_hash` は 4 分の 3 が「作る」側

| 切り分け | ms |
|---|---:|
| `j01_build` 入れ子の Hash/Array/String を作るだけ（200 行 × 2300） | 924.1 |
| `j02_walk` 作ったものを読むだけ（同じ回数） | 377.5 |
| `j03_interp` `"item-#{i}"` と `.size` だけ（46 万回） | 107.7 |
| `j04_hashlit` `{ "score" => i, "ok" => true }`（46 万回） | 170.9 |

`app_json_hash` 全体 1248 ms のうち 924 ms が構築側で、その中身は
**String 鍵の `hash_set`（1.1 の 151 ns）を 1 行あたり 6 回**払っている。
2 要素のハッシュリテラル 1 個で 371 ns。

## 2. 何をやるか（切り分けの結論）

1. **`hash_sync` の O(n)**（1.2）。これが段階 2d でいちばん大きい。
2. **`hash_set` の二度手間**（1.1）。`key_hash` を 2 回計算している。String 鍵の複製も、
   すでに在る鍵なら要らない。
3. **`items()`**（1.3）。ベンチに出る 3 本では効かないと分かったので、O(n²) になっている
   読むだけのネイティブ（`include?` / `member?` / `__count`）だけを直し、効果は
   micro ベンチで示す。ベンチ 22 本では「ぶれの範囲」になるはず。
4. **`vm_optimization_bench` の分類分け**。
