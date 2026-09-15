# 2026-09-16 小さな残り（`leftovers-plan.md` の 1・2・3・4・8・9）

各段階が「判断待ち」「範囲外」として置いていった小さな項目をまとめて片づけた記録。
5・6（ベンチが要る）、7（rubevy 側）、10（著者判断待ち）は触っていない。

---

## 1. `foo(**{})` — 空のキーワード Hash

### 何が落ちていたか

`src/vm.rs` の `native_args` は、フレームが持つキーワード Hash を「空なら無かったことに」していた。

```rust
let empty = match h.obj().map(|o| &self.heap.get(o).kind) { Some(ObjKind::Hash(hd)) => hd.is_empty(), _ => true };
if !empty { args.push(h); kd = Some(h); }
```

本家の `mrb_get_args`（`src/class.c:1553` あたり）も同じ判定をするが、**捨てる対象が違う**。本家は

```c
if (!reqkarg && ci->kw) {
  kdict = ci->stack[mrb_ci_bidx(ci)-1];
  if (mrb_hash_p(kdict) && mrb_hash_size(mrb, kdict) > 0) {
    ...  ci->n++;  /* 位置引数の最後に足す */
    ci->kw = FALSE;
  }
}
```

で、空でないときだけ位置引数に畳み込んで `ci->kw` を下ろす。**空のときは畳み込まないが `ci->kw` は立ったまま**で、Hash もスタックに残る。
SabiRuby は「畳み込まない」と「無かったことにする」を同一視していた。

これが見えるのは `OP_ENTER` の速い経路（`src/vm.c` の `vm_op_enter`）で、必須引数だけのメソッドは

```c
if (mrb_unlikely(argc+ci->kw != m1)) { argnum_error(mrb, m1); ... }
```

と **`ci->kw` を引数 1 個として数える**。つまり本家では `def one(x); end; one(**{})` が `x == {}` になる（CRuby は ArgumentError なので、これは mruby の方言）。
SabiRuby の `op_enter` はこの行をすでに写していたので、バイトコードから直接呼ぶ `one(**{})` は通っていた。落ちていたのは**フレームを書き換えて配り直す経路**、すなわち `send` と `method_missing` の 2 つ。

確かめた入力と出力（`tests/custom/empty_keyword_hash.rb` の原型）:

```
$ docker run … kishima/mruby:4.1.0-rc mruby kw3.rb   # def one(x); p x; end / send(:one, **{})
{}
$ ./target/release/sabiruby kw3.mrb
wrong number of arguments (given 0, expected 1) (ArgumentError)
```

### 直し方

`native_args` を「位置引数」と「フレームが持つキーワード Hash（空でも `Some`）」を返すものにし、
「ネイティブが見る引数」（＝空でないときだけ末尾に足したもの）は `native_call_args` に分けた。
`pending_kw` は `native_call_args` の側だけが立てる。SabiRuby のネイティブは
「`pending_kw` が引数列の最後と同一なら、それがキーワード Hash」という見分け方をしているので、
引数列に入っていない空の Hash をそこへ入れると `foo(h, **h)`（`h` が空）で誤判定する。
本家は書式文字列の `:` で区別するが、SabiRuby にその概念はない。**残った差分**は、
`:` 付きの C 関数に相当するネイティブが空のキーワード Hash を「キーワードあり」として見られないこと。
今あるネイティブ（`Struct.new(keyword_init:)`、`Data.define`、`Random.new`、`Task` 系）はどれも
空のキーワード Hash とキーワード無しを同じに扱うので、観測できる差は無い。

### 捨てた案と、途中で見つけたもの

最初は `send` の再配布（`relay_args`）に無条件で `kd` を渡しただけで済むと思ったが、
**`method_missing` と `send` で本家のフレームの作り方が違う**ことが `tests/custom` の期待値作りで出た。

* `send_method`（`src/vm.c:1688`）はレジスタを 1 つ下へずらすだけで `ci->n` はそのまま。`n == 15`（引数を Array 1 本に詰めた形）なら `mrb_ary_subseq` して **15 のまま**。
* `prepare_missing`（同 `src/vm.c`）は `mrb_args_pack_positional` を通るので、**引数が何個でも `ci->n = CALL_MAXARGS`（15）** にしてから `mid` を unshift する。

`docs/worklog/2026-09-15-stage6c-method-missing.md` は「引数を Array 1 本に詰めるのと、レジスタを 1 つずらすのは `OP_ENTER` から見れば同じこと」と書いているが、
**キーワード Hash があるときだけ同じではない**。速い経路は `argc < 15` のときしか通らないので、詰めた形（15）ではそもそも `ci->kw` が数えられない。
本家で

```ruby
class Positional; def method_missing(name, x); p [name, x]; end; end
Positional.new.zap(**{})
```

が `[:zap, {}]` ではなく `wrong number of arguments (given 1, expected 2)` になるのはこのため。
そこで `relay_args` に `pack` を足し、`method_missing` は常に詰める／`send` は呼ばれた形を保つ（`argc == 15` なら詰めたまま）ようにした。

`send` 側で `argc == 15` を保つのを忘れて `cargo test` が 4 本落ちた（`enumerator`、`gc`、`objects`、`utf8`）。
原因は `Enumerator#__enumerator_block_call` の

```ruby
@obj.__send__ @meth, *@args, **@kwd, &block
```

で、`@kwd` は `initialize` が `**kwd` で受けた**空の Hash**、`*@args` があるので呼び出しは `n=15`。
本家では 15 のまま `Array#each` に入るため `OP_ENTER` は遅い経路を通り、空の kdict は捨てられて何も起きない。
ほどいて `n=0` にすると速い経路に入って `ci->kw` が 1 個に数えられ、`wrong number of arguments (given 1, expected 0)` になる。
「引数の詰め方は表現の違いにすぎない」という思い込みが、この 1 行で 2 回破れたことになる。

### 残っている差

`public_send` はネイティブのまま `vm.funcall` へ落ちるので、空のキーワード Hash を運べない
（`Pub.new.public_send(:one, **{})` は本家が `{}`、SabiRuby は ArgumentError）。
本家の `public_send` は `send_method(mrb, self, TRUE)` で `send` と同じフレーム書き換えなので、
`op_send_redirect` に可視性の判定を足せば揃う。今回の計画書の範囲（`send` と `method_missing`）の外なので触っていない。
