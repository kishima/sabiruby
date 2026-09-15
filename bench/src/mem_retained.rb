# memory: the same allocation rate with a large live set the mark phase walks.
class Node
  def initialize(v)
    @v = v
    @next = nil
  end
  attr_accessor :v, :next
end
live = []
i = 0
while i < 20_000
  live.push(Node.new(i))
  i += 1
end
s = 0
i = 0
while i < 1_600_000
  n = Node.new(i)
  n.next = live[i % 20_000]
  s += n.next.v
  i += 1
end
puts s
