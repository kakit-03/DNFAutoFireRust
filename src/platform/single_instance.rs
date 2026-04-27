use anyhow::{Result, bail};
use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows::Win32::System::Threading::CreateMutexW;
use windows::core::w;

const SINGLE_INSTANCE_MUTEX_NAME: windows::core::PCWSTR = w!("Local\\DNFAutoFire.Gui.Singleton");

pub struct SingleInstanceGuard {
    handle: HANDLE,
}

impl SingleInstanceGuard {
    pub fn acquire() -> Result<Self> {
        let handle = unsafe { CreateMutexW(None, false, SINGLE_INSTANCE_MUTEX_NAME) }?;
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe {
                let _ = CloseHandle(handle);
            }
            bail!("DNFAutoFire GUI 已在运行，请从系统托盘中打开现有实例");
        }

        Ok(Self { handle })
    }
}

impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        if !self.handle.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.handle);
            }
        }
    }
}
