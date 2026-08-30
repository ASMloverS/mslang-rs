# test_fs.ms — fs 模块（task 82）
# 覆盖: mkdir/mkdirs/rmdir/remove/rename/copy round-trip、幂等语义、
#        list_dir 排序、walk 严格先序、is_dir/is_file/is_abs/abs/size/mtime
# （临时目录经 fs.temp_dir() + uuid4 后缀构造唯一路径，结束清理）

import fs
import io
import uuid

root = fs.temp_dir() + "/" + uuid.uuid4()
fs.remove_all(root)
fs.mkdirs(root)
assert(fs.is_dir(root), "mkdirs 后 is_dir")

sub = root + "/sub"
fs.mkdir(sub)
assert(fs.is_dir(sub), "mkdir 单级")

f = root + "/a.txt"
io.write_file(f, "hello")
assert(fs.is_file(f), "is_file")
assert(fs.size(f) == 5, "size 与写入字节数一致")
assert(fs.is_abs(root), "temp_dir 为绝对路径")
assert(fs.mtime(f) > 0, "mtime 正 Unix 秒")
a = fs.abs("rel/x.txt")
assert(fs.is_abs(a), "abs 结果为绝对路径")

g = root + "/b.txt"
fs.rename(f, g)
assert(not fs.is_file(f) and fs.is_file(g), "rename 迁移")

h = root + "/c.txt"
fs.copy(g, h)
assert(io.read_file(h) == "hello", "copy 内容一致")

fs.remove(h)
assert(not fs.is_file(h), "remove 删除文件")

fs.mkdir(root + "/d")
io.write_file(root + "/z.txt", "z")
names = fs.list_dir(root)
assert(names == ["b.txt", "d", "sub", "z.txt"], "list_dir 排序确定")

fs.mkdirs(root + "/d/e")
io.write_file(root + "/d/e/f.txt", "x")
w = fs.walk(root)
# 归一分隔符（fs.temp_dir 在 Windows 含反斜杠，walk join 亦然）
nroot = root.replace("\\", "/")
assert(len(w) == 7, "walk 全路径扁平列表")
assert(w[0] == root, "walk 首元素为 root（原样返回）")
assert(w[1].replace("\\", "/") == nroot + "/b.txt", "先序 b.txt")
assert(w[2].replace("\\", "/") == nroot + "/d", "先序 d")
assert(w[3].replace("\\", "/") == nroot + "/d/e", "先序 d/e")
assert(w[4].replace("\\", "/") == nroot + "/d/e/f.txt", "先序 d/e/f.txt")
assert(w[5].replace("\\", "/") == nroot + "/sub", "先序 sub（空目录原样输出）")
assert(w[6].replace("\\", "/") == nroot + "/z.txt", "兄弟 z 在 d 子树之后")

fs.remove_all(root + "/d")
fs.remove(root + "/z.txt")
fs.rmdir(sub)
assert(not fs.is_dir(sub), "rmdir 空目录")
# 尽力清理临时目录（清理失败不判失败——CI 临时目录容错，验证标准 9）
fs.remove_all(root)

print("ALL PASSED")
