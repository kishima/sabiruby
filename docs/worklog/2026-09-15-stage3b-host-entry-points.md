# 作業記録: ホストが使う入口を VM に足す（段階 3b、sabiruby 側）

`docs/plans/host-bridge-plan.md` の段階 3b。rubevy が `Vm` の公開フィールド（`heap` / `task` / `globals`）に
直接触っている 4 か所を無くすために、VM 側に相当する公開関数を足した。ブランチは `host-entry-points`、
worktree は `sabiruby-wt-host`。rubevy 側の置き換えは rubevy の
`docs/worklog/2026-09-15-stage3b-no-internal-access.md` に別に書いた。

ベンチは回していない（段階 2d の担当が同じ機械で `tools/bench.sh --core 2` を走らせている最中だった。
`pgrep -af "sabiruby|cargo|mruby|bench"` で確認した）。段階 3b はホットパスに触らないので計画書も
「ベンチは不要」としている。ただし本家テストの基準を見るには release ビルドが要るので、
そのビルドだけは相手のベンチが終わるのを待ってから回した（後述）。

## 0. 何が「内部アクセス」だったのかを先に確かめる

rubevy 側の記録（`docs/worklog/2026-09-15-host-state-and-data.md` 2.7 節）が、残した 4 か所と残した理由を
挙げている。それを rubevy の今のコードと突き合わせた（`grep -n "vm.heap\|vm\.task\|vm\.globals" src/`）:

* `src/lib.rs:535` `vm.heap.ivar_set(task, k, Value::Int(entity.to_bits() as i64))` — 起動したタスクに
  「このスクリプトが動かすエンティティ」を持たせる。
* `src/lib.rs:806-808` `vm.task.running` と `vm.heap.ivar_get(task, k)` — `Rubevy.entity` が
  「いま走っているタスク」から上のエンティティを読む。
* `src/lib.rs:610` `vm.globals.insert(n, Slot::from(h))` — `$rubevy` を毎フレーム置く。
* `src/lib.rs:595` `matches!(vm.heap.get(o).kind, ObjKind::Exception)` — 終わったタスクの結果が例外か。

この 4 つは `unsafe` ではない（`Vm` のフィールドが `pub` なので合法）が、VM の内部構造への依存ではある。
`heap` / `globals` / `task` が `pub` なのは crate の中で複数のモジュールが触るためで、外から使う面として
設計されたものではない。段階 6（マクロ）が生成するコードがこれを真似ると困るので、ここで塞ぐ。

## 1. `&self` か `&mut self` か — `intern` が `&mut self` を要る問題

計画書は `ivar_get(&self, obj, name: &str) -> Value` と書いているが、名前から `Sym` を得る `Vm::intern`
（`src/vm.rs:864`）は `Interner::intern_str` を呼ぶので `&mut self` を要る。指示は「`&mut self` にしてよい
（理由を rustdoc に）」だったが、先に `&self` のまま書けないかを見た。

`Heap::ivar_get`（`src/object.rs:865`）は `ivars: Vec<(Sym, Slot)>` を `Sym` の一致で探す。globals も
`HashMap<Sym, Slot>` で、鍵は `Sym`。つまり**まだ誰も intern していない名前は、定義上どのインスタンス変数の
名前でもグローバルの名前でもない**。名前を登録せずに「既にある `Sym` だけ引く」ことができれば、
読み出しは `&self` で書けて、しかも意味は完全に同じ（どちらも nil を返す）になる。

`Interner`（`src/symbol.rs`）は `names: Vec<Box<[u8]>>` と `index: HashMap<Box<[u8]>, Sym>` を持っていて、
`intern` は既にある名前をこの `index` から引いている。そこに 3 行の `lookup_str(&self, &str) -> Option<Sym>` を
足すだけで済んだ。`ivar_get` / `global_get` は `&self` のまま、`ivar_set` / `global_set` は `intern` するので
`&mut self`。

`&mut self` を避けたのは形を揃えるためだけではない。(1) 読むだけの入口が `&mut Vm` を要ると、ホストが
`&Vm` しか持っていない場所（rubevy の `is_exception(vm: &Vm, ...)` がまさにそれ）から呼べない。
(2) 読むだけの呼び出しがシンボル表を増やすのは、毎フレーム動的な名前で読むホストに対して漏れになる。
この理由は rustdoc に 1 文で書いた。

## 2. 足した 6 本

`src/vm.rs` の `task_finished` の直後に `// --- host entry points` という節を作り、6 本を置いた。
`task_running` を `task_*` の並びの隣に置いたのは、名前の規則がそちらだからで、節の頭のコメントに
「`task_running` は上の `task_*` の一員」と書いてある。中身はどれも既存の内部関数を包むだけで、
条件も分岐も足していない:

| 関数 | 包んだもの |
|---|---|
| `task_running(&self) -> Option<ObjId>` | `self.task.running` |
| `ivar_get(&self, ObjId, &str) -> Value` | `syms.lookup_str` + `Heap::ivar_get` |
| `ivar_set(&mut self, ObjId, &str, Value)` | `intern` + `Heap::ivar_set` |
| `global_get(&self, &str) -> Value` | `syms.lookup_str` + `globals.get` |
| `global_set(&mut self, &str, Value)` | `intern` + `globals.insert` |
| `is_exception(&self, Value) -> bool` | `matches!(heap.get(o).kind, ObjKind::Exception)` |

rustdoc には全部に「ホストが使う入口」であることと、なぜ Ruby 側の道（`instance_variable_get`、
`is_a?(Exception)`）ではなくこれなのかを書いた。`is_exception` の理由がいちばんはっきりしている:
`v.is_a?(Exception)` を `funcall` で聞くと、値を分類するだけのために Ruby のメソッド呼び出しが 1 回走り、
`is_a?` が再定義されていればそれが答えることになる。表現を直接見るこちらは失敗もしないし VM に再入もしない。

`global_get` / `global_set` には `$~` の但し書きを付けた。`$~` は「仮想グローバル」で
（`mrb_gv_define_virtual`、`Op::Getgv` が `self.s.backref` を特別扱いする `src/vm.rs:3003`）globals の表には
無いので、この入口からは見えない。

`is_exception` が「Exception の子孫か」を正しく答えるかは確かめた。`instance_alloc`
（`src/builtins/mod.rs:150`）が祖先を遡って最初に見つけた `InstanceKind` で表現を決めるので、
`Exception` の下にあるクラスの `new` は `ObjKind::Exception` を持つ。テストで
`Class.new(RuntimeError).new` まで含めて見ている。

`set_load_path`（`src/vm.rs:841`）が `self.globals.insert(n, Slot::from(ary))` と書いていた 3 行を
`self.global_set("$LOAD_PATH", ary)` に替えた。VM 自身が新しい入口を通ることで、それが既存の書き方と
同じものだと示せる（差分の意味は変わらない）。`run_loop_ctx`（`:2705`）の同じ `matches!` は
`if let Value::Obj(o)` の中で `o` を後で使っているので、そのままにした。

## 3. テスト（`tests/host_api.rs`、5 本）

包んだだけの関数なので、確かめる値があるのは「包んだ先と同じ意味か」ではなく「**Ruby から見た同じものか**」の方。
`ivar_set` で書いたものを `instance_variable_get` が読み、Ruby が `instance_variable_set` したものを
`ivar_get` が読む、という往復を両方向で見ている。globals も同じ形で、加えて `$kept` に入れた配列が
`gc_collect` を跨いで生きること（globals の表が根であること）を見た。

`task_running` は、rubevy が実際にやっている形をそのまま小さくした:

```rust
vm.define_closure(object, "whose_task", |vm, _s, _a, _b| {
    Ok(match vm.task_running() { Some(t) => vm.ivar_get(t, "@entity"), None => Value::Nil })
});
```

タスクを 2 つ起こして片方に `@entity = 11`、もう片方に `22` を持たせ、どちらのスクリプトも
`p whose_task` と書くだけで自分の番号を出すこと、スケジューラの外（ホストが直接走らせた
`load_and_run`）では同じネイティブが `nil` を返すこと、走り終わった後に `task_running()` が `None` に
戻ることを見ている。出力は `11\n22\n`。

`is_exception` は、正例（`RuntimeError.new`、`Exception.new`、`Class.new(RuntimeError).new`、
`rescue` が捕まえた `ZeroDivisionError`）と負例（`nil`、整数、文字列、`Object.new`、
**クラスオブジェクトの `RuntimeError` そのもの**）を並べたうえで、`is_a?` を `true` に再定義した
プレーンなオブジェクトが偽のままであること、逆に `is_a?` を「呼ばれたら数える」クロージャに
差し替えた `RuntimeError` のインスタンスでカウンタが 0 のままであることを見た。後者が
「Ruby を走らせない」の実際の確認になっている。

5 本とも 1 回目で通った。通ってしまうと「何も試していない」ことになりかねないので、
`is_exception` の負例に `RuntimeError`（クラス）と再定義した `is_a?` を、`ivar_get` の負例に
「一度も intern されていない名前」を入れて、1 節の判断（`&self` で足りる）が本当に効いていることを
見えるようにした。

## 4. 確認

* `cargo test --workspace`: 137 passed / 0 failed（doctest 11 本を含む）。`tests/host_api.rs` の 5 本が新規。
* `tools/check_no_std.sh`: `no_std OK`。
* `cargo doc --no-deps`: 警告なし。
* `grep -rn unsafe src/ tests/`: Rust の `unsafe` は 0（当たった 1 件は `tests/mrbtest/src/gem_regexp_syntax.rb` の
  コメントの英単語）。
* 本家テスト: `tests/mrbtest/baseline.txt` の 108 ファイルを `./target/release/sabiruby mrbtest` で回し、
  passed 列を比べた（結果は 5 節）。
