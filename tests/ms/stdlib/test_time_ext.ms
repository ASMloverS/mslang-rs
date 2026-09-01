# test_time_ext.ms — time 扩充（task 83）
# 覆盖: now_ms/monotonic/iso/date_parts/sleep_ms/format_ts/parse（验证标准 1-8、15；
#        9-14 错误路径为原生 Err 不经 try/except，由 time.rs Rust 单测覆盖）

import time
import json

# 1（宽松断言，容忍 CI 抖动）：now_ms 与 time.now() 指同一时刻
diff = time.now_ms() / 1000.0 - time.now()
assert(diff > -2.0 and diff < 2.0, "now_ms 与 now 同刻")

# 2（宽松断言）：monotonic 非降；sleep(0.05) 前后单调差 >= 0.05
a = time.monotonic()
b = time.monotonic()
assert(b >= a, "monotonic 非降")
t0 = time.monotonic()
time.sleep(0.05)
assert(time.monotonic() - t0 >= 0.05, "sleep 0.05s 后单调差")
assert(time.sleep_ms(0) == nil, "sleep_ms(0)")

# 3：iso
assert(time.iso(0) == "1970-01-01T00:00:00Z", "iso epoch")
assert(time.iso(1700000000) == "2023-11-14T22:13:20Z", "iso known")
assert(time.iso().length() == 20, "iso 缺省当前时间")

# 4-5：date_parts（1970-01-01 为周四=3；1970-01-02 为周五=4）
p = time.date_parts(0)
assert(p["year"] == 1970 and p["month"] == 1 and p["day"] == 1, "date_parts 日期")
assert(p["hour"] == 0 and p["minute"] == 0 and p["second"] == 0, "date_parts 时刻")
assert(p["weekday"] == 3, "date_parts epoch 周四")
assert(time.date_parts(86400)["weekday"] == 4, "date_parts 周五")
assert(time.date_parts()["year"] >= 2024, "date_parts 缺省当前时间")

# 6：format_ts
assert(time.format_ts(0, "%Y-%m-%d %H:%M:%S") == "1970-01-01 00:00:00", "format_ts epoch")
assert(time.format_ts(1700000000, "%Y/%m/%d %H:%M:%S") == "2023/11/14 22:13:20", "format_ts known")
assert(time.format_ts(1700000000, "%Y%m%d") == "20231114", "指令相邻无分隔")
assert(time.format_ts(0, "%%Y %%") == "%Y %", "%% 字面百分号")

# 7-8：parse 与往返一致（含闰日 2000-02-29）
assert(time.parse("2023-11-14 22:13:20", "%Y-%m-%d %H:%M:%S") == 1700000000.0, "parse known")
assert(time.parse("1970-01-01 00:00:00", "%Y-%m-%d %H:%M:%S") == 0.0, "parse epoch")
fmt = "%Y-%m-%d %H:%M:%S"
for t in [0.0, 1.0, 951782400.0, 1582977600.0, 1700000000.0] {
    assert(time.parse(time.format_ts(t, fmt), fmt) == t, "往返一致 " + str(t))
}

# 15：json.parse（1 参）× time.parse（2 参）同脚本并存
assert(json.parse("1") == 1, "json.parse 单参")
assert(time.parse("2000-02-29", "%Y-%m-%d") == 951782400.0, "闰日解析")
assert(json.parse("[1, 2]") == [1, 2], "交替调用互不干扰")

print("ALL PASSED")
