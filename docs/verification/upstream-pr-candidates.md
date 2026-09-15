# Upstream candidates: what the port found in mruby-task

Found while porting mruby-task (`mrbgems/mruby-task` of mruby 4.1.0-rc, `3cf73ee`) and while
driving its scheduler from hosts it was not exercised on: a browser event loop and a game frame
loop. Each item says what the reference does, where, how to see it, and what a patch would be.
None of them is reported yet; this file is the queue.

The port's own behaviour and the differences it keeps are in [`../design/gems.md`](../design/gems.md); how far the
port may drift from the reference is the section "How far mruby-task may drift" there.

## 1. The scheduler re-enters the running task when a host drives it a step at a time

**Severity**: crash (the run loop reaches a context with no frames).

**Where**: `src/task.c`. `mrb_task_run` (678) guards nesting with `mrb->task.loop_running`, which
only `Task.run`'s own loop sets. `mrb_task_run_once` (703) — the entry point whose comment says
"Single-step task execution for WASM event loop integration" — does not set it. `task_run_body`
(608) and `task_run_one_iteration` (1054) then take `q_ready_` unconditionally, and a task that is
running is still at the head of the ready queue — `q_get_queue` (202) sends both READY and RUNNING
to `q_ready_`. `execute_task` (434) has no guard either: it
does `t->c.prev = mrb->c` where `mrb->c` is already `&t->c`, and re-enters the context it is
standing in.

**To see it**: drive the scheduler with `mrb_task_run_once` in a loop (a host event loop, one call
per frame), and run a program that ends in `Task.run` — which is how a program written for the
normal mode looks:

```ruby
Task.new { 2.times { Task.pass } }
Task.run          # inside a task under a host driver: picks itself
```

**Suggested patch**: answer nil where the caller is a task, the way the flag already does for the
nested case — in `mrb_task_run`, before setting `loop_running`:

```c
if (mrb->c != mrb->root_c) return mrb_nil_value();   /* the scheduler is already running us */
```

and, as a belt-and-braces guard in `execute_task` / `task_run_body`, skip a task whose context is
`mrb->c`. With that, a program written for `Task.run` runs unchanged under a host driver: the call
does nothing and the outer loop keeps going. (This is what the port does; `tests/task.rs`,
`the_scheduler_is_not_re_entered_from_inside_a_task`.)

## 2. `Task#join` answers the result as it stood *before* the wait

**Severity**: wrong value, silently.

**Where**: `mrb_task_join` (1365). It parks the caller (`MRB_TASK_REASON_JOIN`), raises
`switching_`, and then `return t->result;` (1398). The VM stores that value in the caller's
register before the context switch, so a join that actually waited answers the result as it was at
the call — `nil` — and only a join on an already dormant task (the early return at 1382) answers
anything useful. The intent of `return t->result` is clearly the opposite.

**To see it**:

```ruby
a = Task.new { sleep_ms(50); :done }
b = Task.new { p a.join }     # => nil, though a's result is :done
Task.run
```

**Suggested patch**: hand the result to the joiner when it is woken rather than reading it at the
call — `wake_up_join_waiters` (373) already walks the waiting queue when a task finishes and knows
both sides, so it can write `completed_task->result` into the waiter's return register, or `join`
can re-read `t->result` after the switch. The simplest form that keeps the C shape is
to make `join` return `mrb_nil_value()` and document it, but the useful one is the result.

**Note**: nothing in the gem's tests calls `join`, which is why neither this nor item 1 shows up.

## 3. Test gaps: the methods whose value nobody reads

**Severity**: none by itself; it is the reason 1 and 2 survive.

`mrbgems/mruby-task/test/` has 43 assertions for `Task` and 23 for `Queue`, and they check
effects, never answers. Not covered at all:

* `Task#join` (any form), `Task.current` from the root context, `Task#suspend`/`#resume` answers.
* The value of `Task.pass`, `sleep`, `sleep_ms`, `usleep` — all of which the C returns
  deliberately (`mrb_f_sleep` computes `ms / 1000`).
* `mrb_task_run_once` as a host would use it (the WASM/event-loop configuration the comment
  names). Everything in the suite goes through `Task.run`.

**Suggested patch**: a handful of assertions in `test/task.rb`:

```ruby
assert("Task.pass returns nil")            { assert_nil Task.pass }
assert("sleep returns the seconds asked")  { assert_equal 0, sleep(0.01) }
assert("join answers the task's result")   { ... }
assert("join from the root raises")        { assert_raise(RuntimeError) { Task.new {}.join } }
```

The port carries these as `tests/task.rs` (`a_call_that_parks_answers_its_own_value`), which is
where the wording above comes from.
