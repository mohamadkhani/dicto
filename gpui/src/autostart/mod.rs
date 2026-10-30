//! "Start Dicto in tray at login" — the OS autostart entry.
//!
//! The setting's state IS the entry's existence: nothing is persisted in
//! settings.toml. [`is_enabled`] answers "does the autostart entry exist",
//! [`enable`] writes one that runs the current binary with `--tray`, and
//! [`disable`] removes it.

/// Absolute path to the running executable, falling back to bare `dicto`
/// (looked up through `PATH`) when it cannot be resolved.
fn exe_path_string() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.canonicalize().ok())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "dicto".to_string())
}

/// Whether "start in tray at login" is enabled (the autostart entry exists).
pub fn is_enabled() -> bool {
    platform::is_enabled()
}

/// Write the autostart entry so Dicto starts hidden in the tray at login.
pub fn enable() -> anyhow::Result<()> {
    platform::enable()
}

/// Remove the autostart entry. A missing entry is not an error.
// Wired to the settings toggle in the General tab (see general_panel.rs);
// until then it is only reachable through this API.
#[allow(dead_code)]
pub fn disable() -> anyhow::Result<()> {
    platform::disable()
}

#[cfg(target_os = "linux")]
mod platform {
    //! XDG autostart entry: `~/.config/autostart/dicto.desktop`.

    use std::path::PathBuf;

    use anyhow::{Context, Result};

    fn entry_path() -> PathBuf {
        let config_dir = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
            });
        config_dir.join("autostart").join("dicto.desktop")
    }

    pub fn is_enabled() -> bool {
        entry_path().is_file()
    }

    pub fn enable() -> Result<()> {
        let path = entry_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
        }
        let contents = format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Dicto\n\
             Exec={} --tray\n\
             Icon=dicto\n\
             X-GNOME-Autostart-enabled=true\n\
             StartupNotify=false\n",
            super::exe_path_string(),
        );
        std::fs::write(&path, contents).with_context(|| format!("write {}", path.display()))
    }

    #[allow(dead_code)] // see `disable` above
    pub fn disable() -> Result<()> {
        let path = entry_path();
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e).with_context(|| format!("remove {}", path.display())),
        }
    }
}

#[cfg(target_os = "windows")]
mod platform {
    //! `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, value `dicto`.

    use anyhow::Result;
    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ,
        RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegGetValueW, RegOpenKeyExW, RegSetValueExW,
    };
    use windows::core::{PCWSTR, w};

    const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
    const VALUE_NAME: PCWSTR = w!("dicto");

    pub fn is_enabled() -> bool {
        unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                VALUE_NAME,
                RRF_RT_REG_SZ,
                None,
                None,
                None,
            ) == ERROR_SUCCESS
        }
    }

    pub fn enable() -> Result<()> {
        unsafe {
            let mut hkey = HKEY::default();
            let err = RegCreateKeyExW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                0,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                None,
                &mut hkey,
                None,
            );
            if err != ERROR_SUCCESS {
                return Err(anyhow::anyhow!("RegCreateKeyExW failed: {err:?}"));
            }
            let data = format!("\"{}\" --tray", super::exe_path_string());
            // REG_SZ data is NUL-terminated UTF-16.
            let bytes: Vec<u8> = data
                .encode_utf16()
                .chain(std::iter::once(0u16))
                .flat_map(u16::to_le_bytes)
                .collect();
            let err = RegSetValueExW(hkey, VALUE_NAME, 0, REG_SZ, Some(&bytes));
            let _ = RegCloseKey(hkey);
            if err != ERROR_SUCCESS {
                return Err(anyhow::anyhow!("RegSetValueExW failed: {err:?}"));
            }
            Ok(())
        }
    }

    #[allow(dead_code)] // see `disable` above
    pub fn disable() -> Result<()> {
        unsafe {
            let mut hkey = HKEY::default();
            let err = RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, 0, KEY_SET_VALUE, &mut hkey);
            if err == ERROR_FILE_NOT_FOUND {
                return Ok(());
            }
            if err != ERROR_SUCCESS {
                return Err(anyhow::anyhow!("RegOpenKeyExW failed: {err:?}"));
            }
            let err = RegDeleteValueW(hkey, VALUE_NAME);
            let _ = RegCloseKey(hkey);
            if err == ERROR_SUCCESS || err == ERROR_FILE_NOT_FOUND {
                Ok(())
            } else {
                Err(anyhow::anyhow!("RegDeleteValueW failed: {err:?}"))
            }
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
mod platform {
    //! Unsupported platform: autostart is a no-op that always reads "off".

    pub fn is_enabled() -> bool {
        false
    }

    pub fn enable() -> anyhow::Result<()> {
        Err(anyhow::anyhow!(
            "autostart is not supported on this platform"
        ))
    }

    #[allow(dead_code)] // see `disable` above
    pub fn disable() -> anyhow::Result<()> {
        Ok(())
    }
}
