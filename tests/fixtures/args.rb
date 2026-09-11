def m(a, b = 2, *rest, c)
  [a, b, rest, c]
end
p m(1, 9), m(1, 2, 3), m(1, 2, 3, 4, 5)
def splat(*a) = a
p splat, splat(1), splat(*[1, 2]), splat(1, *[2, 3], 4)
def blk(&b) = b
p blk.nil?, blk { }.class
def pass_through(*a, &b) = yield(*a)
p pass_through(1, 2) { |x, y| x + y }
p [[1, 2], [3, 4]].map { |a, b| a * b }
p [[1, [2, 3]]].map { |a, (b, c)| a + b + c }
a, (b, c), *d = 1, [2, 3], 4, 5
p a, b, c, d
first, *middle, last = [1, 2, 3, 4]
p first, middle, last
x, y = 1
p x, y
def opt_block(a, b = a * 2)
  yield a, b if block_given?
  [a, b]
end
p opt_block(1), opt_block(1) { |q, r| p [q, r] }
def nested
  [1, 2].map { |i| [10, 20].map { |j| i * j } }
end
p nested
def counter_gen
  c = 0
  [-> { c += 1 }, -> { c -= 1 }, -> { c }]
end
up, down, cur = counter_gen
up.(); up.(); down.()
p cur.()
def deep(n)
  return 0 if n == 0
  1 + deep(n - 1)
end
p deep(200)
p 1.then { |v| v + 1 }, 5.tap { |v| p v }
p [1, 2, 3].inject(10) { |s, x| s + x }, [1, 2, 3].reduce(:*)
h = Hash.new(0)
"hello".each_char { |ch| h[ch] += 1 }
p h
h2 = Hash.new { |hash, k| hash[k] = k * 2 }
p h2[3], h2
