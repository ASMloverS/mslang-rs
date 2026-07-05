//! task 45：模块系统。参照 [45-module-system](../../docs/mslang/tasks/45-module-system.md)。

pub mod resolver;

pub use resolver::{compile_module_source, parse_std_prefix, ModuleResolver, MAX_IMPORT_DEPTH};

