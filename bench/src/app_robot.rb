# whole-program: one frame of a SabiRuby Battle robot, radar -> lead -> act,
# as the game's script layer runs it. Objects, floats, arrays and method calls
# in the mix a host actually drives.
class Vec
  attr_reader :x, :y
  def initialize(x, y)
    @x = x
    @y = y
  end
  def -(o); Vec.new(@x - o.x, @y - o.y); end
  def +(o); Vec.new(@x + o.x, @y + o.y); end
  def scale(k); Vec.new(@x * k, @y * k); end
  def length; Math.sqrt(@x * @x + @y * @y); end
end

class Enemy
  attr_reader :pos, :vel
  def initialize(pos, vel)
    @pos = pos
    @vel = vel
  end
  def step
    @pos = @pos + @vel
  end
end

class Robot
  def initialize
    @pos = Vec.new(0.0, 0.0)
    @enemies = []
    k = 0
    while k < 8
      @enemies.push(Enemy.new(Vec.new(k * 13.0, k * 7.0), Vec.new(0.5, -0.25)))
      k += 1
    end
    @shots = 0
  end

  # radar: the nearest enemy
  def radar
    best = nil
    bestd = 1.0e30
    @enemies.each do |e|
      d = (e.pos - @pos).length
      if d < bestd
        bestd = d
        best = e
      end
    end
    best
  end

  # lead: where to aim, two rounds of iteration
  def lead(target)
    aim = target.pos
    2.times do
      t = (aim - @pos).length / 20.0
      aim = target.pos + target.vel.scale(t)
    end
    aim
  end

  # act: turn, move and shoot
  def act(aim)
    d = aim - @pos
    @pos = @pos + d.scale(0.01)
    @shots += 1 if d.length < 200.0
    @enemies.each { |e| e.step }
  end

  def frame
    t = radar
    act(lead(t))
  end

  def shots; @shots; end
end

r = Robot.new
i = 0
while i < 75_000
  r.frame
  i += 1
end
puts r.shots
