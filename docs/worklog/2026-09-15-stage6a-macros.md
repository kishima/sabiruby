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
