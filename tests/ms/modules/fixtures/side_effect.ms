# side_effect.ms — 模块缓存 fixture
# 导入即执行顶层（init_log 记录加载）；record 经闭包持有模块内状态

init_marker = "side-effect-module-loaded-" + "v1"

fn make_recorder() {
    log = ["loaded"]
    return fn(tag) {
        log.push(tag)
        return log.length()
    }
}
