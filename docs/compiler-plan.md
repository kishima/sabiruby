# コンパイラ組み込み指示書（案 A: 本家のコンパイラを C のままリンクする）

対象: この文書だけを読んで、別セッションの実装者（AI）が `sabiruby run foo.rb` を単体で動くようにできること。
方針は著者決定（2026-09-11）: **コンパイラは本の主題ではないので、Rust に移植せず、本家 mruby 4.1.0-rc の `mruby-compiler`（Prism + codegen）を C のままビルドしてリンクする**。
出るバイトコードは本家 `mrbc` と 1 バイトも違わないこと（それが検証方法）。作業前に `README.md` の Rules と Verification を読むこと。

## 0. 前提（調べて確かめた事実）

* 本家 4.1.0-rc のコンパイラは `ref/mruby/mrbgems/mruby-compiler`（`../../ref/mruby` は本家のクローン、コミット `3cf73ee`、タグ 4.1.0-rc）。
  構成: `include/*.h`（`mrc_*.h`、`prism_xallocator.h`、`mruby_compiler.h`）、`src/*.c`（ccontext、cdump、codedump、codegen、compile、debug、diagnostic、dump、irep、mrc_presym、mruby_compat、parser_util、pool）、
  `lib/prism/`（Prism 1.9.0 の git submodule。`src/*.c`、`src/util/*.c`、`include/`）。
* Prism の一部は **ERB テンプレートから生成される**（`templates/template.rb`）。生成物は本家のビルドが `ref/mruby/build/prism/` に置いている:
  `include/prism/ast.h`、`include/prism/diagnostic.h`、`src/{diagnostic,node,prettyprint,serialize,token_type}.c`。**これを vendoring する**（Ruby でテンプレートを再生成しない）。
* **本家 `mrbc` は mruby 本体無しでビルドされている**（`lib/mruby/build.rb` の `generate_mrbc_build`: `disable_libmruby`）。
  `include/mrc_common.h` は `MRC_TARGET_MRUBY` も `MRC_TARGET_MRUBYC` も定義されていないとき「standalone mrbc」経路になり、`mrb_state` を `void` にし、`<mruby.h>` ではなく `<mrbconf.h>`（250 行、自己完結）だけを読む。
  アロケータは libc（`prism_xallocator.h` の最後の `#else`）。`mrb_intern` などを呼ぶ箇所はすべて `#if defined(MRC_TARGET_MRUBY)` の中。
  → **`MRC_TARGET_*` を定義しなければ、Rust の `cc` crate で本体無しにビルドできる**。
* 本家 `mrbc` のサブビルドが実際にコンパイルしているオブジェクト（`ref/mruby/build/host/mrbc/mrbgems/mruby-compiler/` で確認）:
  `src/` の 12 個（`mruby_compat.c` を除く。ただしそれは `MRC_TARGET_MRUBY` 無しでは空になるので入れても害はない）、
  `lib/prism/src/` の `diagnostic encoding node options pack prettyprint prism regexp serialize static_literals token_type`（生成物 5 つを含む）、`lib/prism/src/util/pm_*.c` 10 個。
* コンパイルの define（`mrbgem.rake` から）: `PRISM_XALLOCATOR`、`PRISM_DEPTH_MAXIMUM=256`、`PRISM_BUILD_MINIMAL`。`MRC_NO_STDIO` は**定義しない**（`mrc_parse_string_cxt` が `filename_table` を使うのは stdio 有りのときだけ）。
  `MRC_INT32` を定義しなければ `mrbconf.h` の既定 `MRB_INT64` で、SabiRuby（64 ビット整数）と一致する。
* 公開 API（`include/mrc_compile.h`、`mrc_dump.h`、`mrc_ccontext.h`、`mrc_irep.h`）:
  `mrc_ccontext *mrc_ccontext_new(NULL)`、`mrc_ccontext_filename(c, "name.rb")`、`c->no_ext_ops`／`c->no_optimize`／`c->keep_lv`（ビットフィールド）、
  `mrc_irep *mrc_load_string_cxt(c, &src, len)`（失敗時 NULL。理由は `c->diagnostic_list`: `code`、`message`、`filename`、`line`、`column` の連結リスト）、
  `mrc_irep_remove_lv(c, irep)`、`int mrc_dump_irep(c, irep, flags, &bin, &size)`（`MRC_DUMP_DEBUG_INFO`=1 で DBG と LVAR を含める。戻り値 `MRC_DUMP_OK`=0。`bin` は `mrc_malloc`=libc `malloc` 確保）、
  `mrc_irep_free(c, irep)`、`mrc_ccontext_free(c)`。`mrbc.c`（`ref/mruby/mrbgems/mruby-bin-mrbc/tools/mrbc/mrbc.c`）が使い方の実例。
* ライセンス: mruby-compiler は MIT（mruby and PicoRuby developers）、Prism は MIT（Shopify）。両方の LICENSE を vendoring に含める。

## 1. 設計（決定事項）

### 1.1 crate 構成

* リポジトリ内に **別 crate `sabiruby-compiler`** を作る（`compiler/` ディレクトリ、Cargo workspace のメンバ）。`sabiruby` 本体には依存しない。std 前提（libc をリンクする）。
  `sabiruby` の lib は no_std のまま。**`sabiruby` の bin だけ**が feature `compiler`（既定オン）で `sabiruby-compiler` に依存する。
  `Cargo.toml` の `[features] default = ["std", "compiler"]`、`compiler = ["dep:sabiruby-compiler"]`。lib の `no_std` 検査（`tools/check_no_std.sh`）は `--no-default-features` なので影響しない。
* 公開 API（Rust、安全な関数だけ）:
  ```rust
  pub struct Options { pub filename: String, pub debug_info: bool, pub remove_lv: bool, pub no_ext_ops: bool, pub no_optimize: bool }
  pub struct Diagnostic { pub kind: Kind /* ParserWarning, ParserError, GeneratorWarning, GeneratorError */, pub message: String, pub filename: String, pub line: u32, pub column: u32 }
  pub struct CompileError { pub diagnostics: Vec<Diagnostic> }
  pub fn compile(src: &[u8], opts: &Options) -> Result<Vec<u8>, CompileError>;   // RITE バイナリ
  pub fn version() -> &'static str;   // "mruby 4.1.0-rc (3cf73ee), Prism 1.9.0"
  ```
  `Options::default()` は `filename: "-e"`、`debug_info: false`、他 false。
* **bindgen は使わない**。C のビットフィールド構造体に Rust から触らないため、crate 内に **C のシム** `compiler/csrc/shim.c` を置く:
  ```c
  int sabiruby_mrc_compile(const uint8_t *src, size_t len, const char *filename, unsigned flags,
                           uint8_t **out, size_t *out_len, char **diag /* "code\tline\tcol\tfile\tmessage\n"... */);
  void sabiruby_mrc_free(void *p);
  const char *sabiruby_mrc_version(void);
  ```
  シムが `mrc_ccontext_new(NULL)` → `mrc_ccontext_filename` → フラグ設定 → `mrc_load_string_cxt` → 失敗なら `diagnostic_list` を 1 本の文字列に詰めて返す → 成功なら `remove_lv` 任意 → `mrc_dump_irep` → 後始末、まで全部やる。
  Rust 側は `extern "C"` 宣言 3 つと、バイト列のコピーと文字列の分解だけ。`unsafe` はこの 1 ファイル（`ffi.rs`）に閉じ込める。

### 1.2 vendoring

* `compiler/vendor/mruby-compiler/`: 本家 `mrbgems/mruby-compiler/{include,src,LICENSE,README.md}` をそのままコピー（`lib/prism` は別に置く）。
* `compiler/vendor/prism/`: `lib/prism/{include,src,LICENSE.md}` と、**生成物** `ref/mruby/build/prism/{include,src}` を `generated/` として。
* `compiler/vendor/mrbconf.h`: 本家 `include/mrbconf.h`。
* `compiler/vendor/VENDOR.md`: 由来（mruby コミット `3cf73ee`＝4.1.0-rc、Prism 1.9.0、生成物は本家ビルドの `build/prism`）、コピーした日、更新手順（`tools/vendor_compiler.sh` を書き、`../../ref/mruby` からコピーし直す。生成物は本家で `rake` を一度回してから）。
* コピーは**改変しない**。必要な変更はすべてシムと build.rs 側で吸収する。パッチが必要になったら `VENDOR.md` に差分を記録する。
* `cargo package` の 10 MB 制限に注意（Prism の src は約 1.5 MB、生成物込みで 3 MB 前後。`Cargo.toml` の `exclude` で `README`、テスト、Doxyfile 類を外す）。

### 1.3 build.rs

* `cc` crate で C としてビルド（C++ ではない。Prism は C99 の designated initializer を使う）。
  ```rust
  cc::Build::new()
      .files(mruby_compiler_src /* src/*.c */)
      .files(prism_src /* src/*.c, src/util/*.c */)
      .files(prism_generated_src /* generated/src/*.c */)
      .file("csrc/shim.c")
      .include("vendor/mruby-compiler/include")
      .include("vendor/prism/include")
      .include("vendor/prism/generated/include")
      .include("vendor")            // mrbconf.h
      .define("PRISM_XALLOCATOR", None)
      .define("PRISM_DEPTH_MAXIMUM", "256")
      .define("PRISM_BUILD_MINIMAL", None)
      .std("c99").warnings(false)
      .compile("sabiruby_mrc");
  ```
  `MRC_TARGET_MRUBY`／`MRC_TARGET_MRUBYC`／`PICORB_VM_*`／`MRC_NO_STDIO`／`MRC_INT32` は**定義しない**。
* `cargo:rerun-if-changed=vendor`、`csrc`。
* 対応する target: `x86_64/aarch64` の linux、macOS、windows-msvc（`cc` が MSVC を使う。Prism は MSVC でもビルドできる。確認が取れなければ CI で `windows-latest` を落として README に書く）。
  wasm32 は**対象外**（clang と wasi-sdk が要る。`docs/compiler.md` に「将来」と書く）。

### 1.4 CLI（`src/bin/sabiruby.rs`）

* `sabiruby run FILE`: 先頭 4 バイトが `RITE` なら今までどおり、そうでなければ Ruby ソースとしてコンパイルして実行（`filename` は FILE、`debug_info: true` にして行番号を残す。SabiRuby は DBG を読まないが LVAR は `local_variables` に要る）。
* `sabiruby -e 'CODE'`: ソースを直接。
* `sabiruby compile FILE [-o OUT.mrb] [-g] [--remove-lv] [--no-ext-ops] [--no-optimize]`: `mrbc` 互換。`-o` 省略時は拡張子を `.mrb` に。
* `sabiruby dump FILE`: `.rb` も受け付ける（コンパイルしてから既存の dump）。
* `sabiruby mrbtest`: 変更しない（`.mrb` 前提のまま。テストのコンパイルは引き続き Docker の本家 mrbc。**検証の基準を動かさない**）。
* コンパイルエラーは `FILE:LINE:COL: message` の形で stderr に出し、終了コード 1。`mrbc` と同じ形式に寄せる。

### 1.5 変えないもの

* `tools/mrbtest.sh`、`tools/fixtures.sh`、`tools/bench.sh` は Docker の本家 `mrbc` を使い続ける。理由: 検証の基準は本家のバイナリであること。
  組み込んだコンパイラが本家と同じことは §2 のテストで示す。
* `sabiruby` lib の API と no_std。

## 2. 検証

1. **バイト一致テスト**（必須、`compiler/tests/golden.rs`）: リポジトリにある本家 `mrbc` 生成物と比べる。
   * `tests/fixtures/*.rb` → `tests/fixtures/*.mrb`（`-g` 無し）: `compile(src, filename: "<元のパス>", debug_info: false)` の結果が `.mrb` と **完全一致**。
   * `tests/mrbtest/src/*.rb` → `tests/mrbtest/*.mrb`（`-g` 有り）: `debug_info: true` で完全一致。
   * 注意: DBG セクションはファイル名を含むので、`filename` を本家が使ったのと同じ文字列（`tools/mrbtest.sh` は `/w/src/NAME.rb`、`tools/fixtures.sh` は各自確認）にする。一致しないときは、まず DBG のファイル名の違いを疑う。
   * 生成物が無い .rb（`bench/src/*.rb`）は本家で作ってから加える。
2. エラー: `compile(b"def", ..)` が `Err` で、`diagnostics[0].kind == ParserError`、`line == 1`。`x = 1 +` も。警告だけ（`Warning`）の場合は `Ok`。
3. CLI: `sabiruby run tests/fixtures/hello.rb` の出力が `tests/fixtures/hello.out` と一致。`sabiruby -e 'p 1+2'`。`sabiruby compile tests/fixtures/hello.rb -o /tmp/h.mrb` が `tests/fixtures/hello.mrb` と一致。
4. `cargo test --release`（既存の 16 本と回帰検査）、`tools/mrbtest.sh`（1185/1227 のまま）、`tools/check_no_std.sh`。
5. CI（`.github/workflows/ci.yml`）: `cargo test --workspace`（コンパイラの golden テストを含む）。`ubuntu-latest` に加えて `macos-latest` を足す。windows は §1.3 の判断次第。
6. ビルド時間: clean build で Prism 込みの C コンパイルにかかる時間を README に書く（目安を持つため）。

## 3. 踏みやすい点

* 生成物の `.c` は `#line` 指令で `prism/templates/...` を指す。ビルドには無関係だが、警告が出たら `warnings(false)` で黙らせる（改変しない）。
* `mrc_dump_irep` の `bin` は `malloc` 確保。Rust 側で `Vec` にコピーしたら `sabiruby_mrc_free` で返す。`diag` 文字列も同じ。
* `mrc_ccontext_filename` は文字列を**コピーせず保持する**可能性がある。シムの中で `strdup` した文字列を `mrc_ccontext_free` の後に解放する（`ccontext.c` を読んで確かめる）。
* Prism の再帰下降パーサはホストのスタックを使う。`PRISM_DEPTH_MAXIMUM=256` は本家と同じ値。深い入れ子は `ParserError` になる。SabiRuby の CLI は既に大きなスタックのスレッドで動くので問題ない。
* `no_optimize`／`no_ext_ops` を立てると本家 `mrbc` の同じオプションと一致するはずだが、golden テストは既定オプションだけで十分。
* `codedump.c`（`--verbose` の命令一覧）は stdout に書く。シムから呼ばない（SabiRuby には自前の `dump` がある）。
* Windows: `mrbconf.h` は `_MSC_VER` を見る。`cc` が MSVC を使うとき `-std=c99` は無視されるが問題ない。
* `cargo publish` するなら `sabiruby-compiler` を先に、次に `sabiruby`（bin の依存）。crates.io の 10 MB 制限（§1.2）。

## 4. 文書

* `compiler/README.md`（英語）: 何を vendoring しているか、由来、ライセンス、API、対応環境、wasm が対象外の理由。
* `README.md`: Usage に `sabiruby run foo.rb` と `-e` を追加。Status に「compiler: the reference mruby-compiler (Prism) linked as C, in the `sabiruby-compiler` crate; the VM crate stays free of it」。
* `docs/compiler.md`（英語）: 設計と、本家 `mrbc` サブビルドと同じ standalone 経路を使っていること、バイト一致テストのこと。
* 本のリポジトリ `../../book_mruby3/docs/notes/sabiruby-findings.md` に §14 を足す（日本語）。特に本の第 5 章（`codegen.re`）に足せる事実:
  「`mruby-compiler` は `MRC_TARGET_*` を定義しなければ mruby 本体無しでビルドできる（本家 `mrbc` がその形で作られている。`mrc_common.h` の standalone 経路、`disable_libmruby`）」
  「Prism の一部は ERB 生成で、本家ビルドは `build/prism/` に置く」「`mrc_dump_irep` はメモリに書ける」。本文（`.re`）は触らない。

## 5. 完了条件

1. `cargo run -- run tests/fixtures/hello.rb` が動く。`cargo run -- -e 'p 1'` も。
2. golden テストで fixtures と mrbtest の全 `.rb` がバイト一致。
3. `cargo test --workspace`、`tools/mrbtest.sh`（回帰なし）、`tools/check_no_std.sh` が通る。CI が通る。
4. §4 の文書がある。
5. コミットは「vendoring」「crate と golden テスト」「CLI と文書」の 3 段階以上に分け、push まで。
