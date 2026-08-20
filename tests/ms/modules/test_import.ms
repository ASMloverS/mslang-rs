# test_import.ms — 模块系统（import 三形式 + 包 + 缓存）
# 覆盖: task 45（09-modules.md）；fixtures 位于本脚本同目录 fixtures/ 子目录，
# `ms run` 自动把脚本目录（modules/）加入模块搜索路径，fixtures 经 dotted 路径访问

# 形式 1：import + alias（dotted 路径；无 alias 时绑定首段名指向叶模块，
# 故 dotted 导入惯例配别名使用）
import fixtures.math_utils as m1
assert(m1.double(21) == 42, "import as + 属性调用")
assert(m1.triple(5) == 15, "模块第二个函数")
assert(m1.MAGIC == 1337, "模块常量")

# 形式 2：import as alias
import fixtures.math_utils as mu
assert(mu.double(4) == 8, "import as 别名")
assert((mu.MAGIC == 1337), "别名访问常量")

# 形式 3：from module import name（绑定叶模块导出）
from fixtures.math_utils import double
assert(double(50) == 100, "from import 直接绑定")

from fixtures.math_utils import MAGIC
assert(MAGIC == 1337, "from import 常量")

from fixtures.math_utils import triple as tr
assert(tr(2) == 6, "from import as 组合")

from fixtures.math_utils import double as d2, MAGIC as M2
assert(d2(3) == 6 and M2 == 1337, "from import 多目标")

# 包模块（目录 + index.ms）
from fixtures.shapes import area, KIND
assert(area(3, 4) == 12, "包 index 模块导出")
assert(KIND == "rect", "包常量导出")

# 包内子模块
from fixtures.shapes.tri import tri_area
assert(tri_area(10, 4) == 20.0, "dotted 子模块导出")

# 模块缓存：同一模块多次 import 返回同一实例（is 身份）
import fixtures.side_effect as se1
import fixtures.side_effect as se2
assert(se1 is se2, "重复 import 走缓存（同一模块对象）")

# 闭包 fixture：模块函数创建的计数器
rec = se1.make_recorder()
first = rec("a")
second = rec("b")
assert(first == 2 and second == 3, "闭包计数器跨调用保持")

# 导入的函数是一等值
fn apply_remote(f, v) {
    return f(v)
}
assert(apply_remote(mu.double, 8) == 16, "模块函数作为参数")

print("ALL PASSED")
