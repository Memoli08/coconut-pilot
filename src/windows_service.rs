//! Windows user-session backend. The Copilot hardware key is normally emitted
//! as Win+Shift+F23, so a user-level low-level keyboard hook is sufficient and
//! does not require a kernel driver or administrator privileges.
use crate::{
    config::{Binding, Config},
    platform, runner,
};
use anyhow::{bail, Context, Result};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex, OnceLock,
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::{
    Foundation::{GetLastError, ERROR_ALREADY_EXISTS, LPARAM, LRESULT, WPARAM},
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Input::KeyboardAndMouse::GetAsyncKeyState,
        WindowsAndMessaging::{
            CallNextHookEx, GetMessageW, KillTimer, PostQuitMessage, SetTimer, SetWindowsHookExW,
            UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT, MSG, WH_KEYBOARD_LL, WM_KEYDOWN, WM_QUIT,
            WM_SYSKEYDOWN, WM_TIMER,
        },
    },
};

const VK_F23: u32 = 0x86;
const VK_LWIN: i32 = 0x5b;
const VK_RWIN: i32 = 0x5c;
const VK_SHIFT: i32 = 0x10;
const VK_BACK: i32 = 0x08;
const VK_ESCAPE: i32 = 0x1b;
const VK_RETURN: i32 = 0x0d;
const WINDOWS_DEVICE: &str = "windows:global-copilot";
const WINDOWS_BINDING_KEYS: [u16; 3] = [42, 125, 193];
const WINDOWS_TRIGGER: u16 = 193;
static RUNNING_ACTION: AtomicBool = AtomicBool::new(false);
static PAUSING: AtomicBool = AtomicBool::new(false);
static LEARNED: OnceLock<Mutex<Option<Binding>>> = OnceLock::new();

fn binding() -> Binding {
    Binding {
        device: WINDOWS_DEVICE.into(),
        keys: WINDOWS_BINDING_KEYS.into(),
        trigger: WINDOWS_TRIGGER,
    }
}
fn copilot_held() -> bool {
    unsafe {
        (GetAsyncKeyState(VK_LWIN) < 0 || GetAsyncKeyState(VK_RWIN) < 0)
            && GetAsyncKeyState(VK_SHIFT) < 0
    }
}
fn launch_active() {
    if RUNNING_ACTION.swap(true, Ordering::AcqRel) {
        return;
    }
    std::thread::spawn(|| {
        let result = Config::load().and_then(|config| {
            // Keep the resident hook harmless after `coconut disable`. The
            // process remains alive so re-enabling does not need a log-out.
            if !config.enabled {
                return Ok(false);
            }
            let launch_started = Instant::now();
            runner::run_active(&config)?;
            let launch_request_ms: u64 = launch_started
                .elapsed()
                .as_millis()
                .try_into()
                .unwrap_or(u64::MAX);
            let record = serde_json::json!({
                "recorded_at_ms": unix_millis(),
                "input_to_agent_ms": 0,
                "launch_request_ms": launch_request_ms,
            });
            if let Ok(bytes) = serde_json::to_vec_pretty(&record) {
                let _ = crate::config::atomic_write(
                    &crate::config::state_dir().join("last-trigger.json"),
                    &bytes,
                );
            }
            eprintln!(
                "Coconut timing: input to agent 0 ms; launch request {launch_request_ms} ms."
            );
            Ok(true)
        });
        match result {
            Ok(true) => {
                if let Ok(config) = Config::load() {
                    platform::notify(&config, "Action launched", false);
                }
            }
            Ok(false) => {}
            Err(error) => eprintln!("Coconut action: {error:#}"),
        }
        RUNNING_ACTION.store(false, Ordering::Release);
    });
}
fn emergency_held() -> bool {
    unsafe {
        GetAsyncKeyState(VK_BACK) < 0
            && GetAsyncKeyState(VK_ESCAPE) < 0
            && GetAsyncKeyState(VK_RETURN) < 0
    }
}
fn pause_mapping() {
    if PAUSING.swap(true, Ordering::AcqRel) {
        return;
    }
    std::thread::spawn(|| {
        let result = Config::load().and_then(|mut config| {
            config.enabled = false;
            config.save()?;
            platform::notify(&config, "Input mapping paused by emergency shortcut", true);
            Ok(())
        });
        if let Err(error) = result {
            eprintln!("Coconut emergency stop: {error:#}");
        }
        PAUSING.store(false, Ordering::Release);
    });
}
fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}
unsafe extern "system" fn agent_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let event = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
        if (wparam as u32 == WM_KEYDOWN || wparam as u32 == WM_SYSKEYDOWN) && emergency_held() {
            pause_mapping();
            return 1;
        }
        if event.vkCode == VK_F23 && copilot_held() {
            if wparam as u32 == WM_KEYDOWN || wparam as u32 == WM_SYSKEYDOWN {
                launch_active();
            }
            return 1;
        }
    }
    unsafe { CallNextHookEx(0 as HHOOK, code, wparam, lparam) }
}
unsafe extern "system" fn learn_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let event = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
        if event.vkCode == VK_F23
            && copilot_held()
            && (wparam as u32 == WM_KEYDOWN || wparam as u32 == WM_SYSKEYDOWN)
        {
            if let Ok(mut slot) = LEARNED.get_or_init(|| Mutex::new(None)).lock() {
                *slot = Some(binding());
            }
            unsafe { PostQuitMessage(0) };
            // Do not let Windows open the default Copilot experience while the
            // setup wizard is learning the key.
            return 1;
        }
    }
    unsafe { CallNextHookEx(0 as HHOOK, code, wparam, lparam) }
}
fn install_hook(
    callback: unsafe extern "system" fn(i32, WPARAM, LPARAM) -> LRESULT,
) -> Result<HHOOK> {
    let module = unsafe { GetModuleHandleW(std::ptr::null()) };
    let hook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(callback), module, 0) };
    if hook == 0 as HHOOK {
        bail!("Could not install the Windows keyboard hook: {}", unsafe {
            GetLastError()
        })
    }
    Ok(hook)
}
fn message_loop() -> Result<()> {
    let mut message: MSG = unsafe { std::mem::zeroed() };
    loop {
        let state = unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) };
        if state == -1 {
            bail!("Windows message loop failed: {}", unsafe { GetLastError() })
        }
        if state == 0 || message.message == WM_QUIT || message.message == WM_TIMER {
            return Ok(());
        }
    }
}
fn capture_once() -> Result<Binding> {
    *LEARNED.get_or_init(|| Mutex::new(None)).lock().unwrap() = None;
    let hook = install_hook(learn_hook)?;
    let timer = unsafe { SetTimer(std::ptr::null_mut(), 1, 60_000, None) };
    let result = message_loop();
    if timer != 0 {
        unsafe { KillTimer(std::ptr::null_mut(), timer) };
    }
    unsafe { UnhookWindowsHookEx(hook) };
    result?;
    LEARNED
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap()
        .take()
        .context("Key detection timed out. Press Win+Shift+F23 (the Copilot key) and try again.")
}

pub fn run() -> Result<()> {
    bail!("Windows uses a per-user input agent; run `coconut setup` instead")
}
pub fn probe() -> Result<Vec<DeviceInfo>> {
    Ok(vec![DeviceInfo {
        id: WINDOWS_DEVICE.into(),
        name: "Windows global Copilot shortcut".into(),
        path: "Win+Shift+F23".into(),
        composite: false,
        copilot_keys: vec!["Win+Shift+F23".into()],
    }])
}
pub fn learn_with_updates<F>(mut on_first: F) -> Result<Binding>
where
    F: FnMut(&str, &Binding),
{
    let first = capture_once()?;
    on_first("Windows global Copilot shortcut", &first);
    let second = capture_once()?;
    if first != second {
        bail!("The second key press did not match the first")
    }
    Ok(first)
}
pub fn agent() -> Result<()> {
    use windows_sys::Win32::System::Threading::CreateMutexW;
    let name: Vec<u16> = "Local\\CoconutPilotAgent\0".encode_utf16().collect();
    let mutex = unsafe { CreateMutexW(std::ptr::null(), 1, name.as_ptr()) };
    if mutex.is_null() {
        bail!("Could not create Coconut singleton: {}", unsafe {
            GetLastError()
        })
    }
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        return Ok(());
    }
    let hook = install_hook(agent_hook)?;
    let result = message_loop();
    unsafe { UnhookWindowsHookEx(hook) };
    result
}
pub fn self_test_input() -> Result<()> {
    let captured = binding();
    if captured.valid() {
        Ok(())
    } else {
        bail!("Windows Copilot binding is invalid")
    }
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub path: String,
    pub composite: bool,
    pub copilot_keys: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_copilot_binding_is_valid() {
        let binding = binding();
        assert!(binding.valid());
        assert_eq!(binding.trigger, WINDOWS_TRIGGER);
        assert_eq!(binding.device, WINDOWS_DEVICE);
    }

    #[test]
    fn probe_exposes_the_standard_copilot_chord() {
        let devices = probe().unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].path, "Win+Shift+F23");
    }
}
