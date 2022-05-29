use super::TaskContext;
global_asm!(include_str!("switch.S"));

extern "C" {
    pub fn __switch(
        current_task_cx_ptr2: *mut TaskContext,
        next_task_cx_ptr2: *const TaskContext
    );
}