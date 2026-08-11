//! hok-shim — console-subsystem shim variant.
//!
//! Selected by libscoop for **console** targets (git, python, …): the
//! invoking shell waits for the shim, so interactive children keep working
//! (stdin is not pre-empted by the shell prompt).
//!
//! All logic lives in the `hok_shim` library crate; this binary only wires
//! up the PE entry points and the panic handler. `windows_subsystem` is
//! declared explicitly (rust-lld cannot infer it from the custom entry point
//! under `#![no_main]`).

#![no_std]
#![cfg_attr(not(test), no_main)]
#![windows_subsystem = "console"]
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
