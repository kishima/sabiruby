i = 0
while i < 3
  i += 1
end
p i
v = if i > 2 then :big else :small end
p v
case i
when 1 then p :one
when 3 then p :three
else p :other
end
p(nil ? 1 : 2)
p [i > 0 && i < 10, i > 5 || false, !true]
