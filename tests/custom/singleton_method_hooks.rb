# `BasicObject#singleton_method_added` / `_removed` / `_undefined` (`bob_rom_entries`,
# src/class.c: `mrb_do_nothing` for all three). A change to an object's singleton class is
# reported to the object, not to the class: `mrb_method_added`, `remove_method_id` and
# `undef_method` all look at `c->tt == MRB_TT_SCLASS` and, when it is one, send the
# `singleton_` name to what `__attached__` names.

class Watcher
  def singleton_method_added(name)
    puts "added #{name}"
  end
  def singleton_method_removed(name)
    puts "removed #{name}"
  end
  def singleton_method_undefined(name)
    puts "undefined #{name}"
  end
end

w = Watcher.new
def w.one; 1; end
w.define_singleton_method(:two) { 2 }
p w.one, w.two
p w.singleton_methods.sort

class << w
  def three; 3; end
  remove_method :two
  undef_method :one
end
p w.three
p w.singleton_methods.sort
begin
  w.one
rescue NoMethodError => e
  p e.class
end

# an ordinary method of the class is not a singleton method: the class hears `method_added`
class Watcher
  def self.method_added(name); puts "class heard #{name}"; end
  def four; 4; end
end

# a class is an object too, so `def self.x` on a class is a singleton method of the class
class Counted
  def self.singleton_method_added(name); puts "Counted added #{name}"; end
  def self.five; 5; end
end
p Counted.five

# extending an object does not add a method to its singleton class
module Ext; def six; 6; end; end
w.extend(Ext)
p w.six

# the defaults answer nil
o = Object.new
p o.send(:singleton_method_added, :a)
p o.send(:singleton_method_removed, :b)
p o.send(:singleton_method_undefined, :c)
# and they are BasicObject's, which is what makes them reachable from anything at all
# (`BasicObject.private_instance_methods(false)` would say so too, but the three are public
# here and private there — the visibility list of `docs/verification/coverage.md`)
p [:singleton_method_added, :singleton_method_removed, :singleton_method_undefined]
    .map { |m| BasicObject.instance_method(m).owner }
