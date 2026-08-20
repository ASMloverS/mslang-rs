# hello.ms — mslang 入门示例
const GREETING = "Hello, mslang!"

fn greet(names) {
    result = []
    for name in names {
        result.push(GREETING + " " + name)
    }
    return result
}

for msg in greet(["Alice", "Bob", "Charlie"]) {
    print(msg)
}
