use crate::compiler::Chunk;

#[derive(Clone)]
pub struct CallFrame {
    pub chunk: Chunk,
    pub ip: usize,
    pub stack_base: usize,
}
