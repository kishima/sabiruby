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

---

## 2. マクロの生成する `register` に残っていた `expect`

段階 6a の worklog（`2026-09-15-stage6a-macros.md` の 204 行目）が
「`register` を `VmResult` にすれば消えるが、計画書の `T::register(&mut vm)` の形から離れるので触っていない」
と書き残した 1 つ。生成コードは

```rust
let __meta = vm.singleton_class(::sabiruby::Value::Obj(__class)).expect("a class has a metaclass");
```

を書いていた。到達不能ではある（`__class` は直前に `register_class` が作ったクラスオブジェクトで、
`Vm::singleton_class` がクラスに対して `Err` を返す道は無い）。だが **`expect` はホストの選択であって、
マクロが勝手に決めてよいものではない**。生成されたコードのパニックは、ホストから見ると自分が書いていない行で落ちる。

`pub fn register(vm: &mut Vm) -> ::sabiruby::error::VmResult<::sabiruby::value::ObjId>` にして `?` に置き換えた。
`__meta` はクラスメソッドが 1 つでもあるときだけ生成されるので、クラスメソッドが無い `impl` の `register` は
`Err` を返す道が 1 本も無い `VmResult` になる。これは目をつぶった（形を揃えるほうが使う側に説明しやすい）。

破壊的変更だが、この repo の中の利用者は `macros/tests/player.rs` の 3 か所だけ。
`rubevy` は `#[ruby_methods]` をまだ使っていない（`grep -rn "ruby_methods\|RubyClass" rubevy/src` が空、
クラス登録は `src/lib.rs` で手書き）ので、**rubevy 側に直す呼び出しは無い**。
ただし rubevy `src/lib.rs:1215` に同じ形の `vm.singleton_class(Value::Obj(m)).expect("Rubevy singleton")` が手書きで 1 つある。
これは rubevy のホストコード自身が書いた `expect` なので、今回の変更とは別（直すなら rubevy 側の判断）。

固定した期待値（`macros/tests/expand.rs`）と、`macros/README.md`・`macros/src/lib.rs`・`docs/design/macros.md` の例も直した。

---

## 3. `allocate` で作った素のオブジェクトの文言

`Player.allocate` は `initialize` を走らせずにオブジェクトだけ作るので、
`#[derive(RubyClass)]` の店（`HostStore`）には何も入っていない。そこへ `hp` を呼ぶと
`RubyClass::handle_of`（`src/host_store.rs`）が

```
wrong argument type Player (expected Player)
```

を返していた。同じ名前が 2 回出てくるので、**VM の不具合に見える**。ホストが知りたいのは
「この `Player` には Rust の実体が無い」であって「型が違う」ではない。

本家に同じ場所がある。`mrb_data_check_type`（`src/etc.c:35`）は `DATA_TYPE(obj)` を見て、
別の型なら `wrong argument type %s (expected %s)`、**NULL なら `uninitialized %t (expected %s)`** を投げる。
`%t` はオブジェクト自身のクラス名なので、部分クラスなら部分クラスの名前が出る。確かめた:

```
$ docker run … mruby -e 'begin; Time.allocate.to_i; rescue=>e; p e.message; end'
"uninitialized Time (expected Time)"
$ … 'class MyTime < Time; end; begin; MyTime.allocate.to_i; rescue=>e; p e.message; end'
"uninitialized MyTime (expected Time)"
```

これに合わせて `handle_of` を 3 分岐にした。別のタグの `Data` は今まで通り `wrong argument type`、
`Data` ですらないが**このクラス（か部分クラス）のインスタンス**なら `uninitialized …`、
それ以外（String に `instance_exec` で届いた、など）は `wrong argument type`。

`convert.rs` の `DataRef::from_ruby`（`This<DataRef>` で手書きの `define_fn` が使うほう）は触っていない。
あちらは「期待するクラス」を持っていないので、素のオブジェクトと本当に間違った引数を区別できない。
本家も引数として渡された非 `T_CDATA` は `mrb_check_type` 経由で `wrong argument type Object (expected Data)` と言うだけで、
SabiRuby の今の文言と一致している（`tests/data.rs` がその 2 行を固定している）。
読めない文言だったのは、クラスを知っている `handle_of` の側だけだった。

テストは `macros/tests/player.rs` の
`a_receiver_that_is_not_one_of_ours_is_a_type_error_naming_the_class` を書き換え、
`Ghost.allocate`（部分クラス）・`Player.allocate`・String（クラスが違う側）の 3 つを固定した。

---

## 4. ブロックを取るメソッド（`#[ruby_methods]`）

`Vm::define_fn` には最初から形がある（`src/convert.rs` の `MVmBlock` と `MVmThisBlock`:
`Fn(&mut Vm, A…, Block)` と `Fn(&mut Vm, This<S>, A…, Block)`）。マクロが読んでいなかっただけ。

`method_spec` に「末尾が `Block` なら引数ではなくブロック」を足した。`&mut Vm` の判定（`is_vm`）と同じで、
**綴りだけを見る**（`Block`、`convert::Block`、`sabiruby::convert::Block` のどれでも）。
型解決は proc-macro の手に無いので、これは段階 6a が `&mut Vm` で決めた妥協をそのまま広げたもの。

`define_fn` には `&mut Vm` 無しでブロックを取る形が無い。ブロックを呼ぶには `Vm::call_block` が要るので
実質的にも困らない。`Block` だけ書いて `&mut Vm` を書かなかったら、そう言うコンパイルエラーにした。
`Block` が末尾以外にあるときも同じ（`&mut Vm` が先頭以外にあるときと対になる）。

ブロックは**引数に数えない**。`Method#arity` は `define_fn` が数えるので、
`each_hit(&mut self, vm, times: i64, blk: Block)` の arity は 1。生成される `__ruby_argument_types`
（`FromRuby` を引数の綴りの位置で要求するやつ）にも `Block` は入らない（`Block` は `FromRuby` ではない）。

テストは `macros/tests/player.rs` に `each_hit`（インスタンス、`&mut self` + 引数 + ブロック）と
`from_block`（クラスメソッド、引数なしブロックだけ）を足し、Ruby から

```ruby
p pl.each_hit(3) { |left| seen << left }   #=> 7
p Player.instance_method(:each_hit).arity  #=> 1
begin; pl.each_hit(1); rescue ArgumentError => e; p e.message; end  #=> "no block given"
pl.each_hit(1) { pl.hp }  #=> RuntimeError: Player is already in use by a call on the same object
```

まで固定した。最後の 1 行は狙って入れている。`&mut Vm` を取るメソッドは受け手を店から出したまま走るので、
**ブロックの中から同じオブジェクトに触ると弾かれる**。ブロックを取れるようになって初めて Ruby から普通に書ける道なので、
「書けるが、そこは `RefCell` の規則が効く」ことをテストに残した。
`macros/tests/expand.rs` には生成コードの固定を 1 本と、2 つのコンパイルエラーを足した。
