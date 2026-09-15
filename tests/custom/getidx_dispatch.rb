# OP_GETIDX / OP_GETIDX0 / OP_SETIDX: what the opcode answers itself (Array, Hash and String
# while their `[]` is still the builtin) and what it hands to an ordinary method send.
# regexp-only: the String part sends a Regexp index, which only mruby-regexp answers

# --- a `[]` written in Ruby is an ordinary frame, so it can suspend out of itself
class Box
  def initialize; @h = {}; end
  def [](k); Fiber.yield([:get, k]); @h[k]; end
  def []=(k, v); Fiber.yield([:set, k, v]); @h[k] = v; end
end
f = Fiber.new do
  b = Box.new
  b[:a] = 1
  [b[:a], b[0]]
end
p f.resume
p f.resume(:x)
p f.resume(:y)
p f.resume(:z)

# --- super from a subclass of the classes the opcode answers for
class A2 < Array
  def [](i); ["A2", super]; end
  def []=(i, v); super(i, v.to_s); end
end
a2 = A2.new(3, 0)
p a2[1]
p a2[0]
a2[1] = 5
p a2.inspect

# --- an exception raised inside `[]` / `[]=`
class Boom
  def [](i); raise "boom #{i}"; end
  def []=(i, v); raise ArgumentError, "no #{i}"; end
end
bm = Boom.new
begin; bm[3]; rescue => e; p [e.class, e.message]; end
begin; bm[0]; rescue => e; p [e.class, e.message]; end
begin; bm[1] = 2; rescue => e; p [e.class, e.message]; end

# --- subclasses that redefine `[]`, and ones that do not
class MyA < Array; def [](i); "MyA#{i}"; end; end
class MyH < Hash;  def [](k); "MyH#{k}"; end; end
class MyS < String; def [](i); "MyS#{i}"; end; end
p MyA.new[2], MyH.new[:k], MyS.new("abc")[1]
p MyA.new[0], MyH.new[0], MyS.new("abc")[0]
class PlainA < Array; end
p PlainA.new(2, 7)[1], PlainA.new(2, 7)[0]

# --- a singleton `[]` / `[]=` on an otherwise ordinary Array, Hash and String
sa = [1, 2, 3]
def sa.[](i); "sing#{i}"; end
p sa[1], sa[0]
p [1, 2, 3][1]
sh = { a: 1 }
def sh.[]=(k, v); :ignored; end
p(sh[:b] = 9)
p sh
ss = "abc"
def ss.[](i); "S#{i}"; end
p ss[0], ss[1]

# --- index types the opcode does not answer itself
a = [10, 20, 30, 40]
p a[1..2], a[-1], a[1, 2], a[0..-1], a[7], a[-9]
h = { 1 => :one, "s" => :str, (1..2) => :rng, 0 => :zero }
p h[1], h["s"], h[1..2], h[-1], h[0]
s = "hello world"
p s["world"], s[0..4], s[-5..-1], s[-1], s["zzz"], s[0], s[99]

# --- Hash#default and Hash#default_proc
hd = Hash.new(:dflt)
p hd[:x], hd[0]
hp = Hash.new { |hh, k| hh[k] = "made #{k}" }
p hp[:y], hp[0]
p hp

# --- the value of an index assignment is the right-hand side, whichever path ran
av = [1]; p(av[0] = 99)
hv = {};  p(hv[:k] = 1)
sv = "abc".dup; p(sv[0] = "Z"); p sv
class R; def []=(i, v); :not_the_value; end; end
p(R.new[1] = 7)

# --- receivers the opcode never answers for
begin; 5[1]; rescue NoMethodError => e; p e.class; end
begin; 5[0]; rescue NoMethodError => e; p e.class; end
pr = ->(i) { i * 2 }
p pr[21], pr[0]
S1 = Struct.new(:x, :y)
st = S1.new(1, 2)
p st[0], st[:x], st["y"]
st[0] = 9
p st.x

# --- no `[]` at all: the send reaches method_missing
class MM; def method_missing(n, *args); [n, args]; end; end
mm = MM.new
p mm[1], mm[0]
p(mm[1] = 2)

# --- errors the method raises, not the opcode
fa = [1, 2].freeze
begin; fa[0] = 9; rescue => e; p e.class; end
fh = { a: 1 }.freeze
begin; fh[:b] = 1; rescue => e; p e.class; end
fs = "abc".freeze
begin; fs[0] = "Z"; rescue => e; p e.class; end
u = "abc".dup
begin; u[0] = 1; rescue => e; p e.class; end
v = [1, 2]
begin; v[-5] = 0; rescue => e; p e.class; end
w = "abc".dup
begin; w[9] = "x"; rescue => e; p e.class; end

# --- mruby-regexp widens String#[] past what the opcode answers; the opcode sends a Regexp
p "hello"[/l+/]
p "hello"[/(l)(o)/, 2]
t = "hello".dup; t[/l+/] = "L"; p t

# --- redefining Array#[] disarms the opcode, aliasing the builtin back re-arms it.
# `p` is written in Ruby in mruby-print and reads its own arguments with `args[i]`, so it
# would answer through the override here; these lines use `puts` to print the value itself.
x123 = [1, 2, 3]
class Array
  alias __orig_aref []
  def [](i); "redef #{i}"; end
end
puts x123[1]
puts x123[0]
puts [4, 5, 6][1]
class Array
  alias [] __orig_aref
end
puts x123[1].inspect
puts x123[0].inspect

# --- a redefined Hash#default is honoured by whichever path answers `[]`
class Hash; def default(k = nil); "redef #{k}"; end; end
p({}[:q])
p({}[0])

# --- prepending a module to Array is a redefinition too
module Loud; def [](i); "loud:" + super.to_s; end; end
class Array; prepend Loud; end
puts x123[1]
puts x123[0]
puts x123[1..2].inspect
