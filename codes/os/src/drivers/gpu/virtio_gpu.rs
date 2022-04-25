use crate::mm::{
    frame_alloc,
    frame_dealloc,
    FrameTracker,
    PageTable,
    PhysAddr,
    PhysPageNum,
    StepByOne,
    VirtAddr,
    //kernel_token,
    KERNEL_TOKEN,
};
use crate::sync::UPSafeCell;
use crate::timer::get_time;
use alloc::vec::Vec;
use core::any::Any;
use core::fmt::Result;
use lazy_static::*;
use spin::Mutex;
use virtio_drivers::{VirtIOGpu, VirtIOHeader};
#[allow(unused)]
const VIRTIO1: usize = 0x10002000;

pub trait GpuDevice: Send + Sync + Any {
    fn gputest(&mut self);
    fn flush(&mut self);
}

pub struct VirtIOGPU(Mutex<UPSafeCell<VirtIOGpu<'static>>>);

lazy_static! {
    static ref QUEUE_FRAMES: Mutex<Vec<FrameTracker>> = Mutex::new(Vec::new());
}
impl GpuDevice for VirtIOGPU {
    fn gputest(&mut self) {
         match self.setup_framebuffer(|fb:&mut [u8]|{
            for y in 0..768 {
                for x in 0..1024 {
                    let idx = (y * 1024 + x) * 4;
                    fb[idx] = x as u8;
                    fb[idx + 1] = y as u8;
                    fb[idx + 2] = (x + y) as u8;
                }
             }
             Ok(())
            }
            ){
            Ok(_) =>{
            let start = get_time();
            self.flush();
            let end = get_time();
            println!("[vgpu displaying test]: {}", end - start);
            println!("virtio-gpu test finished");
            },
            _=>{
                println!("failed to get fb");
            }
        } 
    }

    fn flush(&mut self) {
        if let mut vg = self.0.lock().exclusive_access(){
            vg.flush().expect("fail to flush");
        }else{
            println!("fail to flush");
        }
    }
}
impl VirtIOGPU {
    #[allow(unused)]
    pub fn new() -> Self {
        let vg = Self(unsafe{Mutex::new(UPSafeCell::new(
            VirtIOGpu::new(unsafe { &mut *(VIRTIO1 as *mut VirtIOHeader) })
                .expect("failed to create gpu driver"),
        ))});
        vg
    }
    pub fn setup_framebuffer(&mut self,f: impl FnOnce(&mut [u8]) -> Result) -> Result {
        if let Ok(fb) =self.0.get_mut().exclusive_access().setup_framebuffer(){
            f(fb)
        }else{
            println!("fail to get framebuffer");    
            Ok(())
        }
    }
}

// #[no_mangle]
// pub extern "C" fn virtio_dma_alloc(pages: usize) -> PhysAddr {
//     let mut ppn_base = PhysPageNum(0);
//     for i in 0..pages {
//         let frame = frame_alloc().unwrap();
//         if i == 0 {
//             ppn_base = frame.ppn;
//         }
//         assert_eq!(frame.ppn.0, ppn_base.0 + i);
//         QUEUE_FRAMES.lock().push(frame);
//     }
//     ppn_base.into()
// }

// #[no_mangle]
// pub extern "C" fn virtio_dma_dealloc(pa: PhysAddr, pages: usize) -> i32 {
//     let mut ppn_base: PhysPageNum = pa.into();
//     for _ in 0..pages {
//         frame_dealloc(ppn_base);
//         ppn_base.step();
//     }
//     0
// }

// #[no_mangle]
// pub extern "C" fn virtio_phys_to_virt(paddr: PhysAddr) -> VirtAddr {
//     VirtAddr(paddr.0)
// }

// #[no_mangle]
// pub extern "C" fn virtio_virt_to_phys(vaddr: VirtAddr) -> PhysAddr {
//     PageTable::from_token(KERNEL_TOKEN.token())
//         .translate_va(vaddr)
//         .unwrap()
// }
