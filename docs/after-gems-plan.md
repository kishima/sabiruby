# gem 移植後の作業 実装指示書（2026-09-13）

対象: この文書だけを読んで、別セッションの実装者（AI）が次の 4 つの作業を順に進められること。
作業前に `README.md`、`docs/gems.md`（移植済み gem の全体像と差異）、`docs/gc.md`、`docs/gems-plan.md`（着手時の状況）を読むこと。
設計判断はここに書いたとおりにし、変えたい場合は理由を該当の docs に残す。

## 0. 前提と現状

* `default.gembox` の gem は POSIX 依存（io、socket、errno、dir、env、signal、process）を除いて全部移植済み。
  gembox 外の cmath、pack、eval、binding、proc-binding、require／load、task、UTF-8 文字列も済み。
  本家テストは 2484 件中 2313 件。落ちる件はすべて理由が付いている（`docs/mrbtest.md` の note 列、`tests/mrbtest/notes.tsv`、
  `docs/mrbtest-notes.md`）。エンジンの差異（regexp）、C 補助コード、NaN の同一性、`GC.generational_mode` が中身。
* リポジトリ: VM `kishima/sabiruby`（この repo）、Bevy 統合 `kishima/rubevy`（`../rubevy`）、Playground
  `../sabiruby-playground`、移植キット `../mruby-porting-kit`（本の repo `/home/kishima/book/book_mruby3` の
  `tools/build_kit.rb` が生成する。手で編集しない）。
* crates.io の `sabiruby` は 0.2.0（gem のほとんど無い版）。rubevy はそれに依存している（`sabiruby = "0.1"`）。
  新しい版を出すのは著者の操作（`cargo publish`）。それまで rubevy は `.cargo/config.toml` の `paths = ["../sabiruby"]` で
  ローカルの VM を使う。
* 完了の定義は gem のときと同じ: `tools/mrbtest.sh --update`（文字ビルド）と `--bytes`（バイトビルド）で baseline 更新、
  `cargo test --release`、`SABIRUBY_GC_STRESS=1` で触ったテストファイル、`tools/check_no_std.sh`、警告ゼロ。
  分かったことは `docs/*.md` と本の repo（`docs/notes/sabiruby-findings.md`、`data/porting_stages.yml`、`porting.re`）へ。
  push は著者。

## 1. 順序（決定）

```
A. 片付け（push の準備、キット再生成、文書の現状更新）
B. rubevy を Task 駆動にする
C. テストの残り 8 件（source_location 5、backtrace 2、MRUBY_REVISION 1）
D. mruby-sleep と mruby-strftime
```

A は半日、B が本体。C と D は B の合間に入れてよい小さな仕事。

## 2. A. 片付け

1. `docs/gems-plan.md` の冒頭「実装状況」と 0 節が古い（「残るは require、regexp → task」のまま）。`gc-plan.md` と同じく
   冒頭を「実装済み（2026-09-13）。説明と、この指示書から変えた点は `gems.md`」の 1 段落にし、0 節の数字を今の値に直す。
   `docs/gems.md` の「Remaining gems」表は POSIX の行だけになっているので、表の前の文を「残りは無い」に直す。
2. `README.md` の gem の一覧と数字（2313/2484 など）が最新か確かめる。`docs/gems.md` の Deviations kept に regexp の構文の表が
   あることを確かめる（無ければ `gems-plan.md` 3.6 の表を移す）。
3. 本の repo `book_mruby3`: `data/code/vm_enum_fiber.rb` が untracked。`vm.re` の `//list[vm_enum_fiber_rb]` が参照しているので
   add してコミットする（別セッションが作業中なら、そのセッションに任せる。`git log -1` の時刻で判断）。
   `porting.re` の各段階の「到達」に書いた数字（gem のテスト件数、通った件数）が古ければ、最終値
   （`docs/mrbtest.md` の表）に更新する。
4. 移植キット: `mrbtest/ref/codegen.txt`、`syntax.txt` が変更、`samples/cg_eval.rb` が untracked のまま。本の repo で
   `ruby tools/build_kit.rb` を走らせて再生成し（Docker が要る）、キット側で差分を確かめてコミットする。
   段階表（`stages.md`）に段階 7・8 の新しい罠が入っていることを確かめる。push は著者。
5. Playground: `.github/workflows` が SabiRuby のチェックアウトを SHA で固定している。regexp・task を含む最新の SHA に上げ、
   wasm のサイズを `docs/playground.md` の表に追記する（regexp の Unicode 表の差は記録済み: gzip で約 98 KB）。
6. `git log origin/main..HEAD` で未 push を数え、著者に伝える（sabiruby 3 件以上、本の repo、キット）。

## 3. B. rubevy を Task 駆動にする

### 3.1 今の形と、変える理由

今の `src/lib.rs`（196 行）は「`Script` コンポーネント 1 つに VM 1 つ」で、毎フレーム `Vm::step(budget)` で命令数の予算だけ進める。
`$frame`／`$delta` をグローバルに書き、出力をログに流す。ホスト呼び出しの API は無い。
mruby-task を入れた目的はこの形を変えること: **1 つの VM に複数のスクリプトを Task として載せ、優先度と `sleep` で回す**。
NPC ごとに Task を 1 つ持たせ、ほとんどのフレームは眠らせておける（`docs/outlook.ja.md`）。

### 3.2 設計（決定）

* **VM は 1 つ**（`Resource`、`ScriptWorld { vm: Vm }`）。スクリプト同士がグローバルと定数を共有するのは仕様とする
  （分離が要る用途は将来 `Vm` を複数持つ。`Script` に `isolated: bool` を足す余地を残す）。
* **`Script` は Task になる**。アセット（`.mrb`）が届いたら、その irep をトップレベルとして走る Task を作り、`ScriptTask { task: ObjId }`
  を entity に付ける。Task の生成は VM 側に **ホスト用の入口** `Vm::task_spawn(irep: IrepId, priority: u8, name: Option<&str>) -> VmResult<ObjId>`
  を足す（`ext_task.rs` の `Task.new` が Ruby のブロックから作る手順を、irep のトップレベル Proc から作る形にしたもの。
  `Task.current` の扱い、`gc_register` で entity が持つ間は根にすること、`Task#close` で解放することまで含める）。
* **フレームが tick**。`vm.task.tick_every = 0`（命令数の tick を止める）にし、`Update` の system で毎フレーム
  1 回 `Vm::task_run_once()` を呼ぶ（ready な Task を順に 1 スライスずつ走らせて戻る）。時間は `wall_clock` ではなく Bevy の
  `Time` から与える: `Vm::task_advance_ticks(n)`（`tick` を n 進めて deadline を過ぎた sleep を ready に戻す）を足し、
  `delta_secs` を `MRB_TICK_UNIT`（ms）で割った分だけ進める。`sleep(0.5)` が「約 0.5 秒後のフレーム」で起きる。
  1 フレームの上限は命令数の予算（`Vm::step` と同じ仕組みを `task_run_once` にも渡す。`Script::budget` は全体の予算に変える）。
* **ホスト呼び出し**は Ruby から見えるモジュール `Rubevy`（`Rubevy.spawn`、`Rubevy.log`、`Rubevy.entity`）をネイティブで定義する。
  Bevy の `World` へは system の外から触れないので、ネイティブは **コマンドを VM 側のキュー**（`Vec<HostCommand>`）に積むだけにし、
  system が `task_run_once` の後で取り出して `Commands` に流す。Ruby 側からの読み取り（位置など）はフレーム頭にグローバルへ
  書く（今の `$frame`／`$delta` の方式を `$rubevy_state` の Hash に広げる）。読み書きの型は `docs/outlook.ja.md` の
  「エンティティが Ruby のオブジェクトになる」節に合わせる。
* **`require`**: `Host` trait（`src/host.rs`）を rubevy が実装し、`read_file` は Bevy のアセットから読む
  （`.mrb` はそのまま、`.rb` はコンパイラがあるビルドだけ。`sabiruby-compiler` の `host` feature を任意依存にする）。
* **終了と例外**: Task の終了は `Task#value` で取れる。例外は Task の結果になる（`exception_as_result`）ので、
  system が終了した Task を見つけて `ScriptEnded` を出す。`Task#value` の読み方は `ext_task.rs`。
* **GC**: `GC.scheduler_driven` を on にし、idle のときに回収させる（`docs/gc.md` の「スケジューラ駆動」）。
  `gc_register` した Task の ObjId は entity の despawn で `gc_unregister`。

### 3.3 手順

1. VM 側: `Vm::task_spawn`、`Vm::task_advance_ticks`、`task_run_once` の予算引数。`tests/` に host 駆動のテストを 1 つ
   （2 つの Task が交互に進み、`sleep` が tick で起きること）。`docs/gems.md` の task の項に追記。
2. rubevy: `ScriptWorld` resource、`Script` → `ScriptTask`、system を `start_scripts`／`tick_scripts`／`drain_commands` の 3 つに。
   `examples/headless.rs` を Task 2 つ（片方は `sleep` で眠る）に書き換え、出力で確かめる。
3. `Rubevy` モジュールの最小 API（`log`、`spawn`、`entity` の位置の読み書き）と、その `docs/outlook.ja.md` の該当節を「済み」に。
4. rubevy の `README` と `docs/outlook*.md` を更新。Bevy は 0.19.1 のまま。
5. 本の repo `docs/notes/rubevy-design.md` に設計の変更点（VM 1 つ、Task、コマンドキュー、tick の与え方）を追記。
   本文（`.re`）に Rust／Bevy の名前は書かない。

## 4. C. テストの残り 8 件

### 4.1 `Proc#source_location`、`Method#source_location`、`Binding#source_location`（skip 5 件）

* 本家 `mrb_proc_source_location`: C 関数の Proc は nil、alias は `upper` をたどり、irep の先頭（pc 0）の行番号とファイル名の組。
  SabiRuby は `VmIrep.filename` と `line_of(pc)` を持っているので、`ext_proc.rs` の `source_location`（今は nil 固定）を
  `[filename, line_of(0)]` にする。DBG の無い irep は nil。
* `Method#source_location`（`ext_method.rs`）は `_proc` の Proc で同じことを。ネイティブは nil。
* `Binding#source_location`（`ext_binding.rs`）は `binding` を呼んだ**位置**（呼び出し元フレームの `pc - 1` の行）。
  テストは `[__FILE__, __LINE__]` と比べる。
* `gem_method`、`gem_proc`、`gem_binding_binding`、`gem_proc_binding` の skip が外れる。

### 4.2 例外の backtrace（skip 2 件）

* `Exception#backtrace` は今 `@__raised` を見て空を返す。`raise` の時点（`vm.rs` で `@__raised` を立てている箇所）で
  `Vm::backtrace(None)` に相当する**圧縮した記録**（フレームごとの `(irep, pc, mid)` を Integer の Array にした隠し ivar `@__bt`）を
  取り、`backtrace` が呼ばれたときに文字列に展開する。文字列を `raise` のたびに作らない（テストは例外を制御に多用する）。
  すでに `@__bt` がある例外（再 raise）は上書きしない（本家 `mrb_keep_backtrace` も既にあれば残す）。
* 形式は `Vm::backtrace` と同じ（`file:line:in method`、ブロックのフレームは `:in` 無し）。テストの期待は
  `"#{__FILE__}:#{line}"` が先頭（ブロックの中の raise）。`set_backtrace` は与えられた配列をそのまま持つ。
* `exception.rb` の 2 件（`GC in rescue`、`Method call in rescue`）が通る。`backtrace_available?` は先頭に `unknown` が
  無いことを見るので、DBG の無い irep のフレームは `Vm::backtrace` と同じく飛ばす。

### 4.3 `MRUBY_REVISION`（skip 1 件）

* `build.rs`（VM crate）で `git rev-parse --short HEAD` を試し、`cargo:rustc-env=SABIRUBY_REVISION=…` に入れる。git が無ければ
  `HEAD`。`object.rs` の定数を `option_env!` で読む。crates.io からのビルドは git が無いので `HEAD` のまま（本家も既定は `HEAD`）。
  テストが期待するのは「`HEAD` でない」ことなので、開発ツリーでは通る。

## 5. D. mruby-sleep と mruby-strftime（gembox 外、小さい）

### 5.1 mruby-sleep（186 C / 29 test）

* `Kernel#sleep(sec)`、`Kernel#usleep(usec)`。負は `ArgumentError: time interval must not be negative`。`sleep` の戻り値は
  経過した秒（整数）。
* Task の中の `sleep` は ext_task の task-aware な `sleep` が既にある（`docs/gems.md`）。この gem が足すのは **Task の外**の
  `sleep`。no_std の VM は待てないので、ホストの差し込み口 `Vm::sleep_hook: Option<fn(micros: u64)>` を足し（`wall_clock` と同じ流儀）、
  CLI は `std::thread::sleep` を渡す。差し込み口が無ければ待たずに戻る（戻り値は `wall_clock` の差、無ければ 0）。
  ext_task の `sleep` と同じ名前になるので、登録順（task の後に sleep）と「Task の中では task 版、外ではこの版」の分岐を
  1 つの関数にまとめる。
* テストは `tools/mrbtest.sh` の `GEMS` に足すだけ。`sleep(1)` が本当に 1 秒待つ（CLI 経由）。

### 5.2 mruby-strftime（118 C / 152 test、17 件）

* `Time#strftime(fmt)`。本家は libc の `strftime` に渡す。SabiRuby は `ext_time.rs` の `gmtime` の結果から自前で書く。
  対応する変換: `%Y %C %y %m %B %b %h %d %e %j %H %I %M %S %p %A %a %w %u %Z %z %F %T %D %R %c %x %X %n %t %s %%`。
  `%c`／`%x`／`%X` は C ロケール（`Sun Sep 13 08:11:32 2026`、`09/13/26`、`08:11:32`）。`%Z` は `UTC`、`%z` は `+0000`（time の決定どおり）。
  glibc の `%-d`（先頭ゼロなし）、`%_d`（空白詰め）、`%0d` の旗も受ける。知らない変換は glibc と同じくそのまま出す。
* 引数の NUL: 本家は NUL で区切って `strftime` に渡し、NUL をそのまま出力に足す（`"%Y\0"` → `"2026\0"`）。同じにする。
  文字列以外は TypeError（`mrb_get_args "s"`）。
* 本家イメージと `Time.gm(2026,9,13,8,11,32).strftime(...)` を各変換で照合し、スクリプトを `docs/gems.md` に残す。

## 6. 記録

* 各作業の終わりに `docs/gems.md`（gem）、`docs/gc.md`（scheduler_driven を使うなら）、rubevy の docs、
  本の repo の findings（節 19 以降）と `porting_stages.yml`／`porting.re` の段階 8（task）へ。
* この文書は着手時に「実装済み」の注記を頭に足す（`gc-plan.md` と同じ運用）。
