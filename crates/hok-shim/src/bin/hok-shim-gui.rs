//! hok-shim-gui — GUI-subsystem shim variant.
//!
//! Selected by libscoop for **GUI** targets (vscode, notepad, …): no console
//! window appears on double-click, and the invoking shell does not wait —
//! same UX as running the GUI program directly.
//!
//! All logic lives in the `hok_shim` library crate; this binary only wires
//! up the PE entry points and the panic handler, and sets the GUI subsystem.

#![no_std]
#![cfg_attr(not(test), no_main)]
#![windows_subsystem = "windows"]
#![cfg_attr(test, allow(dead_code))]

#[cfg(test)]
extern crate std;

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    unsafe {
        hok_shim::write_stderr(b"shim: panic\n");
    }
    loop {}
}

#[cfg(not(test))]
#[no_mangle]
pub extern "system" fn mainCRTStartup() -> ! {
    let code = unsafe { hok_shim::entry() };
    hok_shim::exit_process(code)
}
