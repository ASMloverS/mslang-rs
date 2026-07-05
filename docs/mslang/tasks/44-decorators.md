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

> **注**：`03-syntax.md` 的语句章节暂未列入 `decorated` 产生式，实际文法以本节（`07-advanced.md` § 装饰器）为准；后续 doc-sync 应将其补入 03-syntax。

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

在解析声明时（装饰器可用于顶层或函数体内的 fn/class 定义）：

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
    
    // 无装饰器时直接返回，避免所有 fn/class 都被包成 Decorated 节点
    if decorators.is_empty() {
        return Ok(stmt);
    }
    
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
编译 @d1 @d2 fn f(args) { body }（等价于 fn f(args){body}; f = d1(d2(f))）:

1. 编译 fn f(args) { body }
   → fn 定义是语句，编译后栈上不留值
   → 函数值已绑定到变量 f（顶层=全局，函数体内=局部）

2. 应用装饰器（从内到外，即从下到上）:
   decorators = [d1, d2]（从上到下存储）
   应用顺序：先 d2，再 d1

   // 应用 d2：计算 d2(f)
   a. 编译 d2 表达式        → 栈: [d2]
   b. emit LOAD f            → 栈: [d2, f]
   c. emit CALL 1            → 栈: [d2(f)]
   d. emit STORE f           → f = d2(f)，栈平衡

   // 应用 d1：计算 d1(f)
   e. 编译 d1 表达式        → 栈: [d1]
   f. emit LOAD f            → 栈: [d1, f]
   g. emit CALL 1            → 栈: [d1(f)]
   h. emit STORE f           → f = d1(d2(f))，栈平衡

3. 最终变量 f 即为装饰后的结果，无需额外收尾
```

关键点：
- **不需要 SWAP/ROT 指令**。先压装饰器、再压被装饰值，CALL 的栈序天然为 `decorator(func)`（callable 在下、参数在上），与 `11-bytecode-vm.md` 现有指令集一致。
- **语句编译不留栈值**：`compile_stmt(target)` 编译 fn/class 定义后栈平衡，值已存入对应变量，由后续 `LOAD` 显式取回。
- **每次循环结尾 STORE 回变量名**，栈始终平衡，无残留。

更精确的实现：

```rust
fn compile_decorated(&mut self, decorated: &Decorated) -> Result<()> {
    // 无装饰器：直接编译目标，避免无谓的 load/store
    if decorated.decorators.is_empty() {
        return self.compile_stmt(&decorated.target);
    }

    // 1. 编译目标（fn 或 class）。语句编译不留栈值，值已绑定到变量。
    self.compile_stmt(&decorated.target)?;

    // 2. 目标名与作用域：顶层用 GLOBAL，函数体内用 LOCAL(slot)
    let target_name = decorated.target.name();
    let is_global = self.current_scope().is_top_level();

    // 3. 反向遍历装饰器（从内到外：靠近函数的先应用）
    for dec_expr in decorated.decorators.iter().rev() {
        self.compile_expr(dec_expr)?;                 // 栈: [decorator]
        self.emit_load(target_name, is_global)?;      // 栈: [decorator, current]
        self.emit_call(1);                            // 栈: [decorator(current)]
        self.emit_store(target_name, is_global)?;     // 变量 = 结果，栈平衡
    }

    Ok(())
}
```

`emit_load` / `emit_store` 按作用域选择指令：
```rust
fn emit_load(&mut self, name: &str, is_global: bool) -> Result<()> {
    if is_global { self.emit_load_global(name) }
    else { let slot = self.resolve_local(name)?; self.emit_load_local(slot); Ok(()) }
}
fn emit_store(&mut self, name: &str, is_global: bool) -> Result<()> {
    if is_global { self.emit_store_global(name) }
    else { let slot = self.resolve_local(name)?; self.emit_store_local(slot); Ok(()) }
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

### 5. 栈序与指令集

装饰器编译**不需要任何新指令**，`11-bytecode-vm.md` 现有的 `LOAD_*` / `STORE_*` / `CALL` 即可完整表达。

CALL 的栈序约定为 callable 在下、参数在上，因此只要编译顺序为"先装饰器表达式、后被装饰值"，栈即为 `[decorator, func]`，直接 `CALL 1` 得 `decorator(func)`，无需 SWAP/ROT/DUP 交换。

> 注：早期方案曾考虑引入 `OpCode::SWAP`，但 `11-bytecode-vm.md` 的 OpCode 集并不包含 `SWAP` 或 `ROT`，故弃用。

### 6. 函数 name 属性

`fn.name` 返回函数定义时的名称，不受装饰影响——装饰只是替换了变量绑定，原 Function/Closure 对象的 `name` 字段不变（`06-oop.md` 已规定函数对象自动拥有 `name` 属性，类对象拥有 `__name__`）。因此本 task 无需为 name 做额外工作。

> **AST 依赖**：编译期需获取被装饰目标的名称，要求 AST 中 `Stmt::Function` / `Stmt::Class` 变体暴露 `name()` 方法（分别返回函数名 / 类名）。该接口应由 Phase 1 的 AST 任务提供。

## 验证标准

1. `@dec` + `fn f()` 等价于 `fn f() {}; f = dec(f)`
2. 多重装饰器从下到上应用
3. 带参数装饰器 `@dec(args)` 正确解析和执行
4. 类装饰器正确工作
5. 装饰后函数可通过原名称调用
6. 原始函数的 name 属性保留
7. 装饰器表达式返回非可调用值（如 int）时，后续通过原名称调用抛出 TypeError
8. `@` 后非 fn/class（如 `@dec\nx = 1`）抛出解析错误
9. 函数体内的局部 `@dec fn ...` 正确绑定到局部作用域（不污染全局）

## 测试用例

```ms
// test_decorators.ms — 装饰器

// 基本装饰器（包装单参数函数，避免调用端 *args 展开）
fn log(func) {
    return fn(x) {
        print("calling " + func.name)
        result = func(x)
        print("returned " + str(result))
        return result
    }
}

@log
fn double(x) {
    return x * 2
}

result = double(5)

// 带参数的装饰器（包装零参数函数）
fn add_tag(tag) {
    return fn(func) {
        return fn() {
            return "<" + tag + ">" + func() + "</" + tag + ">"
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

// 边界：装饰器返回非可调用值，调用时抛 TypeError
fn bad(func) {
    return 42
}

@bad
fn h() {
    return 1
}

try {
    h()
    print("no error")
} except TypeError {
    print("TypeError caught")
}

// 边界：函数体内的局部装饰器，不污染全局
fn make() {
    @log
    fn inner(x) {
        return x + 1
    }
    return inner(10)
}

print(make())
```

预期输出：

```
calling double
returned 10
<b>hello</b>
d1(d2(hi))
Hello from Foo
TypeError caught
calling inner
returned 11
```
