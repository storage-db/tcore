// #![feature(llvm_asm)]
// #[macro_use]
use super::__switch;
use super::{fetch_task, Signals, TaskStatus};
use super::{ProcessControlBlock, TaskContext, TaskControlBlock};
use crate::gdb_print;
use crate::monitor::*;
use crate::task::manager::add_task;
use crate::timer::get_time_us;
use crate::trap::TrapContext;
use alloc::sync::Arc;
use core::{borrow::Borrow, cell::RefCell, cell::RefMut};
use lazy_static::*;
pub fn get_core_id() -> usize{
    let tp:usize;
    unsafe {
        llvm_asm!("mv $0, tp" : "=r"(tp));
    }
    // tp
    0
    
}

pub struct Processor {
    inner: RefCell<ProcessorInner>,
}

pub struct ProcessorInner {
    current: Option<Arc<TaskControlBlock>>,
    idle_task_cx: TaskContext,
    user_clock: usize,   /* Timer usec when last enter into the user program */
    kernel_clock: usize, /* Timer usec when user program traps into the kernel*/
}

unsafe impl Sync for Processor {}

impl Processor {
    pub fn new() -> Self {
        Self {
            inner: RefCell::new(ProcessorInner {
                current: None,
                idle_task_cx: TaskContext::zero_init(),
                user_clock: 0,
                kernel_clock: 0,
            }),
        }
    }
    pub fn acquire_inner_lock(&self) -> RefMut<'_, ProcessorInner> {
        self.inner.borrow_mut()
    }
}

impl ProcessorInner {
    // when trap return to user program, use this func to update user clock
    pub fn update_user_clock(&mut self) {
        self.user_clock = get_time_us();
    }

    // when trap into kernel, use this func to update kernel clock
    pub fn update_kernel_clock(&mut self) {
        self.kernel_clock = get_time_us();
    }

    pub fn get_user_clock(&mut self) -> usize {
        return self.user_clock;
    }

    pub fn get_kernel_clock(&mut self) -> usize {
        return self.kernel_clock;
    }

    fn get_idle_task_cx_ptr(&mut self) -> *mut TaskContext {
        &mut self.idle_task_cx as *mut _
    }

    pub fn take_current(&mut self) -> Option<Arc<TaskControlBlock>> {
        self.current.take()
    }

    pub fn current(&self) -> Option<Arc<TaskControlBlock>> {
        self.current.as_ref().map(Arc::clone)
    }
}

lazy_static! {
    pub static ref PROCESSOR_LIST: [Processor; 2] = [Processor::new(), Processor::new()];
}

pub fn run_tasks() {
    loop {
        let core_id = get_core_id();
        let mut processor = PROCESSOR_LIST[core_id].acquire_inner_lock();
        if let Some(task) = fetch_task() {
            let idle_task_cx_ptr = processor.get_idle_task_cx_ptr();
            // access coming task TCB exclusively
            let mut task_inner = task.acquire_inner_lock();
            let next_task_cx_ptr = &task_inner.task_cx as *const TaskContext;
            task_inner.task_status = TaskStatus::Running;
            drop(task_inner);
            // release coming task TCB manually
            // println!("[cpu {}] switch to process {}", get_hartid(), task.process.upgrade().unwrap().pid.0);
            processor.current = Some(task);
            // release processor manually
            drop(processor);
            unsafe {
                __switch(idle_task_cx_ptr, next_task_cx_ptr);
            }
        }else {
            println!("no tasks available in run_tasks");
        }
    }
}

pub fn take_current_task() -> Option<Arc<TaskControlBlock>> {
    let core_id: usize = get_core_id();
    PROCESSOR_LIST[core_id].acquire_inner_lock().take_current()
}

pub fn current_task() -> Option<Arc<TaskControlBlock>> {
    let core_id: usize = get_core_id();
    PROCESSOR_LIST[core_id].acquire_inner_lock().current()
}
pub fn current_process() -> Arc<ProcessControlBlock> {
    current_task().unwrap().process.upgrade().unwrap()
}

pub fn current_user_token() -> usize {
    let task = current_task().unwrap();
    task.get_user_token()
}

pub fn current_trap_cx() -> &'static mut TrapContext {
    current_task().unwrap().acquire_inner_lock().get_trap_cx()
}

pub fn current_trap_cx_user_va () -> usize {
    current_task()
        .unwrap()
        .acquire_inner_lock()
        .res
        .as_ref()
        .unwrap()
        .trap_cx_user_va()
}

pub fn current_kstack_top() -> usize {
    current_task().unwrap().kstack.get_top()
}
// when trap return to user program, use this func to update user clock
pub fn update_user_clock() {
    let core_id: usize = get_core_id();
    PROCESSOR_LIST[core_id]
        .acquire_inner_lock()
        .update_user_clock();
}

// when trap into kernel, use this func to update kernel clock
pub fn update_kernel_clock() {
    let core_id: usize = get_core_id();
    PROCESSOR_LIST[core_id]
        .acquire_inner_lock()
        .update_kernel_clock();
}

// when trap into kernel, use this func to get time spent in user (it is duration not accurate time)
pub fn get_user_runtime_usec() -> usize {
    let core_id: usize = get_core_id();
    return get_time_us()
        - PROCESSOR_LIST[core_id]
            .acquire_inner_lock()
            .get_user_clock();
}

// when trap return to user program, use this func to get time spent in kernel (it is duration not accurate time)
pub fn get_kernel_runtime_usec() -> usize {
    let core_id: usize = get_core_id();
    return get_time_us()
        - PROCESSOR_LIST[core_id]
            .acquire_inner_lock()
            .get_kernel_clock();
}

pub fn schedule(switched_task_cx_ptr: *mut TaskContext) {
    let mut processor = PROCESSOR_LIST[get_core_id()].acquire_inner_lock();
    let idle_task_cx_ptr = processor.get_idle_task_cx_ptr();
    drop(processor);
    unsafe {
        __switch(switched_task_cx_ptr, idle_task_cx_ptr);
    }
}
