import io

io.write_file("test_io.txt", "hello\nworld\n")
print(io.exists("test_io.txt"))

with io.open("test_io.txt") as f {
    print(f.read())
}

content = io.read_file("test_io.txt")
print(content)
