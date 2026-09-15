# Every way a method lookup's answer can change, each one exercised after the old answer has
# been made cacheable by calling it a few hundred times first (`Vm::find_method_cached`,
# stage 2 candidate 4 of docs/plans/host-bridge-plan.md). Expectation: the reference mruby's own
# output — nothing here is a SabiRuby decision, it is what Ruby says a redefinition does.
def warm(o, m, n = 200)
  n.times { o.send(m) rescue nil }
end

class A
  def x; "A#x"; end
end
class B < A; end
a = A.new; b = B.new

warm(b, :x); puts b.x                       # A#x

# 1. defining a method on a subclass shadows the cached one
class B; def x; "B#x"; end; end
puts b.x                                    # B#x

# 2. redefining it again
class B; def x; "B#x2"; end; end
puts b.x                                    # B#x2

# 3. remove_method falls back to the superclass
class B; remove_method :x; end
puts b.x                                    # A#x

# 4. undef_method stops the lookup
warm(b, :x); class B; undef_method :x; end
begin; b.x; puts "no raise"; rescue NoMethodError; puts "undef ok"; end

# 5. include after the cache is warm
module M; def y; "M#y"; end; end
class C; end
c = C.new
warm(c, :y)
begin; c.y; rescue NoMethodError; puts "y missing ok"; end
class C; include M; end
puts c.y                                    # M#y

# 6. prepend after the cache is warm
module P; def y; "P#y then " + super; end; end
warm(c, :y)
class C; prepend P; end
puts c.y                                    # P#y then M#y

# 7. a singleton method on one object only
class D; def z; "D#z"; end; end
d1 = D.new; d2 = D.new
warm(d1, :z); warm(d2, :z)
def d1.z; "d1#z"; end
puts d1.z                                   # d1#z
puts d2.z                                   # D#z

# 8. extend
module E; def w; "E#w"; end; end
e = Object.new
warm(e, :w)
e.extend(E)
puts e.w                                    # E#w

# 9. define_method
class F; def v; "F#v"; end; end
f = F.new
warm(f, :v); puts f.v
F.send(:define_method, :v) { "F#v redefined" }
puts f.v                                    # F#v redefined

# 10. alias
class G; def g1; "G#g1"; end; def g2; "G#g2"; end; end
g = G.new
warm(g, :g1); puts g.g1
class G; alias g1 g2; end
puts g.g1                                   # G#g2

# 11. alias_method
class G; alias_method :g1, :g2; end
puts g.g1                                   # G#g2

# 12. a module method added after include, seen through the including class
module M2; end
class H; include M2; end
h = H.new
warm(h, :m2)
module M2; def m2; "M2#m2"; end; end
puts h.m2                                   # M2#m2

# 13. Object-level redefinition seen by every class
class I2; end
i = I2.new
warm(i, :top)
class Object; def top; "Object#top"; end; end
puts i.top                                  # Object#top
class I2; def top; "I2#top"; end; end
puts i.top                                  # I2#top

# 14. method_missing arriving after "not found" was cached
class J; end
j = J.new
warm(j, :mm)
begin; j.mm; rescue NoMethodError; puts "mm missing ok"; end
class J; def method_missing(n, *a); "mm:#{n}"; end; end
puts j.mm                                   # mm:mm

# 15. a singleton class of a class (class methods)
class K; def self.cm; "K.cm"; end; end
warm(K, :cm); puts K.cm
class K; def self.cm; "K.cm2"; end; end
puts K.cm                                   # K.cm2

# 16. visibility change does not lose the method
class L; def pub; "L#pub"; end; end
l = L.new
warm(l, :pub); puts l.pub
class L; private :pub; end
begin; l.pub; puts "no raise"; rescue NoMethodError; puts "private ok"; end
puts l.send(:pub)                           # L#pub

# 17. a class reopened through Object.const_get
warm(A.new, :x)
Object.const_get(:A).class_eval { def x; "A#x2"; end }
puts a.x                                    # A#x2

# 18. instance_eval singleton
o = Object.new
warm(o, :ie)
o.instance_eval { def ie; "o#ie"; end }
puts o.ie                                   # o#ie

# 19. Comparable through a module after the fact
class N2
  include Comparable
  attr_accessor :n
  def initialize(n); @n = n; end
  def <=>(x); n <=> x.n; end
end
p1 = N2.new(1); p2 = N2.new(2)
200.times { p1 < p2 }
puts(p1 < p2)                               # true
class N2; def <=>(x); x.n <=> n; end; end
puts(p1 < p2)                               # false

# 20. Struct members (attr readers defined by a native)
S1 = Struct.new(:aa, :bb)
s = S1.new(1, 2)
200.times { s.aa }
puts s.aa                                   # 1
class S1; def aa; "overridden"; end; end
puts s.aa                                   # overridden
