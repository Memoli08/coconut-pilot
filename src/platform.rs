#[cfg(target_os = "linux")]
use crate::config::{self, Config};
use anyhow::{bail, Result};

/// Check the operating-system facilities required by the privileged input
/// service before the setup wizard starts changing user configuration.
pub fn input_platform_preflight() -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        use std::path::Path;
        if !Path::new("/run/systemd/system").is_dir() {
            bail!(
                "Coconut currently needs systemd/logind. This session does not expose /run/systemd/system."
            )
        }
        if !Path::new("/dev/input").is_dir() {
            bail!("Linux input devices are unavailable at /dev/input")
        }
        if !Path::new("/dev/uinput").exists() {
            bail!(
                "The uinput kernel module is unavailable at /dev/uinput. Load it with `sudo modprobe uinput`, then run setup again."
            )
        }
        if crate::catalog::which("systemctl").is_none() {
            bail!("systemctl is not available in PATH")
        }
        if unsafe { libc::geteuid() } != 0 && crate::catalog::which("sudo").is_none() {
            bail!(
                "sudo is required to install the input service. Install sudo or run `coconut system-install` as root."
            )
        }
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        #[cfg(target_os = "windows")]
        return Ok(());
        #[cfg(not(target_os = "windows"))]
        bail!("Copilot key remapping is not implemented on this platform")
    }
}

pub fn compatibility_report() -> Vec<String> {
    #[cfg(target_os = "linux")]
    {
        use std::path::Path;
        let check = |label: &str, available: bool, detail: &str| {
            format!(
                "  {} {} — {}",
                if available { "✓" } else { "✗" },
                label,
                detail
            )
        };
        vec![
            check(
                "systemd/logind",
                Path::new("/run/systemd/system").is_dir(),
                "required by the privileged input service",
            ),
            check(
                "/dev/input",
                Path::new("/dev/input").is_dir(),
                "physical keyboard event nodes",
            ),
            check(
                "/dev/uinput",
                Path::new("/dev/uinput").exists(),
                "virtual keyboard used while a key is mapped",
            ),
            check(
                "graphical session",
                matches!(
                    std::env::var("XDG_SESSION_TYPE").as_deref(),
                    Ok("wayland") | Ok("x11")
                ),
                "must be a local X11 or Wayland session on seat0",
            ),
            check(
                "sudo",
                unsafe { libc::geteuid() } == 0 || crate::catalog::which("sudo").is_some(),
                "required only to install or repair the system service",
            ),
        ]
    }
    #[cfg(not(target_os = "linux"))]
    {
        #[cfg(target_os = "windows")]
        return vec![
            "  ✓ Native Copilot input backend — Win+Shift+F23 user-session hook".into(),
            "  ✓ Startup — current-user Registry Run entry".into(),
        ];
        #[cfg(not(target_os = "windows"))]
        vec!["  ✗ Native Copilot input backend — not implemented on this platform".into()]
    }
}
pub trait SessionIntegration {
    fn autostart(&self, enabled: bool) -> Result<()>;
}
pub struct NativeSession;
impl SessionIntegration for NativeSession {
    fn autostart(&self, enabled: bool) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            hyprland_autostart(enabled)?;
            let dir = config::config_dir().parent().unwrap().join("autostart");
            let path = dir.join("coconut.desktop");
            if !enabled {
                if path.exists() {
                    std::fs::remove_file(path)?;
                }
                return Ok(());
            }
            let exe = std::env::current_exe()?;
            let quoted = exe
                .to_string_lossy()
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('`', "\\`")
                .replace('$', "\\$")
                .replace('%', "%%");
            config::atomic_write(&path,format!("[Desktop Entry]\nType=Application\nName=Coconut Pilot\nExec=\"{quoted}\" agent\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\n").as_bytes())?;
            Ok(())
        }
        #[cfg(not(target_os = "linux"))]
        {
            #[cfg(target_os = "windows")]
            {
                let exe = std::env::current_exe()?;
                let command = format!("\"{}\" agent", exe.display());
                let status = if enabled {
                    std::process::Command::new("reg")
                        .args([
                            "add",
                            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                            "/v",
                            "CoconutPilot",
                            "/t",
                            "REG_SZ",
                            "/d",
                            &command,
                            "/f",
                        ])
                        .status()?
                } else {
                    std::process::Command::new("reg")
                        .args([
                            "delete",
                            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                            "/v",
                            "CoconutPilot",
                            "/f",
                        ])
                        .status()?
                };
                if !status.success() && enabled {
                    bail!("Could not update the Windows startup entry")
                }
                Ok(())
            }
            #[cfg(not(target_os = "windows"))]
            {
                let _ = enabled;
                bail!("Session integration is not implemented on this platform")
            }
        }
    }
}
#[cfg(target_os = "linux")]
pub fn notify(c: &Config, message: &str, error: bool) {
    if c.preferences.notifications == "off" || (!error && c.preferences.notifications != "all") {
        return;
    }
    let _ = std::process::Command::new("notify-send")
        .args(["--app-name=Coconut Pilot", "Coconut Pilot", message])
        .status();
}
#[cfg(target_os = "windows")]
pub fn notify(_c: &crate::config::Config, _message: &str, _error: bool) {}
pub fn install() -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        input_platform_preflight()?;
        if unsafe { libc::geteuid() } != 0 {
            bail!("System installation requires root")
        }
        let source = std::env::current_exe()?;
        let dest = std::path::Path::new("/usr/local/libexec/coconut");
        std::fs::create_dir_all(dest.parent().unwrap())?;
        if source != dest {
            install_file(&source, dest, 0o755)?;
        }
        let unit = include_str!("../packaging/coconut-input.service");
        install_bytes(
            unit.as_bytes(),
            std::path::Path::new("/etc/systemd/system/coconut-input.service"),
            0o644,
        )?;
        let s = std::process::Command::new("systemctl")
            .args(["daemon-reload"])
            .status()?;
        if !s.success() {
            bail!("systemctl daemon-reload failed")
        }
        if !std::process::Command::new("systemctl")
            .args(["enable", "coconut-input.service"])
            .status()?
            .success()
        {
            bail!("Could not start input service")
        }
        if !std::process::Command::new("systemctl")
            .args(["restart", "coconut-input.service"])
            .status()?
            .success()
        {
            bail!("Could not restart input service")
        }
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        #[cfg(target_os = "windows")]
        {
            NativeSession.autostart(true)?;
            ensure_agent()
        }
        #[cfg(not(target_os = "windows"))]
        bail!("System integration is Linux-only")
    }
}

#[cfg(target_os = "linux")]
fn install_file(source: &std::path::Path, destination: &std::path::Path, mode: u32) -> Result<()> {
    install_bytes(&std::fs::read(source)?, destination, mode)
}

#[cfg(target_os = "linux")]
fn install_bytes(bytes: &[u8], destination: &std::path::Path, mode: u32) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let temporary = destination.with_extension(format!("new-{}", std::process::id()));
    let result = (|| -> Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(mode)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(mode))?;
        std::fs::rename(&temporary, destination)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}
pub fn uninstall_system() -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        if unsafe { libc::geteuid() } != 0 {
            bail!("Requires root")
        };
        let _ = std::process::Command::new("systemctl")
            .args(["disable", "--now", "coconut-input.service"])
            .status();
        for p in [
            "/etc/systemd/system/coconut-input.service",
            "/usr/local/libexec/coconut",
        ] {
            if std::path::Path::new(p).exists() {
                std::fs::remove_file(p)?;
            }
        }
        std::process::Command::new("systemctl")
            .arg("daemon-reload")
            .status()?;
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        #[cfg(target_os = "windows")]
        {
            NativeSession.autostart(false)
        }
        #[cfg(not(target_os = "windows"))]
        bail!("System integration is Linux-only")
    }
}
pub fn ensure_agent() -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        std::fs::create_dir_all(config::state_dir())?;
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(config::state_dir().join("agent.log"))?;
        let mut child = std::process::Command::new(std::env::current_exe()?)
            .arg("agent")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(log)
            .spawn()?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        #[cfg(target_os = "windows")]
        {
            let exe = std::env::current_exe()?;
            std::fs::create_dir_all(crate::config::state_dir())?;
            let log = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(crate::config::state_dir().join("agent.log"))?;
            let _ = std::process::Command::new(exe)
                .arg("agent")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(log)
                .spawn()?;
            Ok(())
        }
        #[cfg(not(target_os = "windows"))]
        bail!("Input agent is Linux-only")
    }
}

pub fn request_system_install() -> Result<bool> {
    let exe = std::env::current_exe()?;
    #[cfg(target_os = "linux")]
    let status = std::process::Command::new("sudo")
        .arg(exe)
        .arg("system-install")
        .status()?;
    #[cfg(target_os = "windows")]
    let status = std::process::Command::new(exe)
        .arg("system-install")
        .status()?;
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    let status = return Err(anyhow::anyhow!(
        "System integration is not implemented on this platform"
    ));
    Ok(status.success())
}

pub fn request_system_uninstall() -> Result<bool> {
    let exe = std::env::current_exe()?;
    #[cfg(target_os = "linux")]
    let status = std::process::Command::new("sudo")
        .arg(exe)
        .arg("system-uninstall")
        .status()?;
    #[cfg(target_os = "windows")]
    let status = std::process::Command::new(exe)
        .arg("system-uninstall")
        .status()?;
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    let status = return Err(anyhow::anyhow!(
        "System integration is not implemented on this platform"
    ));
    Ok(status.success())
}

#[cfg(target_os = "linux")]
fn hyprland_autostart(enabled: bool) -> Result<()> {
    use anyhow::Context;
    if !std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_lowercase()
        .contains("hyprland")
    {
        return Ok(());
    }
    let root = config::config_dir();
    let path = root.parent().unwrap().join("hypr/hyprland.conf");
    if !path.exists() {
        return Ok(());
    }
    let target = std::fs::canonicalize(&path)?;
    let original = std::fs::read_to_string(&target)?;
    let exe = std::env::current_exe()?;
    let executable = exe.to_str().context("Executable path is not UTF-8")?;
    if executable.contains(['\n', '\r', '#']) {
        bail!("Executable path cannot be represented in Hyprland startup configuration")
    }
    let escaped = format!("'{}'", executable.replace('\'', "'\"'\"'"));
    let updated = managed_startup(
        &original,
        enabled
            .then_some(format!("exec-once = {escaped} agent"))
            .as_deref(),
    );
    if original != updated {
        let backup = target.with_extension("conf.before-coconut");
        if !backup.exists() {
            config::atomic_write(&backup, original.as_bytes())?;
        }
        config::atomic_write(&target, updated.as_bytes())?;
    }
    Ok(())
}
#[cfg(any(target_os = "linux", test))]
fn managed_startup(original: &str, command: Option<&str>) -> String {
    const BEGIN: &str = "# BEGIN COCONUT PILOT";
    const END: &str = "# END COCONUT PILOT";
    let mut text = original.to_string();
    if let Some(start) = text.find(BEGIN) {
        if let Some(end) = text[start..].find(END) {
            let mut end = start + end + END.len();
            if text.as_bytes().get(end) == Some(&b'\n') {
                end += 1
            }
            text.replace_range(start..end, "");
        }
    }
    if let Some(command) = command {
        if !text.ends_with('\n') {
            text.push('\n')
        }
        text.push_str(&format!("{BEGIN}\n{command}\n{END}\n"));
    }
    text
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn managed_startup_preserves_user_settings() {
        let original = "monitor = ,preferred,auto,1\nexec-once = my-panel\n";
        let added = managed_startup(original, Some("exec-once = coconut agent"));
        assert_eq!(
            managed_startup(&added, Some("exec-once = coconut agent")),
            added
        );
        assert_eq!(managed_startup(&added, None), original);
    }
}
