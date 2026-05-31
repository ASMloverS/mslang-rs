# for..in 循环与迭代器协议

## 所属阶段
Phase 4.1 - 控制流 + 高级语法

## 前置任务
31-default-variadic-params

## 目标
实现 for..in 循环语句及迭代器协议，支持对所有内置可迭代类型（list、tuple、dict、set、string、range、generator）的遍历。

## 设计规格

参照 [05-control-flow](../05-control-flow.md) § for..in：

### 语法

```
for_stmt = "for" IDENTIFIER "in" expression block
         | "for" IDENTIFIER "," IDENTIFIER "in" expression block
```

### 字节码指令

参照 [11-bytecode-vm](../11-bytecode-vm.md) § 迭代：

| OpCode | 操作数 | 说明 |
|---|---|---|
| `ITERATOR` | — | 从栈顶可迭代对象创建迭代器 |
| `FOR_ITER` | `offset(2)` | 获取下一个元素；迭代结束则跳转到 offset |

### for..in 语义

- 每次迭代从可迭代对象取下一个值，赋值给循环变量
- 迭代结束后退出循环
- **循环变量在循环结束后保持最后一次的值**
- break/continue 只影响最内层循环
- 不支持带标签的 break/continue

### 迭代器协议

可迭代对象必须实现：
- `__iter__()` → 返回迭代器（通常返回 self）
- `__next__()` → 返回下一个值；无更多元素时抛出 `StopIteration`

### 可迭代类型

| 类型 | 遍历内容 |
|---|---|
| list | 元素 |
| tuple | 元素 |
| dict | 键 |
| set | 元素（无序） |
| string | 字符 |
| range | 整数序列 |
| generator | yield 产出的值 |

### 双变量形式

`for key, value in expr` 中，`expr` 的每次迭代产出必须为恰好包含两个元素的序列（如 tuple），解包赋值给 key 和 value。

## 实现细节

### 1. 编译 for..in 循环

`src/compiler/statement.rs`：

```
编译 for item in iterable { body }：

1. 编译 iterable 表达式 → 压栈
2. emit ITERATOR → 栈顶变为迭代器
3. 循环起点标记 loop_start
4. emit FOR_ITER end_offset
   - 成功：栈顶为当前元素
   - 结束：跳转到 end
5. 将栈顶元素 STORE_LOCAL 到循环变量
6. 编译循环体
7. emit JUMP_BACK loop_start
8. 循环结束标记 end
9. emit POP（弹出迭代器）
```

双变量形式额外步骤：

```
编译 for key, value in iterable { body }：

4a. FOR_ITER 成功后栈顶为迭代产出
4b. emit UNPACK 2 → 栈顶弹出序列，压入两个元素
4c. STORE_LOCAL key, STORE_LOCAL value
```

### 2. ITERATOR 指令实现

`src/vm/mod.rs`：

```rust
OpCode::ITERATOR => {
    let iterable = self.stack_pop();
    let iter = match &iterable {
        Object::Ref(ptr) => {
            let tag = unsafe { (*(*ptr)).type_tag };
            if tag == TypeTag::LIST as u8 {
                IteratorKind::List(ListIterator::new(ptr))
            } else if tag == TypeTag::TUPLE as u8 {
                IteratorKind::Tuple(TupleIterator::new(ptr))
            } else if tag == TypeTag::DICT as u8 {
                IteratorKind::Dict(DictKeyIterator::new(ptr))
            } else if tag == TypeTag::SET as u8 {
                IteratorKind::Set(SetIterator::new(ptr))
            } else if tag == TypeTag::STRING as u8 {
                IteratorKind::String(CharIterator::new(ptr))
            } else if tag == TypeTag::ITERATOR as u8 {
                IteratorKind::Range(RangeIterator::new(ptr))
            } else if tag == TypeTag::GENERATOR as u8 {
                IteratorKind::Generator(ptr)
            } else {
                // 尝试调用 __iter__() 方法
                if let Some(iter_method) = get_method(&iterable, "__iter__") {
                    let result = call_method(iter_method, &[]);
                    IteratorKind::Custom(result)
                } else {
                    return Err(MspError::RuntimeError {
                        message: format!("type '{}' is not iterable", type_name(&iterable)),
                    });
                }
            }
        }
        other => {
            return Err(MspError::RuntimeError {
                message: format!("type '{}' is not iterable", type_name(other)),
            });
        }
    };
    self.stack_push(alloc_iterator(IteratorState::from(iter)));
}
```

### 3. FOR_ITER 指令实现

```rust
OpCode::FOR_ITER => {
    let offset = self.read_u16();
    let iter = self.stack_peek_mut(0); // 不弹出迭代器
    match iter.next() {
        Some(value) => self.stack_push(value),
        None => {
            self.stack_pop(); // 弹出迭代器
            self.ip += offset as usize; // 跳转
        }
    }
}
```

### 4. break/continue 编译

break：

```
1. 弹出循环体内创建的局部变量（恢复作用域深度）
2. emit POP（弹出迭代器）
3. emit BREAK end_offset
```

continue：

```
1. 弹出循环体内创建的局部变量
2. emit JUMP_BACK loop_start（回到 FOR_ITER）
```

### 5. Iterator 对象

`src/vm/object.rs`：

```rust
enum IteratorKind {
    List(ListIterator),
    Tuple(TupleIterator),
    Dict(DictKeyIterator),
    Set(SetIterator),
    String(CharIterator),
    Range(RangeIterator),
    Generator(*mut MsObjHeader),  // 指向 MsGenerator（TypeTag::GENERATOR）
    Custom(Object),
}

struct MslangIterator {
    kind: IteratorKind,
}
```

每种内置迭代器维护自己的索引和底层数据的引用。

## 验证标准

1. for..in 能正确遍历 list、tuple、dict、set、string、range
2. 循环变量在循环结束后保持最后一次的值
3. break 正确跳出循环
4. continue 正确跳到下一次迭代
5. 双变量形式正确解包键值对
6. 嵌套循环中 break/continue 只影响最内层
7. 对非可迭代类型使用 for..in 抛出运行时错误

## 测试用例

```ms
// test_for_in.ms — for..in 循环与迭代器

// 基本列表遍历
for item in [1, 2, 3] {
    print(item)
}

// 字符串遍历
for ch in "abc" {
    print(ch)
}

// 字典遍历（键）
d = {"x": 1, "y": 2}
for key in d {
    print(key)
}

// 字典键值对遍历
for key, value in d.items() {
    print(key + "=" + str(value))
}

// range 遍历
for i in range(3) {
    print(i)
}

// 循环变量保持最后值
for x in [10, 20, 30] {}
print(x)

// break
for i in range(100) {
    if i == 3 {
        break
    }
}
print(i)

// continue
result = []
for i in range(5) {
    if i % 2 == 0 {
        continue
    }
    result.push(i)
}
print(result)

// 嵌套循环
count = 0
for i in range(3) {
    for j in range(3) {
        count += 1
    }
}
print(count)
```

预期输出：

```
1
2
3
a
b
c
x
y
x=1
y=2
0
1
2
30
3
[1, 3]
9
```
