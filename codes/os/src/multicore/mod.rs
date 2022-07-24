#[inline(always)]
pub fn get_hartid() -> usize {
    let mut hartid;
    unsafe {
        llvm_asm!("mv $0, tp" : "=r"(hartid));
    }
    hartid
}

pub fn save_hartid() {
    unsafe {
        // core::arch::asm!("mv tp, x10", in("x10") hartid);
        llvm_asm!("mv tp, a0");
    }
}

#[inline]
pub fn id() -> usize {
    let cpu_id;
    unsafe {
        llvm_asm!("mv $0, tp" : "=r"(cpu_id));
    }
    cpu_id
}