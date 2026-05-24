# 函数系统

## 函数定义

```
fn_def = "fn" IDENTIFIER "(" param_list? ")" block
param_list = param ("," param)*
param = IDENTIFIER
      | "*" IDENTIFIER        # 可变参数
      | IDENTIFIER "=" expr   # 默认参数值
```

```ms
fn greet(name) {
    return "Hello, " + name
}

fn add(a, b) {
    return a + b
}
```

### 参数类型

#### 普通参数

```ms
fn power(base, exp) {
    return base ** exp
}
```

#### 默认参数

```ms
fn greet(name, prefix = "Hello") {
    return prefix + ", " + name
}

greet("Alice")              # "Hello, Alice"
greet("Alice", "Hi")        # "Hi, Alice"
```

默认参数值在**函数定义时**求值一次（与 Python 一致）。

#### 可变参数

```ms
fn sum(*numbers) {
    total = 0
    for n in numbers {
        total += n
    }
    return total
}

sum(1, 2, 3)    # 6
sum(1, 2, 3, 4, 5)  # 15
```

`*numbers` 将多余的位置参数收集为一个 list。

#### 参数组合

```ms
fn example(a, b, c = 10, *rest) {
    # a, b: 必需参数
    # c: 带默认值的参数
    # rest: 可变参数（list）
}
```

参数顺序规则：普通参数 → 默认参数 → 可变参数。

### 返回值

#### 单返回值

```ms
fn double(x) {
    return x * 2
}
```

函数体执行到 `return` 时返回。无 `return` 语句时返回 `nil`。

#### 多返回值（元组）

```ms
fn divmod(a, b) {
    return a // b, a % b
}

q, r = divmod(10, 3)    # q=3, r=1
result = divmod(10, 3)  # result=(3, 1)
```

多返回值本质是返回元组，然后用元组解包。

## First-class 函数

函数是一等公民，可以：
- 赋值给变量
- 作为参数传递
- 作为返回值
- 存储在数据结构中

```ms
fn add(a, b) { return a + b }
fn mul(a, b) { return a * b }

ops = {"add": add, "mul": mul}
result = ops["add"](3, 4)  # 7

fn apply(f, x, y) {
    return f(x, y)
}
apply(add, 1, 2)  # 3
```

## 匿名函数

```
fn_literal = "fn" "(" param_list? ")" block
```

```ms
double = fn(x) { return x * 2 }

nums.map(fn(x) { return x * x })

fn(x) {
    temp = x * 2
    return temp + 1
}
```

匿名函数是完全功能的闭包，可以有任意复杂的函数体。

## 闭包

函数可以捕获其定义环境中的变量（上值 / upvalue）。

```ms
fn make_counter() {
    count = 0
    return fn() {
        count += 1
        return count
    }
}

counter = make_counter()
counter()   # 1
counter()   # 2
counter()   # 3
```

### 闭包语义

- 内层函数捕获外层变量的**引用**（不是值）
- 多个闭包可以共享同一个外层变量
- 外层函数返回后，被捕获的变量仍然存活（由 GC 管理）

```ms
fn make_pair() {
    x = 10
    getter = fn() { return x }
    setter = fn(v) { x = v }
    return getter, setter
}

get, set = make_pair()
get()       # 10
set(42)
get()       # 42
```

## 递归

函数天然支持递归调用：

```ms
fn factorial(n) {
    if n <= 1 {
        return 1
    }
    return n * factorial(n - 1)
}

factorial(10)  # 3628800
```

尾调用优化暂不实现（后续版本考虑）。

## 内置函数

以下函数全局可用（详见 [10-builtins](10-builtins.md)）：

```ms
print(val)           # 打印到标准输出
type(val)            # 返回类型名称字符串
len(val)             # 返回长度
range(start, end)    # 返回迭代器
input(prompt)        # 读取用户输入
# ... 更多见 builtins 文档
```

## 方法调用

方法调用通过 `.` 运算符：

```ms
"hello".length()         # 5
[1,2,3].push(4)          # 追加元素
"abc".split("b")         # ["a", "c"]
```

方法调用的语义：
1. 查找对象的类型
2. 在类型的方法表中查找方法名
3. 将对象自身作为第一个参数 `self` 传入

等价于：
```ms
str_length("hello")
list_push([1,2,3], 4)
```

用户定义的 class 方法同理：

```ms
class Dog {
    fn bark(self) {
        return "Woof!"
    }
}

d = Dog()
d.bark()    # 调用 Dog.bark，self = d
```
