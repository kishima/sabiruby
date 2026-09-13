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
fn a_host_spawns_tasks_and_drives_the_clock() {
    // the shape a frame loop wants: the host makes the tasks out of compiled programs, gives the
    // scheduler a budget per frame, and moves the clock on by the frame time
    let mut vm = sabiruby::Vm::with_mrblib().expect("vm");
    vm.task_external_clock(true);
    let spawn = |vm: &mut sabiruby::Vm, src: &str, name: &str| {
        let bin = sabiruby_compiler::compile(src.as_bytes(), &sabiruby_compiler::Options {
            filename: name.into(), debug_info: true, ..Default::default()
        }).expect("compile");
        let irep = vm.load(&bin).expect("load");
        let t = vm.task_spawn(irep, 128, Some(name)).expect("spawn");
        vm.gc_register(t);
        t
    };
    let a = spawn(&mut vm, "$order = ($order || []) << :a_start\nsleep 0.1\n$order << :a_woke\n:a_done\n", "a");
    let b = spawn(&mut vm, "$order = ($order || []) << :b\n:b_done\n", "b");

    // frame 1: both run, `a` parks on its sleep and `b` finishes
    vm.task_run_budget(100_000).expect("frame");
    assert!(!vm.task_finished(a), "a must be sleeping, not done");
    assert!(vm.task_finished(b));
    assert_eq!(vm.inspect_str(vm.task_value(b)).unwrap(), ":b_done");

    // a few frames of 16 ms each: nothing is ready until the sleep is over
    let per_frame = 16 / vm.task_tick_unit_ms();
    for _ in 0..3 {
        vm.task_advance_ticks(per_frame);
        vm.task_run_budget(100_000).expect("frame");
    }
    assert!(!vm.task_finished(a), "0.1 s has not passed yet");
    for _ in 0..4 {
        vm.task_advance_ticks(per_frame);
        vm.task_run_budget(100_000).expect("frame");
    }
    assert!(vm.task_finished(a), "the sleep should be over");
    assert_eq!(vm.inspect_str(vm.task_value(a)).unwrap(), ":a_done");

    let bin = sabiruby_compiler::compile(b"p $order\n", &sabiruby_compiler::Options {
        filename: "(test)".into(), debug_info: true, ..Default::default()
    }).expect("compile");
    vm.load_and_run(&bin).expect("run");
    assert_eq!(String::from_utf8_lossy(&vm.take_output()), "[:a_start, :b, :a_woke]\n");
}

#[test]
fn a_budget_bounds_one_turn_of_a_host_loop() {
    // a task that never yields is preempted at its timeslice, so a frame is not lost to it
    let mut vm = sabiruby::Vm::with_mrblib().expect("vm");
    vm.task_external_clock(true);
    let bin = sabiruby_compiler::compile(b"i = 0\nwhile true\n  i += 1\nend\n", &sabiruby_compiler::Options {
        filename: "spin".into(), debug_info: true, ..Default::default()
    }).expect("compile");
    let irep = vm.load(&bin).expect("load");
    let t = vm.task_spawn(irep, 128, Some("spin")).expect("spawn");
    vm.gc_register(t);
    let spent = vm.task_run_budget(50_000).expect("frame");
    assert!(spent >= 50_000 && spent < 200_000, "spent {spent}");
    assert!(!vm.task_finished(t));
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

#[test]
fn a_call_that_parks_answers_its_own_value() {
    // the switch is deferred to the next instruction boundary, as the reference defers it, so the
    // value the native returned is stored first (`docs/gems.md`, mruby-task)
    let mut vm = vm_with(r#"
      done = Task.new(name: "done") { :done_now }
      Task.run
      a = Task.new(name: "a") { sleep(0.05); :done_a }
      Task.new(name: "b") { p [Task.pass, sleep(0.01), a.join, done.join] }
      Task.run
      begin; a.join; rescue => e; p [e.class, e.message]; end
      begin; Task.new { }.join; rescue => e; p e.class; end
    "#);
    assert_eq!(
        String::from_utf8_lossy(&vm.take_output()),
        // Task.pass is nil, sleep answers the seconds asked for, a join that waited answers the
        // result as it stood (nil), a join on a task already done answers its result
        "[nil, 0, nil, :done_now]\n\
         [RuntimeError, \"join can only be called from running task\"]\n\
         RuntimeError\n"
    );
}

#[test]
fn a_host_waits_on_its_own_clock() {
    // what a browser or a frame loop needs to sleep for real: the scheduler says how long it may
    // wait and whether anything is left, and the host moves the clock (`docs/playground.md`)
    let mut vm = vm_with(r#"
      $log = []
      Task.new(name: "slow") { 2.times { |i| $log << "slow#{i}"; sleep 0.1 } }
      Task.new(name: "fast") { 4.times { |i| $log << "fast#{i}"; sleep 0.05 } }
    "#);
    vm.task_external_clock(true);
    let unit = vm.task_tick_unit_ms(); // 4 ms
    let mut clock_ms = 0u32;
    let mut waits = 0;
    while vm.task_pending() {
        vm.task_run_budget(100_000).expect("run");
        match vm.task_next_wakeup_ticks() {
            // nothing to run: a host would wait this long before coming back, and the clock is
            // moved by what it waited — never jumped by the scheduler itself
            Some(ticks) if ticks > 0 => {
                clock_ms += ticks * unit;
                vm.task_advance_ticks(ticks);
                waits += 1;
                assert!(waits < 20, "the host was asked to wait too many times");
            }
            Some(_) => {}
            None => break,
        }
    }
    // 2 x 100 ms and 4 x 50 ms, sleeping in parallel. A wait is rounded up to whole ticks, so
    // 50 ms is 13 of them (52 ms) and the last deadline is the fast task's 4th, at 208 ms
    assert_eq!(clock_ms, 208);
    let bin = sabiruby_compiler::compile(b"p $log\n", &sabiruby_compiler::Options {
        filename: "(test)".into(), debug_info: true, ..Default::default()
    }).expect("compile");
    vm.load_and_run(&bin).expect("run");
    assert_eq!(
        String::from_utf8_lossy(&vm.take_output()),
        "[\"slow0\", \"fast0\", \"fast1\", \"slow1\", \"fast2\", \"fast3\"]\n"
    );
}

#[test]
fn the_scheduler_is_not_re_entered_from_inside_a_task() {
    // `Task.run` from a task would take the head of the ready queue — the running task itself —
    // and resume the context it is standing in. The reference's own loop says "already running"
    // with a flag; a host that drives the scheduler a step at a time leaves that flag clear, so
    // the caller is asked instead (`docs/gems.md`).
    let src = r#"
      Task.new(name: "a") { 2.times { |i| puts "a#{i}"; Task.pass }; :a }
      p Task.run
      puts "after"
    "#;
    let bin = sabiruby_compiler::compile(src.as_bytes(), &sabiruby_compiler::Options {
        filename: "(test)".into(), debug_info: true, ..Default::default()
    }).expect("compile");
    let mut vm = sabiruby::Vm::with_mrblib().expect("vm");
    let irep = vm.load(&bin).expect("load");
    let main = vm.task_spawn(irep, 128, Some("main")).expect("spawn");
    vm.gc_register(main);
    let mut turns = 0;
    while vm.task_pending() {
        vm.task_run_budget(100_000).expect("run");
        turns += 1;
        assert!(turns < 20, "the host loop did not finish");
    }
    // the program's own `Task.run` answered nil and went on; the host ran the other task
    assert_eq!(String::from_utf8_lossy(&vm.take_output()), "nil\nafter\na0\na1\n");
}
