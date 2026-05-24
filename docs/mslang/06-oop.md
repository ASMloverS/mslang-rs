# 面向对象

## 类定义

```
class_def = "class" IDENTIFIER ("<" IDENTIFIER)? "{" class_body "}"
class_body = (method_def | class_var)*
method_def = "fn" IDENTIFIER "(" param_list? ")" block
class_var = "var"? IDENTIFIER "=" expression
```

```ms
class Animal {
    # 类属性（所有实例共享）
    kingdom = "Animalia"

    # 构造方法
    fn __init__(self, name, sound) {
        self.name = name
        self.sound = sound
    }

    # 实例方法
    fn speak(self) {
        return self.name + " says " + self.sound
    }

    # 字符串表示
    fn __repr__(self) {
        return "Animal(" + self.name + ")"
    }
}
```

## 实例化

```ms
dog = Animal("Dog", "Woof")
dog.speak()      # "Dog says Woof"
print(dog)       # "Animal(Dog)"  调用 __repr__
```

使用 `ClassName(args)` 语法创建实例。等价于：
1. 创建新的空对象
2. 调用 `__init__(self, args...)`
3. 返回该对象

## self

`self` 是实例方法的第一个参数，引用当前实例对象。

- 在方法内部通过 `self.attr` 访问实例属性
- 在方法内部通过 `self.attr = val` 设置实例属性
- `self` 不需要在调用时传入（编译器自动绑定）

```ms
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
}

p1 = Point(3, 4)
p2 = Point(0, 0)
p1.distance_to(p2)   # 5.0
```

## 实例属性

实例属性通过 `self.attr = val` 在 `__init__` 或其他方法中创建。

```ms
class Person {
    fn __init__(self, name, age) {
        self.name = name
        self.age = age
    }
}

p = Person("Alice", 30)
print(p.name)     # "Alice"
p.age = 31        # 修改属性
```

也可以在实例创建后动态添加属性：

```ms
p.email = "alice@example.com"    # 动态添加新属性
```

## 类属性

在 class 体内（不在方法内）定义的变量为类属性，所有实例共享。

```ms
class Counter {
    count = 0

    fn __init__(self) {
        Counter.count += 1
    }
}

Counter.count      # 0
a = Counter()
Counter.count      # 1
b = Counter()
Counter.count      # 2
```

通过 `ClassName.attr` 或 `self.attr`（如果实例没有同名属性）访问类属性。

## 继承

```
class_def = "class" IDENTIFIER ("<" IDENTIFIER)? "{" class_body "}"
```

使用 `<` 表示继承（单继承）：

```ms
class Animal {
    fn __init__(self, name) {
        self.name = name
    }

    fn speak(self) {
        return self.name + " speaks"
    }
}

class Dog < Animal {
    fn __init__(self, name, breed) {
        super.__init__(name)
        self.breed = breed
    }

    fn speak(self) {
        return self.name + " barks"
    }
}

d = Dog("Rex", "Shepherd")
d.speak()          # "Rex barks"   （方法覆盖）
d.name             # "Rex"         （继承自 Animal）
```

### 继承规则

- 仅支持**单继承**
- 子类继承父类的所有属性和方法
- 子类可以覆盖父类方法
- `super` 关键字引用父类

### super

在子类方法中使用 `super` 调用父类方法：

```ms
class Child < Parent {
    fn __init__(self, name, extra) {
        super.__init__(name)
        self.extra = extra
    }

    fn method(self) {
        result = super.method()    # 调用父类的 method
        return result + " enhanced"
    }
}
```

### 方法解析顺序 (MRO)

单继承场景下，MRO 就是简单的从子类到父类的线性链。

```
Dog -> Animal -> Object
```

每个类隐式继承自 `Object`（如果不显式指定父类）。

## 魔术方法

魔术方法以 `__` 前缀和后缀命名，在特定场景下自动调用。

### 构造与析构

| 方法 | 触发时机 |
|---|---|
| `__init__(self, ...)` | 实例创建时 |
| `__del__(self)` | 实例被 GC 回收时 |

### 字符串表示

| 方法 | 触发时机 |
|---|---|
| `__repr__(self)` | `print(obj)`, `str(obj)` |
| `__str__(self)` | 字符串转换（优先于 `__repr__`） |

### 比较运算

| 方法 | 对应运算符 |
|---|---|
| `__eq__(self, other)` | `==` |
| `__ne__(self, other)` | `!=` |
| `__lt__(self, other)` | `<` |
| `__le__(self, other)` | `<=` |
| `__gt__(self, other)` | `>` |
| `__ge__(self, other)` | `>=` |

### 算术运算

| 方法 | 对应运算符 |
|---|---|
| `__add__(self, other)` | `+` |
| `__sub__(self, other)` | `-` |
| `__mul__(self, other)` | `*` |
| `__div__(self, other)` | `/` |
| `__floordiv__(self, other)` | `//` |
| `__mod__(self, other)` | `%` |
| `__pow__(self, other)` | `**` |

### 容器协议

| 方法 | 触发时机 |
|---|---|
| `__len__(self)` | `len(obj)` |
| `__getitem__(self, key)` | `obj[key]` |
| `__setitem__(self, key, val)` | `obj[key] = val` |
| `__contains__(self, item)` | `item in obj` |
| `__iter__(self)` | `for x in obj` |

### 上下文管理器

| 方法 | 触发时机 |
|---|---|
| `__enter__(self)` | 进入 `with` 块 |
| `__exit__(self, err_type, err_msg, tb)` | 离开 `with` 块 |

### 可调用对象

| 方法 | 触发时机 |
|---|---|
| `__call__(self, ...)` | `obj(args)` |

```ms
class Multiplier {
    fn __init__(self, factor) {
        self.factor = factor
    }

    fn __call__(self, x) {
        return x * self.factor
    }
}

double = Multiplier(2)
double(5)     # 10
```

### 迭代器协议

实现 `__iter__` 和 `__next__` 的对象是迭代器：

```ms
class Countdown {
    fn __init__(self, start) {
        self.current = start
    }

    fn __iter__(self) {
        return self
    }

    fn __next__(self) {
        if self.current <= 0 {
            throw StopIteration()
        }
        val = self.current
        self.current -= 1
        return val
    }
}

for i in Countdown(5) {
    print(i)    # 5, 4, 3, 2, 1
}
```

## Object 基类

### 类内置属性

每个类对象自动拥有以下属性：

| 属性 | 类型 | 说明 |
|---|---|---|
| `__name__` | string | 类名字符串 |

```ms
class Dog {
    fn __init__(self) {}
}
Dog.__name__     # "Dog"
```

所有类的隐式基类，提供：

```ms
class Object {
    fn __repr__(self) {
        return type(self) + " instance"
    }

    fn __eq__(self, other) {
        return self is other
    }

    fn __ne__(self, other) {
        return not (self == other)
    }
}
```

## 动态特性

### 动态属性

```ms
obj.new_attr = "dynamic"     # 运行时添加属性
```

### 运算符重载

通过魔术方法自定义运算符行为：

```ms
class Vector {
    fn __init__(self, x, y) {
        self.x = x
        self.y = y
    }

    fn __add__(self, other) {
        return Vector(self.x + other.x, self.y + other.y)
    }

    fn __repr__(self) {
        return "Vector(" + str(self.x) + ", " + str(self.y) + ")"
    }
}

v1 = Vector(1, 2)
v2 = Vector(3, 4)
v3 = v1 + v2     # Vector(4, 6)
```
