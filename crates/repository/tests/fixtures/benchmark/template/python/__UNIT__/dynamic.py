from . import math_ops

def dynamic_entry():
    return getattr(math_ops, "base")(2)
