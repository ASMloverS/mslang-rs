# collections — 嵌入式 .ms 标准库（task 84，16-stdlib-expansion.md §4.12）
# deque / Counter / defaultdict 三个 class（dict 为内建类型不可继承，经
# __len__/__getitem__/__iter__ 魔术方法接通 len()/[]/for-in）。

# deque：内部 list 容量倍增 + head 偏移的循环缓冲，两端均摊 O(1)
#（物理尾/头写 buf[(head+count) % cap]，不用 insert(0)）。
class deque {
    fn __init__(self) {
        self.buf = []      # 物理缓冲（弹出位回填 nil）
        self.head = 0      # 逻辑首元素物理下标
        self.count = 0     # 逻辑长度（len == count）
    }
    # 容量倍增：新缓冲 4 起、倍增；存活段按逻辑序搬到新缓冲头部，head 归零。
    fn _grow(self) {
        old_cap = self.buf.length()
        new_cap = 4
        if old_cap > 0 {
            new_cap = old_cap * 2
        }
        nb = []
        for i in range(new_cap) {
            nb.push(nil)
        }
        for i in range(self.count) {
            nb[i] = self.buf[(self.head + i) % old_cap]
        }
        self.buf = nb
        self.head = 0
    }
    fn push_back(self, x) {
        if self.count == self.buf.length() {
            self._grow()
        }
        self.buf[(self.head + self.count) % self.buf.length()] = x
        self.count += 1
    }
    fn push_front(self, x) {
        if self.count == self.buf.length() {
            self._grow()
        }
        self.head = (self.head - 1 + self.buf.length()) % self.buf.length()
        self.buf[self.head] = x
        self.count += 1
    }
    fn pop_front(self) {
        if self.count == 0 {
            throw IndexError("pop_front(): deque is empty")
        }
        v = self.buf[self.head]
        self.buf[self.head] = nil
        self.head = (self.head + 1) % self.buf.length()
        self.count -= 1
        return v
    }
    fn pop_back(self) {
        if self.count == 0 {
            throw IndexError("pop_back(): deque is empty")
        }
        pos = (self.head + self.count - 1) % self.buf.length()
        v = self.buf[pos]
        self.buf[pos] = nil
        self.count -= 1
        return v
    }
    fn front(self) {
        if self.count == 0 {
            throw IndexError("front(): deque is empty")
        }
        return self.buf[self.head]
    }
    fn back(self) {
        if self.count == 0 {
            throw IndexError("back(): deque is empty")
        }
        return self.buf[(self.head + self.count - 1) % self.buf.length()]
    }
    fn extend(self, iterable) {
        for x in iterable {
            self.push_back(x)
        }
    }
    fn to_list(self) {
        out = []
        for i in range(self.count) {
            out.push(self.buf[(self.head + i) % self.buf.length()])
        }
        return out
    }
    fn is_empty(self) {
        return self.count == 0
    }
    fn __len__(self) {
        return self.count
    }
    fn __iter__(self) {
        for i in range(self.count) {
            yield self.buf[(self.head + i) % self.buf.length()]
        }
    }
}

# Counter：缺失键读 0 不写入（Python 语义）；most_common 按频次降序
#（sorted_by，等频稳定保插入序）；elements 按计数重复展开。
class Counter {
    fn __init__(self, iterable = nil) {
        self.counts = {}
        if iterable != nil {
            for x in iterable {
                self.counts[x] = self.counts.get(x, 0) + 1
            }
        }
    }
    fn __getitem__(self, k) {
        if self.counts.contains(k) {
            return self.counts[k]
        }
        return 0
    }
    fn update(self, other) {
        for x in other {
            self.counts[x] = self.counts.get(x, 0) + 1
        }
    }
    fn most_common(self, n = nil) {
        ranked = sorted_by(self.counts.items(), fn(p) { return p[1] }, true)
        if n == nil {
            return ranked
        }
        out = []
        for i in range(ranked.length()) {
            if i >= n {
                break
            }
            out.push(ranked[i])
        }
        return out
    }
    fn elements(self) {
        for k in self.counts {
            for i in range(self.counts[k]) {
                yield k
            }
        }
    }
    fn items(self) {
        return self.counts.items()
    }
    fn get(self, k, d = 0) {
        if self.counts.contains(k) {
            return self.counts[k]
        }
        return d
    }
}

# defaultdict：__getitem__ 缺失调 factory() 存入并返回；get 不触发 factory
#（Python 一致）；factory 为 nil → KeyError。
class defaultdict {
    fn __init__(self, default_factory) {
        self.factory = default_factory
        self.data = {}
    }
    fn __getitem__(self, k) {
        if self.data.contains(k) {
            return self.data[k]
        }
        if self.factory == nil {
            throw KeyError(str(k))
        }
        v = self.factory()
        self.data[k] = v
        return v
    }
    fn get(self, k, d = nil) {
        if self.data.contains(k) {
            return self.data[k]
        }
        return d
    }
}
