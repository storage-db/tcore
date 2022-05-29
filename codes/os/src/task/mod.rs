mod context;
mod id;
mod manager;
mod process;
mod processor;
mod info;
mod switch;
mod task;
mod resource;
pub use resource::RLimit;
use crate::fs::{open, OpenFlags, DiskInodeType, File};
use alloc::sync::Arc;
use lazy_static::*;
pub use manager::fetch_task;
pub use process::ProcessControlBlock;
use switch::__switch;
use lazy_static::*;
pub use context::TaskContext;
pub use id::{kstack_alloc, pid_alloc, KernelStack, PidHandle};
pub use manager::{add_task, pid2process, remove_from_pid2process};
pub use processor::*;
pub use process::*;
pub use info::{Signals,TimeVal,utsname,SIG_DFL};
pub use task::{TaskControlBlock, TaskStatus,AuxHeader};
use alloc::vec;
use alloc::vec::Vec;
use crate::mm::{UserBuffer, add_free, translated_refmut};
use crate::config::PAGE_SIZE;
use crate::utils::log2;
pub fn suspend_current_and_run_next() ->isize{
    // There must be an application running.
    let task = take_current_task().unwrap();

    // ---- access current TCB exclusively
    let mut task_inner = task.acquire_inner_lock();
    let task_cx_ptr = &mut task_inner.task_cx as *mut TaskContext;
    // Change status to Ready
    task_inner.task_status = TaskStatus::Ready;
    drop(task_inner);
    // ---- release current TCB

    // push back to ready queue.
    add_task(task);
    // jump to scheduling cycle
    schedule(task_cx_ptr );
    return 0;
}

pub fn block_current_and_run_next() {
    let task = take_current_task().unwrap();
    let mut task_inner = task.acquire_inner_lock();
    let task_cx_ptr = &mut task_inner.task_cx as *mut TaskContext;
    task_inner.task_status = TaskStatus::Blocking;
    drop(task_inner);
    schedule(task_cx_ptr );
}

pub fn exit_current_and_run_next(exit_code: i32) {
    let task = take_current_task().unwrap();
    let mut task_inner = task.acquire_inner_lock();
    let process = task.process.upgrade().unwrap();
    let tid = task_inner.res.as_ref().unwrap().tid;
    // record exit code
    task_inner.exit_code = Some(exit_code);
    task_inner.res = None;
    // here we do not remove the thread since we are still using the kstack
    // it will be deallocated when sys_waittid is called
    drop(task_inner);
    drop(task);
    // however, if this is the main thread of current process
    // the process should terminate at once
    if tid == 0 {
        remove_from_pid2process(process.getpid());
        let mut process_inner = process.acquire_inner_lock();
        // mark this process as a zombie process
        process_inner.is_zombie = true;
        // record exit code of main process
        process_inner.exit_code = exit_code;

        {
            // move all child processes under init process
            let mut initproc_inner = INITPROC.acquire_inner_lock();
            for child in process_inner.children.iter() {
                child.acquire_inner_lock().parent = Some(Arc::downgrade(&INITPROC));
                initproc_inner.children.push(child.clone());
            }
        }

        // deallocate user res (including tid/trap_cx/ustack) of all threads
        // it has to be done before we dealloc the whole memory_set
        // otherwise they will be deallocated twice
        for task in process_inner.tasks.iter().filter(|t| t.is_some()) {
            let task = task.as_ref().unwrap();
            let mut task_inner = task.acquire_inner_lock();
            task_inner.res = None;
        }

        process_inner.children.clear();
        // deallocate other data in user space i.e. program code/data section
        process_inner.memory_set.recycle_data_pages();
        // drop file descriptors
        process_inner.fd_table.clear();
    }
    drop(process);
    // we do not have to save task context
    let mut _unused = TaskContext::zero_init();
    schedule(&mut _unused as *mut _);
}

lazy_static! {
    pub static ref INITPROC: Arc<ProcessControlBlock> = {
        let inode = open("/","initproc", OpenFlags::RDONLY, DiskInodeType::File).unwrap();
        let v = inode.read_all();
        ProcessControlBlock::new(v.as_slice())
    };
}


// Write initproc & user_shell into file system to be executed
// And then release them to fram_allocator
pub fn add_initproc_into_fs() {
    extern "C" { fn _num_app(); }
    extern "C" { fn _app_names(); }
    let mut num_app_ptr = _num_app as usize as *mut usize;
    // let start = _app_names as usize as *const u8;
    let mut app_start = unsafe {
        core::slice::from_raw_parts_mut(num_app_ptr.add(1), 3)
    };

    open(
        "/",
        "mnt",
        OpenFlags::CREATE,
        DiskInodeType::Directory
    );

    // find if there already exits 
    // println!("Find if there already exits ");
    if let Some(inode) = open(
        "/",
        "initproc",
        OpenFlags::RDONLY,
        DiskInodeType::File
    ){
        println!("Already have init proc in FS");
        //return;
        inode.delete();
    }

    if let Some(inode) = open(
        "/",
        "user_shell",
        OpenFlags::RDONLY,
        DiskInodeType::File
    ){
        println!("Already have init proc in FS");
        //return;
        inode.delete();
    }


    // println!("Write apps(initproc & user_shell) to disk from mem ");

    //Write apps(initproc & user_shell) to disk from mem
    if let Some(inode) = open(
        "/",
        "initproc",
        OpenFlags::CREATE,
        DiskInodeType::File
    ){
        // println!("Create initproc ");
        let mut data: Vec<&'static mut [u8]> = Vec::new();
        data.push( unsafe{
        core::slice::from_raw_parts_mut(
            app_start[0] as *mut u8,
            app_start[1] - app_start[0]
        )}) ;
        // println!("Start write initproc ");
        inode.write(UserBuffer::new(data));
        // println!("Init_proc OK");
    }
    else{
        // panic!("initproc create fail!");
    }

    if let Some(inode) = open(
        "/",
        "user_shell",
        OpenFlags::CREATE,
        DiskInodeType::File
    ){
        //println!("Create user_shell ");
        let mut data:Vec<&'static mut [u8]> = Vec::new();
        data.push(unsafe{
        core::slice::from_raw_parts_mut(
            app_start[1] as *mut u8,
            app_start[2] - app_start[1]
        )});
        //data.extend_from_slice(  )
        // println!("Start write user_shell ");
        inode.write(UserBuffer::new(data));
        // println!("User_shell OK");
    }
    else{
        panic!("user_shell create fail!");
    }
    println!("Write apps(initproc & user_shell) to disk from mem");


    // release
    let mut start_ppn = app_start[0] / PAGE_SIZE + 1;
    println!("Recycle memory: {:x}-{:x}", start_ppn* PAGE_SIZE, (app_start[2] / PAGE_SIZE)* PAGE_SIZE);
    while start_ppn < app_start[2] / PAGE_SIZE {
        add_free(start_ppn);
        start_ppn += 1;
    }

}
pub fn add_initproc() {
    add_initproc_into_fs();
    let _initproc = INITPROC.clone();
}

pub fn check_signals_of_current() -> Option<(i32, &'static str)> {
    let process = current_process();
    let process_inner = process.acquire_inner_lock();
    process_inner.signals.check_error()
}

pub fn current_add_signal(signal: Signals) {
    let process = current_process();
    let mut process_inner = process.acquire_inner_lock();
    process_inner.signals |= signal;
}
