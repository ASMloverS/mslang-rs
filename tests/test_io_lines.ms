import io

io.write_file("test_lines.txt", "line1\nline2\nline3\n")

with io.open("test_lines.txt") as f {
    for line in f.lines() {
        print(">> " + line)
    }
}
