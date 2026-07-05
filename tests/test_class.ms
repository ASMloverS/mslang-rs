// test_class.ms — Class 定义与实例化

class Animal {
    kingdom = "Animalia"

    fn __init__(self, name, sound) {
        self.name = name
        self.sound = sound
    }

    fn speak(self) {
        return self.name + " says " + self.sound
    }

    fn __repr__(self) {
        return "Animal(" + self.name + ")"
    }
}

dog = Animal("Dog", "Woof")
print(dog.speak())
print(dog)
print(dog.kingdom)

// 动态属性
dog.age = 3
print(dog.age)

// 类属性共享
cat = Animal("Cat", "Meow")
print(cat.kingdom)
print(Animal.kingdom)

// 多个实例互不干扰
dog2 = Animal("Another Dog", "Bark")
print(dog2.speak())
print(dog.speak())

// __repr__ 用于 print
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
print(p)
