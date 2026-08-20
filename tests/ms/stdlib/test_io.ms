# test_io.ms — io 模块
# 覆盖: task 46（10-builtins.md § io；副作用文件名前缀 mslang_test_，结束清空）

import io

F = "mslang_test_io_basic.txt"

# write_file → exists → read_file
io.write_file(F, "hello mslang io")
assert(io.exists(F), "write 后 exists")
assert(io.read_file(F) == "hello mslang io", "read_file 内容一致")

# 覆盖写
io.write_file(F, "overwritten")
assert(io.read_file(F) == "overwritten", "覆盖写")

# 不存在的文件
assert((io.exists("mslang_test_no_such_file.txt")) == false, "exists 不存在")

# open + read（with 自动关闭）
io.write_file(F, "payload-42")
content = ""
with f = io.open(F) {
    content = f.read()
}
assert(content == "payload-42", "with + read")
io.write_file(F, "")

# 多行文件：read 全量 / lines() 逐行
io.write_file(F, "alpha\nbeta\ngamma\n")
assert(io.read_file(F) == "alpha\nbeta\ngamma\n", "多行全量读")

line_list = []
with f2 = io.open(F) {
    for ln in f2.lines() {
        line_list.push(ln)
    }
}
assert(line_list == ["alpha", "beta", "gamma"], "lines() 逐行（无换行符）")

# for..in 遍历 lines()（句柄本身不可直接迭代，经 lines() 取行迭代器）
collected = []
with f3 = io.open(F) {
    for ln in f3.lines() {
        collected.push(ln)
    }
}
assert(collected == ["alpha", "beta", "gamma"], "for 遍历句柄")
io.write_file(F, "")

# 追加模式 open(path, "a")
io.write_file(F, "one")
with fa = io.open(F, "a") {
    fa.write("two")
}
assert(io.read_file(F) == "onetwo", "追加模式 write")

# 写模式 open(path, "w") 截断
with fw = io.open(F, "w") {
    fw.write("fresh")
}
assert(io.read_file(F) == "fresh", "写模式截断")
io.write_file(F, "")

print("ALL PASSED")
