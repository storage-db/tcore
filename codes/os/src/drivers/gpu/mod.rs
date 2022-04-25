mod virtio_gpu;
pub use virtio_gpu::*;
use lazy_static::*;
// use core::cell::RefCell;
// use spin::Mutex;
// use alloc::sync::Arc;
use crate::sync::UPSafeCell;

type GpuDeviceImpl = virtio_gpu::VirtIOGPU;
lazy_static! {
    pub static ref GPU_DEVICE: UPSafeCell<GpuDeviceImpl> = unsafe{UPSafeCell::new((GpuDeviceImpl::new()))};
}
#[allow(unused)]
pub fn gpu_device_test() {
    let mut gpu_device = GPU_DEVICE.exclusive_access().gputest();;
    println!("gpu device test passed!");
}