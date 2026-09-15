# `Hash#default_proc=` (`mrb_hash_set_default_proc`, src/hash.c). mruby keeps the plain default
# and the default proc in the one `ifnone` ivar with two flags (MRB_HASH_DEFAULT and
# MRB_HASH_PROC_DEFAULT), so assigning either one takes the other away, and `nil` takes both.
# A lambda is checked for an arity that could take the two arguments a lookup passes
# (`hash_set_default_proc`); a plain proc is not, because a proc takes whatever it is given.

h = {}
p h.default_proc
h.default_proc = proc { |hash, key| hash[key] = key.to_s * 2 }
p h.default_proc.class
p h["ab"]
p h
p h.default

# assigning a proc replaces a plain default, and a plain default replaces the proc
g = Hash.new(:plain)
p g[:missing]
g.default_proc = proc { |_, k| [:from_proc, k] }
p g[:missing], g.default
g.default = :plain_again
p g[:missing], g.default_proc

# nil takes the default away
h.default_proc = nil
p h.default_proc, h.default, h["zz"]

# a lambda must be able to take the two arguments
begin
  h.default_proc = ->(k) { k }
rescue TypeError => e
  p e.message
end
h.default_proc = ->(hash, key) { [:lambda, key] }
p h[:x]
h.default_proc = ->(*a) { [:splat, a.size] }
p h[:y]

# and the argument has to be a Proc at all
begin
  h.default_proc = 1
rescue TypeError => e
  p e.message
end

# a frozen Hash refuses
f = {}.freeze
begin
  f.default_proc = proc { |_, _| 1 }
rescue => e
  p [e.class, e.message]
end

# `dup` carries it, and so does `Hash.new { }`
d = h.dup
p d[:z]
n = Hash.new { |_, k| [:block, k] }
p n.default_proc.class, n[:w]
