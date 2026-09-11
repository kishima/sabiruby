class Point
  attr_reader :x
  def initialize(x, y)
    @x = x
    @y = y
  end
  def sum
    @x + @y
  end
  def to_s
    "(#{@x}, #{@y})"
  end
end
class Point3 < Point
  def initialize(x, y, z)
    super(x, y)
    @z = z
  end
  def sum
    super + @z
  end
end
pt = Point.new(1, 2)
p pt.x, pt.sum, pt.to_s
p Point3.new(1, 2, 3).sum
p Point.name, Point3.superclass, pt.class, pt.is_a?(Point), pt.respond_to?(:sum)
module Greet
  def hi; "hi #{@x}"; end
end
class Point; include Greet; end
p pt.hi
