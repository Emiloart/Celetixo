from math_ops import base as increment

def app() -> int:
    return increment(2)

def test_app():
    assert app() == 3
