pub mod opcode;

/// 编译单元。task 16 定义最小版本；task 17 扩展完整字段
/// （`lines`、`locals`、`upvalues`、`parent`）。
#[derive(Debug, Clone)]
pub struct CompilationUnit {
    pub code: Vec<u8>,
    pub constants: Vec<String>,
}
