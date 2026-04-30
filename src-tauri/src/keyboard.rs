use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc;

static ENABLED: AtomicBool = AtomicBool::new(false);
static TARGET_SCAN_CODE: AtomicU32 = AtomicU32::new(0);
static TARGET_IS_DOWN: AtomicBool = AtomicBool::new(false);
static STARTED: AtomicBool = AtomicBool::new(false);
static TRIGGER_SENDER: Lazy<Mutex<Option<mpsc::Sender<()>>>> = Lazy::new(|| Mutex::new(None));

pub fn configure(enabled: bool, scan_code: u32) {
    TARGET_SCAN_CODE.store(scan_code, Ordering::Relaxed);
    ENABLED.store(enabled && scan_code != 0, Ordering::Relaxed);
    TARGET_IS_DOWN.store(false, Ordering::Relaxed);
}

pub fn start<F>(on_trigger: F)
where
    F: Fn() + Send + 'static,
{
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }

    let (tx, rx) = mpsc::channel();
    *TRIGGER_SENDER.lock() = Some(tx);

    std::thread::spawn(move || {
        while rx.recv().is_ok() {
            on_trigger();
        }
    });

    start_platform_hook();
}

#[cfg(windows)]
fn start_platform_hook() {
    std::thread::spawn(move || {
        use std::ptr::null_mut;
        use windows::Win32::Foundation::{HINSTANCE, HWND};
        use windows::Win32::UI::WindowsAndMessaging::{
            GetMessageW, SetWindowsHookExW, UnhookWindowsHookEx, MSG, WH_KEYBOARD_LL,
        };

        let hook = unsafe {
            SetWindowsHookExW(
                WH_KEYBOARD_LL,
                Some(keyboard_hook_proc),
                HINSTANCE(null_mut()),
                0,
            )
        };
        let Ok(hook) = hook else {
            eprintln!("[Keyboard] failed to install global keyboard hook");
            return;
        };

        let mut message = MSG::default();
        while unsafe { GetMessageW(&mut message, HWND(null_mut()), 0, 0).as_bool() } {}

        let _ = unsafe { UnhookWindowsHookEx(hook) };
    });
}

#[cfg(not(windows))]
fn start_platform_hook() {}

#[cfg(windows)]
unsafe extern "system" fn keyboard_hook_proc(
    code: i32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use std::ptr::null_mut;
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, HHOOK, KBDLLHOOKSTRUCT, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
    };

    if code >= 0 {
        let message = wparam.0 as u32;
        let is_key_down = message == WM_KEYDOWN || message == WM_SYSKEYDOWN;
        let is_key_up = message == WM_KEYUP || message == WM_SYSKEYUP;

        if ENABLED.load(Ordering::Relaxed) && (is_key_down || is_key_up) {
            let key = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
            let scan_code = physical_scan_code(key.scanCode, key.flags.0);
            let target = TARGET_SCAN_CODE.load(Ordering::Relaxed);

            if target != 0 && scan_code == target {
                if is_key_up {
                    TARGET_IS_DOWN.store(false, Ordering::Relaxed);
                } else if !TARGET_IS_DOWN.swap(true, Ordering::Relaxed) {
                    if let Some(sender) = TRIGGER_SENDER.lock().as_ref() {
                        let _ = sender.send(());
                    }
                }
            }
        }
    }

    unsafe { CallNextHookEx(HHOOK(null_mut()), code, wparam, lparam) }
}

#[cfg(windows)]
fn physical_scan_code(scan_code: u32, flags: u32) -> u32 {
    if flags & windows::Win32::UI::WindowsAndMessaging::LLKHF_EXTENDED.0 != 0 {
        scan_code | 0xE000
    } else {
        scan_code
    }
}
