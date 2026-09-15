# Two of `mod_rom_entries`' no-op hooks (src/class.c): `const_added`, fired by `mrb_const_set`
# (src/variable.c), and `method_undefined`, fired by `undef_method` (src/class.c).
#
# What `const_added` is *not* fired by is worth as much as what it is: `class Foo; end` goes
# through `setup_class`, which writes the constant with `mrb_obj_iv_set` and never calls the
# hook, so only an assignment (`OP_SETCONST`, `OP_SETMCNST`) and `Module#const_set` reach it.

module Watched
  def self.const_added(name)
    puts "const_added #{name} = #{const_get(name).inspect}"
  end
  def self.method_undefined(name)
    puts "method_undefined #{name}"
  end

  A = 1
  B = [2, 3]
end

Watched.const_set(:C, :three)
Watched::D = 4
p Watched.constants.sort

# a class body inside it: the constant is written without the hook
module Watched
  class Inner; end
  module Deeper; end
end
p Watched.const_defined?(:Inner)

# but assigning the same class to another name is an assignment
Watched::Alias = Watched::Inner

# `method_undefined`
module Watched
  def self.m1; end
  def m2; end
  undef_method :m2
end
p Watched.instance_methods(false)

# it is the class the method table belongs to that hears, so a subclass undefining an
# inherited method hears it, and the superclass does not
class Base
  def self.method_undefined(name); puts "Base heard #{name}"; end
  def gone; end
end
class Sub < Base
  def self.method_undefined(name); puts "Sub heard #{name}"; end
  undef_method :gone
end
p Base.new.respond_to?(:gone)
begin
  Sub.new.gone
rescue NoMethodError => e
  p e.class
end

# the default hooks answer nil and are there to be overridden
p Module.new.send(:const_added, :X)
p Module.new.send(:method_undefined, :y)
