# test_magic_methods.ms — 魔术方法
# 覆盖: task 43（06-oop.md § 魔术方法）

# __repr__：print 与 str 的显示形式
class Point {
    fn __init__(self, x, y) {
        self.x = x
        self.y = y
    }
    fn __repr__(self) {
        return "Point(" + str(self.x) + ", " + str(self.y) + ")"
    }
}
p = Point(3, 4)
assert(str(p) == "Point(3, 4)", "__repr__ 经 str")

# __eq__：== 运算符分派
class Money {
    fn __init__(self, cents) {
        self.cents = cents
    }
    fn __eq__(self, other) {
        return self.cents == other.cents
    }
}
assert(Money(100) == Money(100), "__eq__ 相等")
assert((Money(100) == Money(200)) == false, "__eq__ 不等")

# __add__：+ 运算符分派
class Vec {
    fn __init__(self, a, b) {
        self.a = a
        self.b = b
    }
    fn __add__(self, other) {
        return Vec(self.a + other.a, self.b + other.b)
    }
    fn __repr__(self) {
        return "Vec(" + str(self.a) + "," + str(self.b) + ")"
    }
}
v = Vec(1, 2) + Vec(10, 20)
assert(v.a == 11 and v.b == 22, "__add__ 返回新实例")
assert(str(v) == "Vec(11,22)", "__add__ 结果可再用")

# __sub__
class Num {
    fn __init__(self, v) {
        self.v = v
    }
    fn __sub__(self, other) {
        return Num(self.v - other.v)
    }
}
diff = Num(10) - Num(4)
assert(diff.v == 6, "__sub__")

# __lt__：排序比较
class Version {
    fn __init__(self, major, minor) {
        self.major = major
        self.minor = minor
    }
    fn __lt__(self, other) {
        if self.major != other.major {
            return self.major < other.major
        }
        return self.minor < other.minor
    }
}
assert((Version(1, 2) < Version(1, 3)), "__lt__ 真")
assert((Version(2, 0) < Version(1, 9)) == false, "__lt__ 假")

# __len__：len() 分派
class Team {
    fn __init__(self, members) {
        self.members = members
    }
    fn __len__(self) {
        return self.members.length()
    }
}
assert(len(Team(["a", "b", "c"])) == 3, "__len__")

# __contains__：in 运算符分派
class Box {
    fn __init__(self, items) {
        self.items = items
    }
    fn __contains__(self, item) {
        return item in self.items
    }
}
bx = Box([10, 20])
assert(10 in bx, "__contains__ 命中")
assert((99 in bx) == false, "__contains__ 未命中")

# __getitem__：下标访问分派
class Grid {
    fn __init__(self, data) {
        self.data = data
    }
    fn __getitem__(self, i) {
        return self.data[i]
    }
}
g = Grid([100, 200, 300])
assert(g[0] == 100 and g[2] == 300, "__getitem__")

# __call__：实例可调用
class Adder {
    fn __init__(self, n) {
        self.n = n
    }
    fn __call__(self, x) {
        return x + self.n
    }
}
add10 = Adder(10)
assert(add10(5) == 15, "__call__ 实例调用")

# 组合：魔术方法与继承（super.__repr__ 复用父类实现）
class NamedVec < Vec {
    fn __init__(self, name, a, b) {
        super.__init__(a, b)
        self.name = name
    }
    fn __repr__(self) {
        return self.name + "=" + super.__repr__()
    }
}
nv = NamedVec("pos", 1, 2)
assert(str(nv) == "pos=Vec(1,2)", "super.__repr__ 复用父类实现")

# 继承的 __add__ 在父类体内构造基类 Vec（返回基类实例）
added = nv + Vec(1, 1)
assert(str(added) == "Vec(2,3)", "继承 __add__ 返回父类构造结果")

print("ALL PASSED")
