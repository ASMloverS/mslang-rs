//! `random` 原生模块（task 81）。
//!
//! 参照 [81-stdlib-random-encoding-uuid](../../../docs/mslang/tasks/81-stdlib-random-encoding-uuid.md)
//! 与 [16-stdlib-expansion](../../../docs/mslang/16-stdlib-expansion.md) §4.4（对齐 Python random 常用集）。
//!
//! 生成器为 thread_local `StdRng`（rand 0.8，可种子；`seed` 后序列确定）。
//! 与 `vm/mod.rs` select 随机分派的 `rand::thread_rng()` 互不影响（各自独立状态）。

use std::cell::RefCell;

use rand::rngs::StdRng;
use rand::{Rng, RngCore, SeedableRng};

use super::{expect_int, expect_list_ref, expect_number};
use crate::vm::builtins::{alloc_native_function, NativeFn, NativeFunction};
use crate::vm::gc;
use crate::vm::object::{
    alloc_list, alloc_module, alloc_string, read_list, read_module_mut, read_str, read_tuple,
    MsObjHeader, Object, TypeTag,
};
use crate::vm::VM;

thread_local! {
    static RNG: RefCell<StdRng> = RefCell::new(StdRng::from_entropy());
}

/// 对 thread_local RNG 独占执行 `f`（uuid4 复用同一生成器，见 uuid.rs）。
pub(super) fn with_rng<T>(f: impl FnOnce(&mut StdRng) -> T) -> T {
    RNG.with(|rng| f(&mut rng.borrow_mut()))
}

/// 构造 `random` 原生模块，返回指向 MsModule 的裸指针（TypeTag::MODULE）。
pub fn register_random_module() -> *mut MsObjHeader {
    let mut exports = std::collections::HashMap::new();
    let funcs: [(&str, NativeFn); 8] = [
        ("random", native_random_random),
        ("randint", native_random_randint),
        ("uniform", native_random_uniform),
        ("gauss", native_random_gauss),
        ("choice", native_random_choice),
        ("shuffle", native_random_shuffle),
        ("sample", native_random_sample),
        ("seed", native_random_seed),
    ];
    for (name, func) in funcs {
        exports.insert(
            name.to_string(),
            alloc_native_function(NativeFunction {
                name: name.to_string(),
                func,
            }),
        );
    }
    let m = alloc_module("random");
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

/// u64 均匀 → [0, n) 均匀（拒绝法；n ∈ [1, 2^64]，u128 中间量免溢出特判）。
/// rand 0.8 `gen_range` 的整型路径无法覆盖 [i64::MIN, i64::MAX] 全区间
///（宽度 2^64 的内部计数 wrapping），故手写（验证标准 11）。
fn gen_index(rng: &mut StdRng, n: u128) -> u64 {
    debug_assert!((1..=(1u128 << 64)).contains(&n));
    // 接受域上界 = 2^64 内 n 的最大整数倍，域外候选值拒绝重抽。
    let limit = ((1u128 << 64) / n) * n;
    loop {
        let v = rng.next_u64() as u128;
        if v < limit {
            return (v % n) as u64;
        }
    }
}

/// 原地（部分）Fisher–Yates：将 [0, n) 槽位与前部随机交换。
/// n == len 时为完整洗牌；sample 用 n < len 取前缀采样。
fn partial_shuffle<T>(items: &mut [T], n: usize) {
    RNG.with(|rng| {
        let mut g = rng.borrow_mut();
        for i in 0..n.min(items.len()) {
            let j = i + gen_index(&mut g, (items.len() - i) as u128) as usize;
            items.swap(i, j);
        }
    });
}

fn native_random_random(_vm: &mut VM, _args: &[Object]) -> Result<Object, String> {
    // rand 0.8 Standard 分布：[0, 1) 均匀。
    Ok(Object::Float(with_rng(|g| g.gen::<f64>())))
}

fn native_random_randint(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let a = expect_int(args.get(0), "randint(a, b)")?;
    let b = expect_int(args.get(1), "randint(a, b)")?;
    if a > b {
        return Err(format!("ValueError: randint(): empty range [{}, {}]", a, b));
    }
    // 闭区间宽度 = b-a+1；u128 计数规避 i64 端点 wrapping（含全区间 2^64）。
    let width = (b as i128 - a as i128 + 1) as u128;
    let off = with_rng(|g| gen_index(g, width)) as i64;
    Ok(Object::Int(a.wrapping_add(off)))
}

fn native_random_uniform(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    // Python 语义：a + (b - a) * random()，端点不保证。
    let a = expect_number(args.get(0), "uniform(a, b)")?;
    let b = expect_number(args.get(1), "uniform(a, b)")?;
    let r = with_rng(|g| g.gen::<f64>());
    Ok(Object::Float(a + (b - a) * r))
}

fn native_random_gauss(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let mu = expect_number(args.get(0), "gauss(mu, sigma)")?;
    let sigma = expect_number(args.get(1), "gauss(mu, sigma)")?;
    if sigma < 0.0 {
        return Err("ValueError: gauss() sigma must not be negative".to_string());
    }
    // Box–Muller：u1/u2 取 1.0 - random() ∈ (0,1]，杜绝 log(0)。
    let z = with_rng(|g| {
        let u1 = 1.0 - g.gen::<f64>();
        let u2 = 1.0 - g.gen::<f64>();
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    });
    Ok(Object::Float(mu + sigma * z))
}

/// 序列参数校验辅助：返回 (tag, ptr)。非 list/tuple/string → TypeError。
fn expect_sequence(arg: Option<&Object>, who: &str) -> Result<(u8, *mut MsObjHeader), String> {
    match arg {
        Some(Object::Ref(ptr)) => {
            // SAFETY: Ref 指向有效 MsObjHeader（alloc_* 分配）。
            let tag = unsafe { (**ptr).type_tag };
            if tag == TypeTag::LIST as u8
                || tag == TypeTag::TUPLE as u8
                || tag == TypeTag::STRING as u8
            {
                Ok((tag, *ptr))
            } else {
                Err(format!(
                    "TypeError: {} requires list/tuple/string, got {}",
                    who,
                    arg.map(|o| o.type_name()).unwrap_or("missing")
                ))
            }
        }
        other => Err(format!(
            "TypeError: {} requires list/tuple/string, got {}",
            who,
            other.map(|o| o.type_name()).unwrap_or("missing")
        )),
    }
}

fn native_random_choice(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let (tag, ptr) = expect_sequence(args.get(0), "choice(seq)")?;
    if tag == TypeTag::STRING as u8 {
        // SAFETY: expect_sequence 已守卫 type_tag 为 STRING。
        let s = unsafe { read_str(ptr) };
        let count = s.chars().count();
        if count == 0 {
            return Err("ValueError: choice(): cannot choose from an empty sequence".to_string());
        }
        let idx = with_rng(|g| gen_index(g, count as u128)) as usize;
        let c = s.chars().nth(idx).expect("index < char count");
        return Ok(alloc_string(&c.to_string()));
    }
    if tag == TypeTag::LIST as u8 {
        // SAFETY: expect_sequence 已守卫 type_tag 为 LIST。
        let items = unsafe { read_list(ptr) };
        if items.is_empty() {
            return Err("ValueError: choice(): cannot choose from an empty sequence".to_string());
        }
        let idx = with_rng(|g| gen_index(g, items.len() as u128)) as usize;
        return Ok(items[idx].clone());
    }
    // SAFETY: expect_sequence 已守卫 type_tag 为 TUPLE。
    let items = unsafe { read_tuple(ptr) };
    if items.is_empty() {
        return Err("ValueError: choice(): cannot choose from an empty sequence".to_string());
    }
    let idx = with_rng(|g| gen_index(g, items.len() as u128)) as usize;
    Ok(items[idx].clone())
}

/// Object → 裸指针（非 Ref 返回 null，适配 write_barrier_obj 签名）。
fn ref_ptr_or_null(obj: &Object) -> *mut MsObjHeader {
    match obj {
        Object::Ref(p) => *p,
        _ => std::ptr::null_mut(),
    }
}

fn native_random_shuffle(vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let ptr = expect_list_ref(args.get(0), "shuffle(lst)")?;
    // GC 安全（task 81 §GC 安全）：克隆 → 洗牌 → 逐槽经屏障写回。裸 Vec 交换在
    // 并发标记活跃期会让被逐出对象仅存于 Rust 局部（不在根集），绕过 Yuasa 快照
    // 删除屏障可能被误清扫。容器版屏障（task 63）同时对 parent 做跨代 card marking。
    // 注：既有 list.reverse / sort 写回为同类裸写，不在本 task 修复范围（已报告）。
    let mut items = {
        // SAFETY: ptr 经 expect_list_ref 校验为 alloc_list 分配的 MsList。
        unsafe { read_list(ptr) }.clone()
    };
    let len = items.len();
    partial_shuffle(&mut items, len);
    // SAFETY: partial_shuffle 无 VM 重入，ptr 保持有效。
    let list = unsafe { read_list(ptr) };
    for (slot, new_val) in items.into_iter().enumerate() {
        let old_val = std::mem::replace(&mut list[slot], new_val);
        // SAFETY: ptr 为有效 MsList header；old/new 为 null（非 Ref）或有效堆对象。
        unsafe {
            gc::write_barrier_obj(
                &vm.gc_runtime,
                ptr,
                ref_ptr_or_null(&old_val),
                ref_ptr_or_null(&list[slot]),
            );
        }
    }
    Ok(Object::Nil)
}

/// sample 越界统一错误（n<0 / n>len，Python 对齐）。
const SAMPLE_ERR: &str = "ValueError: sample(): sample larger than population or is negative";

fn native_random_sample(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let n = expect_int(args.get(1), "sample(pop, n)")?;
    if n < 0 {
        return Err(SAMPLE_ERR.to_string());
    }
    let n = n as usize;
    let (tag, ptr) = expect_sequence(args.get(0), "sample(pop, n)")?;
    // 不放回采样：部分 Fisher–Yates 打乱前 n 槽位后取前缀。
    if tag == TypeTag::STRING as u8 {
        // string 采样：返回单字符 string 的 list。
        // SAFETY: expect_sequence 已守卫 type_tag 为 STRING。
        let s = unsafe { read_str(ptr) };
        let chars: Vec<char> = s.chars().collect();
        if n > chars.len() {
            return Err(SAMPLE_ERR.to_string());
        }
        let mut idx: Vec<usize> = (0..chars.len()).collect();
        partial_shuffle(&mut idx, n);
        let picked: Vec<Object> = (0..n)
            .map(|i| alloc_string(&chars[idx[i]].to_string()))
            .collect();
        return Ok(alloc_list(picked));
    }
    let mut items = if tag == TypeTag::LIST as u8 {
        // SAFETY: expect_sequence 已守卫 type_tag 为 LIST。
        unsafe { read_list(ptr) }.clone()
    } else {
        // SAFETY: expect_sequence 已守卫 type_tag 为 TUPLE。
        unsafe { read_tuple(ptr) }.clone()
    };
    if n > items.len() {
        return Err(SAMPLE_ERR.to_string());
    }
    partial_shuffle(&mut items, n);
    items.truncate(n);
    Ok(alloc_list(items))
}

/// seed 对 uuid.rs 测试可见（跨模块行为契约：uuid4 复用本生成器）。
pub(super) fn native_random_seed(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    // arity MAX（§2.2）：native 内自校验 0-1 参。
    if args.len() > 1 {
        return Err(format!(
            "TypeError: seed(n?) takes 0-1 arguments, got {}",
            args.len()
        ));
    }
    match args.get(0) {
        None | Some(Object::Nil) => {
            // 缺省（或显式 nil，Python 语义对齐）：系统熵重播种。
            RNG.with(|rng| *rng.borrow_mut() = StdRng::from_entropy());
        }
        Some(Object::Int(n)) => {
            // 负数按补码位模式转 u64（as 重解释）。
            RNG.with(|rng| *rng.borrow_mut() = StdRng::seed_from_u64(*n as u64));
        }
        other => {
            return Err(format!(
                "TypeError: seed(n?) expects int, got {}",
                other.map(|o| o.type_name()).unwrap_or("missing")
            ))
        }
    }
    Ok(Object::Nil)
}

#[cfg(test)]
mod tests {
    use super::super::test_util::{fval, run_source, s, vm};
    use super::*;
    use crate::vm::object::{alloc_tuple, read_list, TypeTag};

    /// 依序生成 n 个 random() Float。
    fn draw_randoms(v: &mut VM, n: usize) -> Vec<f64> {
        (0..n)
            .map(|_| fval(&native_random_random(v, &[]).unwrap()))
            .collect()
    }

    #[test]
    fn test_random_module_registration() {
        let ptr = register_random_module();
        // SAFETY: ptr 由 register_random_module 返回的有效 MsModule。
        unsafe {
            assert_eq!((*ptr).type_tag, TypeTag::MODULE as u8);
            let m = read_module_mut(ptr);
            assert_eq!(m.name, "random");
            for name in [
                "random", "randint", "uniform", "gauss", "choice", "shuffle", "sample", "seed",
            ] {
                assert!(m.exports.contains_key(name), "missing export: {}", name);
            }
        }
    }

    #[test]
    fn test_seed_determinism() {
        // 验证标准 1：两次 seed(42) 的 random()/randint 序列一致。
        let mut v = vm();
        native_random_seed(&mut v, &[Object::Int(42)]).unwrap();
        let r1 = draw_randoms(&mut v, 3);
        let i1 = native_random_randint(&mut v, &[Object::Int(1), Object::Int(100)]).unwrap();
        native_random_seed(&mut v, &[Object::Int(42)]).unwrap();
        let r2 = draw_randoms(&mut v, 3);
        let i2 = native_random_randint(&mut v, &[Object::Int(1), Object::Int(100)]).unwrap();
        assert_eq!(r1, r2, "seed 后 random() 序列一致");
        assert_eq!(i1, i2, "seed 后 randint 一致");
        // 不同 seed 序列不同（概率性必然：首值碰撞概率 ~2^-53）
        native_random_seed(&mut v, &[Object::Int(43)]).unwrap();
        let r3 = draw_randoms(&mut v, 3);
        assert_ne!(r1, r3, "不同 seed 序列不同");
    }

    #[test]
    fn test_seed_variants() {
        // 验证标准 10：无参 seed() 不报错；nil 同义；负数按补码位模式。
        let mut v = vm();
        assert_eq!(native_random_seed(&mut v, &[]).unwrap(), Object::Nil);
        assert_eq!(
            native_random_seed(&mut v, &[Object::Nil]).unwrap(),
            Object::Nil
        );
        // 负 seed 确定性：同 seed 同序列。
        native_random_seed(&mut v, &[Object::Int(-1)]).unwrap();
        let a = draw_randoms(&mut v, 2);
        native_random_seed(&mut v, &[Object::Int(-1)]).unwrap();
        let b = draw_randoms(&mut v, 2);
        assert_eq!(a, b, "负 seed 确定性");
        // 非 Int → TypeError；多参 → TypeError（MAX 自校验）。
        let err = native_random_seed(&mut v, &[s("x")]).unwrap_err();
        assert!(err.contains("TypeError"), "got: {}", err);
        let err = native_random_seed(&mut v, &[Object::Int(1), Object::Int(2)]).unwrap_err();
        assert!(
            err.contains("TypeError") && err.contains("0-1"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_randint_range_and_errors() {
        // 验证标准 2：randint(1,1)=1；randint(2,1) → ValueError；非 Int → TypeError。
        let mut v = vm();
        assert_eq!(
            native_random_randint(&mut v, &[Object::Int(1), Object::Int(1)]).unwrap(),
            Object::Int(1)
        );
        assert_eq!(
            native_random_randint(&mut v, &[Object::Int(i64::MIN), Object::Int(i64::MIN)]).unwrap(),
            Object::Int(i64::MIN)
        );
        assert_eq!(
            native_random_randint(&mut v, &[Object::Int(i64::MAX), Object::Int(i64::MAX)]).unwrap(),
            Object::Int(i64::MAX)
        );
        let err = native_random_randint(&mut v, &[Object::Int(2), Object::Int(1)]).unwrap_err();
        assert!(err.contains("ValueError"), "got: {}", err);
        let err = native_random_randint(&mut v, &[Object::Float(1.5), Object::Int(2)]).unwrap_err();
        assert!(err.contains("TypeError"), "got: {}", err);
        let err = native_random_randint(&mut v, &[s("x"), Object::Int(2)]).unwrap_err();
        assert!(err.contains("TypeError"), "got: {}", err);
    }

    #[test]
    fn test_randint_full_range_and_distribution() {
        // 验证标准 11：randint(i64::MIN, i64::MAX) 全区间采样。
        let mut v = vm();
        let mut distinct = std::collections::HashSet::new();
        for _ in 0..2000 {
            let x = match native_random_randint(
                &mut v,
                &[Object::Int(i64::MIN), Object::Int(i64::MAX)],
            )
            .unwrap()
            {
                Object::Int(n) => n,
                other => panic!("randint must return Int, got {}", other.type_name()),
            };
            distinct.insert(x);
        }
        // 2000 次采样几乎必然 ≥1900 个不同值（2^64 值域碰撞概率可忽略）。
        assert!(distinct.len() >= 1900, "distinct = {}", distinct.len());
        // 小值域分布覆盖：randint(1,6) 600 次命中全部 6 个值。
        let mut seen = [false; 7];
        for _ in 0..600 {
            if let Object::Int(d) =
                native_random_randint(&mut v, &[Object::Int(1), Object::Int(6)]).unwrap()
            {
                seen[d as usize] = true;
            }
        }
        assert!(seen[1..=6].iter().all(|&x| x), "骰子六面全覆盖");
    }

    #[test]
    fn test_uniform() {
        // 验证标准 3：uniform(0,0)=0.0；端点序；非数值 → TypeError。
        let mut v = vm();
        assert_eq!(
            fval(&native_random_uniform(&mut v, &[Object::Int(0), Object::Int(0)]).unwrap()),
            0.0
        );
        for _ in 0..100 {
            let u = fval(
                &native_random_uniform(&mut v, &[Object::Int(5), Object::Float(10.0)]).unwrap(),
            );
            assert!((5.0..=10.0).contains(&u), "u = {}", u);
        }
        let err = native_random_uniform(&mut v, &[s("x"), Object::Int(1)]).unwrap_err();
        assert!(err.contains("TypeError"), "got: {}", err);
    }

    #[test]
    fn test_gauss() {
        // 验证标准 3：gauss(0,-1) → ValueError；固定 seed 后均值/方差近似。
        let mut v = vm();
        let err = native_random_gauss(&mut v, &[Object::Int(0), Object::Int(-1)]).unwrap_err();
        assert!(err.contains("ValueError"), "got: {}", err);
        let err = native_random_gauss(&mut v, &[s("x"), Object::Int(1)]).unwrap_err();
        assert!(err.contains("TypeError"), "got: {}", err);
        native_random_seed(&mut v, &[Object::Int(42)]).unwrap();
        let n = 10000;
        let mut sum = 0.0;
        let mut sum2 = 0.0;
        for _ in 0..n {
            let x = fval(&native_random_gauss(&mut v, &[Object::Int(10), Object::Int(2)]).unwrap());
            assert!(x.is_finite(), "gauss 有限值");
            sum += x;
            sum2 += x * x;
        }
        let mean = sum / n as f64;
        let var = sum2 / n as f64 - mean * mean;
        assert!((9.8..=10.2).contains(&mean), "mean = {}", mean);
        assert!((3.0..=5.6).contains(&var), "var = {}（σ²≈4）", var);
    }

    #[test]
    fn test_choice() {
        // 验证标准 4：choice("") → ValueError；非序列 → TypeError；三种序列取值域。
        let mut v = vm();
        let err = native_random_choice(&mut v, &[s("")]).unwrap_err();
        assert!(err.contains("ValueError"), "got: {}", err);
        let err = native_random_choice(&mut v, &[Object::Int(42)]).unwrap_err();
        assert!(
            err.contains("TypeError") && err.contains("list/tuple/string"),
            "got: {}",
            err
        );
        // list
        let elems = vec![Object::Int(1), Object::Int(2), Object::Int(3)];
        let lst = alloc_list(elems);
        for _ in 0..50 {
            match native_random_choice(&mut v, std::slice::from_ref(&lst)).unwrap() {
                Object::Int(n) => assert!((1..=3).contains(&n)),
                other => panic!("choice(list) got {}", other.type_name()),
            }
        }
        // tuple
        let tup = alloc_tuple(vec![Object::Int(7), Object::Int(8)]);
        for _ in 0..50 {
            match native_random_choice(&mut v, std::slice::from_ref(&tup)).unwrap() {
                Object::Int(n) => assert!(n == 7 || n == 8),
                other => panic!("choice(tuple) got {}", other.type_name()),
            }
        }
        // string → 单字符 string
        for _ in 0..50 {
            let c = super::super::test_util::strval(
                &native_random_choice(&mut v, &[s("abc")]).unwrap(),
            );
            assert!(["a", "b", "c"].contains(&c.as_str()), "c = {}", c);
        }
    }

    #[test]
    fn test_shuffle() {
        // 验证标准 4：shuffle("abc") → TypeError；nil 返回；多重集与指针身份保持。
        let mut v = vm();
        let err = native_random_shuffle(&mut v, &[s("abc")]).unwrap_err();
        assert!(err.contains("TypeError"), "got: {}", err);
        // 空/单元素 list 合法
        let empty = alloc_list(Vec::new());
        assert_eq!(
            native_random_shuffle(&mut v, &[empty]).unwrap(),
            Object::Nil
        );
        // 指针身份保持：洗牌不得产生元素复制/替换。
        let items: Vec<Object> = (0..20).map(|i| s(&format!("item{}", i))).collect();
        let ptrs_before: Vec<*mut MsObjHeader> = items.iter().map(ref_ptr_or_null).collect();
        let lst = alloc_list(items);
        assert_eq!(
            native_random_shuffle(&mut v, std::slice::from_ref(&lst)).unwrap(),
            Object::Nil,
            "shuffle 返回 nil"
        );
        let (after, ptrs_after) = match &lst {
            Object::Ref(p) => {
                // SAFETY: lst 由 alloc_list 分配。
                let list = unsafe { read_list(*p) };
                (
                    list.clone(),
                    list.iter().map(ref_ptr_or_null).collect::<Vec<_>>(),
                )
            }
            _ => unreachable!(),
        };
        assert_eq!(after.len(), 20, "长度不变");
        let mut sorted_before = ptrs_before.clone();
        let mut sorted_after = ptrs_after.clone();
        sorted_before.sort();
        sorted_after.sort();
        assert_eq!(sorted_before, sorted_after, "元素指针多重集保持（无复制）");
    }

    #[test]
    fn test_sample() {
        // 验证标准 4/10：sample([1],2) 与 sample([1],-1) → ValueError；非序列 → TypeError。
        let mut v = vm();
        let one = alloc_list(vec![Object::Int(1)]);
        let err = native_random_sample(&mut v, &[one.clone(), Object::Int(2)]).unwrap_err();
        assert!(err.contains("ValueError"), "got: {}", err);
        let err = native_random_sample(&mut v, &[one, Object::Int(-1)]).unwrap_err();
        assert!(err.contains("ValueError"), "got: {}", err);
        let err = native_random_sample(&mut v, &[Object::Int(42), Object::Int(1)]).unwrap_err();
        assert!(err.contains("TypeError"), "got: {}", err);
        let err = native_random_sample(&mut v, &[s("x"), s("n")]).unwrap_err();
        assert!(err.contains("TypeError"), "got: {}", err);
        // n = len：完整采样为原多重集
        let items = vec![
            Object::Int(1),
            Object::Int(2),
            Object::Int(3),
            Object::Int(4),
        ];
        let lst = alloc_list(items);
        let got = native_random_sample(&mut v, &[lst.clone(), Object::Int(4)]).unwrap();
        let vals = match &got {
            Object::Ref(p) => {
                // SAFETY: got 由 alloc_list 分配。
                unsafe { read_list(*p) }.clone()
            }
            _ => unreachable!(),
        };
        let mut ints: Vec<i64> = vals
            .iter()
            .map(|o| match o {
                Object::Int(n) => *n,
                _ => panic!("sample element must be Int"),
            })
            .collect();
        ints.sort();
        assert_eq!(ints, vec![1, 2, 3, 4], "n=len 完整采样");
        // n = 0：空 list
        let got = native_random_sample(&mut v, &[lst.clone(), Object::Int(0)]).unwrap();
        match &got {
            Object::Ref(p) => {
                // SAFETY: 同上。
                assert!(unsafe { read_list(*p) }.is_empty(), "n=0 空 list");
            }
            _ => unreachable!(),
        }
        // string 采样 → 单字符 string 的 list
        let got = native_random_sample(&mut v, &[s("abcd"), Object::Int(2)]).unwrap();
        match &got {
            Object::Ref(p) => {
                // SAFETY: 同上。
                let picked = unsafe { read_list(*p) };
                assert_eq!(picked.len(), 2, "string 采样长度");
                for o in picked {
                    let c = super::super::test_util::strval(o);
                    assert_eq!(c.len(), 1, "单字符 string：{}", c);
                    assert!("abcd".contains(&c), "字符来自总体：{}", c);
                }
            }
            _ => unreachable!(),
        }
        // tuple 总体
        let tup = alloc_tuple(vec![Object::Int(9), Object::Int(8)]);
        let got = native_random_sample(&mut v, &[tup, Object::Int(2)]).unwrap();
        match &got {
            Object::Ref(p) => {
                // SAFETY: 同上。
                assert_eq!(unsafe { read_list(*p) }.len(), 2, "tuple 采样");
            }
            _ => unreachable!(),
        }
    }

    // ---- 端到端集成 ----

    #[test]
    fn test_integration_random_module() {
        let src = r#"
import random
random.seed(42)
r1 = random.random()
i1 = random.randint(1, 100)
random.seed(42)
r2 = random.random()
i2 = random.randint(1, 100)
assert(r1 == r2 and i1 == i2, "seed determinism")
assert(r1 >= 0.0 and r1 < 1.0, "random 值域")
assert(random.randint(1, 1) == 1, "randint 闭区间单点")
assert(random.uniform(0, 0) == 0.0, "uniform(0,0)")
lst = [1, 2, 3, 4, 5]
assert(random.shuffle(lst) == nil, "shuffle 返回 nil")
assert(sorted(lst) == [1, 2, 3, 4, 5], "shuffle 多重集保持")
smp = random.sample("abc", 3)
assert(len(smp) == 3, "sample string")
random.seed()
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "random integration failed: {:?}", r.err());
    }

    #[test]
    fn test_integration_random_error_paths() {
        // 错误路径为原生 Err（不经 try/except，task 80 惯例）：整体 Err + 前缀断言。
        for (call, expect) in [
            ("random.randint(2, 1)", "ValueError"),
            ("random.randint(1.5, 2)", "TypeError"),
            ("random.uniform(\"x\", 1)", "TypeError"),
            ("random.gauss(0, -1)", "ValueError"),
            ("random.choice(\"\")", "ValueError"),
            ("random.choice(42)", "TypeError"),
            ("random.shuffle(\"abc\")", "TypeError"),
            ("random.sample([1], 2)", "ValueError"),
            ("random.sample([1], -1)", "ValueError"),
            ("random.seed(\"x\")", "TypeError"),
        ] {
            let full = format!("import random\n{}", call);
            let r = run_source(&full);
            assert!(r.is_err(), "{} should fail", call);
            let e = r.unwrap_err();
            assert!(e.contains(expect), "{}: expected {} in {}", call, expect, e);
        }
    }

    #[test]
    fn test_integration_random_from_import_and_std() {
        let src = r#"
from random import seed, randint, random
seed(7)
a = random()
seed(7)
b = random()
assert(a == b, "from-import seed determinism")
import @std random
assert(type(random.random()) == "float", "@std random")
assert(random.randint(-3, -1) <= -1, "负区间")
"#;
        let r = run_source(src);
        assert!(r.is_ok(), "from-import/@std failed: {:?}", r.err());
    }
}
