use crate::sbi::shutdown;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    if let Some(location) = info.location() {
        crate::println!(
            "[kernel] panicked at {}:{}: {}",
            location.file(),
            location.line(),
            info.message()
        );
    } else {
        crate::println!("[kernel] panicked: {}", info.message());
    }
    shutdown(true)
}
