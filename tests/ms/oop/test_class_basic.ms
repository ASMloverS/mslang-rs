# test_class_basic.ms — Class 定义与实例化
# 覆盖: task 40/41（06-oop.md § 类定义、self 绑定）

# 基本类：属性、方法、__init__
class Animal {
    kingdom = "Animalia"

    fn __init__(self, name, sound) {
        self.name = name
        self.sound = sound
    }

    fn speak(self) {
        return self.name + " says " + self.sound
    }
}

dog = Animal("Dog", "Woof")
assert(dog.speak() == "Dog says Woof", "实例方法调用")
assert(dog.kingdom == "Animalia", "类属性经实例读取")
assert(Animal.kingdom == "Animalia", "类属性经类读取")

# 类属性为所有实例共享（经类修改）
class Counter3 {
    total = 0

    fn __init__(self) {
        Counter3.total += 1
    }
}
Counter3()
Counter3()
c3 = Counter3()
assert(Counter3.total == 3, "类属性跨实例共享")
assert(c3.total == 3, "实例读取类属性")

# 动态实例属性（task 41）：属性不跨实例（读取不存在属性会报错，
# 以独立动态属性互不干扰验证）
dog.age = 3
assert(dog.age == 3, "动态属性赋值")
cat = Animal("Cat", "Meow")
assert(cat.speak() == "Cat says Meow", "其他实例不受动态属性影响")

# 多实例互不干扰
d2 = Animal("Another", "Bark")
assert(d2.speak() == "Another says Bark", "第二实例独立")
assert(dog.speak() == "Dog says Woof", "第一实例不受影响")

# 方法内修改实例属性
class Accum {
    fn __init__(self) {
        self.sum = 0
    }
    fn add(self, v) {
        self.sum += v
        return self.sum
    }
}
acc = Accum()
acc.add(5)
acc.add(10)
assert(acc.sum == 15, "实例状态在方法调用间保持")

# isinstance 检查
assert(isinstance(dog, Animal), "isinstance 实例")
assert((isinstance(5, Animal)) == false, "isinstance 非实例")
assert(isinstance(5, int), "isinstance 内置类型 int")
assert(isinstance("x", str), "isinstance 内置类型 str")
assert(isinstance([1], list), "isinstance 内置类型 list")

# 实例方法绑定后作为值传递
speak_fn = dog.speak
assert(speak_fn() == "Dog says Woof", "绑定方法作为一等值")

print("ALL PASSED")
