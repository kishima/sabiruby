# `OP_GETIDX` / `OP_GETIDX0` / `OP_SETIDX` を本家と同じ形にする

2026-09-15（作業は日をまたいで 16 日未明まで）。rubevy の `docs/plans/ecs-bridge-plan.md` 段階 A で
`e[:Transform]` が使えなかった件を、VM 側で取り除いた記録。著者の判断は段階 6c と同じ**本家と同じ形**。
worktree は `sabiruby-wt-idx`、ブランチ `getidx-dispatch`。main には触っていない。
並行して動いている担当は無い（`pgrep -af "[s]abiruby|[c]argo|[m]ruby"` が空、load 0.05）。
Docker は動いていたので本家との突き合わせにも使った。

## 壁

`src/vm.rs` の 3 命令は、どれも `self.funcall(recv, aref, …)` だった。

```rust
Op::Getidx => {
    let recv = reg!(a); let idx = reg!(a + 1);
    let v = self.funcall(recv, self.s.aref, &[idx], Value::Nil)?;
    setreg!(a, v);
}
```

`funcall` は入れ子の実行ループで、積むフレームには `Cci::Skip` が付く。`fiber_check_native` は
`ci` を上から見て `Cci::Skip` が 1 つでもあれば真を返すので、Ruby で書いた `[]` の本体は
**常に**境界の内側にいた。段階 6c で `method_missing` について直したのと同じ壁が、`[]` に残っていた
ことになる。まず rubevy を使わずに同じ形を作って確かめた:

```ruby
class Box
  def [](k); Fiber.yield([:get, k]); end
end
f = Fiber.new { Box.new[:a] }
p f.resume
```

main の `0d0808e` では `can't cross C function boundary (FiberError)`。
本家（Docker の `kishima/mruby:4.1.0-rc`）では `[:get, :a]` を返す。VM 側の、しかも本家との差である。

## 本家がどうしているか — 早道と、その早道が効いてよい条件

`ref/mruby/src/vm.c` の `vm_op_getidx`（2707 行）、`vm_op_getidx0`（2783 行）、`vm_op_setidx`（2841 行）と、
`src/class.c` の 2855〜2965 行を読んだ。命令の本体は 3 行しかない:

```c
CASE(OP_GETIDX, B) {
  int r = vm_op_getidx(mrb, a, &mid);
  ci = mrb->c->ci;
  if (r == VM_SEND_SYM) goto L_SEND_SYM;
  NEXT;
}
```

早道が答えるのは 3 つ（`[]=` も同じ 3 つ）:

* **Array** + **Integer** 添字。それ以外の添字（Range、String、Float）は送る。
* **Hash**。添字の型は問わない（`mrb_hash_get` が `default` / `default_proc` まで面倒を見る）。
* **String** + **Integer / String / Range** 添字。Regexp はここに入らないので送られる。

`[]=` は Array が Integer 添字のときだけ、String は**右辺が String のときだけ**答える。右辺が String で
ないのは「代入ではなく TypeError」であり、例外を上げるのはメソッドの仕事だから送る、という理屈が
本家のコメントに書いてある（backtrace に `String#[]=` のフレームが残る）。

**効いてよい条件**が今回の肝である。本家は `mrb->idx_class[]` という 6 本のスロットを持ち、
「そのクラスが、命令が肩代わりしている実装をまだ持っている間だけ」クラスを入れ、そうでなければ NULL を入れる。
命令は受け手のクラスをスロットと比べるだけなので、**この 1 回の比較が 3 つのことを同時に断る**:

1. サブクラス（`class MyA < Array` のインスタンスはクラスが `MyA` なのでスロットと一致しない）
2. 特異メソッド（`def a.[]` は特異クラスを挟むので、やはり一致しない）
3. `Array#[]` そのものの再定義（スロットが NULL になっている）

3 つ目の判定の作り方が丁寧だった。`idx_op_refresh` が見るのは「`[]` に代入されたか」ではなく
**解決した先のメソッドそのもの**で、起動時に記録した `mrb_method_t` と一致するかを比べる。だから
`def`・`alias_method`・`undef_method`・`remove_method`・可視性の変更・`prepend` を列挙しなくても全部拾えるし、
上書きを `alias` で外したら**また armed に戻る**。無関係な module の `include` では外れない。
`mrb_idx_op_update` は `mrb_define_method_raw`（class.c:1092）と `remove_method`（3957 行）から呼ばれる。

もう 1 つ、`mrb_idx_op_rearm` という抜け道がある。mruby-regexp は `String#[]` の名前を取って
Regexp を受けられるように広げるが、Integer / String / Range のときは結局同じ `mrb_str_aref` を呼ぶので、
「命令が答える形については同じ答えを出す」という約束を満たせる。だから gem の init で
`mrb_idx_op_rearm(MRB_IDX_OP_STR_AREF)` を呼んで、スロットを armed に戻している（regexp.c:3626）。

**本家と食い違う点は見つからなかった**ので、止まらずに進めた。

## Rust でどう書いたか

### スロットの作り

本家は「メソッド表が変わるたびに 6 スロットを引き直す」。sabiruby には
`Heap::method_serial`（`heap.class_mut()` を通る変更で必ず 1 進む、メソッドキャッシュの世代番号）が
すでにあるので、**引き直すのを遅らせた**: `idx_serial` に「6 スロットを計算したときの serial」を持ち、
命令が来たときに `self.idx_serial != self.heap.method_serial` なら `idx_sync()` で 6 本まとめて引き直す。

```rust
#[inline]
fn idx_armed(&mut self, slot: usize, cls: ObjId) -> bool {
    if self.idx_serial != self.heap.method_serial { self.idx_sync(); }
    self.idx_class[slot] == Some(cls)
}
```

常の道は「u64 を 1 つ比べる」+「`Option<ObjId>` を 1 つ比べる」で、本家の 1 回のポインタ比較より
1 つ多いだけである。逆に `def` のたびに 6 回の探索をする本家より、そちらは安い。`idx_sync` には
`#[cold]` と `#[inline(never)]` を付けて、命令のコードから追い出した。

記録する側（`idx_op_init` / `idx_op_rearm`）は本家の `idx_op_arm` と同じで、**素のネイティブのときだけ**
記録する（本家の `MRB_METHOD_FUNC_P(m)` に当たる）。比較は関数ポインタの `core::ptr::fn_addr_eq`。
`MethodRef::Closure` は `Arc` 越しで番地で比べられないので、記録しない = そのスロットは永久に disarmed に
なる。ホストが `define_closure` で `Array#[]` を置いた場合がそれで、送るだけなので正しい。

`idx_op_init` は `builtins::init` の最後（本家が `mrb_open_core` の最後で `mrb_idx_op_init` を呼ぶのと同じ位置）。
`idx_op_rearm` は `ext_regexp::init` が `String#[]` と `String#[]=` の名前を取った直後に 2 回。
mrblib はそのあとに読むが、serial が進むので次の命令で自動的に引き直される — 初期化の順番を気にしなくてよい
のが、遅らせたことのもう 1 つの効き目だった。

### 送る側

`prepare_call`（`src/vm.rs:3795`）を読んだら、`c` と `has_blk` から block のレジスタを自分で計算して
**nil を書き、足りなければ stack を伸ばす**ところまでやっていた。本家の `L_SEND_SYM` が
`SET_NIL_VALUE(regs[a+2])` を手で書いているのと同じ仕事である。だから `OP_GETIDX` の落ち道は

```rust
Op::Getidx => {
    if self.op_getidx(base, a)? {
        self.op_send_vis(base, a, self.s.aref, 1, false, false, false)?;
        if let Some(v) = self.loop_exit.take() { return Ok(v); }
    }
}
```

だけで済んだ。レジスタは既に「受け手が `R[a]`、添字が `R[a+1]`」で、これは引数 1 個の呼び出しの並びそのもの
なので、何も動かさなくてよい。`OP_GETIDX0` だけは `R[a]` に受け手、`R[a+1]` に 0 を書いてから送る
（本家も `vm_op_getidx0` の `getidx0_fallback:` でそうしている）。

`explicit` を `false` で渡すところは 6c と同じ理屈で、本家も**見つかったときだけ**、しかも
`insn == OP_SEND | OP_SEND0 | OP_SENDB` のときだけ可視性を見る（vm.c:3663）。`OP_GETIDX` から来た送信は
その 3 つではないので、private な `[]` も答える。

### 早道の中身

Array + Integer だけ、本家と同じくその場で要素を取る（`ary_entry`、`mrb_ary_entry` に当たる）。
Hash と String は既にあるネイティブ（`hash::hash_aref`、`string::str_aref` …）を
`call_native` 経由で直に呼ぶ。本家が `mrb_hash_get` / `mrb_str_aref` を直に呼ぶのと同じ構えで、
省けるのはメソッド探索とフレームである。`call_native` を挟んだのは `native_active` の増減のためで、
これが 0 でない間 GC は先送りされる（`gc_maybe`）。変更前の `funcall` 経由と同じ状態を保ちたかった。

Hash の `[]=` と String の `[]=` はメソッド表にクロージャのリテラルで登録されていたので、
`hash_aset` / `str_aset` という名前付きの関数に切り出した。命令が「肩代わりしている実装」として
記録するのも、直に呼ぶのも、この関数である。`array::ary_aset` は `pub(crate)` にしただけ。

`Array#[]` だけ既存のネイティブを呼ばなかったのには理由がある。`array::ary_aref` は入口で
`a.iter().map(...).collect::<Vec<_>>()`（Float の添字を Int に直すため）をしていて、**添字 1 個の
読みごとに Vec を 1 つ確保している**。早道で一番効かせたいのがそこなので、本家と同じく要素を直に取った。
`ary_aref` 自身のこの割り当ては今回の範囲外なので触っていない（報告に書く）。

## 確かめたこと

### 本家との突き合わせ（`tests/custom/getidx_dispatch`）

`tools/custom.sh` で `kishima/mruby:4.1.0-rc` の `mrbc -g` に掛け、同じ `.rb` を本家の `mruby` で
走らせた出力を `.rc.out` に取り、それを `.expected` にした。中身は、Ruby で書いた `[]`/`[]=` の中の
`Fiber.yield`（Fiber を 4 回 resume して往復を確かめる）、Array のサブクラスからの `super`、
`[]`/`[]=` の中から上がる例外、Array/Hash/String のサブクラスの `[]` と、再定義していないサブクラス、
特異メソッドの `[]`/`[]=`、`x[0]`（`GETIDX0`）を上の全部について、Range・String・負数・範囲外の添字、
`Hash#default` と `default_proc`（`h[k] = ...` を副作用に持つ proc も）、`String#[]` の文字列添字と
Regexp 添字、`[]=` の値が右辺であること、凍結した受け手と型違いの右辺が上げる例外、Proc と Struct と
Integer の `[]`、`method_missing` が受ける `[]`、そして `Array#[]` の再定義 → `alias` で戻す →
`prepend` という armed / disarmed の往復。**81 行、1 行も違わずに一致した**
（`cargo test --release --test custom`: `custom: 10 passed, 0 pending, 0 problems`）。

途中で 1 回はまったので書いておく。最初は再定義の節も `p` で書いていて、
`class Array; def [](i); "redef #{i}"; end; end` のあと `p x[2]` が本家で **`"redef 0"`** になった。
早道の取りこぼしを疑って小さくしていったら、`v = x[2]; puts v` は `redef 2`、`p v` は `"redef 0"` で、
**同じオブジェクトが `puts` と `p` で違って見えた**。原因は命令ではなく `p` の方で、
mruby-print の `Kernel#p` は Ruby で書かれていて自分の引数を `args[i]` で読む。`Array#[]` を再定義した
あとなので、`p` が自分の引数配列に対して再定義された `[]` を呼び、`"redef 0"` を印字していたのだった。
`prepend` の節で `"loud:loud:2"` と 2 重に付いていたのも同じ理由である。sabiruby の `p` は
ネイティブ（`kernel.rs:21`）なのでここは元から本家と違う（今回の変更とは無関係の既存の差）。
再定義の節は `puts` に書き換えて、その理由をテストのコメントに残した。

### mruby-task（`tests/task.rs` に 2 本）

`a_task_parks_on_a_queue_from_inside_a_ruby_aref` は rubevy の `Entity` をそのまま小さくしたもので、
`[]` と `[]=` が `$queue.pop` で止まる。ホストが 2 回答えを push する間に隣のタスクが 4 周し、
答えが `e[:Transform]` の値として返り、`e[:Health] = 3` の値は（`[]=` が返した 71 ではなく）右辺の 3 になる。
`a_task_sleeps_from_inside_a_ruby_aref` は `Slow.new[0]`（`GETIDX0` の落ち道）の中で `sleep 0`。
どちらも変更前は `blocking pop cannot be called from within a C function boundary` になる形である。

### 本家テスト

`tests/mrbtest/baseline.txt` の 108 ファイルを 1 つずつ
`./target/release/sabiruby mrbtest tests/mrbtest/assert.mrb tests/mrbtest/prelude.mrb tests/mrbtest/<name>.mrb < /dev/null`
で回して passed 列を比べた。**108 ファイル 0 差**。`--no-default-features`（バイト文字列）でも
`baseline-bytes.txt` に対して **108 ファイル 0 差**。`gem_ascii_*` は既定ビルドの baseline に載っていない
（載っているのは `gem_unicode_*` の方）ので、どちらのビルドも自分の baseline の 108 行を全部走らせている。

### その他

`cargo test --workspace` 全部 ok（task は 22 passed）。`tools/check_no_std.sh` OK。
`grep -rn unsafe src` は 0 行。`cargo doc --no-deps` 警告なし。`cargo clippy --all-targets` の警告は
**96 件で、main と同数**。

## ベンチ

`tools/bench_ab.sh --a <main のバイナリ> --b <この枝> --core 2 --runs 5` で交互 A/B。A は main の
`0d0808e` から建てたもの（作業中に main へ `f691f5d` まで 2 つ commit が入ったが、どちらも
`docs/plans/` だけで src は動いていないので、A は建て直していない）。同じ形で 2 回取った
（`bench/results/ad54ed4-getidx.tsv` と `...-confirm.tsv`）。

| 分類 | 1 回目 | 2 回目 |
|---|---:|---:|
| whole program | +0.0% | +0.1% |
| data structures | **−3.6%** | **−3.7%** |
| instruction loop | +0.1% | +0.4% |
| calls | +1.9% | +1.8% |
| memory | +0.4% | +0.5% |
| **all** | **+0.1%** | **+0.1%** |

名指しで見るよう言われていた 5 本（1 回目 / 2 回目）:

| ベンチ | 1 回目 | 2 回目 |
|---|---:|---:|
| `ds_array` | −4.2% | −4.1% |
| `ds_hash` | −9.2% | −8.0% |
| `ds_string` | −4.8% | −3.5% |
| `vmo_index` | −7.0% | −7.3% |
| `app_json_hash` | −1.8% | −0.8% |

**早道は効いている。** 1 回目と 2 回目で同じ向き・同じ程度に出ているので、ぶれではない。
`vmo_index` が一番大きいのは、そこが `a[i]` と `h[k]` だけを回すベンチだからである。
`ds_string` が効くのは、mruby-regexp が `String#[]` の名前を持っているせいで、変更前は
`ext_regexp::str_aref` → `is_regexp` → `vm.intern("__aref")`（文字列からシンボルを引く）→ `funcall`
と 2 段で回っていたのが、命令が直に `string::str_aref` を呼ぶようになったため。

**遅くなったところと、その切り分け。** `call_kwargs` が 2 回とも **+7.7〜+8.0%** だった。
ここは `[]` を 1 回も呼ばないベンチなので、命令の変更そのものが効いているはずがない。3 段で切り分けた。

1. **構造体だけ変えたもの**（`Vm` に 3 つのフィールドと 6 スロットの仕組みを足し、3 命令は元の
   `funcall` のまま）を建てて測ったら `call_kwargs` **−0.2%**、`ds_hash` +1.9%。
   つまり `Vm` が大きくなったせいではない。
2. **命令の腕（match の枝）の書き方だけ変えた 2 つの候補**を測った。腕を 1 行の呼び出しにして
   `Option<Value>` を返す形（候補 B、`#[inline(never)]` 付き）と、同じで属性なし（候補 C）:

   | | `call_kwargs` | `ds_hash` | `vmo_index` | `loop_while_add` | `bm_so_mandelbrot` | all |
   |---|---:|---:|---:|---:|---:|---:|
   | 採用（A） | +7.7% | −8.0% | −7.3% | −0.2% | +0.5% | **+0.1%** |
   | 候補 B | +1.9% | −5.3% | −5.7% | **+12.6%** | **+21.9%** | +2.4% |
   | 候補 C | +1.5% | −5.5% | −7.4% | **+11.7%** | **+19.6%** | +1.7% |

   `call_kwargs` は候補 B・C で直るが、代わりに `[]` を一切呼ばない `loop_while_add` と
   `bm_so_mandelbrot` が 12〜22% 遅くなる。**動いているのはコード配置**で、`docs/design/optimizations.md`
   4 節が「呼び出しを含まないベンチは候補ごとに ±4〜12% 動く、9000 行の `exec_frames` の
   ループ本体がキャッシュラインのどこに乗るかの揺れ」と書いているものそのものである。
   3 つの候補は意味の上では同じで、違うのは腕の書き方だけなのに、どのベンチが当たり外れを引くかが
   入れ替わる。4 節の指示どおり**分類の合計**で読むと、採用した形が +0.1% で一番良い。
3. 念のため採用した形を**同じ手順でもう 1 回**取り、1 回目と 2 回目が合うことを確かめた（上の表）。

採ったのは、本家と同じ構え（早道の判定は `vm_op_getidx` に当たる関数、落ちたら `L_SEND_SYM` に当たる
送信）で、かつ合計が一番良い A である。B・C を捨てたのは `call_kwargs` を直すために
`loop_while_add` を 12% 犠牲にする取引だからで、どちらも本質的な改善ではない。
候補 B の通しは `bench/results/getidx-dispatch-candidate-b.tsv` に残した。

best と median の差は、採用した形の 2 回目で 25 本中 24 本が 1% 未満（`mem_short_lived` の A が
2.4%。今回の道とは無関係）。並行して動いている担当は無く、計測中に `cargo build` も `grep -r` も
回していない。

## 捨てた案・やらなかったこと

* **`Array#[]` のネイティブ（`ary_aref`）の Vec 割り当てを直す。** 早道が通る形（Integer 添字）は
  命令が答えるようになったので、残るのは Float 添字と 2 引数の `a[i, n]` と明示的な `a.[](i)` である。
  今回の指示の範囲外なので触っていない。
* **6 スロットを本家と同じく「メソッド表が変わるたび」に引き直す。** `Heap::method_serial` が
  すでにあるので、命令が来たときに遅れて引き直す方が、`def` の側に費用を置かずに済む。
* **`funcall` 側も同じにする。** `mrb_funcall` が入れ子なので、変えると本家と違う振る舞いになる。
  ホストが `vm.funcall(h, "[]", …)` で呼んだときは境界が残る。本家と同じ。
* **スロットを `Closure` にも広げる。** `Arc` は番地で比べられないので「同じ実装か」を言えない。
  本家も C 関数以外は記録しない。
