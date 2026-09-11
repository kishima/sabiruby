# ブラウザ Playground 実装計画（SabiRuby を wasm で動かす）

作成 2026-09-12。目的: ブラウザ上に「Ruby コードの編集欄」と「実行結果の欄」を持つ Playground を作り、SabiRuby（VM）と本家コンパイラ（`sabiruby-compiler`）を
WebAssembly で動かす。サーバは持たない（静的サイト、GitHub Pages）。本の読者が命令列（`dump`）と実行結果を並べて見られることも狙う。

## 0. 前提と調べた事実

* `sabiruby`（lib）は no_std + alloc で、`wasm32-unknown-unknown` のビルドは CI で通っている。GC 済み（`docs/gc.md`）。
* Ruby ソースからのコンパイルは `sabiruby-compiler`（`docs/compiler.md`）で、中身は本家の C（Prism 1.9.0 + codegen）。**ブラウザで Ruby を書いて動かすには、この C も wasm にする必要がある**。
  `docs/compiler.md` は wasm32 を対象外としているが、理由は「clang と wasm 用 sysroot が要る」であって、方式の問題ではない。
* Prism 自身が wasm ビルドを持つ（`lib/prism/Makefile` の `wasm:` ターゲット、`WASI_SDK_PATH=/opt/wasi-sdk`）。CRuby 公式の npm パッケージ `@ruby/prism` は
  この `prism.wasm`（WASI）を `@bjorn3/browser_wasi_shim` でブラウザ上で動かしている。**同じ道を使う**: C は wasi-sdk の clang で `wasm32-wasip1` 向けにビルドし、
  Rust も `wasm32-wasip1` ターゲット、ブラウザ側は browser_wasi_shim で WASI の import（`fd_write`、`clock_time_get`、`random_get` など）を埋める。
  `wasm32-unknown-unknown` に libc を自作して載せる案は、Prism が `snprintf`／`strtod`／`stdio` を使うので勧めない。
* CLI は `sabiruby-cli` に分かれ、`sabiruby` 0.2.0 は VM ライブラリだけになった（`4acc5db`）。Playground は `sabiruby`（lib）と `sabiruby-compiler` に依存する。
  crates.io に出るまでは git 依存（`kishima/sabiruby` のパス）でよい。
* wasm は単一スレッド。CLI が使う「大きなスタックの別スレッド」は使えないので、リンク時にメインスタックを大きくする（`-C link-arg=-zstack-size=16777216`）。
  深い再帰は Prism の `PRISM_DEPTH_MAXIMUM=256` と VM の `CALL_LEVEL_MAX`／`NATIVE_DEPTH_MAX` で止まるので、16 MB で足りる（実測して決める）。
* wasm-bindgen は使わない（wasip1 との組み合わせが複雑、依存が増える）。**C ABI の関数を `#[no_mangle] extern "C"` で公開**し、JS 側は素の `WebAssembly.instantiate` と
  線形メモリの読み書きだけで扱う。JS 側の依存は browser_wasi_shim と CodeMirror だけ。

## 1. 構成

新しいリポジトリ `kishima/sabiruby-playground`（MIT。`sabiruby` リポジトリを大きくしない。Pages の配信元にする）。

```
sabiruby-playground/
  wasm/                 Rust crate `sabiruby-wasm`（cdylib）: sabiruby + sabiruby-compiler を wasm32-wasip1 向けに
    Cargo.toml, src/lib.rs, .cargo/config.toml（target と link-arg）
  web/                  静的サイト
    index.html          編集欄・実行ボタン・結果欄・サンプル選択・命令列の表示切替
    main.js             UI と Worker の橋渡し
    worker.js           wasm の読み込みと実行（UI スレッドを止めない）
    wasi.js             browser_wasi_shim（vendoring。npm は使わない）
    vendor/codemirror/  CodeMirror 6 の bundle（cdnjs から読み込むか vendoring。どちらかに決めて記録）
    samples/            本の例と照合スクリプト（移植キットの samples/ と fixtures/*.rb から選ぶ）
  tools/build.sh        wasm のビルドと web/ への配置
  .github/workflows/    ビルドして Pages にデプロイ
  README.md
```

### 1.1 wasm crate の公開 API（C ABI）

すべて単一の VM を静的に持つ（`static mut` ではなく `thread_local!` か `OnceCell` + `RefCell`。wasm は単一スレッド）。

```
sabi_version() -> *const u8 (NUL 終端)                 VM とコンパイラの版
sabi_alloc(len) -> ptr / sabi_free(ptr, len)            JS がソースを書き込む領域
sabi_compile(src_ptr, len, debug: u32) -> status        Ruby ソース → RITE。失敗時は診断を sabi_take_text で取れる
sabi_load(mrb_ptr, len) -> status                       RITE を直接読み込む（.mrb のドラッグ＆ドロップ用、任意）
sabi_reset()                                            VM を作り直す（mrblib 読み込み済みの状態）
sabi_start() -> status                                  読み込んだ irep を start（Vm::start）
sabi_step(budget: u64) -> 0=paused, 1=finished, 2=error  Vm::step。error のときは sabi_take_text にメッセージ
sabi_take_output(len_out: *mut u32) -> ptr              puts/p の出力（Vm::take_output）。呼ぶたびに空になる
sabi_take_text(len_out) -> ptr                          直前の診断／例外メッセージ（describe_error）
sabi_dump(len_out) -> ptr                               読み込んだ irep の命令列（`sabiruby dump` と同じ文字列）
sabi_stats(insns_out: *mut u64, live_out: *mut u64)     命令数と GC の live（表示用）
```

* `status` は 0 成功、1 コンパイルエラー、2 実行時エラー、3 内部エラー。文字列はすべて UTF-8（Ruby 側はバイト列なので、JS 側で `TextDecoder("utf-8", {fatal:false})`）。
* 実行は必ず `sabi_step` の予算刻み（例: 200 万命令）で回し、Worker が予算ごとに出力を UI に送る。**無限ループでもページは固まらず、停止ボタンは Worker の `terminate()`** で確実に止める。
* `Vm::step` は Fiber の中で止まっても続きから走る（`docs/fibers.md`）ので、予算刻みは VM の意味論に影響しない。

### 1.2 ビルド

* Rust: `rustup target add wasm32-wasip1`。`wasm/.cargo/config.toml`:
  ```toml
  [build]
  target = "wasm32-wasip1"
  [target.wasm32-wasip1]
  rustflags = ["-C", "link-arg=-zstack-size=16777216"]
  ```
* C（`sabiruby-compiler` の build.rs、`cc` crate）: 環境変数で wasi-sdk の clang を指定する。
  `CC_wasm32_wasip1=/opt/wasi-sdk/bin/clang`、`AR_wasm32_wasip1=/opt/wasi-sdk/bin/llvm-ar`（wasi-sdk の clang は sysroot を内蔵しているので `--sysroot` は不要）。
  `sabiruby-compiler` の build.rs に手を入れる必要が無いことを最初に確かめる（`cc` は `CC_<target>` を読む）。必要なら `docs/compiler.md` の「wasm32 は対象外」を「wasi-sdk があれば可」に改める。
* 生成物 `sabiruby_wasm.wasm` を `web/` に置く。サイズの目安: Prism 約 1.5 MB + codegen + VM で 2〜3 MB（gzip で 1 MB 以下）。`wasm-opt -Oz`（binaryen）で縮める。
* CI（GitHub Actions）: wasi-sdk を取得（release の tar.gz を展開）、`cargo build --release`、`wasm-opt`、`web/` を Pages にデプロイ。

### 1.3 Web 側

* 編集欄: CodeMirror 6（Ruby のハイライトは `@codemirror/legacy-modes` の ruby）。cdnjs にあるものを `<script>` で読むか、1 本に bundle して vendoring する。
* 結果欄: 標準出力（逐次追記）、エラー（`FILE:LINE:COL: message` またはバックトレース無しの例外表示）、命令数と時間、GC の live。
* 命令列: 「バイトコードを表示」を押すと `sabi_dump` の結果を右側に出す（本の `mrbc --verbose` と同じ形式）。
* サンプル: プルダウンで本の例（`samples/`）と照合スクリプト（`fixtures/*.rb`）を選ぶ。**照合スクリプトには本家の出力（`.out`）が付いているので、「本家の出力と比較」ボタンで一致・不一致を表示**できる。これは Playground を検証にも使えるということで、本の主張と直結する。
* URL にコードを持たせる（`#code=` に base64）で共有できるようにする。任意。
* 実行はすべて Worker の中。UI は `postMessage` で「run／stop／dump」を送り、「output／error／done／stats」を受ける。

## 2. 手順

1. **技術検証（最大の不確実要素）**: `sabiruby-compiler` を `wasm32-wasip1` でビルドする。`cc` が `CC_wasm32_wasip1` を読むこと、Prism と codegen が wasi-sdk の clang で警告のみで通ること、
   `sabiruby` が `wasm32-wasip1` で `std` 付きビルドできることを確かめる。ここで詰まったら（例: Prism が使う libc 関数が wasi-libc に無い）方式を見直す。
   確認方法: `wasmtime`（または `node --experimental-wasi-unstable-preview1`）で、`hello.rb` をコンパイルして実行し `hello` が出ること。
2. wasm crate（§1.1 の API）と、Node で回す最小テスト（`tests/fixtures/*.rb` を全部コンパイルして実行し `.out` と比べる。**ブラウザ抜きで検証の大半を済ませる**）。
3. `web/`: Worker と素の JS で、テキストエリアだけの版を動かす（CodeMirror は後）。停止ボタンと予算刻み。
4. CodeMirror、サンプル、命令列表示、本家出力との比較、エラー表示の整形。
5. GitHub Pages のデプロイと README。`sabiruby` の README から Playground へリンク。
6. 文書: `docs/playground.md`（設計、API、ビルド手順、対応ブラウザ）。本のリポジトリ `docs/notes/sabiruby-findings.md` に §15（wasm 化で分かったこと。特に「本家コンパイラは wasi-sdk で wasm にできる」「スタックサイズ」「WASI の import で実際に要ったもの」）。

## 3. 踏みやすい点

* `sabiruby-compiler` の C は `fprintf(stderr, ...)`（ファイルが開けないとき）と `printf`（`--verbose`）を含む。WASI の `fd_write` を shim が受けるので問題ないが、
  stderr の内容は JS 側で拾って結果欄に出す（`browser_wasi_shim` の `ConsoleStdout`）。
* `mrc_presym.c` の静的変数（`docs/compiler.md`）: 単一スレッドなので問題ないが、`sabi_compile` を再入しない。
* wasm の線形メモリは伸びる一方（`memory.grow`）。GC があるので Ruby 側のヒープは安定するが、Rust の `Vec` 容量や C の `malloc` の断片化で RSS 相当は増え得る。
  「リセット」でページを再読み込みせずに `sabi_reset` するか、Worker を作り直すか（作り直す方が確実）。
* `clock_time_get` と `random_get` を shim に用意しないと `std::time`／`HashMap` の初期化（hashbrown の乱数シード）で落ちる。hashbrown は `ahash` の乱数を使うことがある。
  `sabiruby` は `hashbrown` を `default-features = false` の `default-hasher`（foldhash、固定シード）で使っているので、VM 側は乱数を要求しない。要求するのは `sabiruby-compiler` 側の libc と `std` の初期化だけ。
* 文字列は Ruby 側がバイト列（`MRB_UTF8_STRING` 無し）なので、出力に不正な UTF-8 が混ざり得る。`TextDecoder` を非 fatal にする。
* 大きなソースを URL に載せると長さの上限に当たる。`#code=` は 8 KB 程度まで、それ以上は載せない。
* CodeMirror を cdnjs から読むなら、オフラインでは動かない。本の付属物として配るなら vendoring する。
* Pages の Worker は同一オリジンなので特別な設定は要らないが、`.wasm` の MIME（`application/wasm`）は GitHub Pages が正しく返す。`WebAssembly.instantiateStreaming` が使える。

## 4. 完了条件

1. Pages の URL で、編集欄に書いた Ruby が実行され、結果欄に `puts` の出力と例外が出る。無限ループを停止ボタンで止められる。
2. `fixtures/*.rb` 全部が Playground の wasm で本家の `.out` と一致する（Node のテストと、ページ上の比較ボタン）。
3. 命令列の表示とサンプル選択が動く。
4. wasm のサイズと初回読み込み時間を README に記録する。
5. `docs/playground.md` と本のノート §15 がある。
6. wasi-sdk の版、CodeMirror の版、browser_wasi_shim の版を README に固定して書く。

## 5. 実装者への注意

* 検証の基準は変えない: `sabiruby` の `tools/*.sh` と golden テストは Docker の本家 `mrbc` のまま。Playground は「同じ VM と同じコンパイラが wasm でも同じ結果を出す」ことを示す側。
* `sabiruby` 本体と `sabiruby-compiler` の変更は最小にする（wasm 向けの build.rs の調整程度）。変更したら理由を `docs/compiler.md` に残す。
* 手順 1 で方式が崩れたら、作業を止めて報告する（例: wasi-libc に無い関数、`cc` が clang を選べない）。代替は「コンパイルだけサーバに任せる」だが、それは著者判断。
