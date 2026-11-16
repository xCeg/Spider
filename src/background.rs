//! Spider in the background: the presence without any window, started from Spider's window or
//! with Windows.
//!
//! `SpiderBackground.exe` (a windowless build of the same loop, `src/bin/spider-background.rs`)
//! holds a named mutex while it runs, so there is never a second one, and waits on a named event
//! that the window sets to stop it cleanly (the cards are taken off the profile first).
//! "Start with Windows" is the usual `Run` value of the current user, pointing at that file.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::runner::{Log, Runner};

/// Held by the background process for its whole life.
const MUTEX: &str = "Local\\SpiderBackground";
/// Set by the window to ask it to stop.
const STOP: &str = "Local\\SpiderBackgroundStop";
/// The `Run` value's name.
const RUN_VALUE: &str = "Spider";
/// The file names the background build may have beside `Spider.exe`.
const NAMES: [&str; 2] = ["SpiderBackground.exe", "spider-background.exe"];

/// The background executable beside this one, when it is there.
pub fn executable() -> Option<PathBuf> {
    let folder = std::env::current_exe().ok()?.parent()?.to_path_buf();
    NAMES
        .iter()
        .map(|name| folder.join(name))
        .find(|path| path.is_file())
}

#[cfg(windows)]
mod sys {
    use std::ffi::c_void;

    pub const SYNCHRONIZE: u32 = 0x0010_0000;
    pub const EVENT_MODIFY_STATE: u32 = 0x0002;
    pub const ERROR_ALREADY_EXISTS: u32 = 183;
    pub const WAIT_OBJECT_0: u32 = 0;
    pub const HKEY_CURRENT_USER: isize = 0x8000_0001_u32 as i32 as isize;
    pub const KEY_READ: u32 = 0x0002_0019;
    pub const KEY_SET_VALUE: u32 = 0x0002;
    pub const REG_SZ: u32 = 1;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        pub fn CreateMutexW(attributes: *const c_void, owner: i32, name: *const u16) -> *mut c_void;
        pub fn OpenMutexW(access: u32, inherit: i32, name: *const u16) -> *mut c_void;
        pub fn CreateEventW(
            attributes: *const c_void,
            manual: i32,
            initial: i32,
            name: *const u16,
        ) -> *mut c_void;
        pub fn OpenEventW(access: u32, inherit: i32, name: *const u16) -> *mut c_void;
        pub fn SetEvent(event: *mut c_void) -> i32;
        pub fn ResetEvent(event: *mut c_void) -> i32;
        pub fn WaitForSingleObject(handle: *mut c_void, milliseconds: u32) -> u32;
        pub fn CloseHandle(handle: *mut c_void) -> i32;
        pub fn GetLastError() -> u32;
    }

    #[link(name = "advapi32")]
    unsafe extern "system" {
        pub fn RegOpenKeyExW(key: isize, sub: *const u16, options: u32, sam: u32, result: *mut isize) -> i32;
        pub fn RegQueryValueExW(
            key: isize,
            name: *const u16,
            reserved: *mut u32,
            kind: *mut u32,
            data: *mut u8,
            size: *mut u32,
        ) -> i32;
        pub fn RegSetValueExW(
            key: isize,
            name: *const u16,
            reserved: u32,
            kind: u32,
            data: *const u8,
            size: u32,
        ) -> i32;
        pub fn RegDeleteValueW(key: isize, name: *const u16) -> i32;
        pub fn RegCloseKey(key: isize) -> i32;
    }

    /// A NUL-terminated UTF-16 copy of a string.
    pub fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// A kernel handle closed when dropped.
    pub struct Handle(pub *mut c_void);

    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: the handle came from a successful Create*/Open* call and is closed once.
            unsafe { CloseHandle(self.0) };
        }
    }

    /// The `Run` key of the current user, open for `access`.
    pub fn run_key(access: u32) -> Option<isize> {
        let path = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
        let mut key = 0isize;
        // SAFETY: the path is NUL-terminated and `key` receives the opened handle.
        let status = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, path.as_ptr(), 0, access, &mut key) };
        (status == 0).then_some(key)
    }
}

/// Whether a background Spider is running.
pub fn running() -> bool {
    #[cfg(windows)]
    {
        let name = sys::wide(MUTEX);
        // SAFETY: the name is NUL-terminated; the handle is closed by `Handle`.
        let handle = unsafe { sys::OpenMutexW(sys::SYNCHRONIZE, 0, name.as_ptr()) };
        if handle.is_null() {
            return false;
        }
        drop(sys::Handle(handle));
        true
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Starts the background Spider (it reads `spider.json` beside it).
pub fn start() -> Result<(), String> {
    let exe = executable().ok_or("SpiderBackground.exe est introuvable : relance build.bat")?;
    if running() {
        return Ok(());
    }
    let mut command = std::process::Command::new(&exe);
    if let Some(folder) = exe.parent() {
        command.current_dir(folder);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|error| format!("{} : {error}", exe.display()))?;
    // It claims its mutex within a moment.
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if running() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err("le mode arrière-plan n'a pas démarré (voir spider.log)".to_owned())
}

/// Asks the background Spider to take its cards down and stop; waits for it (a few seconds).
pub fn stop() -> Result<(), String> {
    #[cfg(windows)]
    {
        if !running() {
            return Ok(());
        }
        let name = sys::wide(STOP);
        // SAFETY: the name is NUL-terminated; the handle is closed by `Handle`.
        let event = unsafe { sys::OpenEventW(sys::EVENT_MODIFY_STATE, 0, name.as_ptr()) };
        if event.is_null() {
            return Err("le mode arrière-plan ne répond pas".to_owned());
        }
        let event = sys::Handle(event);
        // SAFETY: a live event handle.
        unsafe { sys::SetEvent(event.0) };
        let deadline = Instant::now() + Duration::from_secs(12);
        while Instant::now() < deadline {
            if !running() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Err("le mode arrière-plan ne s'est pas arrêté".to_owned())
    }
    #[cfg(not(windows))]
    {
        Ok(())
    }
}

/// The background loop: what `SpiderBackground.exe` runs. Returns the process exit code.
pub fn run(config: &Path) -> i32 {
    #[cfg(windows)]
    {
        let mutex_name = sys::wide(MUTEX);
        // SAFETY: the name is NUL-terminated; the handle is kept for the whole run.
        let mutex = unsafe { sys::CreateMutexW(std::ptr::null(), 1, mutex_name.as_ptr()) };
        // SAFETY: plain call right after CreateMutexW.
        let already = unsafe { sys::GetLastError() } == sys::ERROR_ALREADY_EXISTS;
        if mutex.is_null() || already {
            // Another background Spider owns the presence: nothing to do.
            return 0;
        }
        let _mutex = sys::Handle(mutex);
        let stop_name = sys::wide(STOP);
        // SAFETY: as above; a manual-reset event, cleared in case a stop was left set.
        let stop = unsafe { sys::CreateEventW(std::ptr::null(), 1, 0, stop_name.as_ptr()) };
        if stop.is_null() {
            return 1;
        }
        let stop = sys::Handle(stop);
        // SAFETY: a live event handle.
        unsafe { sys::ResetEvent(stop.0) };
        let mut runner = Runner::new(config);
        let mut log = Log::new(config, false);
        log.line("arrière-plan : démarré");
        loop {
            let wait = runner.step(&mut |line| log.line(line));
            let millis = wait.as_millis().min(u128::from(u32::MAX)) as u32;
            // SAFETY: a live event handle.
            if unsafe { sys::WaitForSingleObject(stop.0, millis) } == sys::WAIT_OBJECT_0 {
                break;
            }
        }
        runner.stop(&mut |line| log.line(line));
        log.line("arrière-plan : arrêté");
        0
    }
    #[cfg(not(windows))]
    {
        let _ = config;
        1
    }
}

/// The program Windows starts at sign-in for Spider, when there is one.
pub fn autostart() -> Option<String> {
    #[cfg(windows)]
    {
        let key = sys::run_key(sys::KEY_READ)?;
        let name = sys::wide(RUN_VALUE);
        let mut data = vec![0u16; 1024];
        let mut size = (data.len() * 2) as u32;
        let mut kind = 0u32;
        // SAFETY: `data` holds `size` bytes; the key is closed right after.
        let status = unsafe {
            sys::RegQueryValueExW(
                key,
                name.as_ptr(),
                std::ptr::null_mut(),
                &mut kind,
                data.as_mut_ptr().cast(),
                &mut size,
            )
        };
        // SAFETY: the key was opened above.
        unsafe { sys::RegCloseKey(key) };
        if status != 0 || kind != sys::REG_SZ {
            return None;
        }
        let length = (size as usize / 2).min(data.len());
        let text = String::from_utf16_lossy(&data[..length]);
        Some(text.trim_end_matches('\0').to_owned())
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Whether Windows starts this folder's background Spider at sign-in.
pub fn autostart_here() -> bool {
    match (autostart(), executable()) {
        (Some(command), Some(exe)) => {
            command.trim_matches('"').eq_ignore_ascii_case(&exe.to_string_lossy())
        }
        _ => false,
    }
}

/// Starts the background Spider with Windows (or stops doing so).
pub fn set_autostart(on: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        let key = sys::run_key(sys::KEY_SET_VALUE)
            .ok_or("le registre de démarrage est inaccessible")?;
        let name = sys::wide(RUN_VALUE);
        let status = if on {
            let exe = executable().ok_or("SpiderBackground.exe est introuvable : relance build.bat")?;
            let command = sys::wide(&format!("\"{}\"", exe.display()));
            // SAFETY: `command` is NUL-terminated UTF-16 of the given byte size.
            unsafe {
                sys::RegSetValueExW(
                    key,
                    name.as_ptr(),
                    0,
                    sys::REG_SZ,
                    command.as_ptr().cast(),
                    (command.len() * 2) as u32,
                )
            }
        } else {
            // SAFETY: the value name is NUL-terminated.
            match unsafe { sys::RegDeleteValueW(key, name.as_ptr()) } {
                2 => 0, // already absent
                other => other,
            }
        };
        // SAFETY: the key was opened above.
        unsafe { sys::RegCloseKey(key) };
        if status == 0 {
            Ok(())
        } else {
            Err(format!("le registre a refusé (code {status})"))
        }
    }
    #[cfg(not(windows))]
    {
        let _ = on;
        Err("le démarrage avec le système n'existe que sous Windows".to_owned())
    }
}
