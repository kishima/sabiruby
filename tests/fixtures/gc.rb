# Garbage collection: what must survive a collection, and what must go.
# Run it with SABIRUBY_GC_STRESS=1 too (a collection after every allocation).

# unreachable objects are reclaimed; GC.disable postpones, GC.enable resumes
GC.start
base = GC.stat[:live]
1000.times { Object.new }
GC.start
p GC.stat[:live] - base < 50
p GC.disable
1000.times { Object.new }
GC.start
p GC.stat[:live] - base > 900
p GC.enable
GC.start
p GC.stat[:live] - base < 50

# keys whose `hash`/`eql?` are Ruby methods that allocate, in a hash literal
# (the hash being built is not in a register yet) and in keyword arguments
class Key
  attr_reader :n
  def initialize(n); @n = n; end
  def hash; ("k" * 3 + @n.to_s).hash; end
  def eql?(o); [@n] == [o.n]; end
end
h = {Key.new(1) => "a", Key.new(2) => "b", Key.new(1) => "c"}
GC.start
p h.size, h.values
def kw(**o) = o.size
p kw(**{Key.new(3) => 1, Key.new(4) => 2}) rescue p $!.class

# closures keep their environments after the frame returned
def counter
  n = 0
  [proc { n += 1 }, proc { n }]
end
inc, get = counter
GC.start
3.times { inc.call }
p get.call

# a block captured inside a suspended fiber keeps the fiber's stack
f = Fiber.new { x = "in fiber"; $pr = proc { x }; Fiber.yield 1; 2 }
p f.resume
f = nil
GC.start
p $pr.call

# a collection while a chain of fibers is resumed
r = Fiber.new do
  Fiber.new do
    GC.start
    Fiber.yield("x" * 5)
  end.resume + "!"
end.resume
p r

# fibers dropped while suspended
100.times { |i| f = Fiber.new { Fiber.yield i }; f.resume }
GC.start
p Fiber.new { :still_works }.resume

# external enumerators run on fibers
e = [1, 2, 3].each
p e.next
GC.start
p e.next, e.next

# a return through ensure keeps its value (RBreak) across a collection
def ret
  [1, 2].each do |x|
    begin
      return "r#{x}"
    ensure
      GC.start
      "t" * 2
    end
  end
end
p ret

# an exception being rescued
begin
  raise "e" * 3
rescue => ex
  GC.start
  p ex.message
end

# deep nesting is marked without recursion
a = []
100000.times { a = [a] }
GC.start
d = 0
while a.size > 0
  a = a[0]
  d += 1
end
p d

# anonymous classes, singleton classes, methods defined at run time
100.times do |i|
  c = Class.new { define_method(:v) { i } }
  o = c.new
  def o.w = 1
end
k = Class.new { def m = "kept" }
Kept = k
GC.start
p Kept.new.m

# a native iterating with a Ruby block
p (1..20).to_a.sort_by { |x| [-x, "s" * 2] }.first(3)
p %w[b c a].sort { |x, y| GC.start; x <=> y }

# ids of collected objects are reused; live objects are unaffected
s = "keep"
ids = []
1000.times { ids << Object.new.object_id }
GC.start
p s, ids.size
