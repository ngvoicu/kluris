//! The per-user PATH in the registry (`HKCU\Environment\Path`), as the
//! PowerShell installer edits it: read without expanding `%VAR%`
//! references, written back as `REG_EXPAND_SZ`, then `WM_SETTINGCHANGE`
//! broadcast so new windows see the change.

use std::io;

use kluris_core::dist::UserPath;
use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_EXPAND_SZ, REG_SZ, RegCloseKey,
    RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
};

/// `HKCU\Environment\Path`.
pub struct RegistryUserPath;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn check(rc: WIN32_ERROR, what: &str) -> io::Result<()> {
    if rc == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{what}: {}",
            io::Error::from_raw_os_error(rc as i32)
        )))
    }
}

/// `HKCU\Environment`, closed on drop.
struct Key(HKEY);

impl Key {
    fn open(access: u32) -> io::Result<Key> {
        let mut key: HKEY = std::ptr::null_mut();
        let name = wide("Environment");
        // SAFETY: `name` is NUL-terminated and outlives the call; `key` is
        // a valid out pointer.
        let rc = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, name.as_ptr(), 0, access, &mut key) };
        check(rc, "cannot open HKCU\\Environment")?;
        Ok(Key(key))
    }
}

impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: the key was opened by `Key::open` and is closed once.
        unsafe {
            RegCloseKey(self.0);
        }
    }
}

impl UserPath for RegistryUserPath {
    fn read(&self) -> io::Result<Option<String>> {
        let key = Key::open(KEY_READ)?;
        let name = wide("Path");
        let mut kind = 0u32;
        let mut len = 0u32;
        // SAFETY: a size query: null data pointer, valid out pointers.
        let rc = unsafe {
            RegQueryValueExW(
                key.0,
                name.as_ptr(),
                std::ptr::null(),
                &mut kind,
                std::ptr::null_mut(),
                &mut len,
            )
        };
        if rc == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        check(rc, "cannot read the user PATH")?;
        let mut data = vec![0u8; len as usize];
        // SAFETY: `data` holds `len` bytes; `len` is updated to what was written.
        let rc = unsafe {
            RegQueryValueExW(
                key.0,
                name.as_ptr(),
                std::ptr::null(),
                &mut kind,
                data.as_mut_ptr(),
                &mut len,
            )
        };
        check(rc, "cannot read the user PATH")?;
        if kind != REG_SZ && kind != REG_EXPAND_SZ {
            return Err(io::Error::other(
                "the user PATH is not a string value; left as it is",
            ));
        }
        data.truncate(len as usize);
        let units: Vec<u16> = data
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .collect();
        let text = String::from_utf16(&units)
            .map_err(|_| io::Error::other("the user PATH is not valid UTF-16; left as it is"))?;
        Ok(Some(text.trim_end_matches('\0').to_string()))
    }

    fn write(&mut self, value: &str) -> io::Result<()> {
        let key = Key::open(KEY_WRITE)?;
        let name = wide("Path");
        let data: Vec<u8> = wide(value).iter().flat_map(|u| u.to_le_bytes()).collect();
        let len =
            u32::try_from(data.len()).map_err(|_| io::Error::other("the user PATH is too long"))?;
        // SAFETY: `data` is `len` bytes of NUL-terminated UTF-16.
        let rc =
            unsafe { RegSetValueExW(key.0, name.as_ptr(), 0, REG_EXPAND_SZ, data.as_ptr(), len) };
        check(rc, "cannot write the user PATH")?;
        drop(key);
        let environment = wide("Environment");
        let mut result = 0usize;
        // SAFETY: a broadcast with a NUL-terminated string that outlives the
        // call; a hung window is skipped after 5 seconds.
        unsafe {
            SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                0,
                environment.as_ptr() as isize,
                SMTO_ABORTIFHUNG,
                5000,
                &mut result,
            );
        }
        Ok(())
    }
}
