# 装饰器

## 所属阶段
Phase 5.5 - 类 + OOP

## 前置任务
43-magic-methods

## 目标
实现装饰器语法（@expr），支持函数装饰器、带参数装饰器、多重装饰器、类装饰器。

## 设计规格

参照 [07-advanced](../07-advanced.md) § 装饰器：

### 语法

```
decorator  = "@" expression newline
decorated  = decorator+ (fn_def | class_def)
```

### 语义

装饰器是语法糖，编译时将函数/类定义与装饰器表达式组合：

```ms
@log
fn greet(name) {
    return "Hello, " + name
}

# 等价于：
fn greet(name) {
    return "Hello, " + name
}
greet = log(greet)
```

### 多重装饰器

```ms
@d1
@d2
fn func() {}

# 等价于：
fn func() {}
func = d1(d2(func))
```

装饰器从下到上应用（靠近函数的先执行）。

### 带参数的装饰器

```ms
@retry(3)
fn unreliable() {}

# 等价于：
fn unreliable() {}
unreliable = (retry(3))(unreliable)
```

`@retry(3)` 先调用 `retry(3)` 返回真正的装饰器函数，再将 `unreliable` 传给该函数。

### 类装饰器

```ms
@add_repr
class Foo {}

# 等价于：
class Foo {}
Foo = add_repr(Foo)
```

## 实现细节

### 1. 解析装饰器

`src/parser/statement.rs`：

```rust
fn parse_decorators(&mut self) -> Result<Vec<Expr>> {
    let mut decorators = Vec::new();
    
    while self.match_token(TokenKind::At)? {
        let expr = self.parse_expression()?;
        self.consume(TokenKind::Newline)?;
        decorators.push(expr);
    }
    
    // decorators 是从上到下的顺序
    // 即 decorators[0] 是最外层装饰器
    Ok(decorators)
}
```

在解析顶层声明时：

```rust
fn parse_declaration(&mut self) -> Result<Stmt> {
    let decorators = self.parse_decorators()?;
    
    let stmt = if self.check(TokenKind::Fn)? {
        self.parse_function()?
    } else if self.check(TokenKind::Class)? {
        self.parse_class()?
    } else {
        return Err(parse_error("expected function or class after decorator"));
    };
    
    Ok(Stmt::Decorated {
        decorators,
        target: Box::new(stmt),
    })
}
```

### 2. AST 节点

```rust
struct Decorated {
    decorators: Vec<Expr>,   // @ 后的表达式列表
    target: Box<Stmt>,        // fn_def 或 class_def
}
```

### 3. 编译装饰器

`src/compiler/statement.rs`：

```
编译 @d1 @d2 fn f(args) { body }:

1. 编译 fn f(args) { body }
   → 函数已定义，f 存入局部/全局变量
   → 栈上留有函数值

2. 应用装饰器（从内到外，即从下到上）:
   对于 decorators = [d1, d2]（从上到下）
   应用顺序：先 d2，再 d1

   // 应用 d2
   a. 编译 d2 表达式       → 压栈装饰器
   b. emit SWAP             → 交换栈顶：[func, d2] → [d2, func]
   c. emit CALL 1           → d2(func)
   // 栈顶现在是 d2(func) 的结果

   // 应用 d1
   d. 编译 d1 表达式       → 压栈装饰器
   e. emit SWAP
   f. emit CALL 1           → d1(d2(func))

3. emit STORE_GLOBAL "f"    → 用装饰后的结果覆盖原函数名
```

更精确的实现：

```rust
fn compile_decorated(&mut self, decorated: &Decorated) -> Result<()> {
    // 先编译目标（fn 或 class）
    self.compile_stmt(&decorated.target)?;
    
    // 目标编译后，函数/类值在栈顶，名称已存入全局变量
    // 重新加载到栈顶
    let target_name = decorated.target.name();
    self.emit_load_global(target_name);
    
    // 反向遍历装饰器（从内到外）
    for dec_expr in decorated.decorators.iter().rev() {
        self.compile_expr(dec_expr)?;  // 装饰器函数
        // 栈: [current_func, decorator]
        // 需要交换参数顺序：decorator(current_func)
        self.emit_swap();
        self.emit_call(1);
        // 栈顶为装饰后的结果
    }
    
    // 将最终结果存回变量名
    self.emit_store_global(target_name);
    
    Ok(())
}
```

### 4. 带参数的装饰器

`@retry(3)` 中 `retry(3)` 本身是一个完整的表达式（函数调用），解析时 `parse_expression()` 会将其解析为 `Call(Identifier("retry"), [Int(3)])`。

编译时直接编译该表达式，得到的是一个函数（`retry` 的返回值），然后以此函数作为装饰器：

```
@retry(3)
fn f() {}

等价编译为:
1. fn f() {}
2. retry(3)        → 返回装饰器函数
3. 调用 装饰器(f)  → 返回装饰后的函数
4. f = 结果
```

不需要特殊处理——`parse_expression()` 已正确解析 `retry(3)` 为函数调用表达式。

### 5. SWAP 指令

需要引入一个 SWAP 或在调用前调整栈：

```rust
OpCode::SWAP => {
    let len = self.stack.len();
    self.stack.swap(len - 1, len - 2);
}
```

或者不引入新指令，使用已有的 DUP + ROT：

```
// 调用 decorator(func)：
// 栈: [func, decorator]

1. emit LOAD_GLOBAL decorator_name
// 栈: [func, decorator]

2. emit SWAP
// 栈: [decorator, func]

不对，应该是 decorator(func)：
栈需要是 [decorator, func]，然后 CALL 1

如果编译顺序是先 func 后 decorator：
栈: [func, decorator]
需要 SWAP: [decorator, func]
然后 CALL 1
```

### 6. 函数 name 属性

被装饰后的函数应保留原始函数名。可以在 Closure 上增加 `name` 属性，装饰后可通过闭包捕获原函数名。

或者更简单：`fn.name` 返回函数定义时的名称，不受装饰影响（因为装饰只是替换了变量绑定，原函数的 name 不变）。

## 验证标准

1. `@dec` + `fn f()` 等价于 `fn f() {}; f = dec(f)`
2. 多重装饰器从下到上应用
3. 带参数装饰器 `@dec(args)` 正确解析和执行
4. 类装饰器正确工作
5. 装饰后函数可通过原名称调用
6. 原始函数的 name 属性保留

## 测试用例

```ms
// test_decorators.ms — 装饰器

// 基本装饰器
fn log(func) {
    return fn(*args) {
        print("calling " + func.name)
        result = func(*args)
        print("returned " + str(result))
        return result
    }
}

@log
fn double(x) {
    return x * 2
}

result = double(5)

// 带参数的装饰器
fn add_tag(tag) {
    return fn(func) {
        return fn(*args) {
            return "<" + tag + ">" + func(*args) + "</" + tag + ">"
        }
    }
}

@add_tag("b")
fn get_text() {
    return "hello"
}

print(get_text())

// 多重装饰器
fn d1(func) {
    return fn() {
        return "d1(" + func() + ")"
    }
}

fn d2(func) {
    return fn() {
        return "d2(" + func() + ")"
    }
}

@d1
@d2
fn greet() {
    return "hi"
}

print(greet())

// 类装饰器
fn add_greet(cls) {
    cls.greet = fn(self) {
        return "Hello from " + cls.__name__
    }
    return cls
}

@add_greet
class Foo {
    fn __init__(self) {}
}

f = Foo()
print(f.greet())
```

预期输出：

```
calling double
returned 10
<b>hello</b>
d1(d2(hi))
Hello from Foo
```
