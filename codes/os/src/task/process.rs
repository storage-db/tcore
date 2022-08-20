use super::id::RecycleAllocator;
use super::info::*;
use super::manager::insert_into_pid2process;
use super::TaskControlBlock;
use super::{add_task, Signals};
use super::{pid_alloc, KernelStack, PidHandle};
use crate::config::*;
use crate::fs::{FileClass, FileDescripter, Stdin, Stdout};
use crate::mm::{
    translated_refmut, MapPermission, MemorySet, MmapArea, PageTableEntry, PhysPageNum, VirtAddr,
    VirtPageNum, KERNEL_SPACE,
};
use crate::syscall::FD_LIMIT;
use crate::task::log2;
use crate::trap::{trap_handler, TrapContext};
use alloc::string::String;
use alloc::sync::{Arc, Weak};
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefMut;
use spin::{Mutex, MutexGuard};
pub struct ProcessControlBlock {
    //immutable
    pub pid: PidHandle,
    //mutable
    inner:  Arc<Mutex<ProcessControlBlockInner>>,
}

pub type FdTable = Vec<Option<FileDescripter>>;

pub struct ProcessControlBlockInner {
    pub is_zombie: bool,
    pub memory_set: MemorySet,
    pub base_size: usize,
    pub heap_start: usize,
    pub heap_pt: usize,
    pub mmap_area: MmapArea,
    pub parent: Option<Weak<ProcessControlBlock>>,
    pub children: Vec<Arc<ProcessControlBlock>>,
    pub exit_code: i32,
    pub fd_table: FdTable,
    pub signals: Signals,
    pub siginfo: SigInfo,
    pub tasks: Vec<Option<Arc<TaskControlBlock>>>,
    pub task_res_allocator: RecycleAllocator,
    pub current_path: String,
}

impl ProcessControlBlockInner {
    #[allow(unused)]
    pub fn get_user_token(&self) -> usize {
        self.memory_set.token()
    }

    pub fn alloc_fd(&mut self) -> usize {
        if let Some(fd) = (0..self.fd_table.len()).find(|fd| self.fd_table[*fd].is_none()) {
            fd
        } else {
            self.fd_table.push(None);
            self.fd_table.len() - 1
        }
    }

    pub fn alloc_tid(&mut self) -> usize {
        self.task_res_allocator.alloc()
    }

    pub fn dealloc_tid(&mut self, tid: usize) {
        self.task_res_allocator.dealloc(tid)
    }

    pub fn thread_count(&self) -> usize {
        self.tasks.len()
    }

    pub fn get_task(&self, tid: usize) -> Arc<TaskControlBlock> {
        self.tasks[tid].as_ref().unwrap().clone()
    }

    pub fn get_work_path(&self) -> String {
        self.current_path.clone()
    }

    pub fn enquire_vpn(&self, vpn: VirtPageNum) -> Option<PageTableEntry> {
        self.memory_set.translate(vpn)
    }
    pub fn cow_alloc(&mut self, vpn: VirtPageNum, former_ppn: PhysPageNum) -> usize {
        let ret = self.memory_set.cow_alloc(vpn, former_ppn);
        // println!{"finished cow_alloc!"}
        ret
    }
    pub fn lazy_alloc_heap(&mut self, vpn: VirtPageNum) -> usize {
        self.memory_set.lazy_alloc_heap(vpn)
    }
    pub fn lazy_alloc_stack(&mut self, vpn: VirtPageNum) -> usize {
        self.memory_set.lazy_alloc_stack(vpn)
    }
}

impl ProcessControlBlock {
    pub fn getpid(&self) -> usize {
        self.pid.0
    }
    pub fn acquire_inner_lock(&self) -> MutexGuard<'_, ProcessControlBlockInner>{
        self.inner.lock()
    }
    pub fn new(elf_data: &[u8]) -> Arc<Self> {
        //memory_set with elf program headers/trampoline/trap context/user stack
        let (memory_set, user_sp, user_heap, entry_point, auxv) = MemorySet::from_elf(elf_data);
        //alloc  a pid
        let pid_handle = pid_alloc();
        //let kernel_stack = KernelStack::new(&pid_handle);
        //let kernel_stack_top = kernel_stack.get_top();
        let process = Arc::new(Self {
            pid: pid_handle,
            inner: Arc::new(Mutex::new(ProcessControlBlockInner {
                is_zombie: false,
                memory_set,
                parent: None,
                children: Vec::new(),
                exit_code: 0,
                fd_table: vec![
                    // 0 -> stdin
                    Some(FileDescripter::new(
                        false,
                        FileClass::Abstr(Arc::new(Stdin)),
                    )),
                    // 1 -> stdout
                    Some(FileDescripter::new(
                        false,
                        FileClass::Abstr(Arc::new(Stdout)),
                    )),
                    // 2 -> stderr
                    Some(FileDescripter::new(
                        false,
                        FileClass::Abstr(Arc::new(Stdout)),
                    )),
                ],
                mmap_area: MmapArea::new(VirtAddr::from(MMAP_BASE), VirtAddr::from(MMAP_BASE)),
                base_size: user_sp,
                heap_start: user_heap,
                heap_pt: user_heap,
                current_path: String::from("/"), // 只有initproc在此建立，其他进程均为fork出
                //resource_list: [RLimit::new();17],
                signals: Signals::empty(),
                siginfo: SigInfo::new(),
                tasks: Vec::new(),
                task_res_allocator: RecycleAllocator::new(),
            })),
        });
        //create a main thread ,we should alloc ustack and trap_cx header
        let task = Arc::new(TaskControlBlock::new(Arc::clone(&process), user_sp, true));
        //prepare trap_cx of main thread
        let task_inner = task.acquire_inner_lock();
        let trap_cx = task_inner.get_trap_cx();
        let ustack_top = task_inner.res.as_ref().unwrap().ustack_top();
        let kstack_top = task.kstack.get_top();
        drop(task_inner);
        *trap_cx = TrapContext::app_init_context(
            entry_point,
            ustack_top,
            KERNEL_SPACE.lock().token(),
            kstack_top,
            trap_handler as usize,
        );
        // add main thread to the process
        let mut process_inner = process.acquire_inner_lock();
        process_inner.tasks.push(Some(Arc::clone(&task)));
        drop(process_inner);
        insert_into_pid2process(process.getpid(), Arc::clone(&process));
        // add main thread to scheduler
        add_task(task);
        process
    }

    pub fn exec(self: &Arc<Self>, elf_data: &[u8], args: Vec<String>) {
        assert_eq!(self.acquire_inner_lock().thread_count(), 1);
        //memory_set with elf program headers/trampoline/trap context/user stack
        let (memory_set, mut user_sp, user_heap, entry_point, _) = MemorySet::from_elf(elf_data);
        let new_token = memory_set.token();
        //substitute memory_set
        self.acquire_inner_lock().memory_set = memory_set;
        // then we alloc user resource for main thread again
        // since memory_set has been changed
        let task = self.acquire_inner_lock().get_task(0);
        let mut task_inner = task.acquire_inner_lock();
        task_inner.res.as_mut().unwrap().ustack_base = user_sp;
        task_inner.res.as_mut().unwrap().alloc_user_res();
        task_inner.trap_cx_ppn = task_inner.res.as_mut().unwrap().trap_cx_ppn();
        // push arguments on user stack
        let mut user_sp = task_inner.res.as_mut().unwrap().ustack_top();
        user_sp -= (args.len() + 1) * core::mem::size_of::<usize>();
        let argv_base = user_sp;
        let mut argv: Vec<_> = (0..=args.len())
            .map(|arg| {
                translated_refmut(
                    new_token,
                    (argv_base + arg * core::mem::size_of::<usize>()) as *mut usize,
                )
            })
            .collect();
        *argv[args.len()] = 0;
        for i in 0..args.len() {
            user_sp -= args[i].len() + 1;
            *argv[i] = user_sp;
            let mut p = user_sp;
            for c in args[i].as_bytes() {
                *translated_refmut(new_token, p as *mut u8) = *c;
                p += 1;
            }
            *translated_refmut(new_token, p as *mut u8) = 0;
        }
        // make the user_sp aligned to 8B for k210 platform
        user_sp -= user_sp % core::mem::size_of::<usize>();
        // initialize trap_cx
        let mut trap_cx = TrapContext::app_init_context(
            entry_point,
            user_sp,
            KERNEL_SPACE.lock().token(),
            task.kstack.get_top(),
            trap_handler as usize,
        );
        trap_cx.x[10] = args.len();
        trap_cx.x[11] = argv_base;
        *task_inner.get_trap_cx() = trap_cx;
    }

    /// Only support processes with a single thread.
    pub fn fork(self: &Arc<Self>,is_created_thread :bool) -> Arc<Self> {
        let mut parent = self.acquire_inner_lock();
        assert_eq!(parent.thread_count(), 1);
        // clone parent's memory_set completely including trampoline/ustacks/trap_cxs
        let memory_set = MemorySet::from_copy_on_write(&mut parent.memory_set);
        // alloc a pid
        let pid = pid_alloc();
        
        // copy fd table
        let mut new_fd_table: FdTable = Vec::new();
        for fd in parent.fd_table.iter() {
            if let Some(file) = fd {
                new_fd_table.push(Some(file.clone()));
            } else {
                new_fd_table.push(None);
            }
        }
        // create child process pcb
        let child = Arc::new(Self {
            pid,
            inner: Arc::new(Mutex::new(ProcessControlBlockInner {
                is_zombie: false,
                memory_set,
                parent: Some(Arc::downgrade(self)),
                children: Vec::new(),
                exit_code: 0,
                mmap_area: MmapArea::new(VirtAddr::from(MMAP_BASE), VirtAddr::from(MMAP_BASE)),
                base_size: parent.base_size,
                heap_start: parent.heap_start,
                heap_pt: parent.heap_pt,
                fd_table: new_fd_table,
                current_path: parent.current_path.clone(),
                //resource_list: [RLimit::new();17],
                signals: Signals::empty(),
                siginfo: SigInfo::new(),
                tasks: Vec::new(),
                task_res_allocator: RecycleAllocator::new(),
            })),
        });
        // add child
        parent.children.push(Arc::clone(&child));
        // create main thread of child process
        let task = Arc::new(TaskControlBlock::new(
            Arc::clone(&child),
            parent
                .get_task(0)
                .acquire_inner_lock()
                .res
                .as_ref()
                .unwrap()
                .ustack_base(),
            // here we do not allocate trap_cx or ustack again
            // but mention that we allocate a new kstack here
            false,
        ));
        // attach task to child process
        let mut child_inner = child.acquire_inner_lock();
        child_inner.tasks.push(Some(Arc::clone(&task)));
        drop(child_inner);
        // modify kstack_top in trap_cx of this thread
        let task_inner = task.acquire_inner_lock();
        let trap_cx = task_inner.get_trap_cx();
        trap_cx.kernel_sp = task.kstack.get_top();
        drop(task_inner);
        insert_into_pid2process(child.getpid(), Arc::clone(&child));
        // add this thread to scheduler
        add_task(task);
        child
    }
    pub fn is_signal_execute(&self) -> bool {
        return self.acquire_inner_lock().siginfo.is_signal_execute;
    }
    pub fn check_signal_handler(&self, signal: Signals) -> bool {
        let mut inner = self.acquire_inner_lock();
        let signal_handler = inner.siginfo.signal_handler.clone();
        if let Some(sigaction) = signal_handler.get(&signal) {
            if sigaction.sa_handler == 0 {
                return false;
            }
            {
                // // avoid borrow mut trap_cx, because we need to modify trapcx_backup
                // let trap_cx = inner.get_trap_cx().clone();
                // inner.trapcx_backup = trap_cx; // backup
            }
            {
                // let trap_cx = inner.get_trap_cx();
                // trap_cx.set_sp(USER_SIGNAL_STACK); // sp-> signal_stack
                // trap_cx.x[10] = log2(signal.bits()); // a0=signum
                // trap_cx.x[1] = SIGNAL_TRAMPOLINE; // ra-> signal_trampoline
                // trap_cx.sepc = sigaction.sa_handler; // sepc-> sa_handler
            }
            inner.siginfo.is_signal_execute = true;
            true
        } else {
            false
        }
    }
    pub fn check_lazy(&self, va: VirtAddr, is_load: bool) -> isize {
        let vpn: VirtPageNum = va.floor();
        //unsafe {

        //let heap_base = self.acquire_inner_lock().heap_start;
        //let heap_pt = self.acquire_inner_lock().heap_pt;
        //let stack_top = self.acquire_inner_lock().base_size;
        //let stack_bottom = stack_top - USER_STACK_SIZE;
        //let mmap_start = self.acquire_inner_lock().mmap_area.mmap_start;
        //let mmap_end = self.acquire_inner_lock().mmap_area.mmap_top;

        //if self.inner.is_locked() {
        //    self.inner.force_unlock();
        //}
        let heap_base = self.acquire_inner_lock().heap_start;
        let heap_pt = self.acquire_inner_lock().heap_pt;
        let stack_top = self.acquire_inner_lock().base_size;
        let stack_bottom = stack_top - USER_STACK_SIZE;
        let mmap_start = self.acquire_inner_lock().mmap_area.mmap_start;
        let mmap_end = self.acquire_inner_lock().mmap_area.mmap_top;

        //println!("get the lock successfully");
        if va >= mmap_start && va < mmap_end {
            // if false { // disable lazy mmap
            //println!("lazy mmap");
            self.lazy_mmap(va.0, is_load)
        } else if va.0 >= heap_base && va.0 <= heap_pt {
            self.acquire_inner_lock().lazy_alloc_heap(vpn);
            return 0;
        } else if va.0 >= stack_bottom && va.0 <= stack_top {
            //println!{"lazy_stack_page: {:?}", va}
            self.acquire_inner_lock().lazy_alloc_stack(vpn);
            0
        } else {
            // get the PageTableEntry that faults
            let pte = self.acquire_inner_lock().enquire_vpn(vpn);
            // if the virtPage is a CoW
            if pte.is_some() && pte.unwrap().is_cow() {
                let former_ppn = pte.unwrap().ppn();
                self.acquire_inner_lock().cow_alloc(vpn, former_ppn);
                0
            } else {
                -1
            }
        }
        //}
    }
    pub fn scan_signal_handler(&self) -> Option<(Signals, usize)> {
        let mut inner = self.acquire_inner_lock();
        let signal_handler = inner.siginfo.signal_handler.clone();
        while !inner.siginfo.signal_pending.is_empty() {
            let signum = inner.siginfo.signal_pending.pop().unwrap();
            if let Some(sigaction) = signal_handler.get(&signum) {
                if sigaction.sa_handler == 0 {
                    continue;
                }
                {
                    // // avoid borrow mut trap_cx, because we need to modify trapcx_backup
                    // let trap_cx = inner.get_trap_cx().clone();
                    // inner.trapcx_backup = trap_cx; // backup
                }
                {
                    // let trap_cx = inner.get_trap_cx();
                    // trap_cx.set_sp(USER_SIGNAL_STACK); // sp-> signal_stack
                    // trap_cx.x[10] = log2(signum.bits()); // a0=signum
                    // trap_cx.x[1] = SIGNAL_TRAMPOLINE; // ra-> signal_trampoline
                    // trap_cx.sepc = sigaction.sa_handler; // sepc-> sa_handler
                    //gdb_println!(SIGNAL_ENABLE, " --- {:?} (si_signo={:?}, si_code=UNKNOWN, si_addr=0x{:X})", signum, signum, sigaction.sa_handler);
                }
                inner.siginfo.is_signal_execute = true;
                return Some((signum, sigaction.sa_handler));
            } else {
                // check SIGTERM independently
                if signum == Signals::SIGTERM || signum == Signals::SIGKILL {
                    //gdb_println!(SIGNAL_ENABLE, " --- {:?} (si_signo={:?}, si_code=UNKNOWN, si_addr=SIG_DFL)", signum, signum);
                    return Some((signum, SIG_DFL));
                }
            }
        }
        None
    }
    pub fn mmap(
        &self,
        start: usize,
        len: usize,
        prot: usize,
        flags: usize,
        fd: isize,
        off: usize,
    ) -> usize {
        // gdb_println!(SYSCALL_ENABLE,"[mmap](0x{:X},{},{},0x{:X},{},{})",start, len, prot, flags, fd, off);

        if start % PAGE_SIZE != 0 {
            panic!("mmap: start_va not aligned");
        }
        let mut inner = self.acquire_inner_lock();
        let fd_table = inner.fd_table.clone();
        let token = inner.get_user_token();
        let mut va_top = inner.mmap_area.get_mmap_top();
        let mut end_va = VirtAddr::from(va_top.0 + len);
        // "prot<<1" is equal to  meaning of "MapPermission"
        let map_flags = (((prot & 0b111) << 1) + (1 << 4)) as u8; // "1<<4" means user

        let mut startvpn = start / PAGE_SIZE;

        if start != 0 {
            // "Start" va Already mapped
            while startvpn < (start + len) / PAGE_SIZE {
                if inner
                    .memory_set
                    .set_pte_flags(startvpn.into(), map_flags as usize)
                    == -1
                {
                    panic!("mmap: start_va not mmaped");
                }
                startvpn += 1;
            }
            return start;
        } else {
            // "Start" va not mapped
            //inner.memory_set.insert_kernel_mmap_area(va_top, end_va, MapPermission::from_bits(map_flags).unwrap());
            inner.memory_set.insert_mmap_area(
                va_top,
                end_va,
                MapPermission::from_bits(map_flags).unwrap(),
            );
            //inner.mmap_area.push_kernel(va_top.0, len, prot, flags, fd, off, fd_table, token);
            inner
                .mmap_area
                .push(va_top.0, len, prot, flags, fd, off, fd_table, token);
            va_top.0
        }
    }
    pub fn munmap(&self, start: usize, len: usize) -> isize {
        let mut inner = self.acquire_inner_lock();
        let start_vpn = VirtAddr::from(start).floor();
        inner
            .memory_set
            .remove_area_with_start_vpn(VirtAddr::from(start_vpn).into());
        inner.mmap_area.remove(start, len)
    }
    pub fn lazy_mmap(&self, stval: usize, is_load: bool) -> isize {
        // println!("lazy_mmap");
        let mut inner = self.acquire_inner_lock();
        let fd_table = inner.fd_table.clone();
        let token = inner.get_user_token();
        let lazy_result = inner.memory_set.lazy_mmap(stval.into());

        if lazy_result == 0 || is_load {
            inner.mmap_area.lazy_map_page(stval, fd_table, token);
        }
        // println!("lazy_mmap");
        return lazy_result;
    }
    pub fn grow_proc(&self,grow_size:isize) -> usize {
        if grow_size > 0 {
            let growed_addr: usize = self.inner.lock().heap_pt + grow_size as usize;
            let limit = self.inner.lock().heap_start + USER_HEAP_SIZE;
            if growed_addr > limit {
                panic!("process doesn't have enough ");
            }
            self.inner.lock().heap_pt = growed_addr;
        }
        else {
            let shrinked_addr: usize = self.inner.lock().heap_pt + grow_size as usize;
            if shrinked_addr < self.inner.lock().heap_start {
                panic!("Memory shrinked");
            }
            self.inner.lock().heap_pt = shrinked_addr;
        }
        return self.inner.lock().heap_pt;
    }
}
