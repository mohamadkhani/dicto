//! Win32 message-loop plumbing shared by the Windows tray and hotkey
//! backends.
//!
//! Both backends own threads that must pump messages: the tray icon's
//! window procedure runs during message dispatch, and `RegisterHotKey`
//! delivers `WM_HOTKEY` to the queue of the thread that owns the hotkey
//! window. Hidden/message-only windows live inside the respective crates;
//! this module only pumps.

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MSG, TranslateMessage,
};

/// Wait for the next message on this thread's queue and dispatch it.
///
/// Returns `false` when the loop should stop: `GetMessageW` returned
/// `WM_QUIT` (0) or an error (-1). This thread never posts `WM_QUIT`, so in
/// practice these threads run for the lifetime of the process.
pub fn wait_and_dispatch_one() -> bool {
    let mut msg = MSG::default();
    // SAFETY: `msg` is a valid out-param; the NULL hwnd means "this
    // thread's queue", with no message filters.
    let ret = unsafe { GetMessageW(&mut msg, HWND::default(), 0, 0) };
    if ret.0 <= 0 {
        return false;
    }
    // SAFETY: `msg` was just filled by GetMessageW.
    unsafe {
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
    true
}

/// Classic GetMessage/Translate/Dispatch loop; runs until
/// [`wait_and_dispatch_one`] reports the loop should stop.
pub fn run_message_loop() {
    while wait_and_dispatch_one() {}
}

/// Spawn a named thread that runs `setup` (creating windows, icons, etc.)
/// and then pumps messages for the rest of the process lifetime.
pub fn spawn_message_thread<F>(name: &str, setup: F) -> std::io::Result<std::thread::JoinHandle<()>>
where
    F: FnOnce() + Send + 'static,
{
    std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(move || {
            setup();
            run_message_loop();
        })
}
