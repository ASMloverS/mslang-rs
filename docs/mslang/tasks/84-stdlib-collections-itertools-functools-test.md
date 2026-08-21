# 标准库 - heapq / collections / itertools / functools / test

## 所属阶段
Phase 9 - 标准库扩展（M6）

## 前置任务
79-embedded-ms, 80-stdlib-math-string-sort

> **依赖说明**：heapq 为原生模块（直接操作 list 堆容器，Rust 实现比较器）；collections/
> itertools/functools/test 为**嵌入式 .ms 模块**（依赖 task 79 的嵌入机制）。Counter.
> most_common 与排序 key 依赖 task 80 的 sorted_by。
> 设计总纲见 [16-stdlib-expansion](../16-stdlib-expansion.md) §4.11-4.15。

## 目标

1. 原生 `heapq` 模块（最小堆，Python heapq 语义，5 个函数）。
2. `.ms` 嵌入模块 `collections`（deque/Counter/defaultdict 三个 class）。
3. `.ms` 嵌入模块 `itertools`（14 个生成器函数）。
4. `.ms` 嵌入模块 `functools`（partial/memoize/reduce）。
5. `.ms` 嵌入模块 `test`（9 个断言函数）。

## 设计规格

### heapq（原生）

参照 [16-stdlib-expansion](../16-stdlib-expansion.md) §4.11：

| 函数 | 签名 | 说明 |
|---|---|---|
| heapify | (lst) -> nil | 原地建堆（sift-down 自底向上） |
| heap_push | (lst, v) -> nil | 尾插 + sift-up |
| heap_pop | (lst) -> value | 首位弹出（尾元素补首 + sift-down）；空 → IndexError |
| push_pop | (lst, v) -> value | push 后立即 pop 最小（合并语义，一次 sift） |
| n_largest/n_smallest | (lst, n) -> list | 前 n 大（降序）/小（升序）；不改原 list；n≤0 → [] |

比较沿用对象 `compare`（同 sorted 语义，CmpOp::Less）；跨类型比较错误上抛。

### collections（.ms 嵌入）

参照 [16-stdlib-expansion](../16-stdlib-expansion.md) §4.12。
三个 class（class 实例而非 dict 子类——dict 为内建类型不可继承；
经 `__len__`/`__getitem__`/`__iter__` 魔术方法接通 `len()`/`[]`/for-in）：

- **deque**：内部 list 容量倍增 + head 偏移的循环缓冲，两端均摊 O(1)。
  `push_back/push_front/pop_back/pop_front/front/back/extend(iter)/to_list/is_empty/
  __len__/__iter__`；空弹出 → IndexError。
- **Counter**：`(iterable?)` 构造；`__getitem__` 缺失返回 0（**不写入**，Python 语义）；
  `update(other)`；`most_common(n?)`（sorted_by 按频次降序）；`elements()` 生成器；
  `items()/get(k, d=0)`。
- **defaultdict**：`(default_factory)` 构造；`__getitem__` 缺失 → 调 factory() 存入并返回；
  factory 为 nil → KeyError；`get` 不触发 factory（Python 一致）。

### itertools（.ms 嵌入，惰性生成器）

参照 [16-stdlib-expansion](../16-stdlib-expansion.md) §4.13：

count(start=0, step=1) / cycle(iter) / repeat(x, n?) / chain(*iters) /
take_while(pred, it) / drop_while(pred, it) / pairwise(it) / accumulate(it, fn?) /
zip_longest(*iters)（fill=nil 固定） / product(*iters) / combinations(it, r) /
permutations(it, r?) / islice(it, start, stop, step=1) / batched(it, n)。

- 无限序列（count/cycle/repeat 无 n）必须以生成器实现，配合 take_while/islice 消费。
- repeat arity MAX（与 string.repeat=2 共享名，.ms 默认参数天然支持 1-2 参自校验）。
- combinations/permutations 输入先物化为 list（索引算法）。

### functools（.ms 嵌入）

参照 [16-stdlib-expansion](../16-stdlib-expansion.md) §4.14：

- `partial(fn, *args)`：`__call__(*more)` 实例，args 在前。
- `memoize(fn)`：dict 缓存，键 `tuple(args)`；unhashable → TypeError（dict 行为上抛）；
  无 LRU 上限（开放问题 5 留白）。
- `reduce(fn, iter, init?)`：iterable 级归约（list.reduce 方法保留不动）。

### test（.ms 嵌入）

参照 [16-stdlib-expansion](../16-stdlib-expansion.md) §4.15：

assert_eq(a,b,msg?) / assert_ne / assert_true(cond,msg?) / assert_false /
assert_almost_eq(a,b,eps=1e-9,msg?) / assert_raises(fn, exc_class, msg?) /
assert_len(v,n,msg?) / assert_contains(coll,item,msg?) / fail(msg)。

失败统一抛 AssertionError，消息含 `str(a)`/`str(b)` 与可选 msg。
**assert_raises 类匹配机制**（开放问题 2 在本 task 闭环）：
实现期验证异常实例的类名获取路径——`type(e)` 若不返回异常类名，则以
异常消息前缀（"ClassName: ..."）解析类名比对；结论回写 16-stdlib-expansion.md §7.2。

## 实现细节

### 文件位置

- `src/vm/stdlib/heapq.rs` — `register_heapq_module` + 5 个 native 函数
- `src/vm/mod.rs` — heapq 注册 + `native_arities`（heapify=1, heap_push=2,
  heap_pop=1, push_pop=2, n_largest=2, n_smallest=2）
- `src/vm/stdlib/ms/collections.ms` / `itertools.ms` / `functools.ms` / `test.ms`
  — 替换 task 79 的占位内容（模块名/导出名固定）
- `src/vm/stdlib/mod.rs` — include_str! 已由 task 79 建好，无改动

### heapq sift 细节

- 直接操作 `read_list_mut` 的 Vec<Object>；比较用 `Object::compare(CmpOp::Less)`，
  Err 上抛（排序中断时 list 处于部分堆序——可接受，与 Python 异常语义一致）。
- push_pop 语义顺序（Python）：若 lst 空 v 直返；若 v ≤ 堆顶直返 v；
  否则弹出堆顶、v 入首并 sift-down。

### deque .ms 参考骨架

```ms
class Deque {
    fn __init__() {
        self.buf = []
        self.head = 0      # 逻辑首在 buf 的偏移
    }
    # buf 物理序 = 逻辑序循环移位 head；len == buf.length()
    fn push_back(x)  { self.buf.push(x) }
    fn push_front(x) { self.buf.insert(0, x); self.head = (self.head + 1) % self.buf.length() }
    # 实现期可改为「物理 head 移动」方案，以单测锁定语义为准
}
```

> 性能注记：insert(0,·) 为 O(n)。M6 验收以正确性优先；若压测前端操作
> 显著慢于后端（.ms 层 10^5 级），记入开放问题 5（deque 原生化留白）并
> 在 10-builtins.md 注明当前常数。

### itertools 关键实现

- `chain(*iters)`：`for it in iters { for x in it { yield x } }` 风格生成器。
- `pairwise`：prev 状态变量逐项推进。
- `accumulate(it, fn?)`：fn 缺省用 `+`（判 nil）。
- `islice(start, stop, step=1)`：索引计数跳过，stop=nil 表示无限
  （位置参数表达：islice(it, start, stop) stop 传 nil）。
- `batched(it, n)`：内部 list 攒批 yield。

### functools.memoize

```ms
fn memoize(fn) {
    cache = {}
    return fn(*args) {
        key = tuple(args)
        if cache.contains(key) { return cache[key] }
        val = fn(*args)
        cache[key] = val
        return val
    }
}
```

（闭包捕获 cache；语法以实际支持的闭包/匿名函数形式为准。）

### test.assert_raises 机制

```ms
fn assert_raises(f, exc_class, msg?) {
    try {
        f()
    } except e {
        # 类名比对：e 的类名获取路径实现期验证（见设计规格）
        return nil
    }
    fail(msg or "expected exception not raised")
}
```

except 绑定变量语法 / 异常类匹配语法以 37-try-except-finally 已实现形态为准
（`except ValueError` 直接匹配类 vs `except e` 绑定实例，实现期对齐）。

## GC 安全

- heapq 原生侧无新根集（元素重排于存活 list 内，见 task 81 shuffle 同款注记）。
- .ms 模块对象图全部由既有 class/instance/closure/generator 机制管理，无 VM 改动。

## 验证标准

1. heapify 后逐次 heap_pop 输出升序；heap_push/heap_pop 随机序列与 sorted 结果一致
2. heap_pop([]) → IndexError；n_largest([3,1,2],2) == [3,2]；n_smallest n≤0 → []
3. heapq 混合类型元素上抛 TypeError（比较错误传播）
4. deque 2000 次混合 push/pop 序列与对拍 list 行为一致；空弹出 IndexError；
   for-in / len() / to_list 正确
5. Counter 计数 / 缺失 0 不写入 / most_common 排序 / elements 展开
6. defaultdict factory 触发与 get 不触发；factory nil → KeyError
7. itertools：count 配 islice 截断；cycle 首轮重复；chain/zip_longest/product/
   combinations/permutations 结果与手写期望一致；accumulate 默认 `+` 与自定义 fn
8. functools：partial 参数顺序；memoize 命中（副作用函数仅执行一次）；
   reduce 与 list.reduce 等价
9. test：assert_eq 失败消息含双值；assert_almost_eq 默认 eps；assert_raises
   捕获与不匹配两种路径；assert_len/assert_contains
10. 全部 .ms 模块经**嵌入**路径 import（无磁盘依赖）；repeat(3)（string）与
    itertools.repeat(3, 2) 同脚本并存（同名冲突回归）
11. `cargo test` 全绿

## 测试用例

### tests/ms/stdlib/test_heapq.ms

验证标准 1-3。

### tests/ms/stdlib/test_collections.ms

验证标准 4-6（deque 对拍以 .ms 内参考实现对照）。

### tests/ms/stdlib/test_itertools.ms

验证标准 7。

### tests/ms/stdlib/test_functools.ms

验证标准 8。

### tests/ms/stdlib/test_test_module.ms

验证标准 9（用 test 模块断言 test 模块自身；失败路径用 try/except AssertionError 包裹）。

### Rust 单测（heapq.rs 内）

- sift-up/down 纯函数级用例（小数组手算期望堆序）。
