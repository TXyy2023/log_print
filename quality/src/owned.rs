//! Stable process identity for destructive fault injection into owned fixtures only.
#[cfg(unix)]
use crate::support::*;
#[cfg(unix)]
pub struct Owned {
    pid: u32,
    identity: String,
}
#[cfg(unix)]
impl Owned {
    fn info(pid: u32) -> Option<(String, String)> {
        let r = capture(
            std::process::Command::new("ps").args([
                "-ww",
                "-o",
                "stat=",
                "-o",
                "lstart=",
                "-o",
                "command=",
                "-p",
                &pid.to_string(),
            ]),
            3,
        );
        let line = String::from_utf8_lossy(&r.stdout).trim().to_owned();
        let (state, identity) = line.split_once(char::is_whitespace)?;
        Some((state.into(), identity.trim().into()))
    }
    pub fn new(pid: u32) -> Self {
        let (state, identity) = Self::info(pid).expect("fixture not alive");
        assert!(!state.starts_with('Z'));
        Self { pid, identity }
    }
    pub fn alive(&self) -> bool {
        Self::info(self.pid)
            .is_some_and(|(state, id)| id == self.identity && !state.starts_with('Z'))
    }
    pub fn kill(&self) {
        if self.alive() {
            unsafe {
                libc::kill(self.pid as i32, libc::SIGKILL);
            }
        }
    }
}
#[cfg(windows)]
pub struct Owned {
    handle: windows_sys::Win32::Foundation::HANDLE,
}
#[cfg(windows)]
impl Owned {
    pub fn new(pid: u32) -> Self {
        use windows_sys::Win32::System::Threading::*;
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_TERMINATE, 0, pid) };
        assert!(!handle.is_null());
        Self { handle }
    }
    pub fn alive(&self) -> bool {
        unsafe { windows_sys::Win32::System::Threading::WaitForSingleObject(self.handle, 0) == 258 }
    }
    pub fn kill(&self) {
        if self.alive() {
            unsafe {
                windows_sys::Win32::System::Threading::TerminateProcess(self.handle, 97);
            }
        }
    }
}
impl Drop for Owned {
    fn drop(&mut self) {
        self.kill();
        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}
