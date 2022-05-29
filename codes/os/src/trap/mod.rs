mod context;

use crate::config::{TRAMPOLINE, TRAP_CONTEXT, USER_STACK_SIZE};
use crate::gdb_print;
use crate::mm::{print_free_pages, VirtAddr, VirtPageNum};
use crate::monitor::*;
use crate::syscall::{syscall, test};
use crate::task::*;
use crate::timer::set_next_trigger;
use riscv::register::{
    mtvec::TrapMode,
    scause::{self, Exception, Interrupt, Trap},
    sepc, sie, stval, stvec,
};

global_asm!(include_str!("trap.S"));


pub fn init() {
    set_kernel_trap_entry();
}

fn set_kernel_trap_entry() {
    unsafe {
        stvec::write(trap_from_kernel as usize, TrapMode::Direct);
    }
}

fn set_user_trap_entry() {
    unsafe {
        stvec::write(TRAMPOLINE as usize, TrapMode::Direct);
    }
}

pub fn enable_timer_interrupt() {
    unsafe {
        sie::set_stimer();
    }
}

pub struct GlobalSatp {
    satp: usize,
    syscall: usize,
}

// impl GlobalSatp {
//     pub fn set(&mut self, satp: usize) {
//         self.satp = satp;
//     }
//     pub fn get(&self) -> usize {
//         self.satp
//     }
//     pub fn set_syscall(&mut self, syscall_id: usize) {
//         self.syscall = syscall_id;
//     }
//     pub fn get_syscall(&self) -> usize {
//         self.syscall
//     }
// }

// use alloc::sync::Arc;
// use lazy_static::lazy_static;
// use spin::Mutex;
// lazy_static! {
//     pub static ref G_SATP: Arc<Mutex<GlobalSatp>> = Arc::new(Mutex::new(GlobalSatp {
//         satp: 0,
//         syscall: 0
//     }));
// }

#[no_mangle]
pub fn trap_handler() -> ! {
    set_kernel_trap_entry();
    //let mut is_schedule = false;
    let scause = scause::read();
    let stval = stval::read();
    match scause.cause() {
        Trap::Exception(Exception::UserEnvCall) => {
            // jump to next instruction anyway
            let mut cx = current_trap_cx();
            
            cx.sepc += 4;
            // get system call return value
            let result = syscall(
                cx.x[17],
                [cx.x[10], cx.x[11], cx.x[12], cx.x[13], cx.x[14], cx.x[15]],
            );
            
            // cx is changed during sys_exec, so we have to call it again
            cx = current_trap_cx();
           
            cx.x[10] = result as usize;
        }
        Trap::Exception(Exception::InstructionFault)
        | Trap::Exception(Exception::InstructionPageFault) => {
            let task = current_task().unwrap();
            // println!{"pinLoadFault"}
            //println!("prev syscall = {}", G_SATP.lock().get_syscall());
            /*println!(
                "[kernel] {:?} in application-{}, bad addr = {:#x}, bad instruction = {:#x}, core dumped.",
                scause.cause(),
                task.pid.0,
                stval,
                current_trap_cx().sepc,
            );
            */
            drop(task);
            // page fault exit code
            let process = current_process();
            if process.is_signal_execute() || !process.check_signal_handler(Signals::SIGSEGV) {
                drop(process);
                exit_current_and_run_next(-2);
            }
        }
        Trap::Exception(Exception::LoadFault)
        | Trap::Exception(Exception::StoreFault)
        | Trap::Exception(Exception::StorePageFault)
        | Trap::Exception(Exception::LoadPageFault) => {
            // println!("page fault 1");
            let is_load: bool;
            if scause.cause() == Trap::Exception(Exception::LoadFault)
                || scause.cause() == Trap::Exception(Exception::LoadPageFault)
            {
                is_load = true;
            } else {
                is_load = false;
            }
            let va: VirtAddr = (stval as usize).into();
            // The boundary decision
            if va > TRAMPOLINE.into() {
                panic!("VirtAddr out of range!");
            }
            //println!("check_lazy 1");
            let lazy = current_process().check_lazy(va, is_load);
            if lazy != 0 {
                // page fault exit code
                let process = current_process();
                if process.is_signal_execute() || !process.check_signal_handler(Signals::SIGSEGV) {
                    process.acquire_inner_lock().memory_set.print_pagetable();
                    println!(
                        "[kernel] {:?} in application, bad addr = {:#x}, bad instruction = {:#x}, core dumped.",
                        scause.cause(),
                        stval,
                        current_trap_cx().sepc,
                    );
                    drop(process);
                    exit_current_and_run_next(-2);
                }
            }
            unsafe {
                llvm_asm!("sfence.vma" :::: "volatile");
                llvm_asm!("fence.i" :::: "volatile");
            }
            // println!{"Trap solved..."}
        }
        Trap::Exception(Exception::IllegalInstruction) => {
            // println!{"pinIllegalInstruction"}
            println!("[kernel] IllegalInstruction in application, continue.");
            //let mut cx = current_trap_cx();
            //cx.sepc += 4;
            println!(
                "         {:?} in application, bad addr = {:#x}, bad instruction = {:#x}, core dumped.",
                scause.cause(),
                stval,
                current_trap_cx().sepc,
            );
            // illegal instruction exit code
            exit_current_and_run_next(-3);
        }
        Trap::Interrupt(Interrupt::SupervisorTimer) => {
            gdb_print!(TIMER_ENABLE, "[timer]");
            set_next_trigger();
            suspend_current_and_run_next();
            //is_schedule = true;
        }
        _ => {
            panic!(
                "Unsupported trap {:?}, stval = {:#x}!",
                scause.cause(),
                stval
            );
        }
    }
    if let Some((errno, msg)) = check_signals_of_current() {
        println!("[kernel] {}", msg);
        exit_current_and_run_next(errno);
    }
    // println!("before trap_return");
    trap_return();
}

#[no_mangle]
pub fn trap_return() -> ! {
    set_user_trap_entry();
    let trap_cx_user_va = current_trap_cx_user_va();
    let user_satp = current_user_token();
    extern "C" {
        fn __alltraps();
        fn __restore();
    }
    let restore_va = __restore as usize - __alltraps as usize + TRAMPOLINE;
    unsafe {
        asm!(
            "fence.i",
            "jr {restore_va}",
            restore_va = in(reg) restore_va,
            in("a0") trap_cx_user_va,
            in("a1") user_satp,
            options(noreturn)
        );
    }
}

#[no_mangle]
pub fn trap_from_kernel() -> ! {
    panic!(
        "a trap {:?} from kernel! Stvec:{:x}, Stval:{:X}",
        scause::read().cause(),
        stvec::read().bits(),
        stval::read()
    );
}

pub use context::TrapContext;
