# 闭包与上值机制

## 所属阶段
Phase 3.2 - 函数 + 闭包

## 前置任务
- 27-call-frame（调用帧与函数调用）

## 目标
实现闭包对象、上值（upvalue）机制、相关指令，使函数能正确捕获并修改外层作用域的变量，闭包语义符合引用捕获要求。

## 设计规格

### Closure 对象

参照 [11-bytecode-vm](../11-bytecode-vm.md) § Closure：

```
Closure {
    function: Gc<Function>
    upvalues: Vec<Gc<Upvalue>>
}
```

每个函数在运行时都包装为 Closure，即使不捕获任何上值（空 upvalues 数组）。

### Upvalue 机制

参照 [11-bytecode-vm](../11-bytecode-vm.md) § 编译单元 Upvalue：

```
Upvalue {
    index: usize          // 外层局部变量索引
    is_local: bool        // 是直接的外层局部变量，还是外层的上值
}
```

运行时存在两种状态的 upvalue：

- **开放上值（Open Upvalue）**：指向栈上的局部变量槽位。当变量仍在作用域内时使用。
- **关闭上值（Closed Upvalue）**：变量离开作用域时，将值从栈拷贝到堆分配的 `closed` 字段中，后续访问改为读写 `closed`。

```
RuntimeUpvalue {
    location: usize            // 栈位置（开放时）
    closed: Option<Object>     // 堆存储（关闭后）
    is_open: bool
}
```

### 指令

参照 [11-bytecode-vm](../11-bytecode-vm.md) § 闭包：

| OpCode | 操作数 | 说明 |
|---|---|---|
| `CLOSURE` | `func_idx(2)` | 创建闭包：从常量池取出 Function，创建 Closure 并捕获上值 |
| `LOAD_UPVALUE` | `idx(1)` | 将当前闭包的上值[idx]压栈 |
| `STORE_UPVALUE` | `idx(1)` | 将栈顶存入当前闭包的上值[idx] |
| `CLOSE_UPVALUE` | — | 关闭栈顶位置对应的所有开放上值 |

### 闭包语义

参照 [04-functions](../04-functions.md) § 闭包语义：

- 内层函数捕获外层变量的**引用**（不是值）
- 多个闭包可以共享同一个外层变量
- 外层函数返回后，被捕获的变量仍然存活（由 GC 管理）

## 实现细节

### 1. src/vm/object.rs — RuntimeUpvalue 与 Closure

```rust
pub struct RuntimeUpvalue {
    pub location: usize,
    pub closed: Option<Object>,
}

impl RuntimeUpvalue {
    pub fn new(location: usize) -> Self {
        Self {
            location,
            closed: None,
        }
    }

    pub fn get(&self, stack: &[Object]) -> Object {
        match &self.closed {
            Some(val) => val.clone(),
            None => stack[self.location].clone(),
        }
    }

    pub fn set(&mut self, stack: &mut [Object], value: Object) {
        if self.closed.is_some() {
            self.closed = Some(value);
        } else {
            stack[self.location] = value;
        }
    }

    pub fn close(&mut self, stack: &[Object]) {
        if self.closed.is_none() {
            self.closed = Some(stack[self.location].clone());
        }
    }
}

pub struct Closure {
    pub function: Gc<Function>,
    pub upvalues: Vec<Gc<RuntimeUpvalue>>,
}
```

### 2. src/compiler/mod.rs — 编译单元上值追踪

扩展 `CompilationUnit` 增加上值解析：

```rust
impl CompilationUnit {
    pub fn resolve_upvalue(&mut self, name: &str, parent: &mut CompilationUnit) -> Option<usize> {
        if let Some(idx) = parent.resolve_local(name) {
            parent.locals[idx].is_captured = true;
            let upvalue = Upvalue { index: idx, is_local: true };
            return Some(self.add_upvalue(upvalue));
        }

        if let Some(idx) = parent.resolve_upvalue_from_parent(name) {
            let upvalue = Upvalue { index: idx, is_local: false };
            return Some(self.add_upvalue(upvalue));
        }

        None
    }

    fn add_upvalue(&mut self, upvalue: Upvalue) -> usize {
        for (i, existing) in self.upvalues.iter().enumerate() {
            if existing.index == upvalue.index && existing.is_local == upvalue.is_local {
                return i;
            }
        }
        self.upvalues.push(upvalue);
        self.upvalues.len() - 1
    }
}
```

### 3. 编译闭包捕获

变量解析优先级调整：

1. 当前编译单元的局部变量 → `LOAD_LOCAL`
2. 外层编译单元的局部变量或上值 → 标记为上值，生成 `LOAD_UPVALUE`
3. 全局变量 → `LOAD_GLOBAL`

编译函数声明/匿名函数时：
1. 创建子编译单元
2. 编译函数体
3. 设置 `upvalue_count = sub_unit.upvalues.len()`
4. 在父编译单元生成 `CLOSURE(func_idx)`，紧跟 `upvalue_count` 个上值操作数

### 4. src/vm/mod.rs — CLOSURE 指令

```rust
OpCode::CLOSURE => {
    let func_idx = self.read_u16();
    let func = self.current_frame().function.constants[func_idx as usize].clone();

    let function = match func {
        Object::Function(f) => f,
        _ => return self.runtime_error("CLOSURE expects a Function"),
    };

    let upvalue_count = function.upvalue_count;
    let mut upvalues = Vec::with_capacity(upvalue_count);

    for _ in 0..upvalue_count {
        let is_local = self.read_byte() == 1;
        let index = self.read_byte() as usize;

        if is_local {
            let stack_base = self.current_frame().stack_base;
            let location = stack_base + index;
            let upvalue = self.capture_upvalue(location);
            upvalues.push(upvalue);
        } else {
            let parent_upvalue = self.current_frame().upvalues[index].clone();
            upvalues.push(parent_upvalue);
        }
    }

    let closure = Closure { function, upvalues };
    self.stack.push(Object::Closure(Gc::new(closure)));
}
```

### 5. src/vm/mod.rs — 上值捕获

```rust
impl VM {
    fn capture_upvalue(&mut self, location: usize) -> Gc<RuntimeUpvalue> {
        for upvalue in &self.open_upvalues {
            if upvalue.borrow().location == location {
                return upvalue.clone();
            }
        }

        let upvalue = Gc::new(RuntimeUpvalue::new(location));
        self.open_upvalues.push(upvalue.clone());
        upvalue
    }
}
```

### 6. LOAD_UPVALUE / STORE_UPVALUE

```rust
OpCode::LOAD_UPVALUE => {
    let idx = self.read_byte() as usize;
    let closure = self.current_frame_closure();
    let upvalue = &closure.upvalues[idx];
    let value = upvalue.borrow().get(&self.stack);
    self.stack.push(value);
}

OpCode::STORE_UPVALUE => {
    let idx = self.read_byte() as usize;
    let value = self.stack.last().unwrap().clone();
    let closure = self.current_frame_closure();
    let upvalue = closure.upvalues[idx].clone();
    upvalue.borrow_mut().set(&mut self.stack, value);
}
```

### 7. CLOSE_UPVALUE

```rust
OpCode::CLOSE_UPVALUE => {
    let stack_top = self.stack.len() - 1;
    self.close_upvalues_from(stack_top);
    self.stack.pop();
}
```

在作用域结束（block 退出）时，编译器对每个被捕获的局部变量生成 `CLOSE_UPVALUE`。

在 RETURN 时，也需要关闭当前帧所有开放上值：

```rust
fn close_upvalues_from(&mut self, last: usize) {
    let mut i = self.open_upvalues.len();
    while i > 0 {
        i -= 1;
        let upvalue = &self.open_upvalues[i];
        if upvalue.borrow().location < last {
            break;
        }
        upvalue.borrow_mut().close(&self.stack);
        self.open_upvalues.remove(i);
    }
}
```

### 8. CALL 指令适配闭包

修改 Task 27 的 CALL 指令，增加 Closure 分支：

```rust
Object::Closure(closure) => {
    let func = &closure.function;
    if argc != func.arity {
        return self.runtime_error(
            &format!("expected {} arguments, got {}", func.arity, argc)
        );
    }

    if self.call_stack.len() >= MAX_CALL_DEPTH {
        return self.runtime_error("stack overflow");
    }

    let stack_base = callee_idx;
    self.call_stack.push(CallFrame::new_closure(
        closure.clone(),
        stack_base,
    ));
}
```

同时修改 `CallFrame`，将 `function` 替换为 `closure`：

```rust
pub struct CallFrame {
    pub closure: Gc<Closure>,
    pub ip: usize,
    pub stack_base: usize,
    pub defer_stack_base: usize,
}
```

读取字节码和常量池改为从 `closure.function` 获取。顶层脚本也包装为无上值的 Closure。

## 验证标准

1. 内层函数能正确捕获外层局部变量（引用捕获，非值捕获）
2. 闭包能修改外层变量，修改对其他共享同一变量的闭包可见
3. 外层函数返回后，被捕获变量仍然存活
4. 多个闭包共享同一变量时，修改互相可见
5. 嵌套多层闭包时上值链正确解析
6. 开放上值在变量离开作用域时正确关闭
7. 所有函数调用统一使用 Closure 对象（包括无上值函数）

## 测试用例

```ms
fn make_counter() {
    count = 0
    return fn() {
        count += 1
        return count
    }
}

counter = make_counter()
print(counter())
print(counter())
print(counter())

fn make_pair() {
    x = 10
    getter = fn() { return x }
    setter = fn(v) { x = v }
    return getter, setter
}

get, set = make_pair()
print(get())
set(42)
print(get())
```

预期输出：

```
1
2
3
10
42
```
