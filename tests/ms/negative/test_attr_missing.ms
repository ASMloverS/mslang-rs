# 期望：AttributeError —— 实例没有该属性
class Plain2 {
}
p = Plain2()
print(p.no_such_attr)
