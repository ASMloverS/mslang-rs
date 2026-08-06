//! mslang MVP 垃圾回收（Young 代复制 + Old/LES 代 STW 标记-清除）。
//!
//! 参照 [52-gc](../../docs/mslang/tasks/52-gc.md) 与 [14-gc](../../docs/mslang/14-gc.md)。
//!
//! # 布局路径决策（订正 A1/A3，data_ptr-aware）
//!
//! task 20/22 的 `alloc_*`（`src/vm/object.rs`/`builtins.rs`）采用「header + 独立 Box
//! 载荷（data_ptr）」模型，且调用点遍布编译器/VM/builtins（100+ 处）。spec §1.5 要求
//! 迁移为 inline 单次分配。但 Cheney 半空间（`Vec<u8>` 字节缓冲）要求对象内联于连续
//! 缓冲，而 Rust 占有类型（Vec/HashMap/HashSet/Box<[u8]>）在缓冲整体清零时**不会**执行
//! `Drop`，导致内部 bucket 泄漏；且 inline 迁移须改写全部 `alloc_*`/`read_*` 与调用点，
//! 风险波及 task 20-26 的 373+ 测试。
//!
//! 故本实现采用**等价语义的列表式复制 GC**：每个 GC 托管对象为独立 `Box`，登记于
//! `Vec<*mut>`；minor GC 将存活对象克隆至新 `Box`、转发根/对象内 Ref 槽、释放旧 Box
//! （经逐类型 `free` 钩子正确 `Drop` 载荷）；major GC 为 STW 标记-清除。此路径无泄漏、
//! 无 UB，且不改动既有 `alloc_*` API（既有 VM 分配暂不托管，见下方「集成范围」）。
//!
//! # 集成范围（MVP 过渡）
//!
//! GC 堆挂于 `VM.heap`，经 `VM::maybe_gc` 在主循环触发。当前 GC 托管**自身 API 分配**的
//! 对象（`gc_alloc_*`）；将 `src/vm/object.rs` 的既有 `alloc_*` 全量接入 GC 堆（使 VM 日常
//! 分配被回收）为后续增量（spec §1.5 过渡期允许 `alloc_*` 仍返回 `Object::Ref`），以避免
//! 破坏 Phase 2 对象模型。根集扫描为增量策略：MVP 仅 `stack`+`globals`+`frames`，其余根源
//! 随对应 task 落地（task 28/36/45/53/65）。

use crate::vm::frame::CallFrame;
use crate::vm::object::{
    read_bound_method, read_class, read_file_handle, read_instance, read_module, read_module_mut,
    DictMap, MsBoundMethod, MsClass, MsFileHandle, MsInstance, MsModule, MsObjHeader, Object,
    TypeTag,
};
use crate::vm::DeferEntry;
use std::collections::{HashMap, HashSet};

// ---------------------------------------------------------------------------
// task 62：并发标记模块（tri-color + 写屏障）。子模块经 super:: 访问本文件的
// Color/Generation/type_descriptor/sweep_heap 等私有项（子模块可见祖先私有项）。
// ---------------------------------------------------------------------------
pub mod barrier;
pub mod cardtable;
pub mod header;
pub mod major;
pub mod runtime;
pub mod safepoint;

pub use barrier::{alloc_during_gc, write_barrier, write_barrier_obj};
pub use cardtable::CardTable;
pub use header::{
    color_atomic, generation_atomic, set_color_atomic, try_color_transition, GcPhase,
};
pub use major::{
    close_concurrent_cycle, init_concurrent_mark, major_collect_stw, GcCoordinator, GcWorkerPool,
};
pub use runtime::{GcRuntime, GrayQueue};
pub use safepoint::SafepointCoordinator;

// ---------------------------------------------------------------------------
// 常量（参照 14-gc.md / 52-gc.md）
// ---------------------------------------------------------------------------

/// 大对象阈值：超过则进入 LES 独立分配。
pub const LARGE_OBJ_THRESHOLD: usize = 32 * 1024;
/// 默认晋升年龄。
pub const DEFAULT_PROMOTION_AGE: u8 = 2;
/// major GC 触发倍率（相对 bytes_allocated）。
const MAJOR_GC_RATIO: f64 = 2.0;
/// 初始 minor GC 阈值。
const INITIAL_MINOR_THRESHOLD: usize = 1024 * 1024;
/// 初始 major GC 阈值。
const INITIAL_MAJOR_THRESHOLD: usize = 2 * 1024 * 1024;

// ---------------------------------------------------------------------------
// gc_meta 位域（参照 14-gc.md § MsObjHeader；object.rs 的 _padding 为 u32）
// ---------------------------------------------------------------------------

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    White = 0,
    Gray = 1,
    Black = 2,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Generation {
    Young = 0,
    Old = 1,
    Immortal = 2,
}

impl MsObjHeader {
    const COLOR_MASK: u8 = 0b0000_0011;
    const GEN_MASK: u8 = 0b0000_1100;
    const GEN_SHIFT: u8 = 2;
    const HAS_FINALIZER: u8 = 0b0001_0000;
    const PINNED: u8 = 0b0010_0000;

    pub fn color(&self) -> Color {
        // 订正 R4：位值 3 越界，安全 match + 防御 fallback。
        match self.gc_meta & Self::COLOR_MASK {
            0 => Color::White,
            1 => Color::Gray,
            2 => Color::Black,
            _ => Color::White,
        }
    }
    pub fn set_color(&mut self, c: Color) {
        self.gc_meta = (self.gc_meta & !Self::COLOR_MASK) | (c as u8);
    }
    pub fn generation(&self) -> Generation {
        match (self.gc_meta & Self::GEN_MASK) >> Self::GEN_SHIFT {
            0 => Generation::Young,
            1 => Generation::Old,
            2 => Generation::Immortal,
            _ => Generation::Young,
        }
    }
    pub fn set_generation(&mut self, g: Generation) {
        self.gc_meta = (self.gc_meta & !Self::GEN_MASK) | ((g as u8) << Self::GEN_SHIFT);
    }
    pub fn age(&self) -> u8 {
        self.gc_meta >> 6
    }
    pub fn inc_age(&mut self) {
        let a = self.age();
        if a < 3 {
            self.gc_meta = (self.gc_meta & 0b0011_1111) | ((a + 1) << 6);
        }
    }
    pub fn has_finalizer(&self) -> bool {
        self.gc_meta & Self::HAS_FINALIZER != 0
    }
    pub fn set_has_finalizer(&mut self, on: bool) {
        if on {
            self.gc_meta |= Self::HAS_FINALIZER;
        } else {
            self.gc_meta &= !Self::HAS_FINALIZER;
        }
    }
    pub fn is_pinned(&self) -> bool {
        self.gc_meta & Self::PINNED != 0
    }
}

/// 构造一个 gc_meta=0、generation=Young 的对象头。
pub fn header_for(tag: TypeTag, size: u16) -> MsObjHeader {
    MsObjHeader {
        gc_meta: 0,
        type_tag: tag as u8,
        size,
        _padding: 0,
        class_ptr: 0,
    }
}

// ---------------------------------------------------------------------------
// TypeDescriptor（参照 14-gc.md § 类型描述表；所有 17 种 TypeTag 必须覆盖）
// ---------------------------------------------------------------------------

/// 字段 type_tag/name/size_base 为 14-gc.md 类型描述表设计的一部分，供后续类型落地
/// 与诊断使用，当前逻辑未直接读取，故放行 dead_code。
#[allow(dead_code)]
struct TypeDescriptor {
    type_tag: TypeTag,
    name: &'static str,
    /// 只读遍历对象内联载荷中的 Ref 指针（mark 用）。
    trace: fn(*mut MsObjHeader, &mut dyn FnMut(*mut MsObjHeader)),
    /// 把 src 载荷克隆到一个新分配的对象，返回新指针（list 式复制 GC）。
    copy_for_gc: fn(src: *mut MsObjHeader) -> *mut MsObjHeader,
    /// 遍历对象内联载荷中的每个 `&mut Object` 槽，交由 forwarder 转发（minor GC 修正子指针）。
    #[allow(clippy::type_complexity)]
    forward_fields: fn(obj: *mut MsObjHeader, forwarder: &mut dyn FnMut(&mut Object)),
    /// 释放对象（typed Box::from_raw，正确 Drop 载荷 + 释放头部）。
    free: fn(*mut MsObjHeader),
    /// C 侧注册的 finalizer（MVP：内置类型无；task 41 INSTANCE 落地后由 VM 调 __del__）。
    finalize: Option<fn(*mut MsObjHeader)>,
    size_base: usize,
}

fn trace_noop(_obj: *mut MsObjHeader, _cb: &mut dyn FnMut(*mut MsObjHeader)) {}

// ---- 内联载荷结构（header 后紧跟数据，偏移 16） ----

/// GC 托管的 String：header + Box<[u8]>（字符缓冲经 Box 单独持有，句柄内联）。
#[repr(C)]
pub struct GcString {
    pub header: MsObjHeader,
    pub data: Box<[u8]>,
}

#[repr(C)]
pub struct GcList {
    pub header: MsObjHeader,
    pub items: Vec<Object>,
}

#[repr(C)]
pub struct GcTuple {
    pub header: MsObjHeader,
    pub items: Vec<Object>,
}

#[repr(C)]
pub struct GcDict {
    pub header: MsObjHeader,
    pub map: DictMap,
}

#[repr(C)]
pub struct GcSet {
    pub header: MsObjHeader,
    pub inner: HashSet<Object>,
}

// ---- trace 函数 ----

fn trace_list(obj: *mut MsObjHeader, cb: &mut dyn FnMut(*mut MsObjHeader)) {
    // SAFETY: obj 由 gc_alloc_list 分配，偏移 16 为 Vec<Object>。
    let items = unsafe { &*((obj as *mut u8).add(16) as *const Vec<Object>) };
    for item in items.iter() {
        if let Object::Ref(r) = item {
            cb(*r);
        }
    }
}

fn trace_tuple(obj: *mut MsObjHeader, cb: &mut dyn FnMut(*mut MsObjHeader)) {
    let items = unsafe { &*((obj as *mut u8).add(16) as *const Vec<Object>) };
    for item in items.iter() {
        if let Object::Ref(r) = item {
            cb(*r);
        }
    }
}

fn trace_dict(obj: *mut MsObjHeader, cb: &mut dyn FnMut(*mut MsObjHeader)) {
    let map = unsafe { &*((obj as *mut u8).add(16) as *const DictMap) };
    for (k, v) in map.items().iter() {
        if let Object::Ref(r) = k {
            cb(*r);
        }
        if let Object::Ref(r) = v {
            cb(*r);
        }
    }
}

fn trace_set(obj: *mut MsObjHeader, cb: &mut dyn FnMut(*mut MsObjHeader)) {
    let set = unsafe { &*((obj as *mut u8).add(16) as *const HashSet<Object>) };
    for item in set.iter() {
        if let Object::Ref(r) = item {
            cb(*r);
        }
    }
}

// ---- copy_for_gc：克隆载荷至新 Box（list 式复制） ----

fn copy_string(src: *mut MsObjHeader) -> *mut MsObjHeader {
    // SAFETY: src 由 gc_alloc_string 分配。
    let g = unsafe { &*(src as *const GcString) };
    let new = Box::new(GcString {
        header: header_for(TypeTag::STRING, g.header.size),
        data: g.data.clone(),
    });
    Box::into_raw(new) as *mut MsObjHeader
}

fn copy_list(src: *mut MsObjHeader) -> *mut MsObjHeader {
    let g = unsafe { &*(src as *const GcList) };
    let new = Box::new(GcList {
        header: header_for(TypeTag::LIST, g.header.size),
        items: g.items.clone(),
    });
    Box::into_raw(new) as *mut MsObjHeader
}

fn copy_tuple(src: *mut MsObjHeader) -> *mut MsObjHeader {
    let g = unsafe { &*(src as *const GcTuple) };
    let new = Box::new(GcTuple {
        header: header_for(TypeTag::TUPLE, g.header.size),
        items: g.items.clone(),
    });
    Box::into_raw(new) as *mut MsObjHeader
}

fn copy_dict(src: *mut MsObjHeader) -> *mut MsObjHeader {
    let g = unsafe { &*(src as *const GcDict) };
    let new = Box::new(GcDict {
        header: header_for(TypeTag::DICT, g.header.size),
        map: g.map.clone(),
    });
    Box::into_raw(new) as *mut MsObjHeader
}

fn copy_set(src: *mut MsObjHeader) -> *mut MsObjHeader {
    let g = unsafe { &*(src as *const GcSet) };
    let new = Box::new(GcSet {
        header: header_for(TypeTag::SET, g.header.size),
        inner: g.inner.clone(),
    });
    Box::into_raw(new) as *mut MsObjHeader
}

/// 未托管类型（FUNCTION/CLOSURE/CLASS/INSTANCE/MODULE/GENERATOR/FUTURE/CHANNEL/
/// BOUND_METHOD/JOIN_HANDLE）的占位 copy：仅复制头部字节，不复制载荷。
/// 这些类型当前不经 gc_alloc_* 分配，故 copy 实际不会被调用；注册以防悬垂。
/// SAFETY: 字节复制头部（16 bytes）；载荷随对应 task 落地后由该 task 改为真实 copy。
fn copy_placeholder(src: *mut MsObjHeader) -> *mut MsObjHeader {
    let layout = std::alloc::Layout::new::<MsObjHeader>();
    // SAFETY: src 指向有效 MsObjHeader（至少头部 16 bytes）。
    let dst = unsafe { std::alloc::alloc(layout) as *mut MsObjHeader };
    unsafe {
        std::ptr::copy_nonoverlapping(src, dst, 1);
    }
    dst
}

// ---- forward_fields：修正对象内联载荷中的子 Ref 槽（minor GC 用） ----

fn forward_vec_fields(obj: *mut MsObjHeader, forwarder: &mut dyn FnMut(&mut Object)) {
    // SAFETY: obj 偏移 16 为 Vec<Object>；借 &mut 逐槽转发。
    let items = unsafe { &mut *((obj as *mut u8).add(16) as *mut Vec<Object>) };
    for item in items.iter_mut() {
        forwarder(item);
    }
}

fn forward_list_fields(obj: *mut MsObjHeader, forwarder: &mut dyn FnMut(&mut Object)) {
    forward_vec_fields(obj, forwarder);
}

fn forward_tuple_fields(obj: *mut MsObjHeader, forwarder: &mut dyn FnMut(&mut Object)) {
    forward_vec_fields(obj, forwarder);
}

/// Dict 的 forward：HashMap 不暴露 &mut 键，且转发只改 Ref 指针不改内容（哈希不变），
/// 故收集转发后的 (k,v) 重建 DictMap（O(n)，MVP 可接受）。
fn forward_dict_fields(obj: *mut MsObjHeader, forwarder: &mut dyn FnMut(&mut Object)) {
    // SAFETY: obj 偏移 16 为 DictMap。
    let map = unsafe { &mut *((obj as *mut u8).add(16) as *mut DictMap) };
    let pairs: Vec<(Object, Object)> = map
        .items()
        .into_iter()
        .map(|(k, v)| {
            let mut k = k.clone();
            let mut v = v.clone();
            forwarder(&mut k);
            forwarder(&mut v);
            (k, v)
        })
        .collect();
    *map = DictMap::new();
    for (k, v) in pairs {
        map.insert(k, v);
    }
}

/// Set 的 forward：同理收集转发后的元素重建 HashSet。
fn forward_set_fields(obj: *mut MsObjHeader, forwarder: &mut dyn FnMut(&mut Object)) {
    let set = unsafe { &mut *((obj as *mut u8).add(16) as *mut HashSet<Object>) };
    let forwarded: Vec<Object> = std::mem::take(set)
        .into_iter()
        .map(|mut x| {
            forwarder(&mut x);
            x
        })
        .collect();
    set.extend(forwarded);
}

fn forward_noop(_obj: *mut MsObjHeader, _fwd: &mut dyn FnMut(&mut Object)) {}

// ---- free：typed Box::from_raw，正确 Drop 载荷 ----

fn free_string(obj: *mut MsObjHeader) {
    // SAFETY: obj 由 gc_alloc_string / copy_string 经 Box::into_raw 分配。
    unsafe {
        drop(Box::from_raw(obj as *mut GcString));
    }
}
fn free_list(obj: *mut MsObjHeader) {
    unsafe {
        drop(Box::from_raw(obj as *mut GcList));
    }
}
fn free_tuple(obj: *mut MsObjHeader) {
    unsafe {
        drop(Box::from_raw(obj as *mut GcTuple));
    }
}
fn free_dict(obj: *mut MsObjHeader) {
    unsafe {
        drop(Box::from_raw(obj as *mut GcDict));
    }
}
fn free_set(obj: *mut MsObjHeader) {
    unsafe {
        drop(Box::from_raw(obj as *mut GcSet));
    }
}
/// 占位类型的释放：仅 dealloc 头部 16 bytes（载荷未托管，无 Drop 需求）。
fn free_placeholder(obj: *mut MsObjHeader) {
    let layout = std::alloc::Layout::new::<MsObjHeader>();
    unsafe {
        std::alloc::dealloc(obj as *mut u8, layout);
    }
}

// ---- task 40：CLASS/INSTANCE 的 trace / forward / copy / free ----
// 当前 VM 日常分配（alloc_class/alloc_instance）未接入 GC 堆，trace/copy/free 实际不被
// 主循环 GC 调用；注册真实实现以防未来接入后子指针悬垂（spec §11 V2 修复）。

/// 遍历 MsClass 内所有 Ref 槽：methods 值 + parent + class_attrs 值。
fn trace_class(obj: *mut MsObjHeader, cb: &mut dyn FnMut(*mut MsObjHeader)) {
    let c = unsafe { read_class(obj) };
    for m in c.methods.values() {
        cb(*m);
    }
    if let Some(p) = c.parent {
        cb(p);
    }
    for v in c.class_attrs.values() {
        if let Object::Ref(r) = v {
            cb(*r);
        }
    }
}

/// 遍历 MsInstance 内所有 Ref 槽：class + fields 值。
fn trace_instance(obj: *mut MsObjHeader, cb: &mut dyn FnMut(*mut MsObjHeader)) {
    let i = unsafe { read_instance(obj) };
    cb(i.class);
    for v in i.fields.values() {
        if let Object::Ref(r) = v {
            cb(*r);
        }
    }
}

/// Cheney 复制时修正 MsClass 内的 Ref 槽（Minor GC）。
fn forward_fields_class(obj: *mut MsObjHeader, forwarder: &mut dyn FnMut(&mut Object)) {
    let c = unsafe { read_class(obj) };
    for m in c.methods.values_mut() {
        let mut tmp = Object::Ref(*m);
        forwarder(&mut tmp);
        if let Object::Ref(new) = tmp {
            *m = new;
        }
    }
    if let Some(p) = c.parent {
        let mut tmp = Object::Ref(p);
        forwarder(&mut tmp);
        if let Object::Ref(new) = tmp {
            c.parent = Some(new);
        }
    }
    for v in c.class_attrs.values_mut() {
        forwarder(v);
    }
}

/// Cheney 复制时修正 MsInstance 内的 Ref 槽（Minor GC）。
fn forward_fields_instance(obj: *mut MsObjHeader, forwarder: &mut dyn FnMut(&mut Object)) {
    let i = unsafe { read_instance(obj) };
    let mut class_obj = Object::Ref(i.class);
    forwarder(&mut class_obj);
    if let Object::Ref(new) = class_obj {
        i.class = new;
    }
    for v in i.fields.values_mut() {
        forwarder(v);
    }
}

/// Minor GC 复制：MsClass 含 HashMap（独立堆缓冲），不可盲字节拷贝。
fn copy_for_gc_class(src: *mut MsObjHeader) -> *mut MsObjHeader {
    let s = unsafe { read_class(src) };
    let new = Box::new(MsClass {
        header: header_for(TypeTag::CLASS, s.header.size),
        name: s.name.clone(),
        methods: s.methods.clone(),
        parent: s.parent,
        class_attrs: s.class_attrs.clone(),
    });
    Box::into_raw(new) as *mut MsObjHeader
}

/// Minor GC 复制：MsInstance 含 HashMap，不可盲字节拷贝。
fn copy_for_gc_instance(src: *mut MsObjHeader) -> *mut MsObjHeader {
    let s = unsafe { read_instance(src) };
    let new = Box::new(MsInstance {
        header: header_for(TypeTag::INSTANCE, s.header.size),
        class: s.class,
        fields: s.fields.clone(),
    });
    Box::into_raw(new) as *mut MsObjHeader
}

fn free_class(obj: *mut MsObjHeader) {
    unsafe {
        drop(Box::from_raw(obj as *mut MsClass));
    }
}

fn free_instance(obj: *mut MsObjHeader) {
    unsafe {
        drop(Box::from_raw(obj as *mut MsInstance));
    }
}

// ---- task 41：BOUND_METHOD 的 trace / forward / copy / free ----
// MsBoundMethod 持有 receiver（可能为 Object::Ref）与 method（*mut MsClosure），
// 二者均为堆引用，须经 trace/forward/copy 处理，否则 Minor GC 复制后悬垂。

/// 遍历 MsBoundMethod 内所有 Ref 槽：receiver（若为 Ref）+ method 指针。
/// 用于 Major GC 三色标记。
fn trace_bound_method(obj: *mut MsObjHeader, cb: &mut dyn FnMut(*mut MsObjHeader)) {
    let b = unsafe { read_bound_method(obj) };
    if let Object::Ref(r) = &b.receiver {
        cb(*r);
    }
    cb(b.method);
}

/// Cheney 复制时修正 MsBoundMethod 内的 Ref 槽（Minor GC）。
/// receiver 经 forwarder 修正；method 裸指针包成 Object::Ref 修正后写回。
fn forward_fields_bound_method(obj: *mut MsObjHeader, forwarder: &mut dyn FnMut(&mut Object)) {
    let b = unsafe { read_bound_method(obj) };
    forwarder(&mut b.receiver);
    let mut method_tmp = Object::Ref(b.method);
    forwarder(&mut method_tmp);
    if let Object::Ref(new) = method_tmp {
        b.method = new;
    }
}

/// Minor GC 复制：MsBoundMethod 无 HashMap 载荷，直接字段克隆至新 Box。
fn copy_for_gc_bound_method(src: *mut MsObjHeader) -> *mut MsObjHeader {
    let s = unsafe { read_bound_method(src) };
    let new = Box::new(MsBoundMethod {
        header: header_for(TypeTag::BOUND_METHOD, s.header.size),
        receiver: s.receiver.clone(),
        method: s.method,
    });
    Box::into_raw(new) as *mut MsObjHeader
}

fn free_bound_method(obj: *mut MsObjHeader) {
    unsafe {
        drop(Box::from_raw(obj as *mut MsBoundMethod));
    }
}

// ---- task 45 §9：MODULE 的 trace / forward / copy / free ----
// MsModule 持有 exports + globals（均为 HashMap<String, Object>），其中 Object 值可能
// 为 Ref。trace/forward 遍历二者的 values；copy 克隆整个 MsModule；free 经 Box::from_raw
// 正确 Drop。当前 VM 日常分配（alloc_module）未接入 GC 堆，trace/copy/free 实际不被
// 主循环 GC 调用；注册真实实现以防未来接入后 Ref 槽悬垂（与 CLASS/INSTANCE 同策略）。

fn trace_module(obj: *mut MsObjHeader, cb: &mut dyn FnMut(*mut MsObjHeader)) {
    let m = unsafe { read_module(obj) };
    for v in m.exports.values() {
        if let Object::Ref(r) = v {
            cb(*r);
        }
    }
    for v in m.globals.values() {
        if let Object::Ref(r) = v {
            cb(*r);
        }
    }
}

fn forward_fields_module(obj: *mut MsObjHeader, forwarder: &mut dyn FnMut(&mut Object)) {
    let m = unsafe { read_module_mut(obj) };
    for v in m.exports.values_mut() {
        forwarder(v);
    }
    for v in m.globals.values_mut() {
        forwarder(v);
    }
}

fn copy_for_gc_module(src: *mut MsObjHeader) -> *mut MsObjHeader {
    let s = unsafe { read_module(src) };
    let new = Box::new(MsModule {
        header: header_for(TypeTag::MODULE, s.header.size),
        name: s.name.clone(),
        exports: s.exports.clone(),
        globals: s.globals.clone(),
    });
    Box::into_raw(new) as *mut MsObjHeader
}

fn free_module(obj: *mut MsObjHeader) {
    unsafe {
        drop(Box::from_raw(obj as *mut MsModule));
    }
}

// ---- task 46：FILE_HANDLE 的 trace / forward / copy / free / finalize ----
// MsFileHandle 持 Rust 资源（std::fs::File，不实现 Clone）：
// - trace 为 noop（path/mode/file 均非 GC 对象）。
// - copy_for_gc 不应被调用——对象 Immortal 代，不进 Young 半空间复制。防御 panic。
// - finalize 关闭 fd（drop File）；free 回收 3 个二级 Box + 主体。
// 当前 alloc_file_handle 用 Box::into_raw（未接入 GC 堆），trace/copy/free 实际不被
// 主循环 GC 调用；注册真实实现以防未来接入后资源泄漏（与 CLASS/INSTANCE 同策略）。

fn copy_for_gc_file_handle(_src: *mut MsObjHeader) -> *mut MsObjHeader {
    // FileHandle 为 Immortal 代，不应进入 Young 复制。若被调用表明逻辑错误。
    panic!("FileHandle (Immortal) must not be copied by minor GC");
}

fn free_file_handle(obj: *mut MsObjHeader) {
    // SAFETY: obj 由 alloc_file_handle 经 Box::into_raw 分配。
    unsafe {
        let h = Box::from_raw(obj as *mut MsFileHandle);
        // 关闭可能仍打开的 File（finalize 后为 None，但 free 可能在未 finalize 路径触发）。
        if let Some(f) = (*h.file_ptr).take() {
            drop(f);
        }
        // 回收 3 个二级 Box（path / mode / file）。
        let path_fat = std::ptr::slice_from_raw_parts_mut(h.path_ptr as *mut u8, h.path_len as usize);
        drop(Box::from_raw(path_fat));
        let mode_fat = std::ptr::slice_from_raw_parts_mut(h.mode_ptr as *mut u8, h.mode_len as usize);
        drop(Box::from_raw(mode_fat));
        drop(Box::from_raw(h.file_ptr));
        // h（MsFileHandle 主体）随 Box 超出作用域自动回收。
    }
}

/// finalizer：关闭 fd（兜底清理，task 46 §8）。仅关闭 File，不回收内存（由 free 在
/// 下次 GC 回收）。run_finalizers 调用后清 has_finalizer，对象下次 GC 正常释放。
fn finalize_file_handle(obj: *mut MsObjHeader) {
    // SAFETY: obj 由 alloc_file_handle 分配的有效 MsFileHandle。
    unsafe {
        let h = read_file_handle(obj);
        if let Some(f) = (*h.file_ptr).take() {
            drop(f);
        }
    }
}

// ---- 静态描述表 ----

static STRING_DESC: TypeDescriptor = TypeDescriptor {
    type_tag: TypeTag::STRING,
    name: "string",
    trace: trace_noop,
    copy_for_gc: copy_string,
    forward_fields: forward_noop,
    free: free_string,
    finalize: None,
    size_base: std::mem::size_of::<GcString>(),
};

static LIST_DESC: TypeDescriptor = TypeDescriptor {
    type_tag: TypeTag::LIST,
    name: "list",
    trace: trace_list,
    copy_for_gc: copy_list,
    forward_fields: forward_list_fields,
    free: free_list,
    finalize: None,
    size_base: std::mem::size_of::<GcList>(),
};

static TUPLE_DESC: TypeDescriptor = TypeDescriptor {
    type_tag: TypeTag::TUPLE,
    name: "tuple",
    trace: trace_tuple,
    copy_for_gc: copy_tuple,
    forward_fields: forward_tuple_fields,
    free: free_tuple,
    finalize: None,
    size_base: std::mem::size_of::<GcTuple>(),
};

static DICT_DESC: TypeDescriptor = TypeDescriptor {
    type_tag: TypeTag::DICT,
    name: "dict",
    trace: trace_dict,
    copy_for_gc: copy_dict,
    forward_fields: forward_dict_fields,
    free: free_dict,
    finalize: None,
    size_base: std::mem::size_of::<GcDict>(),
};

static SET_DESC: TypeDescriptor = TypeDescriptor {
    type_tag: TypeTag::SET,
    name: "set",
    trace: trace_set,
    copy_for_gc: copy_set,
    forward_fields: forward_set_fields,
    free: free_set,
    finalize: None,
    size_base: std::mem::size_of::<GcSet>(),
};

// task 40：CLASS/INSTANCE 由 GC 托管的 trace/forward/copy/free（finalize 由 task 52
// run_finalizers 在 VM 侧调用 __del__，此处 finalize=None）。
static CLASS_DESC: TypeDescriptor = TypeDescriptor {
    type_tag: TypeTag::CLASS,
    name: "class",
    trace: trace_class,
    copy_for_gc: copy_for_gc_class,
    forward_fields: forward_fields_class,
    free: free_class,
    finalize: None,
    size_base: std::mem::size_of::<MsClass>(),
};

static INSTANCE_DESC: TypeDescriptor = TypeDescriptor {
    type_tag: TypeTag::INSTANCE,
    name: "instance",
    trace: trace_instance,
    copy_for_gc: copy_for_gc_instance,
    forward_fields: forward_fields_instance,
    free: free_instance,
    finalize: None,
    size_base: std::mem::size_of::<MsInstance>(),
};

// task 41：BOUND_METHOD 由 GC 托管的 trace/forward/copy/free。
static BOUND_METHOD_DESC: TypeDescriptor = TypeDescriptor {
    type_tag: TypeTag::BOUND_METHOD,
    name: "bound_method",
    trace: trace_bound_method,
    copy_for_gc: copy_for_gc_bound_method,
    forward_fields: forward_fields_bound_method,
    free: free_bound_method,
    finalize: None,
    size_base: std::mem::size_of::<MsBoundMethod>(),
};

// task 45 §9：MODULE 已接入真实 trace/forward/copy/free。
static MODULE_DESC: TypeDescriptor = TypeDescriptor {
    type_tag: TypeTag::MODULE,
    name: "module",
    trace: trace_module,
    copy_for_gc: copy_for_gc_module,
    forward_fields: forward_fields_module,
    free: free_module,
    finalize: None,
    size_base: std::mem::size_of::<MsModule>(),
};

// task 46：FILE_HANDLE。Immortal 代（不进 Young 复制），has_finalizer 关闭 fd。
static FILE_HANDLE_DESC: TypeDescriptor = TypeDescriptor {
    type_tag: TypeTag::FILE_HANDLE,
    name: "file",
    trace: trace_noop,
    copy_for_gc: copy_for_gc_file_handle,
    forward_fields: forward_noop,
    free: free_file_handle,
    finalize: Some(finalize_file_handle),
    size_base: std::mem::size_of::<MsFileHandle>(),
};

// task 70：NATIVE_C_FUNCTION。Box 分配（alloc_c_native_function），未接入 GC 堆。
// trace 为 noop（func/arity/name_ptr 为原始类型，无 Object 引用）。
// free 回收 name_ptr 的 Box<[u8]> + MsCNativeFunction 主体。
#[cfg(feature = "capi")]
fn free_c_native_function(obj: *mut MsObjHeader) {
    use crate::vm::builtins::MsCNativeFunction;
    unsafe {
        let h = Box::from_raw(obj as *mut MsCNativeFunction);
        let name_fat =
            std::ptr::slice_from_raw_parts_mut(h.name_ptr as *mut u8, h.name_len as usize);
        drop(Box::from_raw(name_fat));
    }
}

#[cfg(feature = "capi")]
static NATIVE_C_FUNCTION_DESC: TypeDescriptor = TypeDescriptor {
    type_tag: TypeTag::NATIVE_C_FUNCTION,
    name: "native_c_function",
    trace: trace_noop,
    copy_for_gc: copy_placeholder,
    forward_fields: forward_noop,
    free: free_c_native_function,
    finalize: None,
    size_base: std::mem::size_of::<crate::vm::builtins::MsCNativeFunction>(),
};

#[cfg(not(feature = "capi"))]
static NATIVE_C_FUNCTION_DESC: TypeDescriptor = TypeDescriptor {
    type_tag: TypeTag::NATIVE_C_FUNCTION,
    name: "native_c_function",
    trace: trace_noop,
    copy_for_gc: copy_placeholder,
    forward_fields: forward_noop,
    free: free_placeholder,
    finalize: None,
    size_base: std::mem::size_of::<MsObjHeader>(),
};

// task 76：NATIVE_ASYNC_FUNCTION。Box 分配（alloc_native_async_function），未接入 GC 堆。
// trace 为 noop（name/func/arity 为原始类型，无 Object 引用）。
// free 回收 NativeAsyncFunction 主体（String name 由 Drop 自动释放）。
#[cfg(feature = "capi")]
fn free_native_async_function(obj: *mut MsObjHeader) {
    use crate::vm::object::NativeAsyncFunction;
    unsafe {
        let _ = Box::from_raw(obj as *mut NativeAsyncFunction);
    }
}

#[cfg(feature = "capi")]
static NATIVE_ASYNC_FUNCTION_DESC: TypeDescriptor = TypeDescriptor {
    type_tag: TypeTag::NATIVE_ASYNC_FUNCTION,
    name: "native_async_function",
    trace: trace_noop,
    copy_for_gc: copy_placeholder,
    forward_fields: forward_noop,
    free: free_native_async_function,
    finalize: None,
    size_base: std::mem::size_of::<crate::vm::object::NativeAsyncFunction>(),
};

#[cfg(not(feature = "capi"))]
static NATIVE_ASYNC_FUNCTION_DESC: TypeDescriptor = TypeDescriptor {
    type_tag: TypeTag::NATIVE_ASYNC_FUNCTION,
    name: "native_async_function",
    trace: trace_noop,
    copy_for_gc: copy_placeholder,
    forward_fields: forward_noop,
    free: free_placeholder,
    finalize: None,
    size_base: std::mem::size_of::<MsObjHeader>(),
};

// FUNCTION/CLOSURE/ITERATOR/GENERATOR/FUTURE/CHANNEL/JOIN_HANDLE 在当前 Phase 2.5
static PLACEHOLDER_DESC: TypeDescriptor = TypeDescriptor {
    type_tag: TypeTag::FUNCTION, // 占位 tag，实际查找不依赖此字段
    name: "placeholder",
    trace: trace_noop,
    copy_for_gc: copy_placeholder,
    forward_fields: forward_noop,
    free: free_placeholder,
    finalize: None,
    size_base: std::mem::size_of::<MsObjHeader>(),
};

// task 54：CHANNEL。Box 分配（alloc_channel），当前未接入 GC 堆，故 copy/forward/free
// 为占位 noop。trace 遍历 buffer + 等待协程的值栈/待发送值/帧闭包（14-gc.md § 根集扩展）。
fn trace_channel(obj: *mut MsObjHeader, cb: &mut dyn FnMut(*mut MsObjHeader)) {
    use crate::async_runtime::channel::read_channel;
    // SAFETY: obj 由 alloc_channel 分配，type_tag = CHANNEL。
    let ch = unsafe { read_channel(obj) };

    // 1. 缓冲区中的 Object::Ref。
    if let Ok(buffer) = ch.buffer.try_borrow() {
        for item in buffer.iter() {
            if let Object::Ref(r) = item {
                cb(*r);
            }
        }
    }

    // 辅助：遍历一个协程的 GC 根（值栈 + 各帧闭包/current_exc）。
    let trace_coro = |coro: &crate::vm::Coroutine, cb: &mut dyn FnMut(*mut MsObjHeader)| {
        for item in coro.stack.iter() {
            if let Object::Ref(r) = item {
                cb(*r);
            }
        }
        for frame in coro.call_stack.iter() {
            if !frame.closure.is_null() {
                cb(frame.closure);
            }
            if let Some(Object::Ref(r)) = &frame.current_exc {
                cb(*r);
            }
        }
    };

    // 2. 等待发送者：value + 协程值栈/帧闭包。
    if let Ok(senders) = ch.waiting_senders.try_borrow() {
        for sender in senders.iter() {
            if let Object::Ref(r) = &sender.value {
                cb(*r);
            }
            trace_coro(&sender.coroutine, cb);
        }
    }

    // 3. 等待接收者：协程值栈/帧闭包。
    if let Ok(receivers) = ch.waiting_receivers.try_borrow() {
        for receiver in receivers.iter() {
            trace_coro(&receiver.coroutine, cb);
        }
    }
}

static CHANNEL_DESC: TypeDescriptor = TypeDescriptor {
    type_tag: TypeTag::CHANNEL,
    name: "channel",
    trace: trace_channel,
    copy_for_gc: copy_placeholder,
    forward_fields: forward_noop,
    free: free_placeholder,
    finalize: None,
    size_base: std::mem::size_of::<crate::async_runtime::channel::MsChannel>(),
};

// task 55：JOIN_HANDLE。Box 分配（alloc_join_handle），当前未接入 GC 堆，故
// copy/forward/free 为占位 noop。trace 遍历 result/error 中的 Object::Ref（14-gc.md § 根集扩展）。
fn trace_join_handle(obj: *mut MsObjHeader, cb: &mut dyn FnMut(*mut MsObjHeader)) {
    use crate::async_runtime::join_handle::read_join_handle;
    // SAFETY: obj 由 alloc_join_handle 分配，type_tag = JOIN_HANDLE。
    let handle = unsafe { read_join_handle(obj) };

    // result 中的 Ref
    if let Ok(result) = handle.result.try_borrow() {
        if let Some(Object::Ref(r)) = result.as_ref() {
            cb(*r);
        }
    }

    // error 中的 Ref（异常实例）
    if let Ok(error) = handle.error.try_borrow() {
        if let Some(Object::Ref(r)) = error.as_ref() {
            cb(*r);
        }
    }
}

static JOIN_HANDLE_DESC: TypeDescriptor = TypeDescriptor {
    type_tag: TypeTag::JOIN_HANDLE,
    name: "join_handle",
    trace: trace_join_handle,
    copy_for_gc: copy_placeholder,
    forward_fields: forward_noop,
    free: free_placeholder,
    finalize: None,
    size_base: std::mem::size_of::<crate::async_runtime::join_handle::MsJoinHandle>(),
};

/// 类型描述表查找：为每个 TypeTag 返回对应的 TypeDescriptor。
/// 参照 14-gc.md — 所有 TypeTag 必须覆盖。当前仅 STRING/LIST/DICT/TUPLE/SET
/// 由 GC 托管；FUNCTION..EXCEPTION_CLASS(6..=19) 与 LARGE_OBJECT(0xFF) 以占位 noop
/// 注册，随对应 task 落地补真实 trace。仅对**真正未知**的字节值 debug panic（spec V7）。
fn type_descriptor(tag: u8) -> &'static TypeDescriptor {
    match tag {
        t if t == TypeTag::STRING as u8 => &STRING_DESC,
        t if t == TypeTag::LIST as u8 => &LIST_DESC,
        t if t == TypeTag::DICT as u8 => &DICT_DESC,
        t if t == TypeTag::TUPLE as u8 => &TUPLE_DESC,
        t if t == TypeTag::SET as u8 => &SET_DESC,
        // task 40：CLASS/INSTANCE 已接入真实 trace/forward/copy/free。
        t if t == TypeTag::CLASS as u8 => &CLASS_DESC,
        t if t == TypeTag::INSTANCE as u8 => &INSTANCE_DESC,
        // task 41：BOUND_METHOD 已接入真实 trace/forward/copy/free。
        t if t == TypeTag::BOUND_METHOD as u8 => &BOUND_METHOD_DESC,
        // task 45 §9：MODULE 已接入真实 trace/forward/copy/free。
        t if t == TypeTag::MODULE as u8 => &MODULE_DESC,
        // task 46：FILE_HANDLE（Immortal + finalizer）。trace/copy/free/defensive。
        t if t == TypeTag::FILE_HANDLE as u8 => &FILE_HANDLE_DESC,
        // task 70：NATIVE_C_FUNCTION（Box 分配，未接入 GC 堆）→ 占位 noop。
        t if t == TypeTag::NATIVE_C_FUNCTION as u8 => &NATIVE_C_FUNCTION_DESC,
        // task 76：NATIVE_ASYNC_FUNCTION（Box 分配，未接入 GC 堆）→ 占位 noop。
        t if t == TypeTag::NATIVE_ASYNC_FUNCTION as u8 => &NATIVE_ASYNC_FUNCTION_DESC,
        // task 54：CHANNEL（Box 分配，未接入 GC 堆）→ 真实 trace，其余占位。
        t if t == TypeTag::CHANNEL as u8 => &CHANNEL_DESC,
        // task 55：JOIN_HANDLE（Box 分配，未接入 GC 堆）→ 真实 trace，其余占位。
        t if t == TypeTag::JOIN_HANDLE as u8 => &JOIN_HANDLE_DESC,
        // 合法但当前未托管 TypeTag（6..=19 除 MODULE，与 0xFF）→ 占位 noop trace。
        // 这些类型不经 gc_alloc_* 分配（CLOSURE/UPVALUE/EXCEPTION 用 Box::into_raw），故
        // trace/copy/free 实际不被调用；防悬垂。
        // CLOSURE/UPVALUE 真实 trace 待 GC 接管闭包堆分配后补全（future task）。
        // TODO task 52/26: ITERATOR。
        // task 39: GENERATOR — Box 分配（alloc_generator），当前不经 gc_alloc，
        // trace/finalize 不被调用；GC 接管后需 trace stack_snapshot + receiver。
        // TODO task 53: FUTURE。
        // [task 38 回填] current_exc 作根集已在 minor_gc/major_gc 扫描（见上方 frames 参数）；
        //   exception_handlers 仅持元数据（无 Object 引用），不需扫描。
        t if (TypeTag::FUNCTION as u8..=TypeTag::EXCEPTION_CLASS as u8).contains(&t)
            || t == TypeTag::LARGE_OBJECT as u8 =>
        {
            &PLACEHOLDER_DESC
        }
        _ => {
            debug_assert!(false, "unknown type_tag {} in GC", tag);
            &PLACEHOLDER_DESC
        }
    }
}

// ---------------------------------------------------------------------------
// 堆内存组织
// ---------------------------------------------------------------------------

pub struct MsHeap {
    /// Young 代对象列表（list 式复制 GC 的 from/to 等价物）。
    young_objects: Vec<*mut MsObjHeader>,
    /// Old 代对象列表（标记-清除）。
    old_objects: Vec<*mut MsObjHeader>,
    /// 大对象空间（>32KB，独立分配）。
    los_objects: Vec<*mut MsObjHeader>,
    /// LES 真实字节数侧表（header.size 置 0，dealloc 据此构造 Layout）。
    los_sizes: HashMap<*mut MsObjHeader, usize>,
    /// 待执行 finalizer 的对象队列。
    finalizer_queue: Vec<*mut MsObjHeader>,
    pub bytes_allocated: usize,
    pub next_minor_gc: usize,
    pub next_major_gc: usize,
    pub promotion_age: u8,
    /// task 60：GC 统计字段。
    pub minor_count: u64,
    pub major_count: u64,
    pub total_pause_ns: u64,
    pub last_pause_ns: u64,
    pub bytes_freed: u64,
    pub gc_enabled: bool,
    /// 用户经 gc.set_gc_threads 设置的偏好值（MVP STW 单线程，不生效；Phase 7.5 用）。
    pub gc_threads_setting: u32,
    /// task 74：GC 调试模式（仅 debug_assertions 构建中由 msGcSetDebug 设置）。
    /// 启用后 root/unroot 配对检查、类型标签校验、堆一致性验证。
    /// MVP：存储不用（检查项随后续 task 落地）。
    #[allow(dead_code)]
    pub debug: bool,
}

impl MsHeap {
    pub fn new() -> Self {
        MsHeap {
            young_objects: Vec::new(),
            old_objects: Vec::new(),
            los_objects: Vec::new(),
            los_sizes: HashMap::new(),
            finalizer_queue: Vec::new(),
            bytes_allocated: 0,
            next_minor_gc: INITIAL_MINOR_THRESHOLD,
            next_major_gc: INITIAL_MAJOR_THRESHOLD,
            promotion_age: DEFAULT_PROMOTION_AGE,
            minor_count: 0,
            major_count: 0,
            total_pause_ns: 0,
            last_pause_ns: 0,
            bytes_freed: 0,
            gc_enabled: true,
            gc_threads_setting: 1,
            debug: false,
        }
    }

    /// 是否到达 minor 触发阈值。
    pub fn should_collect_minor(&self) -> bool {
        self.bytes_allocated >= self.next_minor_gc
    }

    /// 是否到达 major 触发阈值。
    pub fn should_collect_major(&self) -> bool {
        self.bytes_allocated >= self.next_major_gc
    }

    // ---- task 60：堆大小统计访问器 ----

    /// Young 代存活对象总字节数（遍历 header.size）。
    pub fn young_size(&self) -> usize {
        self.young_objects
            .iter()
            .map(|p| unsafe { (**p).size as usize })
            .sum()
    }

    /// Old 代存活对象总字节数。
    pub fn old_size(&self) -> usize {
        self.old_objects
            .iter()
            .map(|p| unsafe { (**p).size as usize })
            .sum()
    }

    /// LES 代存活对象总字节数（header.size 置 0，取侧表 los_sizes）。
    pub fn los_size(&self) -> usize {
        self.los_objects
            .iter()
            .map(|p| self.los_sizes.get(p).copied().unwrap_or(0))
            .sum()
    }

    /// 全部存活对象总字节数（young + old + los）。
    pub fn live_size(&self) -> usize {
        self.young_size() + self.old_size() + self.los_size()
    }

    /// task 62：Old 代对象数（集成测试/调试用）。
    pub fn old_objects_len(&self) -> usize {
        self.old_objects.len()
    }

    /// task 62：Old 代是否为空（循环回收测试断言用）。
    pub fn old_objects_is_empty(&self) -> bool {
        self.old_objects.is_empty()
    }

    /// 登记一个 Young 对象（由 gc_alloc_* 调用）。
    fn register_young(&mut self, ptr: *mut MsObjHeader, size: usize) {
        self.young_objects.push(ptr);
        self.bytes_allocated += size;
    }

    // 注：不提供通用的「直接 Old 分配」。Old 代对象**仅经 minor GC 晋升**产生
    // （copy_for_gc 分配类型正确的对象），故 major GC 的逐类型 free 可正确 Drop 载荷。
    // 通用 alloc_old(size, tag) 会写出「裸头 + 未初始化载荷」，与逐类型 free 不兼容
    // （spec 的通用 dealloc 仅对 inline POD 载荷成立）。LES（>32KB）独立走 alloc_los。

    /// LES 分配（>32KB），type_tag 设为 LARGE_OBJECT，真实大小记入 los_sizes 侧表。
    pub fn alloc_los(&mut self, size: usize, tag: TypeTag) -> *mut MsObjHeader {
        let aligned = (size + 7) & !7;
        let layout = std::alloc::Layout::from_size_align(aligned, 8).unwrap();
        let ptr = unsafe { std::alloc::alloc(layout) as *mut MsObjHeader };
        unsafe {
            std::ptr::write(ptr, header_for(TypeTag::LARGE_OBJECT, 0));
            // 复用 class_ptr 存储实际 TypeTag（spec §4 alloc_los 约定）。
            (*ptr).class_ptr = tag as u64;
        }
        self.los_objects.push(ptr);
        self.los_sizes.insert(ptr, aligned);
        self.bytes_allocated += aligned;
        ptr
    }
}

impl Default for MsHeap {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// gc_alloc_*：经 GC 堆分配托管对象（返回 Object::Ref，与既有 alloc_* 接口一致）
// ---------------------------------------------------------------------------

pub fn gc_alloc_string(heap: &mut MsHeap, s: &str) -> Object {
    let obj = Box::new(GcString {
        header: header_for(TypeTag::STRING, std::mem::size_of::<GcString>() as u16),
        data: Box::from(s.as_bytes()),
    });
    let ptr = Box::into_raw(obj) as *mut MsObjHeader;
    heap.register_young(ptr, std::mem::size_of::<GcString>());
    Object::Ref(ptr)
}

pub fn gc_alloc_list(heap: &mut MsHeap, items: Vec<Object>) -> Object {
    let obj = Box::new(GcList {
        header: header_for(TypeTag::LIST, std::mem::size_of::<GcList>() as u16),
        items,
    });
    let ptr = Box::into_raw(obj) as *mut MsObjHeader;
    heap.register_young(ptr, std::mem::size_of::<GcList>());
    Object::Ref(ptr)
}

pub fn gc_alloc_tuple(heap: &mut MsHeap, items: Vec<Object>) -> Object {
    let obj = Box::new(GcTuple {
        header: header_for(TypeTag::TUPLE, std::mem::size_of::<GcTuple>() as u16),
        items,
    });
    let ptr = Box::into_raw(obj) as *mut MsObjHeader;
    heap.register_young(ptr, std::mem::size_of::<GcTuple>());
    let _ = ptr;
    Object::Ref(ptr)
}

pub fn gc_alloc_dict(heap: &mut MsHeap, map: DictMap) -> Object {
    let obj = Box::new(GcDict {
        header: header_for(TypeTag::DICT, std::mem::size_of::<GcDict>() as u16),
        map,
    });
    let ptr = Box::into_raw(obj) as *mut MsObjHeader;
    heap.register_young(ptr, std::mem::size_of::<GcDict>());
    Object::Ref(ptr)
}

pub fn gc_alloc_set(heap: &mut MsHeap, inner: HashSet<Object>) -> Object {
    let obj = Box::new(GcSet {
        header: header_for(TypeTag::SET, std::mem::size_of::<GcSet>() as u16),
        inner,
    });
    let ptr = Box::into_raw(obj) as *mut MsObjHeader;
    heap.register_young(ptr, std::mem::size_of::<GcSet>());
    Object::Ref(ptr)
}

/// 读取 GC 托管的 List（测试用）。
/// # Safety
/// `ptr` 必须指向由 `gc_alloc_list` 分配的有效 GcList。
pub unsafe fn gc_read_list<'a>(ptr: *mut MsObjHeader) -> &'a Vec<Object> {
    &(*(ptr as *const GcList)).items
}

/// 读取 GC 托管的 List（可变，测试用）。不得与 `gc_read_list` 嵌套调用（借用约束）。
/// # Safety
/// `ptr` 必须指向由 `gc_alloc_list` 分配的有效 GcList。
pub unsafe fn gc_read_list_mut<'a>(ptr: *mut MsObjHeader) -> &'a mut Vec<Object> {
    &mut (*(ptr as *mut GcList)).items
}

/// 读取 GC 托管的 String（测试用）。
/// # Safety
/// `ptr` 必须指向由 `gc_alloc_string` 分配的有效 GcString，且内容为合法 UTF-8。
pub unsafe fn gc_read_str<'a>(ptr: *mut MsObjHeader) -> &'a str {
    let g = &*(ptr as *const GcString);
    std::str::from_utf8_unchecked(&g.data)
}

// ---------------------------------------------------------------------------
// Minor GC（Young 代 list 式复制 + 晋升）
// ---------------------------------------------------------------------------

struct Copier<'a> {
    heap: &'a mut MsHeap,
    map: HashMap<*mut MsObjHeader, *mut MsObjHeader>,
    new_young: Vec<*mut MsObjHeader>,
    worklist: Vec<*mut MsObjHeader>,
    promotion_age: u8,
    /// task 60：from-space 指针集合。forward_slot 仅转发此集合内的 Young 对象，
    /// 避免 GC 误复制经 alloc_*（非 GC 堆）分配的对象（gc_meta=0 但非 GC 托管）。
    old_young_set: HashSet<*mut MsObjHeader>,
}

impl<'a> Copier<'a> {
    /// 复制对象到新 Box（晋升 Old 或留 Young），登记转发，返回新指针（幂等）。
    fn copy(&mut self, old: *mut MsObjHeader) -> *mut MsObjHeader {
        if let Some(&np) = self.map.get(&old) {
            return np;
        }
        let cur_age = unsafe { (*old).age() };
        let new_age = if cur_age < 3 { cur_age + 1 } else { 3 };
        let promote = new_age >= self.promotion_age;
        let tag = unsafe { (*old).type_tag };
        let np = (type_descriptor(tag).copy_for_gc)(old);
        unsafe {
            let mut meta = (*old).gc_meta;
            meta = (meta & 0b0011_1111) | (new_age << 6);
            let gen = if promote {
                Generation::Old
            } else {
                Generation::Young
            };
            meta = (meta & !MsObjHeader::GEN_MASK) | ((gen as u8) << MsObjHeader::GEN_SHIFT);
            meta &= !MsObjHeader::COLOR_MASK; // White
            (*np).gc_meta = meta;
            let size = (*np).size as usize;
            self.heap.bytes_allocated = self.heap.bytes_allocated.saturating_add(size);
        }
        if promote {
            self.heap.old_objects.push(np);
        } else {
            self.new_young.push(np);
        }
        self.map.insert(old, np);
        self.worklist.push(np);
        np
    }

    /// 转发一个 Object 槽：若为 Ref 指向 from-space（old_young_set）的 Young 存活对象，
    /// 改写为新指针。非 GC 托管对象（经 alloc_* 分配，gc_meta=0 但不在 from-space）不转发。
    fn forward_slot(&mut self, slot: &mut Object) {
        if let Object::Ref(r) = slot {
            // 仅转发 from-space 内的 Young 对象；Old/LES/非托管对象不动。
            if self.old_young_set.contains(r) {
                let np = self.copy(*r);
                *slot = Object::Ref(np);
            }
        }
    }
}

/// Young 代复制 GC。扫描根集（stack + globals），存活对象克隆转发，不可达者释放。
/// frames 在 MVP 无 closure（task 28 起），故仅扫 stack+globals。
pub fn minor_gc(
    heap: &mut MsHeap,
    stack: &mut [Object],
    globals: &mut HashMap<String, Object>,
    defer_stack: &mut [DeferEntry],
    frames: &mut [CallFrame],
) {
    // task 60：GC 计时（入口快照，出口累加 pause_ns）。
    let t0 = std::time::Instant::now();
    let promotion_age = heap.promotion_age;
    let old_young = std::mem::take(&mut heap.young_objects);
    // task 60：构建 from-space 指针集合，供 forward_slot 快速判定哪些对象是 GC 托管的。
    let old_young_set: HashSet<*mut MsObjHeader> = old_young.iter().copied().collect();
    let mut c = Copier {
        heap,
        map: HashMap::new(),
        new_young: Vec::new(),
        worklist: Vec::new(),
        promotion_age,
        old_young_set,
    };

    // 根集转发（&mut 槽）。
    for v in stack.iter_mut() {
        c.forward_slot(v);
    }
    for v in globals.values_mut() {
        c.forward_slot(v);
    }
    // [task 28] frame.closure / open_upvalues
    // [task 36] defer_stack：每个 DeferEntry.call_tuple 作根转发。
    for entry in defer_stack.iter_mut() {
        c.forward_slot(&mut entry.call_tuple);
    }
    // [task 37/38] CallFrame.current_exc 作根转发：异常对象在 current_exc 持有期间
    // 会调用用户代码（with 的 __enter__/__exit__），CALL 安全点触发 GC 时若不扫描，
    // 异常对象可能被误回收。exception_handlers 仅持元数据（无 Object 引用），不扫。
    for frame in frames.iter_mut() {
        if let Some(exc) = frame.current_exc.as_mut() {
            c.forward_slot(exc);
        }
    }
    // [task 45] module_cache
    // [task 65] c_roots
    // [task 53] 暂停协程及其 Future.waiters

    // Cheney 扫描：遍历新对象，用 forward_fields 修正其内部子 Ref 槽。
    while let Some(obj) = c.worklist.pop() {
        let tag = unsafe { (*obj).type_tag };
        let ff = type_descriptor(tag).forward_fields;
        ff(obj, &mut |slot| c.forward_slot(slot));
    }

    // 释放全部旧 Young 对象（from-space 整体丢弃）。存活者已克隆为**新**对象
    // （new_young / old_objects，指针各异），故释放旧对象无双重释放风险。
    for old in old_young {
        let tag = unsafe { (*old).type_tag };
        let size = unsafe { (*old).size as usize };
        c.heap.bytes_allocated = c.heap.bytes_allocated.saturating_sub(size);
        // task 60：非存活对象（未转发）计入 bytes_freed。
        if !c.map.contains_key(&old) {
            c.heap.bytes_freed += size as u64;
        }
        (type_descriptor(tag).free)(old);
    }

    c.heap.young_objects = c.new_young;

    // task 60：统计计数（计时 + minor_count）。
    let elapsed = t0.elapsed().as_nanos() as u64;
    c.heap.total_pause_ns += elapsed;
    c.heap.last_pause_ns = elapsed;
    c.heap.minor_count += 1;
}

// ---------------------------------------------------------------------------
// Major GC（Old + LES 代 STW 标记-清除）
// ---------------------------------------------------------------------------

/// 从根集标记所有可达对象，清除未标记的 Old/LES 对象。有 finalizer 的对象入队复活。
/// 注：major_gc 仅读根集（stack+globals），不改写槽。
pub fn major_gc(
    heap: &mut MsHeap,
    stack: &[Object],
    globals: &HashMap<String, Object>,
    defer_stack: &[DeferEntry],
    frames: &[CallFrame],
) {
    // task 60：GC 计时。
    let t0 = std::time::Instant::now();
    let mut gray: Vec<*mut MsObjHeader> = Vec::new();

    // GC 托管对象集合：仅 old_objects + los_objects 中的对象需要标记/追踪。
    // VM 经 alloc_* 分配的对象（MsList/MsDict 等）不在 GC 堆中，布局与 Gc* 不同
    // （data_ptr 间接 vs 内联），trace 函数仅适配 Gc* 布局，故跳过以避免类型混淆。
    let gc_managed: HashSet<*mut MsObjHeader> = heap
        .old_objects
        .iter()
        .chain(heap.los_objects.iter())
        .copied()
        .collect();

    // mark：把 White 的 GC 托管对象标 Gray 并入栈。非托管对象（alloc_* 分配）跳过。
    fn mark(
        obj: *mut MsObjHeader,
        gray: &mut Vec<*mut MsObjHeader>,
        gc_managed: &HashSet<*mut MsObjHeader>,
    ) {
        if !gc_managed.contains(&obj) {
            return;
        }
        let h = unsafe { &mut *obj };
        if h.color() == Color::White {
            h.set_color(Color::Gray);
            gray.push(obj);
        }
    }

    for v in stack.iter() {
        if let Object::Ref(r) = v {
            mark(*r, &mut gray, &gc_managed);
        }
    }
    for v in globals.values() {
        if let Object::Ref(r) = v {
            mark(*r, &mut gray, &gc_managed);
        }
    }
    // [task 28] frames.closure / open_upvalues
    // [task 36/45/65/53] defer_stack / module_cache / c_roots / 协程
    for entry in defer_stack {
        if let Object::Ref(r) = &entry.call_tuple {
            mark(*r, &mut gray, &gc_managed);
        }
    }
    // [task 37/38] CallFrame.current_exc 作根标记（同 minor_gc 的根集扩展）。
    for frame in frames {
        if let Some(Object::Ref(r)) = &frame.current_exc {
            mark(*r, &mut gray, &gc_managed);
        }
    }

    while let Some(obj) = gray.pop() {
        let tag = unsafe { (*obj).type_tag };
        let desc = type_descriptor(tag);
        (desc.trace)(obj, &mut |child| mark(child, &mut gray, &gc_managed));
        unsafe {
            (*obj).set_color(Color::Black);
        }
    }

    // 清扫 Old + LES 代（task 62：抽为 sweep_heap，供并发标记的 STW 收尾复用）。
    sweep_heap(heap);

    // task 62：bytes_allocated=0（空堆）时 computed=0 会让 should_collect_major 恒真，
    // 每条指令触发 GC（并发模式下 → 每条指令一个完整周期 = 死级/死锁）。回退到初始阈值。
    let computed = (heap.bytes_allocated as f64 * MAJOR_GC_RATIO) as usize;
    heap.next_major_gc = if computed == 0 {
        INITIAL_MAJOR_THRESHOLD
    } else {
        computed
    };

    // task 60：统计计数（计时 + major_count）。
    let elapsed = t0.elapsed().as_nanos() as u64;
    heap.total_pause_ns += elapsed;
    heap.last_pause_ns = elapsed;
    heap.major_count += 1;
}

// ---------------------------------------------------------------------------
// Finalizer（GC 后由 mutator 线程执行）
// ---------------------------------------------------------------------------

/// task 62：清扫 Old + LES 代（标记完成后的 retain/free 逻辑）。
/// 从 major_gc 抽出，供并发标记的 STW 收尾（close_concurrent_cycle）复用，
/// 避免标记逻辑重复。Black→White 重置；White+finalizer→复活入队；White+无 finalizer→释放。
/// pinned 对象即使 White 也保留（C 侧 pin，14-gc.md 84-85 行）。
pub(super) fn sweep_heap(heap: &mut MsHeap) {
    // 清扫 Old 代。
    heap.old_objects.retain(|&obj| {
        // SAFETY: obj 由 gc_alloc_*/copy_for_gc 经 Box::into_raw 分配，有效 MsObjHeader。
        let h = unsafe { &mut *obj };
        if h.color() == Color::Black {
            h.set_color(Color::White);
            true
        } else if h.has_finalizer() {
            heap.finalizer_queue.push(obj);
            h.set_color(Color::White); // 复活，下次 GC 再回收
            true
        } else if h.is_pinned() {
            // task 62：pinned 对象保留（C 侧 pin 不可回收）。
            h.set_color(Color::White);
            true
        } else {
            let size = h.size as usize;
            heap.bytes_allocated = heap.bytes_allocated.saturating_sub(size);
            heap.bytes_freed += size as u64; // task 60
            let tag = h.type_tag;
            (type_descriptor(tag).free)(obj);
            false
        }
    });

    // 清扫 LES（真实大小取自侧表 los_sizes）。LES 不参与 pin（C 侧 pin 对象走 Old 路径）。
    heap.los_objects.retain(|&obj| {
        // SAFETY: obj 由 alloc_los 经 alloc+write 分配，有效 MsObjHeader。
        let h = unsafe { &mut *obj };
        if h.color() == Color::Black {
            h.set_color(Color::White);
            true
        } else if h.has_finalizer() {
            heap.finalizer_queue.push(obj);
            h.set_color(Color::White);
            true
        } else {
            let size = heap.los_sizes.remove(&obj).unwrap_or(0);
            heap.bytes_allocated = heap.bytes_allocated.saturating_sub(size);
            heap.bytes_freed += size as u64; // task 60
            if size > 0 {
                unsafe {
                    let layout = std::alloc::Layout::from_size_align_unchecked(size, 8);
                    std::alloc::dealloc(obj as *mut u8, layout);
                }
            }
            false
        }
    });
}

/// 执行 finalizer_queue 中的对象，执行后清除 has_finalizer（避免无限复活）。
pub fn run_finalizers(heap: &mut MsHeap) {
    for obj in heap.finalizer_queue.drain(..) {
        let h = unsafe { &mut *obj };
        let tag = h.type_tag;
        if let Some(fin) = type_descriptor(tag).finalize {
            fin(obj);
        }
        // INSTANCE 的用户 __del__ 由 task 41 落地后在 VM 侧调用（需 &mut VM）。
        unsafe {
            (*obj).set_has_finalizer(false);
            (*obj).set_color(Color::White);
        }
    }
}

// ---------------------------------------------------------------------------
// GC 触发决策（Full = minor 后 major；finalizer 由 mutator 在 GC 后执行）
// ---------------------------------------------------------------------------

pub fn maybe_gc(
    heap: &mut MsHeap,
    stack: &mut [Object],
    globals: &mut HashMap<String, Object>,
    defer_stack: &mut [DeferEntry],
    frames: &mut [CallFrame],
) {
    // task 60：gc_enabled guard — 禁用时自动 GC 为 no-op（手动 gc.collect() 不受此限）。
    if !heap.gc_enabled {
        return;
    }
    let mut ran = false;
    if heap.should_collect_major() {
        minor_gc(heap, stack, globals, defer_stack, frames);
        major_gc(heap, stack, globals, defer_stack, frames);
        ran = true;
    } else if heap.should_collect_minor() {
        minor_gc(heap, stack, globals, defer_stack, frames);
    }
    if ran {
        run_finalizers(heap);
    }
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::object::Object;

    fn heap_list(obj: &Object) -> *mut MsObjHeader {
        match obj {
            Object::Ref(r) => *r,
            _ => panic!("not a Ref"),
        }
    }

    #[test]
    fn test_young_alloc() {
        let mut heap = MsHeap::new();
        let obj = gc_alloc_string(&mut heap, "hi");
        let ptr = heap_list(&obj);
        assert!(!ptr.is_null());
        unsafe {
            assert_eq!((*ptr).type_tag, TypeTag::STRING as u8);
            assert_eq!((*ptr).generation(), Generation::Young);
        }
    }

    #[test]
    fn test_minor_gc_copies_survivors_and_updates_root_slot() {
        // 根栈保留一个 list 引用；另分配一个不可达对象。minor_gc 后存活者被克隆转发。
        let mut heap = MsHeap::new();
        let live = gc_alloc_list(&mut heap, vec![Object::Int(1), Object::Int(2)]);
        let _dead = gc_alloc_list(&mut heap, vec![Object::Int(99)]); // 不可达
        let mut stack = vec![live.clone()];
        let mut globals = HashMap::new();
        let before = heap_list(&live);

        minor_gc(&mut heap, &mut stack, &mut globals, &mut [], &mut []);

        let after = heap_list(stack.last().unwrap());
        assert_ne!(
            before as usize, after as usize,
            "survivor should be forwarded"
        );
        unsafe {
            assert_eq!(
                gc_read_list(after).clone(),
                vec![Object::Int(1), Object::Int(2)]
            );
        }
        // 不可达对象已被释放；young_objects 仅含存活副本。
        assert_eq!(heap.young_objects.len(), 1);
    }

    #[test]
    fn test_minor_gc_dead_object_freed() {
        let mut heap = MsHeap::new();
        let dead = gc_alloc_string(&mut heap, "unreachable");
        let ptr = heap_list(&dead);
        let mut stack = Vec::new();
        let mut globals = HashMap::new();
        minor_gc(&mut heap, &mut stack, &mut globals, &mut [], &mut []);
        assert!(heap.young_objects.is_empty());
        let _ = ptr; // 已释放；不可解引用
    }

    #[test]
    fn test_promotion_to_old() {
        let mut heap = MsHeap::new();
        heap.promotion_age = 2;
        let live = gc_alloc_list(&mut heap, vec![Object::Int(9)]);
        let mut stack = vec![live];
        let mut globals = HashMap::new();
        // 连续两次 minor_gc，age 累积达 promotion_age → 晋升 Old。
        minor_gc(&mut heap, &mut stack, &mut globals, &mut [], &mut []);
        minor_gc(&mut heap, &mut stack, &mut globals, &mut [], &mut []);
        let r = heap_list(stack.last().unwrap());
        unsafe {
            assert_eq!((*r).generation(), Generation::Old);
        }
        // 晋升后 Young 不含该对象，Old 含。
        assert!(heap.young_objects.is_empty());
        assert_eq!(heap.old_objects.len(), 1);
    }

    #[test]
    fn test_major_gc_collects_unreachable_old() {
        // 经晋升产生一个 Old 对象（类型正确，free 可安全 Drop 载荷），清除根后 major 回收。
        let mut heap = MsHeap::new();
        heap.promotion_age = 1;
        let live = gc_alloc_string(&mut heap, "temp");
        let mut stack = vec![live];
        let mut globals = HashMap::new();
        minor_gc(&mut heap, &mut stack, &mut globals, &mut [], &mut []); // 晋升到 Old
        assert_eq!(heap.old_objects.len(), 1);
        assert!(heap.bytes_allocated > 0);
        stack.clear(); // 解除根：Old 对象不可达
        major_gc(&mut heap, &stack, &globals, &[], &[]);
        // 不可达 Old 对象被清除，bytes_allocated 回落为 0。
        assert!(heap.old_objects.is_empty());
        assert_eq!(heap.bytes_allocated, 0);
    }

    #[test]
    fn test_major_gc_keeps_reachable_old() {
        let mut heap = MsHeap::new();
        heap.promotion_age = 1; // 一次 minor 即晋升
        let live = gc_alloc_string(&mut heap, "kept");
        let mut stack = vec![live.clone()];
        let mut globals = HashMap::new();
        minor_gc(&mut heap, &mut stack, &mut globals, &mut [], &mut []); // 晋升到 Old
        assert_eq!(heap.old_objects.len(), 1);
        // stack 仍指向晋升后的对象（minor 转发了根槽）。
        major_gc(&mut heap, &stack, &globals, &[], &[]);
        assert_eq!(heap.old_objects.len(), 1, "reachable Old must survive");
    }

    #[test]
    fn test_cycle_collection() {
        // 循环引用：a=[1], b=[2], a.push(b), b.push(a)；解除根后 major GC 应回收两者。
        let mut heap = MsHeap::new();
        let a = gc_alloc_list(&mut heap, vec![Object::Int(1)]);
        let b = gc_alloc_list(&mut heap, vec![Object::Int(2)]);
        {
            let ap = heap_list(&a);
            let bp = heap_list(&b);
            unsafe {
                gc_read_list_mut(ap).push(b.clone());
                gc_read_list_mut(bp).push(a.clone());
            }
        }
        // 晋升两者到 Old（promotion_age=1），使 major GC 管辖。
        heap.promotion_age = 1;
        let mut stack = vec![a.clone(), b.clone()];
        let mut globals = HashMap::new();
        minor_gc(&mut heap, &mut stack, &mut globals, &mut [], &mut []);
        // 清除根 → 两者仅彼此引用（循环），major 应回收。
        stack.clear();
        major_gc(&mut heap, &stack, &globals, &[], &[]);
        assert!(
            heap.old_objects.is_empty(),
            "cycle should be collected by major GC"
        );
    }

    #[test]
    fn test_les_alloc_and_dealloc() {
        let mut heap = MsHeap::new();
        let size = LARGE_OBJ_THRESHOLD + 1;
        let ptr = heap.alloc_los(size, TypeTag::STRING);
        assert!(!ptr.is_null());
        unsafe {
            assert_eq!((*ptr).type_tag, TypeTag::LARGE_OBJECT as u8);
            assert_eq!((*ptr).size, 0); // header.size 置 0
            assert_eq!((*ptr).class_ptr, TypeTag::STRING as u64); // 真实 tag 存于 class_ptr
        }
        assert_eq!(heap.los_objects.len(), 1);
        assert_eq!(*heap.los_sizes.get(&ptr).unwrap(), (size + 7) & !7);
        let before = heap.bytes_allocated;
        // major GC 清扫 LES（不可达 → 释放，bytes 回落）。
        let stack = Vec::new();
        let globals = HashMap::new();
        major_gc(&mut heap, &stack, &globals, &[], &[]);
        assert!(heap.los_objects.is_empty());
        assert!(heap.bytes_allocated < before);
    }

    #[test]
    fn test_les_reachable_traced_noop() {
        // 可达 LES 对象（type_tag LARGE_OBJECT=0xFF）经 noop trace 不 panic 且存活。
        // 覆盖 type_descriptor 对 0xFF 的占位注册（非 panic）。
        let mut heap = MsHeap::new();
        let ptr = heap.alloc_los(LARGE_OBJ_THRESHOLD + 8, TypeTag::STRING);
        let stack = vec![Object::Ref(ptr)];
        let globals = HashMap::new();
        major_gc(&mut heap, &stack, &globals, &[], &[]);
        assert_eq!(heap.los_objects.len(), 1, "reachable LES must survive");
    }

    #[test]
    fn test_nested_list_forwarded() {
        // 嵌套：outer=[inner], inner=[1]。两者均存活，minor_gc 后均转发且结构保持。
        let mut heap = MsHeap::new();
        let inner = gc_alloc_list(&mut heap, vec![Object::Int(1)]);
        let outer = gc_alloc_list(&mut heap, vec![inner]);
        let mut stack = vec![outer];
        let mut globals = HashMap::new();
        minor_gc(&mut heap, &mut stack, &mut globals, &mut [], &mut []);
        let new_outer = heap_list(stack.last().unwrap());
        unsafe {
            let items = gc_read_list(new_outer);
            assert_eq!(items.len(), 1);
            if let Object::Ref(new_inner) = items[0] {
                assert_eq!(gc_read_list(new_inner).clone(), vec![Object::Int(1)]);
            } else {
                panic!("child not a Ref");
            }
        }
    }

    #[test]
    fn test_bytes_allocated_no_underflow() {
        let mut heap = MsHeap::new();
        let live = gc_alloc_string(&mut heap, "x");
        let mut stack = vec![live];
        let mut globals = HashMap::new();
        minor_gc(&mut heap, &mut stack, &mut globals, &mut [], &mut []);
        // bytes_allocated 经 saturating_sub 不应下溢（usize 下溢会 panic）。
        assert!(heap.bytes_allocated < usize::MAX);
    }

    #[test]
    fn test_finalizer_lifecycle() {
        // 有 finalizer 的对象被回收时入队复活（不立即释放）；run_finalizers 在 GC 后
        // 由 mutator 执行并清 has_finalizer；下次 GC 正常回收（不无限复活）。
        // 用户 __del__（INSTANCE）由 task 41 落地后在 VM 侧调用，此处验证机制。
        let mut heap = MsHeap::new();
        heap.promotion_age = 1;
        let obj = gc_alloc_string(&mut heap, "fin");
        let mut stack = vec![obj];
        let mut globals = HashMap::new();
        minor_gc(&mut heap, &mut stack, &mut globals, &mut [], &mut []); // 晋升到 Old
        let old_ptr = *heap.old_objects.last().unwrap();
        unsafe {
            (*old_ptr).set_has_finalizer(true);
        }
        assert_eq!(heap.old_objects.len(), 1);

        stack.clear(); // 解除根
        major_gc(&mut heap, &stack, &globals, &[], &[]);
        // has_finalizer → 入队复活，未被释放。
        assert_eq!(heap.finalizer_queue.len(), 1);
        assert_eq!(heap.old_objects.len(), 1);

        // mutator 线程在 GC 后执行 finalizer，清 has_finalizer。
        run_finalizers(&mut heap);
        assert!(heap.finalizer_queue.is_empty());
        assert!(!unsafe { (*old_ptr).has_finalizer() });

        // 再次 major：finalizer 已清，对象正常回收（无无限复活）。
        major_gc(&mut heap, &stack, &globals, &[], &[]);
        assert!(heap.old_objects.is_empty());
    }

    #[test]
    fn test_maybe_gc_full_path() {
        // 验证 maybe_gc 的 minor/major 触发与 finalizer 串联。
        let mut heap = MsHeap::new();
        heap.promotion_age = 1;
        heap.next_minor_gc = 0; // 强制触发
        heap.next_major_gc = 0; // 强制 full GC
        let live = gc_alloc_list(&mut heap, vec![Object::Int(7)]);
        let mut stack = vec![live];
        let mut globals = HashMap::new();
        let mut defer_stack: Vec<DeferEntry> = Vec::new();
        maybe_gc(
            &mut heap,
            &mut stack,
            &mut globals,
            &mut defer_stack,
            &mut [],
        );
        // live 经 minor 晋升 Old，major 标记可达 → 存活。
        assert_eq!(heap.old_objects.len(), 1);
        unsafe {
            let r = *heap.old_objects.last().unwrap();
            assert_eq!(gc_read_list(r).clone(), vec![Object::Int(7)]);
        }
    }

    // ---- task 40：CLASS / INSTANCE GC 集成 ----
    use crate::vm::object::{alloc_bound_method, alloc_class, alloc_instance};

    #[test]
    fn test_trace_class_methods_and_attrs() {
        let class_obj = alloc_class("Foo".to_string());
        let Object::Ref(cls_ptr) = class_obj else {
            unreachable!()
        };
        let lst = gc_alloc_list(&mut MsHeap::new(), vec![Object::Int(1)]);
        let Object::Ref(lst_ptr) = lst else {
            unreachable!()
        };
        unsafe { read_class(cls_ptr) }
            .class_attrs
            .insert("data".to_string(), lst);
        unsafe { read_class(cls_ptr) }
            .methods
            .insert("bar".to_string(), lst_ptr);

        let mut traced: Vec<*mut MsObjHeader> = Vec::new();
        trace_class(cls_ptr, &mut |p| traced.push(p));
        assert_eq!(traced.len(), 2);
        assert!(traced.contains(&lst_ptr));
    }

    #[test]
    fn test_trace_instance_class_and_fields() {
        let class_obj = alloc_class("Bar".to_string());
        let Object::Ref(cls_ptr) = class_obj else {
            unreachable!()
        };
        let inst_obj = alloc_instance(cls_ptr);
        let Object::Ref(inst_ptr) = inst_obj else {
            unreachable!()
        };
        let lst = gc_alloc_list(&mut MsHeap::new(), vec![Object::Int(2)]);
        let Object::Ref(lst_ptr) = lst else {
            unreachable!()
        };
        unsafe { read_instance(inst_ptr) }
            .fields
            .insert("items".to_string(), lst);

        let mut traced: Vec<*mut MsObjHeader> = Vec::new();
        trace_instance(inst_ptr, &mut |p| traced.push(p));
        assert_eq!(traced.len(), 2);
        assert!(traced.contains(&cls_ptr));
        assert!(traced.contains(&lst_ptr));
    }

    #[test]
    fn test_copy_for_gc_class_deep_copy() {
        let class_obj = alloc_class("Orig".to_string());
        let Object::Ref(cls_ptr) = class_obj else {
            unreachable!()
        };
        unsafe { read_class(cls_ptr) }
            .class_attrs
            .insert("n".to_string(), Object::Int(42));

        let copy_ptr = copy_for_gc_class(cls_ptr);
        unsafe { read_class(cls_ptr) }
            .class_attrs
            .insert("n".to_string(), Object::Int(99));
        assert_eq!(
            unsafe { read_class(copy_ptr) }.class_attrs.get("n"),
            Some(&Object::Int(42))
        );
        free_class(copy_ptr);
    }

    #[test]
    fn test_copy_for_gc_instance_deep_copy() {
        let class_obj = alloc_class("C".to_string());
        let Object::Ref(cls_ptr) = class_obj else {
            unreachable!()
        };
        let inst_obj = alloc_instance(cls_ptr);
        let Object::Ref(inst_ptr) = inst_obj else {
            unreachable!()
        };
        unsafe { read_instance(inst_ptr) }
            .fields
            .insert("x".to_string(), Object::Int(7));

        let copy_ptr = copy_for_gc_instance(inst_ptr);
        unsafe { read_instance(inst_ptr) }
            .fields
            .insert("x".to_string(), Object::Int(8));
        assert_eq!(
            unsafe { read_instance(copy_ptr) }.fields.get("x"),
            Some(&Object::Int(7))
        );
        free_instance(copy_ptr);
    }

    #[test]
    fn test_type_descriptor_routes_class_and_instance() {
        let class_obj = alloc_class("D".to_string());
        let Object::Ref(cls_ptr) = class_obj else {
            unreachable!()
        };
        let desc = type_descriptor(unsafe { (*cls_ptr).type_tag });
        assert_eq!(desc.name, "class");

        let inst_obj = alloc_instance(cls_ptr);
        let Object::Ref(inst_ptr) = inst_obj else {
            unreachable!()
        };
        let desc2 = type_descriptor(unsafe { (*inst_ptr).type_tag });
        assert_eq!(desc2.name, "instance");
    }

    // ---- task 41：BOUND_METHOD GC 集成 ----

    #[test]
    fn test_trace_bound_method_receiver_and_method() {
        // 验证标准 8：trace 遍历 receiver（Ref）与 method 裸指针。
        let class_obj = alloc_class("K".to_string());
        let Object::Ref(cls_ptr) = class_obj else {
            unreachable!()
        };
        let inst_obj = alloc_instance(cls_ptr);
        let bound = alloc_bound_method(inst_obj.clone(), cls_ptr);
        let Object::Ref(bptr) = bound else {
            unreachable!()
        };
        let mut traced: Vec<*mut MsObjHeader> = Vec::new();
        trace_bound_method(bptr, &mut |p| traced.push(p));
        // receiver(inst) + method(cls_ptr)。
        assert_eq!(traced.len(), 2);
        assert!(traced.contains(&cls_ptr));
        let inst_ref = match &inst_obj {
            Object::Ref(r) => *r,
            _ => unreachable!(),
        };
        assert!(traced.contains(&inst_ref));
    }

    #[test]
    fn test_copy_for_gc_bound_method_deep_copy() {
        // 验证标准 8：copy 深拷贝 receiver 与 method，互不影响。
        let class_obj = alloc_class("C".to_string());
        let Object::Ref(cls_ptr) = class_obj else {
            unreachable!()
        };
        let inst_obj = alloc_instance(cls_ptr);
        let bound = alloc_bound_method(inst_obj.clone(), cls_ptr);
        let Object::Ref(bptr) = bound else {
            unreachable!()
        };
        let copy_ptr = copy_for_gc_bound_method(bptr);
        // 改原对象的 method 指针，副本应不变（method 为 Copy 指针值，深拷贝后独立）。
        unsafe { read_bound_method(bptr).method = std::ptr::null_mut() };
        assert_eq!(unsafe { read_bound_method(copy_ptr).method }, cls_ptr);
        free_bound_method(copy_ptr);
    }

    #[test]
    fn test_type_descriptor_routes_bound_method() {
        let class_obj = alloc_class("T".to_string());
        let Object::Ref(cls_ptr) = class_obj else {
            unreachable!()
        };
        let inst_obj = alloc_instance(cls_ptr);
        let bound = alloc_bound_method(inst_obj, cls_ptr);
        let Object::Ref(bptr) = bound else {
            unreachable!()
        };
        let desc = type_descriptor(unsafe { (*bptr).type_tag });
        assert_eq!(desc.name, "bound_method");
    }

    #[test]
    fn test_forward_fields_bound_method_updates_slots() {
        // 验证标准 8：forward 修正 receiver 与 method 槽（模拟 minor GC 转发）。
        let class_obj = alloc_class("F".to_string());
        let Object::Ref(cls_ptr) = class_obj else {
            unreachable!()
        };
        let inst_obj = alloc_instance(cls_ptr);
        let bound = alloc_bound_method(inst_obj.clone(), cls_ptr);
        let Object::Ref(bptr) = bound else {
            unreachable!()
        };
        // forwarder 将所有 Ref 重写为 cls_ptr（模拟转发到新地址）。
        forward_fields_bound_method(bptr, &mut |slot| {
            if let Object::Ref(_) = slot {
                *slot = Object::Ref(cls_ptr);
            }
        });
        let b = unsafe { read_bound_method(bptr) };
        assert_eq!(b.method, cls_ptr);
        match &b.receiver {
            Object::Ref(r) => assert_eq!(*r, cls_ptr),
            _ => unreachable!(),
        }
    }

    // ---- task 60：GC 统计字段追踪测试 ----

    #[test]
    fn test_minor_gc_increments_stats() {
        let mut heap = MsHeap::new();
        let _dead = gc_alloc_string(&mut heap, "unreachable");
        let mut stack = Vec::new();
        let mut globals = HashMap::new();
        minor_gc(&mut heap, &mut stack, &mut globals, &mut [], &mut []);
        assert_eq!(heap.minor_count, 1);
        assert_eq!(heap.major_count, 0);
        assert!(heap.last_pause_ns < u64::MAX); // 被写入
        assert!(heap.total_pause_ns >= heap.last_pause_ns);
    }

    #[test]
    fn test_major_gc_increments_stats() {
        let mut heap = MsHeap::new();
        heap.promotion_age = 1;
        let live = gc_alloc_string(&mut heap, "temp");
        let mut stack = vec![live];
        let mut globals = HashMap::new();
        minor_gc(&mut heap, &mut stack, &mut globals, &mut [], &mut []);
        stack.clear();
        major_gc(&mut heap, &stack, &globals, &[], &[]);
        assert_eq!(heap.major_count, 1);
        assert!(heap.bytes_freed > 0);
    }

    #[test]
    fn test_maybe_gc_respects_disabled() {
        let mut heap = MsHeap::new();
        heap.gc_enabled = false;
        heap.next_minor_gc = 0; // 强制触发条件
        heap.next_major_gc = 0;
        let mut stack = Vec::new();
        let mut globals = HashMap::new();
        let mut defer_stack: Vec<DeferEntry> = Vec::new();
        maybe_gc(
            &mut heap,
            &mut stack,
            &mut globals,
            &mut defer_stack,
            &mut [],
        );
        // disabled → no GC ran。
        assert_eq!(heap.minor_count, 0);
        assert_eq!(heap.major_count, 0);
    }

    #[test]
    fn test_maybe_gc_runs_when_enabled() {
        let mut heap = MsHeap::new();
        heap.gc_enabled = true;
        heap.next_minor_gc = 0;
        heap.next_major_gc = 0;
        let live = gc_alloc_list(&mut heap, vec![Object::Int(1)]);
        let mut stack = vec![live];
        let mut globals = HashMap::new();
        let mut defer_stack: Vec<DeferEntry> = Vec::new();
        maybe_gc(
            &mut heap,
            &mut stack,
            &mut globals,
            &mut defer_stack,
            &mut [],
        );
        assert!(heap.minor_count >= 1);
        assert!(heap.major_count >= 1);
    }

    #[test]
    fn test_heap_size_accessors() {
        let mut heap = MsHeap::new();
        // 空堆 → 全 0。
        assert_eq!(heap.young_size(), 0);
        assert_eq!(heap.old_size(), 0);
        assert_eq!(heap.los_size(), 0);
        assert_eq!(heap.live_size(), 0);

        // 分配 Young 对象 → young_size > 0。
        let _ = gc_alloc_string(&mut heap, "hello");
        assert!(heap.young_size() > 0);
        assert_eq!(heap.live_size(), heap.young_size());

        // LES 分配 → los_size > 0。
        heap.alloc_los(LARGE_OBJ_THRESHOLD + 8, TypeTag::STRING);
        assert!(heap.los_size() > 0);
    }

    #[test]
    fn test_gc_threads_setting_default() {
        let heap = MsHeap::new();
        assert_eq!(heap.gc_threads_setting, 1);
        assert!(heap.gc_enabled);
    }
}
