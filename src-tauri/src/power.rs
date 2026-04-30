use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

static STARTED: AtomicBool = AtomicBool::new(false);
static RESUME_SENDER: Lazy<Mutex<Option<mpsc::Sender<()>>>> = Lazy::new(|| Mutex::new(None));

pub fn start<F>(on_resume: F)
where
    F: Fn() + Send + 'static,
{
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }

    let (tx, rx) = mpsc::channel();
    *RESUME_SENDER.lock() = Some(tx);

    std::thread::spawn(move || {
        while rx.recv().is_ok() {
            on_resume();
        }
    });

    start_platform_watcher();
}

#[cfg(windows)]
fn start_platform_watcher() {
    std::thread::spawn(move || {
        use std::ptr::null_mut;
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::{HINSTANCE, HWND};
        use windows::Win32::System::LibraryLoader::GetModuleHandleW;
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DispatchMessageW, GetMessageW, RegisterClassW, TranslateMessage, MSG,
            WINDOW_EX_STYLE, WINDOW_STYLE, WNDCLASSW,
        };

        let class_name = wide_null("VDSleepPowerWatcher");
        let module = unsafe { GetModuleHandleW(PCWSTR::null()) }.ok();
        let hinstance = module
            .map(|module| HINSTANCE(module.0))
            .unwrap_or_else(|| HINSTANCE(null_mut()));

        let window_class = WNDCLASSW {
            lpfnWndProc: Some(power_window_proc),
            hInstance: hinstance,
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };

        if unsafe { RegisterClassW(&window_class) } == 0 {
            eprintln!("[Power] failed to register power watcher window class");
            return;
        }

        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                PCWSTR(class_name.as_ptr()),
                PCWSTR(class_name.as_ptr()),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                HWND(null_mut()),
                None,
                hinstance,
                None,
            )
        };

        if hwnd.is_err() {
            eprintln!("[Power] failed to create power watcher window");
            return;
        }

        let mut message = MSG::default();
        while unsafe { GetMessageW(&mut message, HWND(null_mut()), 0, 0).as_bool() } {
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    });
}

#[cfg(not(windows))]
fn start_platform_watcher() {}

#[cfg(windows)]
unsafe extern "system" fn power_window_proc(
    hwnd: windows::Win32::Foundation::HWND,
    message: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::{
        DefWindowProcW, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND, WM_POWERBROADCAST,
    };

    if message == WM_POWERBROADCAST {
        let event = wparam.0 as u32;
        if event == PBT_APMRESUMEAUTOMATIC || event == PBT_APMRESUMESUSPEND {
            if let Some(sender) = RESUME_SENDER.lock().as_ref() {
                let _ = sender.send(());
            }
        }
    }

    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

#[cfg(windows)]
fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}
