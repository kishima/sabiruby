# 作業記録: 段階 4（`FromRuby` / `IntoRuby` / `define_fn`）と段階 5（Data とハンドル）

`docs/host-bridge-plan.md` の段階 4・5 を実装したときの記録。結果だけでなく、読んだ場所、選んだ理由、
捨てた案、引っかかった点を残す（同人誌の「C 拡張の代わりに Rust でホストとつなぐ」章の素材）。
作業は worktree `sabiruby-wt-bridge`、ブランチ `bridge-host2`。ベンチは回していない（段階 2 の担当が
同じ機械で計測中のため、性能の確認は本体がマージ後に行う）。

## 0. 手をつける前に読んだところ

- `src/object.rs:17-50` — `NativeFn`、`NativeClosure = Arc<ClosureBody>`、`ClosureBody(Box<dyn Fn …>)`、
  `Method` の 6 variant。段階 3 が `Closure` を足したところ。
- `src/vm.rs:1044-1120` — `define_method` / `define_closure` / `set_host_state`。
- `src/vm.rs:1694-1730` — `call_native` / `call_closure` と、SEND から呼ばれる `*_direct` 版。
- `src/builtins/ext_method.rs:116-130` — `native_arity`。**関数アドレスで表を引く**ので、環境を持つ
  クロージャはそもそも表に載せられず `-1` になる（段階 3 の知見のとおり）。
- `src/builtins/mod.rs:188-207` — `check_argc` と `argc!`。例外の文言
  `wrong number of arguments (given N, expected M)` はここが唯一の出どころ。
- `src/vm.rs:899-970` — `raise` / `raise_type` / `raise_arg` / `argnum_error` / `expect_str` / `expect_int`。
- `src/object.rs:196-226`（`ObjKind`）、`:370-420`（`mark_drain`）、`:422-450`（`sweep`）。
- `src/vm.rs:2097-2162` — `gc_collect`。sweep はこの 1 か所からしか呼ばれない。
- `src/builtins/object.rs:16-70`（`==` / `eql?` / `hash` の定義）、`:410-460`（`dup`）、`:351-370`（`clone`）。
- `src/builtins/ext_objectspace.rs:13-37` — `TYPES` と `type_index`。
- `src/inspect.rs:270-300` — デバッグ用の `render_text`（Ruby の `inspect` ではない）。

## 1. 段階 4: `FromRuby` / `IntoRuby` と `define_fn`

### 1.1 何が問題だったか

段階 3 の `define_closure` は生の呼び出しをそのまま渡す。ホストは `args` の個数を自分で数え、
`Value` を自分で場合分けし、答えを自分で `Value` に組み立てる。同じ定型が登録ごとに増えるうえ、
**引数の個数を VM が知らない**ので `Method#arity` が `-1` のままになる（段階 3 の知見の最後の項）。
段階 4 の仕事はこの定型を Rust の型で置き換えること。

### 1.2 `FromRuby` の文言をどこから取るか

例外の文言は新しく作らず、VM が既に使っているものに合わせる方針だったので、`src/vm.rs:957` の
`expect_int` をそのまま呼ぶことにした（`no implicit conversion of nil into Integer`、Rational/Complex/
BigInt/Float の面倒も込みで既に正しい）。`f64` は `src/builtins/numeric.rs:51` の `coerce_fail`
（`can't convert Symbol into Float`）、Array は `src/builtins/array.rs:158` の
`no implicit conversion into Array`、`i32` の範囲外は `expect_int` が BigInt に対して出している
`integer out of range`（`RangeError`）を使い回した。

String だけ既存の受け皿が合わなかった。`Vm::expect_str(v, what)` の `what` は
**引数の役割名**（`"format"`、`"separator"`）で、`"{what} cannot be converted to String"` と出る。
型で頼む変換では役割名が無いので、`vm.describe_for_type_error(v)` で値の型名を入れ、
`Integer cannot be converted to String` とした（`src/builtins/numeric.rs:57` の `not_int` が
Integer 側で同じ形を使っている）。

計画書は `String` の変換に `as_string`（`mrb_obj_as_string`、`to_s` を呼ぶ）と書いてあったが、
**採らなかった**。理由は 3 つ。(1) `as_string` は失敗しないので、同じ段落にある「変換失敗は本家と同じ
`TypeError` の文言」と両立しない。(2) `as_string(nil)` は `""` になるので、`greet(nil)` が
`hi ` になってしまい、型で頼んだ意味が無い。(3) `to_s` は Ruby のメソッドなので、引数の変換の途中で
任意の Ruby コードが走り、VM に再入する。本家の `mrb_get_args("S")` も `TypeError` を上げる。
`to_s` による変換が欲しいホストは `Value` で受けて `vm.as_string` を自分で呼べる、と rustdoc に書いた。

### 1.3 `Vec<u8>` と `Bytes`（コヒーレンスで詰まったところ）

計画書は `FromRuby` の一覧に `Vec<u8>`（String のバイト列）と `Vec<T>`（Array）の両方を挙げているが、
この 2 つは同居できない。`impl<T: FromRuby> FromRuby for Vec<T>` と `impl FromRuby for Vec<u8>` は
コヒーレンス上重なる（`u8: FromRuby` を今書いていなくても、コンパイラは「無い」ことを根拠にしない）。
`IntoRuby` 側でも同じ衝突が起きる。

案を 3 つ比べた。(a) `Vec<u8>` をバイト列にして `Vec<T>` を諦める — Array を受け取れなくなるので却下。
(b) `u8: FromRuby` を定義して `Vec<u8>` は Integer の Array にする — バイト列を受ける手段が消える。
(c) `Bytes(Vec<u8>)` という newtype を足す。**(c) を採った。** `Vec<T>` は Array、`Bytes` は String の
バイト列、と読んで分かる。`IntoRuby` 側も対称になる（`Bytes` は String を作り、`Vec<u8>` は
Integer の Array を作る）。

### 1.4 戻り値の `Result` をどう通すか

計画書は `IntoRuby` の impl として `Result<T, E: Into<String>>` を挙げているが、`into_ruby` の型が
`-> Value` なので、そこから例外は上げられない。かといって `IntoRubyRet` のような別 trait を
「`T: IntoRuby` への包括 impl + `Result<T, E>` への impl」で作ると、また重なる。

採ったのは marker 型パラメータで分ける形（`IntoRubyRet<M>`）。`RetValue` / `RetVmResult` /
`RetString` / `RetStr` の 4 つの marker があり、impl の marker が違うので重ならない。呼ぶ側は
`R: IntoRubyRet<RM>` と書くだけで、`RM` はコンパイラが 1 つに決める（`VmResult<i64>` は `IntoRuby`
を実装していないので `RetValue` の候補に上がらない）。`E` を `Into<String>` の境界ではなく
`VmError` / `String` / `&'static str` の**具体型 3 つ**にしたのも同じ理由で、境界にすると
`VmError: Into<String>` があり得るかをコンパイラが否定できず、重なりと判定される。

`Result<T, VmError>`（= `VmResult<T>`）を入れたのが実用上いちばん効く。これがあると
`Err(vm.raise_arg("negative"))` で例外クラスを選べる。`&mut Vm` を取らない形のために
`Result<T, String>` と `Result<T, &'static str>`（どちらも `RuntimeError`）も残した。

### 1.5 「受け手」と「ブロック」をどう見分けるか

`|a: i64, b: i64|` と `|this: i64, a: i64|` は型だけでは区別できない。Bevy の system と同じで、
trait に marker 型パラメータを持たせ、impl ごとに違う marker を割り当てて分ける。受け手は
`This<T>` という newtype にして、パラメータの位置ではなく**型**で宣言させる形にした
（位置で決めると「1 引数の関数」と「引数 0 の受け手付き」が曖昧になる）。

ブロックは `Block(Option<Value>)` を末尾に置く形。`&mut Vm` を取る形にだけ用意した。
理由は単純で、**ブロックを呼ぶには `vm.call_block` が要る**ので、`&mut Vm` の無い関数が
ブロックだけ受け取っても何もできない。これで組み合わせは 6 通り（`{}`, `This`, `Vm`, `Vm+This`,
`Vm+Block`, `Vm+This+Block`）× 引数 0〜6 = 42 impl。`Vm` 無しでブロックを許すと 56 になり、
使えない impl が 14 増えるだけだった。

マクロは `macro_rules!` 1 本（`ruby_fn_impls!`）で、引数の個数と `(型, 添字)` の組を受ける。
最初は `let mut _i = 0; … _i += 1;` という走る添字で書いたが、引数 0 個の展開で
`unused_mut` が出るので、添字をリテラルで渡す形に変えた。`let $t = <$t as FromRuby>::from_ruby(…)`
は型名と同じ名前の束縛を作る（名前空間が違うので通る）が、`non_snake_case` の警告が 42 impl 分
出るので `#[allow(non_snake_case)]` を `call_ruby` に付けた。

### 1.6 arity を `Closure` に持たせる

`src/builtins/ext_method.rs:116` の `native_arity` は `vm.native_arity` を**関数アドレスで**引く。
環境を持つクロージャにはアドレスが無いので、この表には載せられない。`ClosureBody` を
タプル構造体から名前付き構造体に変え、`arity: i64` を足した。`define_closure` は `-1`（宣言が無い、
という本来の意味）、`define_fn` は Rust の引数の個数を入れる。`Method` は今も 16 バイト
（`Arc<ClosureBody>` の薄いポインタ 1 つ）で、段階 3 が気にしていた点は変わらない。

`define_closure_body`（arity を取る版）は `pub(crate)` にした。ホストに公開する理由が今は無く、
公開すると `define_closure` / `define_closure_body` / `define_fn` の 3 つが並んで選びにくくなる。

### 1.7 確認

`cargo test --workspace` 全通過（`tests/convert.rs` 8 本を含む）、`tools/check_no_std.sh` OK、
`grep -rn unsafe src` は 0 行、`cargo doc --no-deps` 警告なし。`tests/mrbtest.rs` の
baseline 比較（108 ファイル）も通過。ベンチは回していない。
