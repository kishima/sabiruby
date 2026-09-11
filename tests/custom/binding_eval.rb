# Kernel#binding carries a frame's proc and env; Binding#eval reads and writes
# through it, and a local made by one eval is visible to the next one on the
# same binding (mruby-eval's expand_lvspace).
# expected-from: CRuby 3.2
# pending: binding (mruby-binding and mruby-eval are not implemented)
def get_binding
  x = 5
  binding
end
b = get_binding
p b.eval("x")
b.eval("x = 7; y = 1")
p b.eval("x + y")
p b.local_variables.sort
