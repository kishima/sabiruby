def counter
  n = 0
  inc = -> { n += 1 }
  get = -> { n }
  [inc, get]
end
inc, get = counter
inc.call; inc.call
p get.call
def find_first(a)
  a.each { |x| return x if x > 1 }
  nil
end
p find_first([1, 2, 3])
r = [1, 2, 3, 4].each { |x| break x * 100 if x == 3 }
p r
p [3, 1, 2].sort, [3, 1, 2].sort { |a, b| b <=> a }
p (1..5).select { |x| x.even? }, (1..4).inject(:+), (1..4).inject { |s, x| s * x }
[1, 2].each_with_index { |x, i| p [x, i] }
p %w[b a c].sort_by { |s| s }, [1, 2, 3].reverse, [[1, 2], [3]].flatten
p [1, 2, 3].include?(2), [1, 2, 3].map { |x| x.to_s }.join("-")
i = 0
loop do
  i += 1
  break if i >= 3
end
p i
def yielder
  yield 1, 2
end
yielder { |a, b| p [a, b] }
yielder { |a| p a }
pr = proc { |a, b| [a, b] }
p pr.call(1), pr.call(1, 2, 3), pr.call([4, 5])
l = lambda { |a, b| [a, b] }
p l.arity, pr.arity, l.lambda?
begin
  l.call(1)
rescue ArgumentError => e
  p e.message
end
