#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

#[unsafe(no_mangle)]
extern "C" fn main() -> i32 {
    println!("Drawing part 3 of tangram 'OS'...");
    unsafe { 
        // 唤起系统调用 SYS_DRAW: 233, id: 寄存器 a0
        core::arch::asm!("ecall", in("a7") 233, in("a0") 3); 
    }
    // 插入大运算量的 busy-loop 来构造肉眼可见的拼接演示延迟！(无需 sleep 系统调用)
    for _ in 0..2000 { 
        core::hint::black_box(0); 
    }
    0
}
