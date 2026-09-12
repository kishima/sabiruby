//! mruby-task's scheduler from the host's side: `Vm::task_run_once`, which is the shape a frame
//! loop wants where `Task.run` runs until every task is done. The Ruby surface is checked by the
//! reference's own tests (`tools/mrbtest.sh`, `gem_task`, `gem_queue`, `gem_gc_task`).

fn vm_with(src: &str) -> sabiruby::Vm {
    let bin = sabiruby_compiler::compile(src.as_bytes(), &sabiruby_compiler::Options {
        filename: "(test)".into(), debug_info: true, ..Default::default()
    }).expect("compile");
    let mut vm = sabiruby::Vm::with_mrblib().expect("vm");
    vm.load_and_run(&bin).expect("run");
    vm
}

#[test]
fn run_once_advances_one_task_at_a_time() {
    let mut vm = vm_with(r#"
      $order = []
      Task.new(name: "a") { $order << :a1; Task.pass; $order << :a2 }
      Task.new(name: "b") { $order << :b1 }
    "#);
    // one ready task per call, and nil once there is nothing left to run
    let mut turns = 0;
    while !vm.task_run_once().expect("run_once").is_nil() {
        turns += 1;
        assert!(turns < 10, "the scheduler never went idle");
    }
    let out = String::from_utf8_lossy(&vm.take_output()).into_owned();
    assert_eq!(out, "");
    let bin = sabiruby_compiler::compile(b"p $order\n", &sabiruby_compiler::Options {
        filename: "(test)".into(), debug_info: true, ..Default::default()
    }).expect("compile");
    vm.load_and_run(&bin).expect("run");
    assert_eq!(String::from_utf8_lossy(&vm.take_output()), "[:a1, :b1, :a2]\n");
}

#[test]
fn a_timeslice_is_a_fixed_amount_of_work() {
    // there is no timer here, so the tick is the instruction count: a task that never yields is
    // preempted all the same (`docs/gems.md`)
    let mut vm = vm_with(r#"
      $order = []
      Task.new { 200000.times { |i| $order << :a if i == 0 }; $order << :a_end }
      Task.new { $order << :b }
      Task.run
      p $order
    "#);
    assert_eq!(String::from_utf8_lossy(&vm.take_output()), "[:a, :b, :a_end]\n");
}

#[test]
fn a_task_that_raises_keeps_the_scheduler_going() {
    let mut vm = vm_with(r#"
      t = Task.new { raise "boom" }
      Task.new { p :other }
      Task.run
      p [t.value.class, t.value.message, t.status]
    "#);
    assert_eq!(String::from_utf8_lossy(&vm.take_output()), ":other\n[RuntimeError, \"boom\", :DORMANT]\n");
}
