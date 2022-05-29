use core::any::Any;

pub trait BlockDevice : Send + Sync + Any {
    fn read_block(&self, block_id: usize, buf: &mut [u8]);//磁盘读入内存缓冲
    fn write_block(&self, block_id: usize, buf: &[u8]);//内存缓冲中写入到磁盘
}
