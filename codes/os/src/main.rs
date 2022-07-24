#![no_std]
#![no_main]
#![feature(global_asm)]
#![feature(llvm_asm)]
#![feature(asm)]
#![feature(panic_info_message)]
#![feature(alloc_error_handler)]

use lazy_static::lazy_static;
use sbi::sbi_send_ipi;
use spin::*;
use timer::get_timeval;
use syscall::*;
use alloc::sync::Arc;
use core::time;
extern crate alloc;

#[macro_use]
extern crate bitflags;

#[macro_use]
mod console;
mod lang_items;
mod sbi;
mod syscall;
mod trap;
mod config;
mod utils;
#[macro_use]
mod monitor;
mod task;
mod timer;
mod mm;
mod fs;
mod drivers;
mod sync;

global_asm!(include_str!("entry.asm"));
global_asm!(include_str!("start_app.S"));

fn clear_bss() {
    extern "C" {
        fn sbss();
        fn ebss();
    }
    (sbss as usize..ebss as usize).for_each(|a| {
        unsafe { (a as *mut u8).write_volatile(0) }
    });
}

pub fn id() -> usize {
    let cpu_id;
    unsafe {
        llvm_asm!("mv $0, tp" : "=r"(cpu_id));
    }
    cpu_id
}

pub const SYSCALL_GETPPID:usize = 173;
pub fn test() {
    // let start = get_timeval();
    // for _ in 0..100000000{
    //     syscall(SYSCALL_GETPPID,[0,0,0,0,0,0]);
    // }
    // let end = get_timeval();
    // println!("test: run sys_getppid 100000000 times, spent {:?}",end-start);
}

struct Core2flag{
    is_in: bool,
}

impl Core2flag{
    pub fn is_in(&self)->bool{
        self.is_in
    }
    pub fn set_in(&mut self){
        self.is_in = true;
    }
}


lazy_static! {
    static ref CORE2_FLAG: Arc<Mutex<Core2flag>> = Arc::new(Mutex::new(
        Core2flag{
            is_in:false,
        }
    ));
}

#[no_mangle]
pub fn rust_main(_hartid: usize, device_tree_paddr: usize) -> ! {
    println!("device_tree_paddr:{}",device_tree_paddr);
    let core = id();
    println!("core {} is running",core);
    if core != 0 {
        loop{}
        /// WARNING: Multicore mode only supports customized RustSBI platform, especially not including OpenSBI
        /// We use OpenSBI in qemu and customized RustSBI in k210, if you want to try Multicore mode, you have to
        /// try to switch to RustSBI in qemu and try to wakeup, which needs some effort and you can refer to docs.
        // while !CORE2_FLAG.lock().is_in(){}
        mm::init_othercore();
        println!("other core start");
        trap::init();
        trap::enable_timer_interrupt();
        timer::set_next_trigger();
        println!("other core start run tasks");
        loop{};
        task::run_tasks();
        panic!("Unreachable in rust_main!");
    }
    clear_bss();
    mm::init();
    mm::remap_test();
    println!("TCore: memory initialized");
    trap::init();
    trap::enable_timer_interrupt();
    timer::set_next_trigger();
    println!("TCore: interrupt initialized");
    fs::init_rootfs();
    println!("TCore: fs initialized");
    task::add_initproc();
    println!("TCore: task initialized");
    println!("TCore: wake other cores");
    let mask:usize = 1 << 1;
    sbi_send_ipi(&mask as *const usize as usize);
    // CORE2_FLAG.lock().set_in();
    // test();
    //sys_gputest();
    println!("TCore: run tasks");
    task::run_tasks();
    panic!("Unreachable in rust_main!");
}