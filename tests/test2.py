import foo, sys
if sys.argv!=['tests/test2.py', 'arg1', 'arg2', 'arg3']:
    foo.exit(1)