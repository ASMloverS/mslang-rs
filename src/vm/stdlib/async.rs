//! `async` 原生模块。
//!
//! 参照 [61-stdlib-async](../../../docs/mslang/tasks/61-stdlib-async.md)。

use crate::vm::builtins::{alloc_native_function, NativeFunction, NativeFn};
use crate::vm::object::{
    alloc_exception, alloc_future, alloc_module, alloc_string, read_module_mut, FutureState,
    MsObjHeader, Object, TypeTag,
};
use crate::vm::VM;

// ---------------------------------------------------------------------------
// async 模块（task 61）
// ---------------------------------------------------------------------------

/// async.sleep / async.timeout 的休眠毫秒数上限（24 小时），避免 Instant + Duration 溢出。
const ASYNC_MAX_MS: i64 = 86_400_000;

/// 构造 `async` 原生模块，返回指向 MsModule 的裸指针（TypeTag::MODULE）。
/// exports 含 sleep/timeout 两个原生函数。
pub fn register_async_module() -> *mut MsObjHeader {
    let mut exports = std::collections::HashMap::new();
    let funcs: [(&str, NativeFn); 2] = [("sleep", async_sleep), ("timeout", async_timeout)];
    for (name, func) in funcs {
        exports.insert(
            name.to_string(),
            alloc_native_function(NativeFunction {
                name: name.to_string(),
                func,
            }),
        );
    }
    let m = alloc_module("async");
    match m {
        Object::Ref(p) => {
            // SAFETY: alloc_module 返回有效 MsModule Ref。
            unsafe {
                read_module_mut(p).exports = exports;
            }
            p
        }
        _ => unreachable!("alloc_module must return Ref"),
    }
}

/// 构造一个 Rejected Future（验证失败时使用）。await 时经 AWAIT Rejected 路径
/// 抛出异常，可被 try/except 捕获（与显式 throw 语义一致）。
fn rejected_future(class_name: &str, message: &str) -> Object {
    let exc = alloc_exception(
        class_name,
        alloc_string(message),
        alloc_string(""),
        Object::Nil,
    );
    alloc_future(FutureState::Rejected(exc))
}

/// async.sleep(ms) → Future<nil>。异步休眠指定毫秒数，返回 Pending Future，
/// 由 EventLoop timer 推进。
fn async_sleep(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ms = match args.get(0) {
        Some(Object::Int(n)) => *n,
        _ => {
            return Ok(rejected_future(
                "TypeError",
                "async.sleep expects non-negative int argument",
            ))
        }
    };
    if ms < 0 {
        return Ok(rejected_future(
            "TypeError",
            &format!("async.sleep expects non-negative int, got {}", ms),
        ));
    }
    let ms_clamped = if ms > ASYNC_MAX_MS { ASYNC_MAX_MS } else { ms };

    // 分配 Pending Future + 注册 timer
    let future_obj = alloc_future(FutureState::Pending);
    let Object::Ref(future_ptr) = &future_obj else {
        unreachable!()
    };
    vm.push_sleep_timer(*future_ptr, ms_clamped);
    Ok(future_obj)
}

/// async.timeout(fn, ms) → Future<value>。带超时执行函数；返回 Pending Future，
/// 子协程与定时器竞争 resolve/reject。
fn async_timeout(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    // fn 校验：须为 FUNCTION / CLOSURE 堆对象
    let fn_ptr = match args.get(0) {
        Some(Object::Ref(p)) => {
            let tag = unsafe { (**p).type_tag };
            if tag != TypeTag::FUNCTION as u8 && tag != TypeTag::CLOSURE as u8 {
                return Ok(rejected_future(
                    "TypeError",
                    "async.timeout expects function as first argument",
                ));
            }
            *p
        }
        _ => {
            return Ok(rejected_future(
                "TypeError",
                "async.timeout expects function as first argument",
            ))
        }
    };
    let ms = match args.get(1) {
        Some(Object::Int(n)) => *n,
        _ => {
            return Ok(rejected_future(
                "TypeError",
                "async.timeout expects non-negative int as second argument",
            ))
        }
    };
    if ms < 0 {
        return Ok(rejected_future(
            "TypeError",
            &format!("async.timeout expects non-negative int, got {}", ms),
        ));
    }
    let ms_clamped = if ms > ASYNC_MAX_MS { ASYNC_MAX_MS } else { ms };

    // 1. 分配外层 Future（Pending）——子协程与定时器竞争 resolve/reject
    let outer_future = alloc_future(FutureState::Pending);
    let Object::Ref(outer_ptr) = &outer_future else {
        unreachable!()
    };

    // 2. 创建子协程运行 fn（复用 task 53 async fn CALL 路径的协程构造）
    let sub_coro = vm.spawn_timeout_subcoroutine(fn_ptr, *outer_ptr);
    let sub_coro_handle = sub_coro
        .handle
        .expect("timeout subcoro must have handle");
    vm.event_loop.ready_queue.push_back(sub_coro);

    // 3. 注册竞争 timer：到时 reject outer Future 为 TimeoutError
    vm.push_timeout_timer(*outer_ptr, ms_clamped, sub_coro_handle);

    Ok(outer_future)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::test_util::run_source;

    // ---- task 61：async 模块 ----

    #[test]
    fn test_async_module_registration() {
        let ptr = register_async_module();
        // SAFETY: register_async_module 返回有效 MODULE Ref。
        unsafe {
            assert_eq!((*ptr).type_tag, TypeTag::MODULE as u8);
            let module = read_module_mut(ptr);
            assert!(module.exports.contains_key("sleep"));
            assert!(module.exports.contains_key("timeout"));
            // 校验函数对象 type_tag
            for name in &["sleep", "timeout"] {
                match &module.exports[*name] {
                    Object::Ref(p) => assert_eq!((**p).type_tag, TypeTag::FUNCTION as u8),
                    other => panic!("{} export is not a function ref: {:?}", name, other),
                }
            }
        }
    }

    #[test]
    fn test_integration_async_sleep() {
        let src = r#"
import async
async fn quick() {
    await async.sleep(10)
    return "ok"
}
result = await quick()
assert(result == "ok")
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "async sleep integration failed: {:?}", r.err());
    }
}
