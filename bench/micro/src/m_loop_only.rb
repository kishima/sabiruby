# The empty loop the other micro files wrap their calls in, at the largest of their counts.
a = Array.new(1000) { |i| i }
n = 0
i = 0
while i < 200_000
  n += 1
  i += 1
end
