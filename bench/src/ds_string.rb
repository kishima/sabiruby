# data structures: String#<<, String#+, String#[] and String#==.
s = 0
i = 0
while i < 1_000
  t = ""
  j = 0
  while j < 500
    t << "ab"
    t = t + "c"
    j += 1
  end
  j = 0
  while j < 500
    s += 1 if t[j] == "a"
    j += 1
  end
  i += 1
end
puts s
