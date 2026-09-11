i = 0; while i < 1_000_000; a = [i, "x" * 10, {k: i}]; i += 1; end; p GC.stat[:live]
