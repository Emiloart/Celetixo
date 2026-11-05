package calc
import "example.test/calc/math"

func App() int { return math.Base(2) }
func TestApp() bool { return App() == 3 }
