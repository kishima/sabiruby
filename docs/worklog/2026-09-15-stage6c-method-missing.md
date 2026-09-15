# 段階 6c（VM 側）: `method_missing` を呼び出し元のフレームで動かす

2026-09-15。`docs/plans/host-bridge-plan.md` 段階 6c の前提として、rubevy の担当が見つけた壁
（「実装で分かったこと」段階 6b・6c、rubevy の `docs/worklog/2026-09-15-stage6bc-futures-proxy.md`）を
VM 側で取り除いた記録。著者の判断は**案 A（本家と同じ形）**。worktree は `sabiruby-wt-mm`、
ブランチ `method-missing-dispatch`。main には触っていない。並行して動いている担当は無い
（`pgrep -af "[s]abiruby|[c]argo|[m]ruby"` が空、load 1.16）。

## 壁を自分の目で見るところから

rubevy の worklog が言っているのは「`method_missing` の中では `Rubevy.ask(...).pop` で止まれない」
である。まずそれが VM の素の性質なのか、rubevy の事情なのかを分けたかったので、rubevy を使わずに
同じ形を作って走らせた。Fiber なら rubevy が要らない:

```ruby
f = Fiber.new do
  o = Object.new
  def o.method_missing(n, *a); Fiber.yield([n, a]); :resumed; end
  o.hello(1, 2)
end
p f.resume
```

変更前のバイナリ（main の `73a1cf4`）はここで `can't cross C function boundary (FiberError)` で落ちる。
本家 mruby 4.1.0-rc（Docker の `kishima/mruby:4.1.0-rc`）に同じものを食わせると `[:hello, [1, 2]]` と
`:resumed` を返す。つまり**本家では通る**。rubevy の Task::Queue と同じ理由で、原因は VM 側にあり、
しかも本家との差でもある、とここで確かめられた。

理由は `src/vm.rs` の `op_send_vis` の `None =>` 分岐（当時 3850 行付近）にあった。見つけた
`method_missing` を `call_proc_with` で呼んでいる。これは入れ子の実行ループで、フレームには
`Cci::Skip` が付く。`fiber_check_native`（`src/vm.rs:2091`）は `ci` を上から見て `Cci::Skip` が
1 つでもあれば真を返し、`Fiber.yield` も `ext_task.rs:886` の `queue_pop_try` も、それを見て断る。
`method_missing` の本体は常にその内側にいた。

## 本家がどうしているか

`ref/mruby/src/vm.c` の `prepare_missing`（1375 行）を読んだ。OP_SEND は**探索の前に**すでに
`cipush` でフレームを積んでいて（3652 行付近）、探索が空振ったら `prepare_missing` が
**そのフレームを書き換える**。新しいフレームは作らない。やっていることは 4 つ:

1. `mrb_args_pack_positional` で位置引数を 1 本の Array に詰める。`ci->n` は `CALL_MAXARGS`（15）になり、
   キーワード Hash とブロックは `argv[1]`・`argv[2]` に寄せられる。
2. `mrb_func_basic_p(recv, missing, mrb_obj_missing)` が真、つまり `method_missing` がまだ
   BasicObject の C のものなら、そこで NoMethodError を上げる。
3. `if (mid != missing) ci->u.target_class = mrb_class(mrb, recv);` として `method_missing` を探し直す。
4. `mrb_ary_unshift(args, symbol_value(mid))` で名前を先頭に差し込み、`ci->mid = missing` にして返す。

戻った `m` は普通のディスパッチに乗る。だから本体は境界の内側ではないし、`ci->mid` が
`method_missing` なので本体の `super` は 1 つ上の `method_missing` に行く。可視性の検査は
「見つかったときだけ」（3661 行のコメント）なので、private な `method_missing` も答える。
sabiruby の `send` 再ディスパッチ（`op_send_redirect`）がすでに同じ構えなので、形は揃っている。

## 引数の詰め直しで本家と違うところ（食い違いではなく、見えない差）

指示は「メソッド名を第 1 引数に挿入し、レジスタを 1 つ後ろにずらす」だったが、本家は
**常に配列 1 本に詰める**（`n` は必ず 15 になる）。ここは最初 「食い違いを見つけたら止まれ」に
当たるかと思って止まりかけたが、`OP_ENTER` から見ると両者は同じものである。`n == 15` は
「引数は `stack[base+1]` の Array 1 本」という意味で、`OP_ENTER` はそれを展開してから
仮引数に配る。`n == 3` でレジスタに 3 個置くのと、`n == 15` で 3 要素の Array を置くのとで、
本体が受け取る値も `*args` の中身も arity のエラー文も変わらない。詰めた Array は
`mrb_args_pack_positional` がその場で新しく作ったものなので、本体から書き換えても
呼び出し側には届かない（別名にならない）という点も同じ。

確かめるために、引数 0・1・14・15・20 個を両方で走らせて出力を突き合わせた（下の「確かめたこと」）。
**一致した。**よって止まらず進めた。採ったのは指示どおりのレジスタずらしで、15 個以上に
なったときだけ配列に詰める。理由は 2 つ: `op_send_redirect` と同じ関数を使い回せること、
引数が 14 個以下（ほぼ全部）のときに Array を 1 つ作らずに済むこと。

もう 1 つ、本家と違うが**変更前からそうだった**ところ: 空のキーワード Hash。`native_args`
（`src/vm.rs:3810`）は `kw` が真でも Hash が空なら `kd` を `None` にして落とす。本家の
`prepare_missing` は `ci->kw` をそのまま持ち越すので `**{}` が空 Hash として本体に届く。
`foo(**{})` を `method_missing` で受けたときだけ差が出る。`send` の再ディスパッチも同じ扱いなので、
今回は揃えるだけにして触っていない（直すなら `native_args` の側の話になる）。

## 変更

`op_send_redirect` の後半（引数をレジスタに書き戻して `c` を作るところ）を `relay_args` として
切り出し、`method_missing` の分岐から同じものを呼ぶ。`relay_args` がするのは、位置引数を
15 個以上なら Array 1 本に、14 個以下ならレジスタに並べ、続けてキーワード Hash、続けてブロックを
置き、`c`（`n | (nk << 4)`）を返すこと。書き戻す先のレジスタの並びは `prepare_call` が読む並びと
同じでなければならないが、そこは `op_send_redirect` がすでに合わせていたので写すだけで済んだ。

`method_missing` 側は、見つけたものが `Method::Ruby` のときだけ

```rust
let c = self.relay_args(base + a, nargs, kd, blk);
return self.op_send_vis(base, a, mm, c, has_blk, false, false);
```

にした。`mm`（`method_missing` のシンボル）で送るので本体の `super` は上位の `method_missing` に行き、
`explicit` が `false` なので private でも答える。`is_super` は `false` で渡す — `mm_class` は
`is_super` でも `is_super` でなくても `class_of(recv)` なので、`op_send_vis` が再計算する
`start_class` と一致する（この等式が崩れると `super` から来た `method_missing` の探索開始点がずれるので、
`mm_class` の式は指示どおり変えていない）。

変えなかったもの: `Method::Native` の `method_missing`（BasicObject の C のもの）は今までどおり
NoMethodError を組み立てる分岐に落ちる。`Method::Closure` は `call_closure_direct` のまま
（ホストが `define_closure` で置いた `method_missing`。ネイティブなので境界があって当然）。
`funcall` 側（`src/vm.rs:1770` 付近）の分岐は本家の `mrb_funcall` も入れ子なので手を付けていない。

無限再帰の心配はしなくてよい: 再ディスパッチに入るのは `find_method(mm_class, mm)` が
`Method::Ruby` を返したときだけで、`op_send_vis` の中の `find_method_cached` は同じクラス・同じ名前を
引くので必ず同じものを見つける。本家が `goto method_missing /* just in case */` を置いているのは
探索を 2 回に分けているからで、こちらは 1 回で済んでいる。

## 確かめたこと

**本家との突き合わせ（`tests/custom/method_missing_dispatch`）。** `tools/custom.sh` で
`kishima/mruby:4.1.0-rc` の `mrbc -g` に掛け、同じ `.rb` を本家の `mruby` で走らせた出力を
`.rc.out` に取り、それをそのまま `.expected` にした。中身は、引数 0/1/14/15/20 個、
ブロック付き、15 個＋ブロック、キーワード（位置と混在、`**h`、15 個＋キーワード）、
`respond_to_missing?` と `method(:x).call`、3 段の `super`、private な `method_missing`、
`send` で直接呼ぶ、C の `method_missing` が出す NoMethodError の `message`/`name`/`args`、
本体から上がる例外とその `backtrace`、本体が `yield` したブロックからの `raise`、
`Fiber.yield`（1 段と 2 段）、`return`、呼び出し元のブロックからの `break`、`__method__`、
`method_missing` を経由した無限再帰が SystemStackError で止まること。
**29 行、1 行も違わずに一致した**（`cargo test --test custom`: 9 passed, 0 problems）。

面白かったのは、この 29 行のうち変更前の VM でも合っていたのが 22 行あったことである。
引数の渡し方も `super` も private も backtrace も `break` も `__method__` も、入れ子の実行ループで
すでに正しかった。変更前のバイナリでこのケースを走らせると、22 行目まで一致して 23 行目の
`Fiber.yield` で `can't cross C function boundary (FiberError)` と言って死ぬ。
**この変更が変えたのは「本体の中で止まれるかどうか」だけ**で、それ以外の見え方は動いていない。

**mruby-task（`tests/task.rs` に 2 本）。** 本家の `mruby` には mruby-task が無いので Rust 側に置いた。
1 本目 `a_task_parks_on_a_queue_from_inside_method_missing` は rubevy の `Rubevy::Proxy` をそのまま
小さくしたもので、`method_missing` が `"#{@kind}.#{name}"` を記録して `$queue.pop` で止まる。
ホストが 2 回答えを push する間に隣のタスクが 4 周し、答えが呼び出しの値として返る。
2 本目 `a_task_sleeps_from_inside_method_missing` は `sleep 0` で、順番が
`[[:before, :nap], :b, [:after, :nap], [:done, :nap]]` になる（`sleep 0` で本当に turn を手放している）。
どちらも変更前は `blocking pop cannot be called from within a C function boundary` になる形である。

**本家テスト。** `tests/mrbtest/baseline.txt` の 108 ファイルを 1 つずつ
`./target/release/sabiruby mrbtest tests/mrbtest/assert.mrb tests/mrbtest/prelude.mrb tests/mrbtest/<name>.mrb`
で回して passed 列を比べた。変更前 **108 ファイル 0 差**、変更後も **108 ファイル 0 差**。
`--no-default-features`（バイト文字列）でも `baseline-bytes.txt` に対して **108 ファイル 0 差**。
名指しで見るよう言われていた 4 つ: `kernel 37/37`、`basicobject 2/2`、`gem_metaprog 33/33`、`gem_task 43/43`。
（`gem_ascii_*` は既定ビルドの baseline に載っていない — 載っているのは `gem_unicode_*` の方で、
除外の向きは `tools/mrbtest.sh` の `OTHER_BUILD` と逆だった。どちらのビルドも自分の baseline の
108 行を全部走らせている。）

**その他。** `cargo test --workspace` 全部 ok（新しい 2 本を含む task は 20 passed）。
`tools/check_no_std.sh` OK。`grep -rn unsafe src` は 0 行。`cargo doc --no-deps` 警告なし
（`--workspace` を付けると lib と cli で `sabiruby/index.html` が衝突するという警告が 1 件出るが、
これは変更前から出ている既存のもの）。`cargo clippy --all-targets` の警告は **96 件で、main と同数**。

## ベンチ

通常の呼び出しに影響が無いことの確認。触ったのは探索が空振ったときだけ通る道なので、
本来どのベンチも動かないはずである。`tools/bench_ab.sh --core 2 --runs 5` で main のバイナリと
交互 A/B（`bench/results/6314b84-method-missing.tsv`）:

| 分類 | A ms | B ms | 変化 |
|---|---:|---:|---:|
| whole program | 20102 | 20053 | −0.2% |
| data structures | 3848 | 3718 | −3.4% |
| instruction loop | 9885 | 9914 | +0.3% |
| calls | 6710 | 6607 | −1.5% |
| memory | 2084 | 2048 | −1.7% |
| **all** | 42630 | 42340 | **−0.7%** |

名指しの 2 本は `call_args` +0.9%、`bm_fib` −0.1%。これだけだと「速くなった」ように見える
（`bm_so_lists` −6.4%、`ds_array` −3.2% など、触っていないはずのものが動いている）ので、
コード配置の揺れ（段階 2 の worklog の「9000 行の `exec_frames` がキャッシュラインのどこに乗るか」）
だろうと見て、名指しの 2 本だけ `--runs 15` で取り直した:
`call_args` **+1.1%**、`bm_fib` **+0.7%**（`bench/results/6314b84-method-missing-calls.tsv`、
`...-fib.tsv`）。同じ変更・同じ機械で `bm_fib` が −0.1% と +0.7% の間を動くので、
**±1% はこの機械の地の揺れ**であり、全体が −0.7% であることと合わせて「ぶれの範囲、系統的な差は無い」
と読む。best と median の差は 25 本中 23 本で 1% 未満だった（`mem_retained` の B が 2.4%、
`app_robot` の B が 1.3%。どちらも今回の道とは無関係で、取り直した 2 本には影響しない）。

## 捨てた案

* **`op_send_vis` を呼び直さず、その場で `CallInfo` を積む。** 探索を 2 回しないぶん速いが、
  `prepare_call` が読むレジスタの並びと `relay_args` が書く並びが一致しているかを目で守ることになる。
  `op_send_redirect` がすでに「書き戻して `op_send_vis` に戻す」形で通っているので、同じ道を通す方を採った。
  空振りの後の 1 回の探索は、そもそも滅多に来ない道の話である。
* **本家と同じく常に Array に詰める。** 上のとおり `OP_ENTER` から見て同じで、14 個以下のときに
  Array を 1 つ余分に作る。指示のレジスタずらしを採った。
* **`funcall` 側も同じにする。** `mrb_funcall` が入れ子なので本家と違う振る舞いになる。指示どおり触っていない。
