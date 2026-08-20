# test_inheritance.ms — 继承与 super
# 覆盖: task 42（06-oop.md § 继承、super）

# 单继承：方法与属性继承
class Base {
    fn __init__(self, x) {
        self.x = x
    }
    fn get(self) {
        return self.x
    }
    fn tag(self) {
        return "base"
    }
}

class Child < Base {
    fn __init__(self, x, y) {
        super.__init__(x)
        self.y = y
    }
    fn sum(self) {
        return self.x + self.y
    }
    # 覆写
    fn tag(self) {
        return "child"
    }
}

c = Child(10, 20)
assert(c.get() == 10, "继承父类方法")
assert(c.tag() == "child", "子类覆写方法")
assert(c.sum() == 30, "子类自有方法")

# super 调用父类被覆写方法（自动绑定 self，不需显式传）
class Sub < Base {
    fn tag(self) {
        return "sub+" + super.tag()
    }
}
s = Sub(1)
assert(s.tag() == "sub+base", "super 调用父类实现")

# 多级继承链（三层）
class A1 {
    fn name(self) {
        return "A"
    }
}
class B1 < A1 {
    fn name(self) {
        return "B>" + super.name()
    }
}
class C1 < B1 {
    fn name(self) {
        return "C>" + super.name()
    }
}
assert(C1().name() == "C>B>A", "三层继承链 super 逐级上溯")

# 继承链上方法查找（子类未定义时沿链上溯）
class P2 {
    fn inherited_method(self) {
        return "from-p2"
    }
}
class Q2 < P2 {
}
assert(Q2().inherited_method() == "from-p2", "空子类继承父类方法")

# instanceof 沿继承链
q = Q2()
assert(isinstance(q, Q2), "isinstance 自身类")
assert(isinstance(q, P2), "isinstance 父类")

# 父类 __init__ 未调用时属性来自子类自己的赋值
class SkipInit < Base {
    fn __init__(self) {
        self.x = 999
    }
}
assert(SkipInit().get() == 999, "子类自定义初始化")

# super 在多态组合中的表现
fn describe(obj) {
    return obj.tag()
}
assert(describe(c) == "child", "多态分派到子类实现")
assert(describe(Base(1)) == "base", "多态分派到父类实现")

print("ALL PASSED")
