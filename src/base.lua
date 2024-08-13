register("neg", 1, function(x)
  return -x
end)

register("d2r", 1, function(x)
  return x * math.pi / 180
end)

register("r2d", 1, function(x)
  return x * 180 / math.pi
end)

register("root", 2, function(x, y)
  return x^(1/y)
end)
