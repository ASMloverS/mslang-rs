// test_self_attrs.ms — self 绑定与实例属性

// self 绑定与方法调用
class Point {
    fn __init__(self, x, y) {
        self.x = x
        self.y = y
    }

    fn distance_to(self, other) {
        dx = self.x - other.x
        dy = self.y - other.y
        return (dx * dx + dy * dy) ** 0.5
    }

    fn __repr__(self) {
        return "Point(" + str(self.x) + ", " + str(self.y) + ")"
    }
}

p1 = Point(3, 4)
p2 = Point(0, 0)
print(p1.distance_to(p2))
print(p1)

// 动态属性
p1.z = 10
print(p1.z)

// 属性查找顺序：实例字段优先于类属性
class Config {
    value = "class"

    fn __init__(self) {
        self.value = "instance"
    }
}

c = Config()
print(c.value)

// 不同实例互不干扰
a = Point(1, 0)
b = Point(0, 1)
a.label = "A"
b.label = "B"
print(a.label)
print(b.label)
print(a.x)
print(b.x)

// 方法链
class Builder {
    fn __init__(self) {
        self.parts = []
    }

    fn add(self, part) {
        self.parts.push(part)
        return self
    }

    fn build(self) {
        return self.parts.join(", ")
    }
}

result = Builder().add("a").add("b").add("c").build()
print(result)
