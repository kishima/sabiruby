class Animal
  include Comparable
  attr_accessor :name, :age
  @@count = 0
  COUNT_LIMIT = 10
  def initialize(name, age)
    @name = name
    @age = age
    @@count += 1
  end
  def self.count
    @@count
  end
  def <=>(other)
    age <=> other.age
  end
  def to_s
    "#{self.class.name}(#{@name})"
  end
  def inspect
    "#<#{self}>"
  end
  def method_missing(name, *args)
    if name.to_s.start_with?("say_")
      "#{@name} says #{name.to_s[4..]}"
    else
      super
    end
  end
  def respond_to_missing?(name, priv = false)
    name.to_s.start_with?("say_") || super
  end
end
class Dog < Animal
  def initialize(name, age)
    super
    @tricks = []
  end
  def learn(t)
    @tricks << t
    self
  end
  def tricks; @tricks; end
end
a = Animal.new("cat", 3)
d = Dog.new("rex", 5)
p a, d, Animal.count, a < d, [d, a].min, a.between?(a, d), a.clamp(a, d)
p d.learn(:sit).learn(:roll).tricks
p d.say_hello
begin
  d.unknown
rescue NoMethodError => e
  p e.message
end
p Animal::COUNT_LIMIT, Dog::COUNT_LIMIT, Dog.superclass, Dog.ancestors.first(3)
p Dog.instance_of?(Class), Dog.is_a?(Module), Animal === d, Comparable === d
o = Object.new
def o.hello; "singleton"; end
p o.hello, o.singleton_methods rescue p :no_singleton_methods
p o.respond_to?(:hello), 5.respond_to?(:+), nil.to_a, nil.to_s, nil.inspect
module Util
  PI2 = 6.28
  def self.twice(x) = x * 2
  def helper; "helper from #{self.class}"; end
end
class Dog; include Util; end
p Util.twice(4), Util::PI2, d.helper, Dog.include?(Util)
p d.instance_variables, d.instance_variable_get(:@name)
d.name = "max"
p d.name, d.frozen?, d.dup.name
x = 5
p(case x
  when 1..3 then :low
  when 4..6 then :mid
  else :high
  end)
p(case "str" when String then :string when Integer then :int end)
h = {a: 1, b: 2}
h.each { |k, v| p [k, v] }
p h.map { |k, v| [k, v * 2] }.to_h, h.select { |k, v| v > 1 }, h.keys, h.values, h.to_a
p h.merge(c: 3), h.key?(:a), h.fetch(:z, 0), h.any? { |k, v| v > 1 }, h.count, h.min { |x, y| x[1] <=> y[1] }
p [1, 2, 3].sum, [1, 2, 3].max, [[1, :a], [2, :b]].to_h, [1, 2, 3].zip([4, 5, 6])
p [1, [2, [3, [4]]]].flatten(1), [3, 1, 2].sort.reverse, [1, 2, 3, 4].partition { |x| x.even? }
p [1, 2, 3].find { |x| x > 1 }, [1, 2, 3].all? { |x| x > 0 }, ![1, 2, 3].any? { |x| x > 5 }, [1, 2, 2].uniq, [1, 2, 3].select { |x| x < 3 }
