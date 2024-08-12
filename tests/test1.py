import foo
if True:
    print(1)
    print(2)
    print(3)
if True:
    print(1)
    print(2)
    foo.exit(1)
    print(3)