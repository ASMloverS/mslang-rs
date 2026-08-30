//! `fs` 原生模块（task 82）。
//!
//! 参照 [82-stdlib-fs-os-sys](../../../docs/mslang/tasks/82-stdlib-fs-os-sys.md)
//! 与 [16-stdlib-expansion](../../../docs/mslang/16-stdlib-expansion.md) §4.7。
//!
//! 与 io 模块分工：io 保内容读写（read_file/write_file/exists/open），
//! fs 管目录结构与元数据。错误一律 IOError 前缀（§4.7）。

use super::expect_string;
use crate::vm::builtins::{alloc_native_function, NativeFn, NativeFunction};
use crate::vm::object::{
    alloc_list, alloc_module, alloc_string, read_module_mut, MsObjHeader, Object,
};
use crate::vm::VM;

/// 构造 `fs` 原生模块，返回指向 MsModule 的裸指针（TypeTag::MODULE）。
/// exports 含 17 个原生函数（§4.7 全集）。
pub fn register_fs_module() -> *mut MsObjHeader {
    let mut exports = std::collections::HashMap::new();
    let funcs: [(&str, NativeFn); 17] = [
        ("mkdir", native_fs_mkdir),
        ("mkdirs", native_fs_mkdirs),
        ("rmdir", native_fs_rmdir),
        ("remove", native_fs_remove),
        ("remove_all", native_fs_remove_all),
        ("rename", native_fs_rename),
        ("copy", native_fs_copy),
        ("list_dir", native_fs_list_dir),
        ("walk", native_fs_walk),
        ("is_dir", native_fs_is_dir),
        ("is_file", native_fs_is_file),
        ("is_abs", native_fs_is_abs),
        ("abs", native_fs_abs),
        ("size", native_fs_size),
        ("mtime", native_fs_mtime),
        ("temp_dir", native_fs_temp_dir),
        ("home_dir", native_fs_home_dir),
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
    let m = alloc_module("fs");
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

fn native_fs_mkdir(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let path = expect_string(args.get(0), "mkdir(path)")?;
    // 单级创建：path 已存在（文件/目录）或父目录缺失均 IOError。
    std::fs::create_dir(&path).map_err(|e| format!("IOError: cannot mkdir '{}': {}", path, e))?;
    Ok(Object::Nil)
}

fn native_fs_mkdirs(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let path = expect_string(args.get(0), "mkdirs(path)")?;
    // 递归创建；幂等（已存在目录成功，create_dir_all 语义）。
    std::fs::create_dir_all(&path)
        .map_err(|e| format!("IOError: cannot mkdirs '{}': {}", path, e))?;
    Ok(Object::Nil)
}

fn native_fs_rmdir(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let path = expect_string(args.get(0), "rmdir(path)")?;
    // 仅空目录：非空 / 不存在 / 为文件均 IOError。
    std::fs::remove_dir(&path).map_err(|e| format!("IOError: cannot rmdir '{}': {}", path, e))?;
    Ok(Object::Nil)
}

fn native_fs_remove(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let path = expect_string(args.get(0), "remove(path)")?;
    if std::path::Path::new(&path).is_dir() {
        return Err(format!(
            "IOError: remove(path) target is a directory: '{}'",
            path
        ));
    }
    std::fs::remove_file(&path).map_err(|e| format!("IOError: cannot remove '{}': {}", path, e))?;
    Ok(Object::Nil)
}

fn native_fs_remove_all(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let path = expect_string(args.get(0), "remove_all(path)")?;
    // 幂等：路径不存在返回 nil（Go os.RemoveAll 语义）。
    match std::fs::remove_dir_all(&path) {
        Ok(()) => Ok(Object::Nil),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Object::Nil),
        Err(e) => Err(format!("IOError: cannot remove_all '{}': {}", path, e)),
    }
}

fn native_fs_rename(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let old_path = expect_string(args.get(0), "rename(old, new)")?;
    let new_path = expect_string(args.get(1), "rename(old, new)")?;
    std::fs::rename(&old_path, &new_path).map_err(|e| {
        format!(
            "IOError: cannot rename '{}' to '{}': {}",
            old_path, new_path, e
        )
    })?;
    Ok(Object::Nil)
}

/// fs.copy(src, dst)：文件 → 文件（dst 存在则覆盖，std::fs::copy 语义）。
/// §2.2 同名冲突治理：与全局内置 copy(val) 同名不同 arity，native_arities
/// 升级 usize::MAX，此处自校验恰 2 参（builtin_copy 自校验恰 1 参）。
/// dst 为目录 → IOError（不自动拼接文件名，显式优于隐式）。
fn native_fs_copy(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    if args.len() != 2 {
        return Err(format!(
            "TypeError: copy(src, dst) takes exactly 2 arguments but {} were given",
            args.len()
        ));
    }
    let src = expect_string(args.get(0), "copy(src, dst)")?;
    let dst = expect_string(args.get(1), "copy(src, dst)")?;
    if std::path::Path::new(&dst).is_dir() {
        return Err(format!(
            "IOError: copy(src, dst) dst is a directory: '{}'",
            dst
        ));
    }
    std::fs::copy(&src, &dst)
        .map_err(|e| format!("IOError: cannot copy '{}' to '{}': {}", src, dst, e))?;
    Ok(Object::Nil)
}

/// read_dir 子项文件名排序返回（list_dir 与 walk 共用同一排序，
/// 保证两者顺序规则一致；read_dir 本身不返回 `.`/`..`）。
fn sorted_entry_names(dir: &std::path::Path) -> Result<Vec<String>, String> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| format!("IOError: cannot read dir '{}': {}", dir.display(), e))?;
    let mut names = Vec::new();
    for entry in entries {
        let name = entry
            .map_err(|e| format!("IOError: cannot read dir '{}': {}", dir.display(), e))?
            .file_name()
            .to_string_lossy()
            .into_owned();
        names.push(name);
    }
    names.sort();
    Ok(names)
}

fn native_fs_list_dir(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let path = expect_string(args.get(0), "list_dir(path)")?;
    let names = sorted_entry_names(std::path::Path::new(&path))?;
    Ok(alloc_list(names.iter().map(|n| alloc_string(n)).collect()))
}

/// walk(path)：显式栈迭代 DFS 严格先序（避免深目录递归栈溢出）。
/// pop 时输出、子项排序后逆序压栈 → 字典序最小子项先展开，后继兄弟排在
/// 先前兄弟的子树之后（与 Go filepath.Walk 同序）。输出含 root 自身
/// （首元素）；不跟随符号链接（is_dir && !is_symlink）。
fn native_fs_walk(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let root = expect_string(args.get(0), "walk(path)")?;
    let mut out: Vec<Object> = Vec::new();
    let mut stack = vec![std::path::PathBuf::from(&root)];
    while let Some(p) = stack.pop() {
        out.push(alloc_string(&p.to_string_lossy()));
        if p.is_dir() && !p.is_symlink() {
            let mut children = sorted_entry_names(&p)?;
            children.reverse();
            for name in children {
                stack.push(p.join(name));
            }
        }
    }
    Ok(alloc_list(out))
}

fn native_fs_is_dir(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let path = expect_string(args.get(0), "is_dir(path)")?;
    Ok(Object::Bool(std::path::Path::new(&path).is_dir()))
}

fn native_fs_is_file(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let path = expect_string(args.get(0), "is_file(path)")?;
    Ok(Object::Bool(std::path::Path::new(&path).is_file()))
}

fn native_fs_is_abs(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let path = expect_string(args.get(0), "is_abs(path)")?;
    Ok(Object::Bool(std::path::Path::new(&path).is_absolute()))
}

/// abs(path)：std::path::absolute 词法绝对化，不解析符号链接。
/// 平台差异：Unix 保留 `..`（仅前置 cwd）；Windows 经 GetFullPathNameW
/// 词法归一 `..`（10-builtins.md 注明，测试仅断言 is_absolute 不变量）。
fn native_fs_abs(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let path = expect_string(args.get(0), "abs(path)")?;
    let abs = std::path::absolute(&path)
        .map_err(|e| format!("IOError: cannot absolutize '{}': {}", path, e))?;
    Ok(alloc_string(&abs.to_string_lossy()))
}

fn native_fs_size(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let path = expect_string(args.get(0), "size(path)")?;
    let len = std::fs::metadata(&path)
        .map_err(|e| format!("IOError: cannot stat '{}': {}", path, e))?
        .len();
    Ok(Object::Int(len as i64))
}

fn native_fs_mtime(_vm: &mut VM, args: &[Object]) -> Result<Object, String> {
    let path = expect_string(args.get(0), "mtime(path)")?;
    let modified = std::fs::metadata(&path)
        .map_err(|e| format!("IOError: cannot stat '{}': {}", path, e))?
        .modified()
        .map_err(|e| format!("IOError: no mtime for '{}': {}", path, e))?;
    let secs = modified
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| format!("IOError: mtime before epoch for '{}': {}", path, e))?
        .as_secs_f64();
    Ok(Object::Float(secs))
}

fn native_fs_temp_dir(_vm: &mut VM, _args: &[Object]) -> Result<Object, String> {
    Ok(alloc_string(&std::env::temp_dir().to_string_lossy()))
}

/// home_dir()：env USERPROFILE/HOME；均缺失 → IOError。
fn native_fs_home_dir(_vm: &mut VM, _args: &[Object]) -> Result<Object, String> {
    match std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        Some(home) => Ok(alloc_string(&home.to_string_lossy())),
        None => Err("IOError: home_dir: USERPROFILE/HOME not set".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_util::{run_source, s, strval, vm};
    use super::*;
    use crate::vm::object::{read_list, TypeTag};

    /// 每个测试独占 temp_dir 下唯一子目录（先清残留再建，测试后调用方清理）。
    fn temp_root(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("mslang_fs_{}", name));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Path → mslang string 参数（反斜杠转正斜杠，ms 字面量转义安全）。
    fn ps(p: &std::path::Path) -> Object {
        s(&p.to_string_lossy().replace('\\', "/"))
    }

    /// list 对象 → Vec<String>（分隔符归一为 '/'，跨平台比较）。
    fn norm_items(list: &Object) -> Vec<String> {
        let Object::Ref(ptr) = list else {
            panic!("expected Ref");
        };
        // SAFETY: walk/list_dir 返回 alloc_list 分配的 LIST。
        unsafe { read_list(*ptr) }
            .iter()
            .map(|o| strval(o).replace('\\', "/"))
            .collect()
    }

    #[test]
    fn test_fs_module_registration() {
        let ptr = register_fs_module();
        // SAFETY: ptr 由 register_fs_module 返回的有效 MsModule。
        unsafe {
            assert_eq!((*ptr).type_tag, TypeTag::MODULE as u8);
            let m = read_module_mut(ptr);
            assert_eq!(m.name, "fs");
            assert_eq!(m.exports.len(), 17, "fs 导出恰 17 个函数");
            for name in [
                "mkdir",
                "mkdirs",
                "rmdir",
                "remove",
                "remove_all",
                "rename",
                "copy",
                "list_dir",
                "walk",
                "is_dir",
                "is_file",
                "is_abs",
                "abs",
                "size",
                "mtime",
                "temp_dir",
                "home_dir",
            ] {
                assert!(m.exports.contains_key(name), "missing export: {}", name);
            }
        }
    }

    #[test]
    fn test_fs_mkdir_mkdirs_rmdir() {
        let mut v = vm();
        let dir = temp_root("mkdir_rmdir");
        let sub = dir.join("sub");

        // mkdir → nil；已存在 → IOError
        assert_eq!(native_fs_mkdir(&mut v, &[ps(&sub)]).unwrap(), Object::Nil);
        let err = native_fs_mkdir(&mut v, &[ps(&sub)]).unwrap_err();
        assert!(err.contains("IOError"), "mkdir 已存在 → IOError: {}", err);

        // mkdirs 递归 + 幂等（已存在成功）
        let deep = dir.join("a/b/c");
        assert_eq!(native_fs_mkdirs(&mut v, &[ps(&deep)]).unwrap(), Object::Nil);
        assert_eq!(native_fs_mkdirs(&mut v, &[ps(&deep)]).unwrap(), Object::Nil);
        assert!(deep.is_dir());

        // rmdir 非空 → IOError；清空后成功
        std::fs::write(dir.join("sub/f.txt"), "x").unwrap();
        let err = native_fs_rmdir(&mut v, &[ps(&sub)]).unwrap_err();
        assert!(err.contains("IOError"));
        std::fs::remove_file(dir.join("sub/f.txt")).unwrap();
        assert_eq!(native_fs_rmdir(&mut v, &[ps(&sub)]).unwrap(), Object::Nil);
        assert!(!sub.exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_fs_remove_remove_all() {
        let mut v = vm();
        let dir = temp_root("remove");
        let f = dir.join("f.txt");
        std::fs::write(&f, "data").unwrap();

        // remove 文件 → nil；目录 → IOError
        assert_eq!(native_fs_remove(&mut v, &[ps(&f)]).unwrap(), Object::Nil);
        let err = native_fs_remove(&mut v, &[ps(&dir)]).unwrap_err();
        assert!(err.contains("IOError") && err.contains("directory"));

        // remove_all 幂等：不存在 → nil；递归删除树 → nil
        let missing = dir.join("no_such_dir");
        assert_eq!(
            native_fs_remove_all(&mut v, &[ps(&missing)]).unwrap(),
            Object::Nil
        );
        std::fs::create_dir_all(dir.join("x/y")).unwrap();
        std::fs::write(dir.join("x/y/z.txt"), "z").unwrap();
        assert_eq!(
            native_fs_remove_all(&mut v, &[ps(&dir.join("x"))]).unwrap(),
            Object::Nil
        );
        assert!(!dir.join("x").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_fs_rename_copy() {
        let mut v = vm();
        let dir = temp_root("rename_copy");
        let src = dir.join("a.txt");
        std::fs::write(&src, "hello").unwrap();

        // rename → nil，内容随迁
        let dst = dir.join("b.txt");
        assert_eq!(
            native_fs_rename(&mut v, &[ps(&src), ps(&dst)]).unwrap(),
            Object::Nil
        );
        assert!(!src.exists());
        assert_eq!(std::fs::read_to_string(&dst).unwrap(), "hello");

        // copy → nil，内容一致；dst 为目录 → IOError
        let cp = dir.join("c.txt");
        assert_eq!(
            native_fs_copy(&mut v, &[ps(&dst), ps(&cp)]).unwrap(),
            Object::Nil
        );
        assert_eq!(std::fs::read_to_string(&cp).unwrap(), "hello");
        let err = native_fs_copy(&mut v, &[ps(&cp), ps(&dir)]).unwrap_err();
        assert!(err.contains("IOError") && err.contains("directory"));

        // copy 自校验恰 2 参（§2.2 MAX 治理）：1 参 / 3 参 / 非法类型 → TypeError
        let err = native_fs_copy(&mut v, &[ps(&cp)]).unwrap_err();
        assert!(err.contains("TypeError") && err.contains("copy(src, dst)"));
        let err = native_fs_copy(&mut v, &[ps(&cp), ps(&cp), ps(&cp)]).unwrap_err();
        assert!(err.contains("TypeError") && err.contains("copy(src, dst)"));
        let err = native_fs_copy(&mut v, &[Object::Int(1), s("x")]).unwrap_err();
        assert!(err.contains("TypeError") && err.contains("copy(src, dst)"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_fs_list_dir_sorted() {
        let mut v = vm();
        let dir = temp_root("list_dir");
        for name in ["b.txt", "a.txt", "c"] {
            if name == "c" {
                std::fs::create_dir(dir.join("c")).unwrap();
            } else {
                std::fs::write(dir.join(name), "x").unwrap();
            }
        }
        // 排序后返回，不含 `.`/`..`
        let items = norm_items(&native_fs_list_dir(&mut v, &[ps(&dir)]).unwrap());
        assert_eq!(items, vec!["a.txt", "b.txt", "c"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_fs_walk_preorder_lock() {
        // 锁定确切顺序（验证标准 3）：root → a.txt → b → b/c.txt → b/d →
        // b/d/e.txt → z.txt（后继兄弟 z 在先前兄弟 b 的整个子树之后）。
        let mut v = vm();
        let dir = temp_root("walk");
        std::fs::write(dir.join("a.txt"), "a").unwrap();
        std::fs::write(dir.join("z.txt"), "z").unwrap();
        std::fs::create_dir_all(dir.join("b/d")).unwrap();
        std::fs::write(dir.join("b/c.txt"), "c").unwrap();
        std::fs::write(dir.join("b/d/e.txt"), "e").unwrap();

        let items = norm_items(&native_fs_walk(&mut v, &[ps(&dir)]).unwrap());
        let expected: Vec<String> = [
            dir.clone(),
            dir.join("a.txt"),
            dir.join("b"),
            dir.join("b/c.txt"),
            dir.join("b/d"),
            dir.join("b/d/e.txt"),
            dir.join("z.txt"),
        ]
        .iter()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect();
        assert_eq!(items, expected, "walk 严格先序（含 root 首元素）");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_fs_predicates_abs_size_mtime() {
        let mut v = vm();
        let dir = temp_root("predicates");
        let f = dir.join("f.bin");
        std::fs::write(&f, b"12345").unwrap();

        assert_eq!(
            native_fs_is_dir(&mut v, &[ps(&f)]).unwrap(),
            Object::Bool(false)
        );
        assert_eq!(
            native_fs_is_file(&mut v, &[ps(&f)]).unwrap(),
            Object::Bool(true)
        );
        assert_eq!(
            native_fs_is_dir(&mut v, &[ps(&dir)]).unwrap(),
            Object::Bool(true)
        );
        assert_eq!(
            native_fs_is_file(&mut v, &[ps(&dir)]).unwrap(),
            Object::Bool(false)
        );

        // is_abs：绝对路径 true；相对路径 false
        assert_eq!(
            native_fs_is_abs(&mut v, &[ps(&dir)]).unwrap(),
            Object::Bool(true)
        );
        assert_eq!(
            native_fs_is_abs(&mut v, &[s("rel/x.txt")]).unwrap(),
            Object::Bool(false)
        );

        // abs：仅断言 is_absolute 不变量（Windows 词法归一 `..`，平台有差异）
        let abs = strval(&native_fs_abs(&mut v, &[s("rel/x.txt")]).unwrap());
        assert!(
            std::path::Path::new(&abs).is_absolute(),
            "abs 结果为绝对路径: {}",
            abs
        );

        // size 与写入字节数一致；mtime 为现代 Unix 秒（Float）
        assert_eq!(native_fs_size(&mut v, &[ps(&f)]).unwrap(), Object::Int(5));
        match native_fs_mtime(&mut v, &[ps(&f)]).unwrap() {
            Object::Float(t) => assert!(t > 1_600_000_000.0, "mtime: {}", t),
            other => panic!("mtime 应为 Float，got {}", other.type_name()),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_fs_temp_home() {
        let mut v = vm();
        let t = strval(&native_fs_temp_dir(&mut v, &[]).unwrap());
        assert!(!t.is_empty());
        assert!(
            std::path::Path::new(&t).is_absolute(),
            "temp_dir 为绝对路径"
        );
        match native_fs_home_dir(&mut v, &[]) {
            Ok(home) => assert!(!strval(&home).is_empty()),
            Err(e) => assert!(e.contains("IOError"), "home 缺失时报 IOError: {}", e),
        }
    }

    #[test]
    fn test_fs_copy_same_name_global_builtin() {
        // §2.2 同名交叉调用回归：全局 copy(val)（自校验恰 1 参）与
        // fs.copy(src, dst)（自校验恰 2 参）并存均可用
        //（子进程级端到端另见 tests/sys_stdin.rs）。
        let dir = temp_root("copy_governance");
        let src = dir.join("src.txt").to_string_lossy().replace('\\', "/");
        let dst = dir.join("dst.txt").to_string_lossy().replace('\\', "/");
        let source = format!(
            r#"
import fs
import io
a = copy([1, 2])
assert(len(a) == 2 and a[0] == 1 and a[1] == 2, "全局 copy 浅拷贝 list")
d = copy({{"k": 1}})
assert(d["k"] == 1, "全局 copy 浅拷贝 dict")
io.write_file("{src}", "data")
fs.copy("{src}", "{dst}")
assert(io.read_file("{dst}") == "data", "fs.copy 二参文件复制")
"#
        );
        let r = run_source(&source);
        std::fs::remove_dir_all(&dir).ok();
        assert!(r.is_ok(), "copy 同名共存失败: {:?}", r.err());
    }

    #[test]
    fn test_integration_fs() {
        // 等价 test_fs.ms 核心：round-trip + walk + 元数据（temp_dir 唯一子目录）。
        let dir = temp_root("integration");
        let root = dir.join("root").to_string_lossy().replace('\\', "/");
        let source = format!(
            r#"
import fs
import io
root = "{root}"
fs.mkdirs(root + "/b/d")
io.write_file(root + "/a.txt", "hello")
assert(fs.is_file(root + "/a.txt"))
assert(fs.size(root + "/a.txt") == 5)
w = fs.walk(root)
assert(len(w) == 4, "root + a.txt + b + b/d")
assert(w[0] == root, "walk 首元素为 root")
fs.remove_all(root)
assert(not fs.is_dir(root), "remove_all 清理")
"#
        );
        let r = run_source(&source);
        std::fs::remove_dir_all(&dir).ok();
        assert!(r.is_ok(), "fs integration failed: {:?}", r.err());
    }
}
