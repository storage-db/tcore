mod block;
pub mod serial;
mod gpu;
pub use block::BLOCK_DEVICE;
pub use gpu::*;
pub use serial::ns16550a::Ns16550a;