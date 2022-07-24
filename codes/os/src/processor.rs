use core::cell::{RefCell, RefMut};

use super::{__switch, add_task};
use super::{fetch_task, TaskStatus};
use super::{ProcessControlBlock, TaskContext, ProcessControlBlock};

use crate::config::MAX_CPU_NUM;
use crate::multicore::get_hartid;
use crate::trap::TrapContext;
use alloc::sync::Arc;
use lazy_static::*;

pub struct Processor {
    inner: RefCell<ProcessorInner>,
}

pub struct ProcessorInner {
    current: Option<Arc<ProcessControlBlock>>,
    idle_task_cx: TaskContext,
}

unsafe impl Sync for Processor {}

impl Processor {
    pub fn new() -> Self {
        Self {
            inner: RefCell::new(ProcessorInner {
                current: None,
                idle_task_cx: TaskContext::zero_init(),
            }),
        }
    }
    pub fn inner_exclusive_access(&self) -> RefMut<'_, ProcessorInner> {
        self.inner.borrow_mut()
    }
}

impl ProcessorInner {
    fn get_idle_task_cx_ptr(&mut self) -> *mut TaskContext {
        &mut self.idle_task_cx as *mut _
    }

    pub fn take_current(&mut self) -> Option<Arc<ProcessControlBlock>> {
        self.current.take()
    }

    pub fn current(&self) -> Option<Arc<ProcessControlBlock>> {
        self.current.as_ref().map(Arc::clone)
    }
}

lazy_static! {
    pub static ref PROCESSORS: [Processor; MAX_CPU_NUM] = [
        Processor::new(),
        Processor::new(),
        Processor::new(),
        Processor::new()
    ];
}

pub fn run_tasks() {
    loop {
        let mut processor = PROCESSORS[get_hartid()].inner_exclusive_access();

        // 本来下面这段代码应该由suspend_current_and_run_next完成
        // 但是若如此做，则内核栈会被其他核“趁虚而入”
        // 将suspend_current_and_run_next中的add_task延后到调度完成后
        if let Some(last_task) = processor.take_current() {
            add_task(last_task);
        }
        if let Some(task) = fetch_task() {
            //println!("core {} is fetching task",get_hartid());
            let idle_task_cx_ptr = processor.get_idle_task_cx_ptr();
            // access coming task TCB exclusively
            let mut task_inner = task.inner_exclusive_access();
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
        }
    }
}

pub fn take_current_task() -> Option<Arc<ProcessControlBlock>> {
    PROCESSORS[get_hartid()]
        .inner_exclusive_access()
        .take_current()
}

pub fn current_task() -> Option<Arc<ProcessControlBlock>> {
    PROCESSORS[get_hartid()].inner_exclusive_access().current()
}

pub fn current_process() -> Arc<ProcessControlBlock> {
    current_task().unwrap().process.upgrade().unwrap()
}

pub fn current_user_token() -> usize {
    let task = current_task().unwrap();
    task.get_user_token()
}

pub fn current_trap_cx() -> &'static mut TrapContext {
    current_task()
        .unwrap()
        .inner_exclusive_access()
        .get_trap_cx()
}

pub fn current_trap_cx_user_va() -> usize {
    current_task()
        .unwrap()
        .inner_exclusive_access()
        .res
        .as_ref()
        .unwrap()
        .trap_cx_user_va()
}

pub fn current_kstack_top() -> Option<usize> {
    // backtrace时一些核心可能没有current_task
    if let Some(task) = current_task() {
        Some(task.kstack.get_top())
    } else {
        None
    }
}

pub fn schedule(switched_task_cx_ptr: *mut TaskContext) {
    let mut processor = PROCESSORS[get_hartid()].inner_exclusive_access();
    let idle_task_cx_ptr = processor.get_idle_task_cx_ptr();
    drop(processor);
    unsafe {
        __switch(switched_task_cx_ptr, idle_task_cx_ptr);
    }
}
