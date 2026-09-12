RE_TESTS_NEED_STACK = 48

def need_backtracking_stack
  if Regexp::STACK_LIMIT < RE_TESTS_NEED_STACK
    skip "MRB_REGEXP_STACK_LIMIT stands below what these patterns need"
  end
end
