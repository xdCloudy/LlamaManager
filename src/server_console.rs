use thiserror::Error;

#[derive(Debug, Error)]
pub enum ServerConsoleControlError {
    #[error("Windows console control is only available on Windows")]
    UnsupportedPlatform,

    #[error("the LlamaWave process is already attached to a console; refusing to broadcast Ctrl+C outside the managed server console")]
    CallerAlreadyAttached,

    #[error("failed to attach to managed server console for pid {pid}: {message}")]
    Attach { pid: u32, message: String },

    #[error("failed to configure temporary Ctrl+C handling: {message}")]
    Handler { message: String },

    #[error("failed to send Ctrl+C to managed server console for pid {pid}: {message}")]
    Signal { pid: u32, message: String },
}

/// Request the same graceful shutdown path that a user pressing Ctrl+C in the
/// managed `llama-server` console would trigger.
///
/// The release LlamaWave process uses the Windows GUI subsystem and therefore
/// normally owns no console. We temporarily attach to the child console, make
/// LlamaWave ignore the broadcast Ctrl+C, send the event, and detach again.
/// If LlamaWave already owns a console (for example a debug build launched from
/// a terminal), this function refuses to broadcast rather than risking an
/// interrupt to unrelated processes. The UI can then offer the explicit
/// force-kill path instead.
pub fn request_graceful_console_interrupt(pid: u32) -> Result<(), ServerConsoleControlError> {
    platform_request_graceful_console_interrupt(pid)
}

/// Best-effort product polish: when the GUI parent owns no console, hide the
/// console window belonging to the managed child after spawn. Failure to hide
/// never changes lifecycle truth and should be surfaced as a warning only.
pub fn hide_managed_console_window(pid: u32) -> Result<(), ServerConsoleControlError> {
    platform_hide_managed_console_window(pid)
}

#[cfg(windows)]
fn platform_request_graceful_console_interrupt(pid: u32) -> Result<(), ServerConsoleControlError> {
    use std::{io, ptr, thread, time::Duration};

    type Bool = i32;
    type Dword = u32;
    type HandlerRoutine = Option<unsafe extern "system" fn(Dword) -> Bool>;
    type Hwnd = *mut std::ffi::c_void;

    const CTRL_C_EVENT: Dword = 0;

    unsafe extern "system" {
        fn GetConsoleWindow() -> Hwnd;
        fn AttachConsole(process_id: Dword) -> Bool;
        fn FreeConsole() -> Bool;
        fn SetConsoleCtrlHandler(handler: HandlerRoutine, add: Bool) -> Bool;
        fn GenerateConsoleCtrlEvent(ctrl_event: Dword, process_group_id: Dword) -> Bool;
    }

    if !unsafe { GetConsoleWindow() }.is_null() {
        return Err(ServerConsoleControlError::CallerAlreadyAttached);
    }

    if unsafe { AttachConsole(pid) } == 0 {
        return Err(ServerConsoleControlError::Attach {
            pid,
            message: io::Error::last_os_error().to_string(),
        });
    }

    struct ConsoleGuard;
    impl Drop for ConsoleGuard {
        fn drop(&mut self) {
            unsafe extern "system" {
                fn FreeConsole() -> i32;
                fn SetConsoleCtrlHandler(
                    handler: Option<unsafe extern "system" fn(u32) -> i32>,
                    add: i32,
                ) -> i32;
            }
            unsafe {
                let _ = SetConsoleCtrlHandler(None, 0);
                let _ = FreeConsole();
            }
        }
    }
    let _guard = ConsoleGuard;

    if unsafe { SetConsoleCtrlHandler(None, 1) } == 0 {
        return Err(ServerConsoleControlError::Handler {
            message: io::Error::last_os_error().to_string(),
        });
    }

    if unsafe { GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0) } == 0 {
        return Err(ServerConsoleControlError::Signal {
            pid,
            message: io::Error::last_os_error().to_string(),
        });
    }

    // Give the target console control dispatcher time to invoke llama-server's
    // registered handler before detaching this GUI process from that console.
    thread::sleep(Duration::from_millis(75));
    let _ = ptr::null::<u8>(); // keeps this block free of platform-specific imports elsewhere
    Ok(())
}

#[cfg(windows)]
fn platform_hide_managed_console_window(pid: u32) -> Result<(), ServerConsoleControlError> {
    use std::io;

    type Bool = i32;
    type Hwnd = *mut std::ffi::c_void;
    const SW_HIDE: i32 = 0;

    unsafe extern "system" {
        fn GetConsoleWindow() -> Hwnd;
        fn AttachConsole(process_id: u32) -> Bool;
        fn FreeConsole() -> Bool;
        fn ShowWindow(window: Hwnd, command: i32) -> Bool;
    }

    if !unsafe { GetConsoleWindow() }.is_null() {
        return Err(ServerConsoleControlError::CallerAlreadyAttached);
    }
    if unsafe { AttachConsole(pid) } == 0 {
        return Err(ServerConsoleControlError::Attach {
            pid,
            message: io::Error::last_os_error().to_string(),
        });
    }

    let window = unsafe { GetConsoleWindow() };
    if !window.is_null() {
        unsafe {
            let _ = ShowWindow(window, SW_HIDE);
        }
    }
    unsafe {
        let _ = FreeConsole();
    }
    Ok(())
}

#[cfg(not(windows))]
fn platform_request_graceful_console_interrupt(
    _pid: u32,
) -> Result<(), ServerConsoleControlError> {
    Err(ServerConsoleControlError::UnsupportedPlatform)
}

#[cfg(not(windows))]
fn platform_hide_managed_console_window(_pid: u32) -> Result<(), ServerConsoleControlError> {
    Err(ServerConsoleControlError::UnsupportedPlatform)
}
