import fileio
import os

fn test_fileio() {
    tmpdir = os.tmpdir()
    testfile = tmpdir + "/mslang_test.txt"

    fileio.write(testfile, "hello from mslang")
    content = fileio.read(testfile)
    assert content == "hello from mslang", "read/write roundtrip"

    exists = fileio.exists(testfile)
    assert exists == true, "file exists"

    not_exists = fileio.exists(tmpdir + "/nonexistent_mslang_file.txt")
    assert not_exists == false, "nonexistent file"
}

test_fileio()
print("extension tests passed")
