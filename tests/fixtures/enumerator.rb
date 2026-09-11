# needs mruby-enumerator (Fiber-based) — see the gem porting stage
p %w[a b].each_with_index.map { |s, i| "#{i}#{s}" }
p [1, 2, 3].each_slice(2).to_a, (1..3).each_cons(2).to_a
e = [1, 2].each
p e.next, e.next
begin; e.next; rescue StopIteration; p :done; end
