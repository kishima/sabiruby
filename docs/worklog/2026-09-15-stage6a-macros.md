# 段階 6a: `sabiruby-macros`（`#[derive(RubyClass)]`、`#[ruby_methods]`、`HostStore`）

`docs/plans/host-bridge-plan.md` の段階 6a。ブランチ `macros`（worktree `sabiruby-wt-macros`）。
段階 3（`define_closure` と `host_state`）・4（`FromRuby`/`IntoRuby`/`define_fn`）・5（`Data` と解放フック）の上に載せる。

## 1. `HostStore<T>` と Data のタグ（`src/host_store.rs`）

### 読んだもの、確かめたこと

先に `src/convert.rs` の冒頭 60 行（`define_fn` が「文脈」として読む引数の並び）、`src/vm.rs` の
`define_closure_body`（1192〜1220 行）・`set_host_state`/`host_state_mut`（1226〜1240 行）・`data_new`/`data_of`/`set_on_free`
（1266〜1300 行）、`tests/data.rs` 全部、`gc_collect` の末尾（`src/vm.rs` 2386〜2393 行）を読んだ。
`tests/data.rs` の `a_handle_goes_back_to_the_hosts_own_table` が、この段階で自動化したいことをそのまま手で書いている:
`Arc<Mutex<Vec<Option<String>>>>` の slab を作り、`set_on_free` のクロージャにその `Arc` を捕まえさせ、
メソッドごとに `store.clone()` して `define_fn` に渡す。マクロが生成すべきなのはこの 5 つ（クラス、タグ、slab、解放、メソッド）である。

### 決めたこと 1: `HostStore` は `Vm` が型ごとに持つ（TypeId で引く）

指示書が挙げた 2 案のうち、**案 B（`Vm` が `TypeId` をキーに型ごとの `HostStore` を持つ）**を選んだ。理由は 3 つある。

第 1 に、案 A（利用者の型を `host_state` に置き、その中に複数の `HostStore` を並べる）だと、生成コードが
「利用者の状態の型 `S`」を知っていなければならない。`#[derive(RubyClass)] struct Player` を書いた人が、別に
`struct MyGame { players: HostStore<Player>, monsters: HostStore<Monster> }` を書き、マクロにその型と、
`MyGame` から `HostStore<Player>` を取り出す道筋を教える必要がある。属性で書かせる設計はできるが、
「`#[derive(RubyClass)]` と `#[ruby_methods]` を書けば `Player::register(&mut vm)` が全部やる」という到達点から遠い。

第 2 に、**解放フックが決め手になった**。`set_on_free` のフックは `Fn(u32, u64)` で `&mut Vm` を受け取らない
（段階 5 の判断: GC の後始末の途中に VM へ再入させない）。したがって案 A では slab をフックから触るために
`Arc<Mutex<_>>` が要る。VM 本体は `no_std + alloc` で `Mutex` が無い（`spin` などの依存を増やすことになる）。
一方、案 B なら slab は VM の中にあるので、**VM 自身が解放できる**。`gc_collect` の末尾、`heap.freed_data` を
ホストのフックに渡す直前に、タグの一致する自分の store から `remove` するだけでよい（`Vm::host_store_free`、9 行）。
結果として、生成コードは `set_on_free` を一切呼ばない。これは副次的に **2 つ目の問題も消す**:
`set_on_free` はフックを 1 つしか持てないので、`Player::register` と `Monster::register` が両方フックを張る設計は
後から登録したほうが前のを潰す。VM が解放する形なら型が何個あっても衝突しないし、`set_on_free` は
ホスト自身の用途（rubevy がエンティティの後始末を知る、など）に空いたまま残る。

第 3 に、案 B は VM ごとに store が分かれるので、同じ Rust の型を 2 つの VM から使っても混ざらない。
`static` の slab（rubevy が段階 3 で捨てた形）に戻らずに済む。

代償として、`Vm` に `host_stores: Vec<HostStoreEntry>` と `next_tag: u32` の 2 フィールドが増えた。
`Box<dyn AnyStore>`（`AnyStore: Any + Send + Sync`）なので `Vm: Send + Sync` は保たれる（`tests/host_store.rs` で確認）。
`HashMap<TypeId, _>` ではなく `Vec` の線形探索にしたのは、ホストの型は数個（rubevy で 2〜3）で、
`TypeId` の比較が 16 バイトの整数比較 1 回だから。ハッシュを計算するより速いし、`no_std` で余計なものが増えない。
探索が走るのはホストのメソッド呼び出しの中だけで、実行ループには乗らない。

### 決めたこと 2: `take` / `restore`（`&mut Vm` を取るメソッドのため）

store が `Vm` の中にあると、`vm.host_store_mut::<Player>()` は `&mut Vm` を借りたままになる。
つまり **`fn damage(&mut self, vm: &mut Vm, n: i64)` の形が書けない**（`&mut self` と `&mut Vm` が同時に要る）。
3 案を比べた:

* (a) store ごと `Vm` から外して呼び出し、終わったら戻す。`Player` の別の個体に触るメソッドが中から呼ばれると
  store が丸ごと無いので、素直に壊れる。捨てた。
* (b) `Arc<Mutex<T>>` を slab に入れる。`no_std` に `Mutex` が無い。捨てた。
* (c) **その 1 個だけを slab から抜き、番号は予約したまま、呼び出しが終わったら戻す。** 採用。
  `HostStore::take(handle)`（`remove` と違って空き番号リストに積まない）と `restore(handle, value)` の 2 本。
  抜いている間に同じオブジェクトへ再入したら `take` が `None` を返すので、
  `RuntimeError: Player is already in use by a call on the same object` になる。`RefCell` の二重借り panic の代わりに
  Ruby の例外が上がる形で、VM の中で起きることとしてはこれが正しい。

`&mut Vm` を取らないメソッドは (c) を通さず `borrow`/`borrow_mut` で直接借りる（移動が無い分ただの参照）。
生成コードが 2 系統になるが、どちらも数行で、`expand.rs` に両方を固定した。

### 決めたこと 3: `RubyClass` trait は `sabiruby` 側に置く

proc-macro crate は関数も型も公開できない（マクロだけ）ので、生成コードが寄りかかる共通部分はどこかに要る。
全部 `quote!` で吐く案もあったが、`borrow` / `handle_of` / `into_handle` は借用と例外の扱いが細かく、
生成コードとして毎回展開するには重い。`src/host_store.rs` に `pub trait RubyClass`（必須は `const NAME` だけ、
あとは既定実装）を置き、`#[derive(RubyClass)]` は実質 2 行（`impl RubyClass` と `impl IntoRuby`）を吐くだけにした。
手書きのホストも同じ 4 行で同じ道具が使える（rustdoc の例で示した）。

`impl IntoRuby for Player` を derive が吐くのは、地味だが効く。おかげで
**`fn new(hp: i64) -> Self` を特別扱いしなくてよくなった**: 戻り値が `Self` なら `IntoRuby` が store に入れて
`Data` を作るので、生成側は「戻り値を `IntoRubyRet` に渡す」の 1 本で済む。`Result<Self, String>` も
`Vec<Self>` もそのまま通る。`FromRuby for Player`（値ごと Ruby から取り出す）は入れていない。
ハンドルの指す実体を持ち去ることになり、残ったハンドルが stale になるため。

`u32` のタグは `Vm::next_data_tag()` が 1 から採番する。`install_host_store::<T>()` は冪等で、
2 回目以降は同じタグを返す（`Player::register` を 2 回呼んでも tag が増えない）。
手でタグを振るホスト（`tests/data.rs` の `PLAYER = 7`）と混ぜても衝突しないよう、
`next_data_tag` を公開して「手で振るならここから取れ」と rustdoc に書いた。

### 確かめたこと

`tests/host_store.rs` を新設（8 本、すべて通る）。slab の番号の再利用、`take` が番号を予約したままにすること、
`restore` が予約していない番号を捨てること、1 つの VM が型ごとに別の store と別のタグを持つこと、
`Vm: Send + Sync`、別の型のハンドルを渡したときの `TypeError` の文言、ホストが自分で `remove` した後の stale ハンドル、
そして **GC が Ruby のオブジェクトを回収すると実体が store から消え、ホストの `set_on_free` にも届くこと**
（2 つの型を同時に登録して、それぞれの store から正しく消えることも）。

`cargo test --workspace` 148 passed / 0 failed、doc-test 13 passed、`tools/check_no_std.sh` OK、
`cargo doc --no-deps` 警告なし。

## 2. crate `macros/`（`sabiruby-macros`）

### 決めたこと 4: クラスメソッドの判定規則は「`self` を取らないもの」

計画書は「`self` を取らず `Self` を返すものはクラスメソッド」と書いているが、実装してみると
**戻り値を見る必要が無かった**。`#[derive(RubyClass)]` が `impl IntoRuby for Player` を吐くので、
`-> Self` は `IntoRuby` として store に入り `Data` になる。つまり生成側は「戻り値を `IntoRubyRet` に渡す」の 1 本で、
`-> Self` も `-> i64` も `-> Result<Self, String>` も `-> Vec<Self>` も同じコードで通る。

そこで規則を **「`self` を取らない `fn` はクラスメソッド、`&self`/`&mut self` はインスタンスメソッド」** に単純化した。
計画書の規則の上位互換で、`fn strongest(vm: &mut Vm, a: i64, b: i64) -> Self` のような「`new` ではないクラスメソッド」も、
`Self` を返さないクラスメソッドも書ける。`self` を値で取る形（`fn into_hp(self)`）は拒否する:
実体を store から持ち去ることになり、残った Ruby のオブジェクトが何も指さなくなるため。エラーメッセージにそう書いた。

### `#[ruby(skip)]` と `#[ruby(name = "...")]`

計画書には無いが 2 つ足した。`impl` ブロックにはホストが自分用に使う `fn` が普通にあるので `skip` が無いと
マクロを実 impl に付けられない。`name` は Ruby の名前（`alive?` の `?`）が Rust の識別子にならないため。
どちらも属性 1 つ、パースは 20 行。これ以上は足していない（ブロック引数、可変長引数、キーワード引数は
`define_closure` に落ちる、と `docs/design/macros.md` に書いた）。

### 生成コードの形

`Vm::define_fn` に丸投げする形にした。生成されるのは 1 メソッドにつき 1 つの `define_fn` 呼び出しで、
クロージャの引数は Rust の署名そのまま（`|vm: &mut Vm, __this: This<Value>, __a0: i64| -> VmResult<Value>`）。
こうすると `FromRuby`/`IntoRuby`/引数の個数の検査/`Method#arity` が段階 4 のものになり、マクロが自前で持つものが無い。
`This<Value>` で受けて `RubyClass::borrow_mut` に渡すのは、`This<DataRef>` だと「Data ではあるが別の型」の
エラーメッセージがクラス名を言えないため。

**引数の型のエラーメッセージが読めなかった。** `BTreeMap` を引数に取るメソッドを書いて確かめたところ、

```
error[E0277]: the trait bound `{closure@…badtype.rs:7:1: 7:16}: RubyFn<_>` is not satisfied
 7 | #[ruby_methods]
   | ^^^^^^^^^^^^^^^ the trait `RubyFn<_>` is not implemented for closure
```

型も行も出ない（`define_fn` の境界がクロージャ全体に付いているので当然）。生成コードに、呼ばれない
`fn __ruby_argument_types()` を足し、その中で `__arg::<#ty>()` を **引数の型の span で** `quote_spanned!` して
`FromRuby` を要求するようにした。結果:

```
error[E0277]: the trait bound `BTreeMap<String, i64>: FromRuby` is not satisfied
 9 |     fn table(&self, t: BTreeMap<String, i64>) -> i64 { … }
   |                        ^^^^^^^^^^^^^^^^^^^^^ the trait `FromRuby` is not implemented for …
   = help: the following other types implement trait `FromRuby`: DataRef, Option<T>, Value, Vec<T>, bool, f64, i32, i64 …
```

`&str` を書いた場合は `help: the trait FromRuby is implemented for String` まで出る。
戻り値側はもともと読めた（`IntoRubyRet` の境界が具体型に付いているため）ので触っていない。

### 展開結果の固定（`macros/tests/expand.rs`）

`cargo expand` は使えない（proc-macro crate はマクロ以外を公開できないので、展開を呼ぶ側が書けない）。
展開の中身を `macros/src/expand.rs` に普通のコードとして置き、テスト側から `#[path = "../src/expand.rs"] mod expand;`
で直に取り込む形にした（この module が `proc_macro` crate を触らないことが条件で、`proc_macro2` だけで書いてある）。

比較は最初 `TokenStream::to_string()` 同士でやろうとして落ちた。`quote!` が付ける punct の
joint/alone が字句解析器のそれと少し違い、`>::` が `> :: ` になったりならなかったりする。
「1 度印字して読み直して印字し直す」正規化も効かなかった（spacing が保存される）ので、
**空白を全部落としてから比べる** ことにした。両辺とも本物のトークン列なので、隣り合う 2 つのトークンが
1 つに化けて誤って一致することはない。失敗時は `;` と `{` で改行を入れた読める形を出す。
テストが本当に差を見ているかは、期待値の `borrow_mut` を `borrow` に変えて落ちることで確かめた。

## 3. 結合テスト（`macros/tests/player.rs`、12 本）

計画書の `Player` をそのまま書き、Ruby から回した。`Player.new(100)`、`damage`/`hp`/`rename`/`alive?`、
`Method#arity`（`greet` は `&mut Vm` を取るので 1 で、文脈は数えない）、引数の個数と型の例外、
`GC.start` で 10 個が store から消えて 1 個残ること、`dup`/`clone` が `TypeError`、
`Player` と `Monster`（`#[ruby(name)]` で改名した `Beast`）が同じ VM でタグも store も別なこと、
同じ型を 2 つの VM で使っても混ざらないこと、Rust 側から `borrow`/`borrow_mut`/`IntoRuby` で触れること。

書いていて分かったこと 2 つ:

* **Ruby から「Player 以外のレシーバで Player のメソッドを呼ぶ」のは難しい。** `UnboundMethod#bind` が
  先に `bind argument must be an instance of Player` で弾く。到達できるのは `allocate`（ハンドルの入っていない
  素の Player オブジェクト）とその Ruby 側サブクラスで、そこを突くテストに書き換えた
  （`Ghost.allocate.hp` → `wrong argument type Ghost (expected Player)`）。別の型のハンドルを渡す経路は
  Rust 側から `Player::borrow` を呼んで確かめている。
* **再入のテストで、最初に出た例外は `in use` ではなく `stale` だった。** `greet` が実体を抜いている間に
  Ruby 側から同じオブジェクトの `name` を呼ぶと、`borrow` は「slab の枠はあるが空」を見て
  「ホストが捨てた（stale）」と答えていた。`HostStore` の枠を `Empty` / `Full(T)` / `Out` の 3 状態に変え
  （`take` は `Out` を置く）、`is_out` で区別できるようにした。`remove` は `Out` の枠を解放しない
  （貸し出し中のものを GC が回収しようとしても番号を再利用しない）。ついでに `len` が
  「貸し出し中を数えない」という意味に揃った。

## 4. 確認

* `cargo test --workspace`: **169 passed / 0 failed**（追加分: `tests/host_store.rs` 8、`macros/tests/expand.rs` 9、
  `macros/tests/player.rs` 12。doc-test は sabiruby 13 本すべて通る）。
* `tools/check_no_std.sh`: `no_std OK`。`macros` は proc-macro なので対象外、生成コードは `no_std` の VM に対して
  動く（`::sabiruby::` の公開 API しか呼ばない）。
* `cargo doc --no-deps --workspace`: 警告なし。唯一出る
  `output filename collision at target/doc/sabiruby/index.html`（`sabiruby-cli` の bin と `sabiruby` の lib が同名）は
  **この段階より前からあるもの**で、main の作業ツリーでも同じものが出ることを確かめた。
* `cargo publish --dry-run --workspace`: 4 crate すべて成功（EXIT=0）。`sabiruby` の package に `macros/` は入らない
  （`include` が `/src/**/*` などの白名簿なので）。`cargo package --list -p sabiruby | grep macro` は
  `docs/worklog/2026-09-15-stage6a-macros.md`（docs は元から入る）だけ。`sabiruby-macros` の package は 7 ファイルで、
  テストは `include` から外してある。
* 本家テスト: 108 ファイル（`gem_ascii_*` を除く）を
  `./target/release/sabiruby mrbtest tests/mrbtest/assert.mrb tests/mrbtest/prelude.mrb tests/mrbtest/<name>.mrb` で回し、
  `ok` 列が `tests/mrbtest/baseline.txt` と **1 ファイルの差も無く一致**。
* ベンチは取っていない（指示どおり。VM に足したのは `host_stores: Vec` と `next_tag: u32` の 2 フィールドと、
  `gc_collect` の末尾で `freed_data` が空でないときだけ回る 3 行で、実行ループにも割り当て経路にも触っていない）。

## 5. 残した宿題・気づいたこと

* **`register` が `expect` を 1 つ持つ。** クラスメソッドがあるとき `vm.singleton_class(...)` の
  `VmResult` を `.expect("a class has a metaclass")` で開いている。クラスには必ずメタクラスがある
  （`define_class` が作る）ので届かないはずだが、生成コードに panic の芽が 1 つあるのは事実。
  `register` を `VmResult<ObjId>` にすれば消えるが、計画書の `T::register(&mut vm)` の形から離れるので触っていない。
* **`Player.allocate` の穴。** `Class#new` は上書きしているが `allocate` は残るので、Ruby から
  ハンドルの入っていない `Player` を作れる。メソッドを呼べば `TypeError` になる（テスト済み）ので壊れはしないが、
  メッセージが `wrong argument type Player (expected Player)` になるのは読みにくい。
  `allocate` を潰すか、メッセージを分けるかは著者判断。
* **ブロックを取るメソッドが書けない。** `define_fn` には `Block` を末尾に取る形があるので、マクロが
  末尾の `Block` を文脈として読む拡張は 5 行で入る。計画書に無いので入れていない。
* rubevy 側（6b・6c の担当）へ: `Rubevy::Entity` を `#[derive(RubyClass)]` に載せ替えられるはずだが、
  エンティティの実体は Bevy の `World` 側にあって `HostStore` に入れるものが無いので、そのままでは合わない。
  `HostStore` が要るのは「Rust の値を Ruby に持たせる」場合で、「Bevy の ID を渡す」だけなら段階 5 の
  `data_new` のままでよい。
