# memory: objects that die at once, so the sweep has everything to take back.
class Point
  def initialize(x, y)
    @x = x
    @y = y
  end
  attr_reader :x, :y
end
s = 0
i = 0
while i < 3_000_000
  p1 = Point.new(i, i + 1)
  s += p1.x - p1.y
  i += 1
end
puts s
