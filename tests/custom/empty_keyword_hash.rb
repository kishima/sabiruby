# `foo(**{})`: an empty keyword Hash. mruby keeps it on the stack with `ci->kw` set —
# `mrb_get_args` folds a keyword Hash into the positional arguments only when it has
# entries (`mrb_hash_size(kdict) > 0`, src/class.c), and `OP_ENTER`'s fast path counts
# `ci->kw` as one argument (`argc+ci->kw != m1`, src/vm.c), so a method that takes one
# required parameter binds it to the empty Hash. Everything here is the reference's
# output; what used to differ is every line that goes through `send` or `method_missing`,
# because those rewrite the frame and SabiRuby dropped the empty Hash on the way.

def one(x); p x; end
def two(a, b); p [a, b]; end
def rest(*a); p a; end
def opt(x = :none, **k); p [x, k]; end
def kwonly(k: :none); p k; end

# --- called straight from bytecode
one(**{})
two(1, **{})
rest(**{})
opt(**{})
kwonly(**{})

empty = {}
one(**empty)
one(**{}) { :block_too }

# --- through `send` / `__send__`, which shift the registers down (`send_method`)
p send(:one, **{})
send(:two, 1, **{})
send(:rest, **{})
send(:opt, **{})
send(:kwonly, **{})
__send__(:one, **empty)

# `send` with nothing but the keyword Hash still has no method name
begin
  send(**{})
rescue ArgumentError => e
  p e.message
end

# --- through `method_missing`, which gains an argument instead of losing one
class Missing
  def method_missing(name, *a, **k); p [name, a, k]; end
end
Missing.new.zap(**{})
Missing.new.zap(1, 2, **{})
Missing.new.send(:zap, **{})

# `prepare_missing` packs the positional arguments into one Array (`ci->n = 15`), which
# is what keeps `OP_ENTER`'s fast path — the one that counts `ci->kw` — out of the way:
# a `method_missing` with two required parameters is one argument short, not two.
class Positional
  def method_missing(name, x); p [name, x]; end
end
begin
  Positional.new.zap(**{})
rescue ArgumentError => e
  p e.message
end
Positional.new.zap(1)

# --- a non-empty keyword Hash is unchanged: it becomes the last positional argument
# of anything that does not declare keywords
def pos(*a); p a; end
pos(k: 1)
send(:pos, k: 1)
Missing.new.zap(k: 1)

# --- and a native method still sees a non-empty one appended, an empty one not at all
p [].push(**{})
p({ a: 1 }.merge({ b: 2 }))
