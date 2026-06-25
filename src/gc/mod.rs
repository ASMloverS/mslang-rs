pub struct GarbageCollector {
    bytes_allocated: usize,
    next_gc: usize,
}

impl GarbageCollector {
    pub fn new() -> Self {
        GarbageCollector {
            bytes_allocated: 0,
            next_gc: 1024 * 1024,
        }
    }

    pub fn should_collect(&self) -> bool {
        self.bytes_allocated >= self.next_gc
    }

    pub fn collect(&mut self) {
        // MVP: no-op，Phase 后续实现标记-清除
    }
}

impl Default for GarbageCollector {
    fn default() -> Self {
        Self::new()
    }
}
