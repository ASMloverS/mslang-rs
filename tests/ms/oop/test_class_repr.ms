# test_class_repr.ms — 对象显示格式（输出比对类，配 .expected）
# 覆盖: task 40/43（06-oop.md；隐式 Object 基类 __repr__）

# 无 __repr__ 的类：默认 "ClassName instance"
class Plain {
}
print(Plain())

# 有 __repr__ 的类
class Point {
    fn __init__(self, x, y) {
        self.x = x
        self.y = y
    }
    fn __repr__(self) {
        return "Point(" + str(self.x) + ", " + str(self.y) + ")"
    }
}
print(Point(3, 4))
print(Point(-1, 0))

# __repr__ 内嵌容器显示
class Bag {
    fn __init__(self, items) {
        self.items = items
    }
    fn __repr__(self) {
        return "Bag" + str(self.items)
    }
}
print(Bag([1, "a", true]))

# __repr__ 返回非字符串由 negative 覆盖

# print 容器内实例（嵌套显示）
print([Point(1, 2), Point(3, 4)])

# 继承下的 __repr__ 复用（Object 基类提供）
class Sub < Plain {
}
print(Sub())

print("ALL PASSED")
