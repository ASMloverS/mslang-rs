fn main() {
    #[cfg(feature = "capi")]
    {
        // Compile C va_list shim (msStringFmt implemented in C).
        cc::Build::new()
            .file("src/capi/vsnprintf_shim.c")
            .compile("mslang_capi_shim");

        let crate_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let config = cbindgen::Config::from_file("cbindgen.toml")
            .expect("Unable to read cbindgen.toml");

        let bindings = cbindgen::Builder::new()
            .with_crate(&crate_dir)
            .with_config(config.clone())
            .generate();

        match bindings {
            Ok(_bindings) => {
                // V2/V3: absolute path (based on crate_dir) to avoid CWD dependency.
                let include_dir = std::path::Path::new(&crate_dir).join("include/mslang");
                // V2: create_dir_all failure must panic, not be silently ignored.
                std::fs::create_dir_all(&include_dir)
                    .expect("cannot create include/mslang directory");

                let hand_written = ["types.h", "mslang.h"];

                let modules = [
                    ("vm", "vm.h"),
                    ("value", "value.h"),
                    ("call", "call.h"),
                    ("error", "error.h"),
                    ("module", "module.h"),
                    ("class", "class.h"),
                    ("gc", "gc.h"),
                ];

                for (module_name, filename) in &modules {
                    // Each module header gets a unique include guard so that
                    // mslang.h can include them all without one masking the rest.
                    let guard_name = format!("MSLANG_{}_H", module_name.to_uppercase());
                    let mut module_config = config.clone();
                    module_config.include_guard = Some(guard_name);

                    let mut builder = cbindgen::Builder::new()
                        .with_crate(&crate_dir)
                        .with_config(module_config);

                    // V3: use absolute path (based on crate_dir) to avoid CWD dependency.
                    let src_path = std::path::Path::new(&crate_dir)
                        .join(format!("src/capi/{}.rs", module_name));
                    builder = builder.with_src(src_path.to_str().unwrap());

                    if let Ok(module_bindings) = builder.generate() {
                        // `filename` is `&&str`; `[&str; N]::contains` expects `&&str`.
                        if !hand_written.contains(filename) {
                            let out_path = include_dir.join(filename);
                            module_bindings.write_to_file(&out_path);
                            // task 70: append #include "call_macros.h" to generated call.h
                            // so standalone inclusion of call.h provides the msCall0-3 macros.
                            if *filename == "call.h" {
                                let content = std::fs::read_to_string(&out_path)
                                    .expect("cannot read generated call.h");
                                let patched = if content.contains("call_macros.h") {
                                    content
                                } else {
                                    let marker = "#endif /* MSLANG_CALL_H */";
                                    content.replace(
                                        marker,
                                        "#include \"call_macros.h\"\n\n#endif /* MSLANG_CALL_H */",
                                    )
                                };
                                std::fs::write(&out_path, patched)
                                    .expect("cannot write patched call.h");
                            }
                        }
                    }
                }
            }
            Err(e) => {
                // R1: panic on cbindgen failure (rather than silent degradation)
                // so CI immediately surfaces the problem.
                panic!("cbindgen failed: {}", e);
            }
        }
    }
}
