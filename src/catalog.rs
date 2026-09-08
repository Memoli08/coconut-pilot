#[cfg(not(target_os = "linux"))]
use anyhow::bail;
use anyhow::Result;
use serde::Serialize;
#[derive(Clone, Debug, Serialize)]
pub struct App {
    pub id: String,
    pub name: String,
    pub description: String,
    pub executable: String,
}
pub trait ApplicationCatalog {
    fn applications(&self) -> Result<Vec<App>>;
    fn browsers(&self) -> Result<Vec<App>>;
}
pub struct NativeCatalog;
#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use gio::prelude::*;
    fn convert(a: gio::AppInfo) -> Option<App> {
        if !a.should_show() {
            return None;
        }
        Some(App {
            id: a.id()?.to_string(),
            name: a.display_name().to_string(),
            description: a.description().map(|s| s.to_string()).unwrap_or_default(),
            executable: a.executable().to_string_lossy().into(),
        })
    }
    pub fn list(browser: bool) -> Vec<App> {
        let raw = if browser {
            let mut a = gio::AppInfo::all_for_type("x-scheme-handler/https");
            a.extend(gio::AppInfo::all_for_type("x-scheme-handler/http"));
            a
        } else {
            gio::AppInfo::all()
        };
        let mut apps: Vec<_> = raw.into_iter().filter_map(convert).collect();
        apps.sort_by_key(|a| (a.name.to_lowercase(), a.id.clone()));
        let mut seen = std::collections::HashSet::new();
        apps.retain(|a| seen.insert(a.id.clone()));
        apps
    }
    pub fn launch(id: &str, uri: Option<&str>) -> Result<()> {
        let app = gio::DesktopAppInfo::new(id).ok_or_else(|| {
            anyhow::anyhow!("Application {id} is no longer installed; choose another action")
        })?;
        if let Some(u) = uri {
            app.launch_uris(&[u], None::<&gio::AppLaunchContext>)?
        } else {
            app.launch(&[], None::<&gio::AppLaunchContext>)?
        }
        Ok(())
    }
    pub fn default_uri(uri: &str) -> Result<()> {
        gio::AppInfo::launch_default_for_uri(uri, None::<&gio::AppLaunchContext>)?;
        Ok(())
    }
}
#[cfg(target_os = "windows")]
mod windows {
    use super::*;
    use std::{ffi::OsStr, os::windows::ffi::OsStrExt, path::Path};
    use windows_sys::Win32::UI::Shell::ShellExecuteW;

    fn wide(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(Some(0)).collect()
    }
    fn start_menu_roots() -> Vec<std::path::PathBuf> {
        let mut roots = Vec::new();
        if let Some(appdata) = std::env::var_os("APPDATA") {
            roots.push(
                std::path::PathBuf::from(appdata).join("Microsoft/Windows/Start Menu/Programs"),
            );
        }
        if let Some(program_data) = std::env::var_os("ProgramData") {
            roots.push(
                std::path::PathBuf::from(program_data)
                    .join("Microsoft/Windows/Start Menu/Programs"),
            );
        }
        roots
    }
    fn visit(dir: &Path, apps: &mut Vec<App>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                visit(&path, apps);
            } else if matches!(
                path.extension().and_then(|extension| extension.to_str()),
                Some(extension) if extension.eq_ignore_ascii_case("lnk") || extension.eq_ignore_ascii_case("url") || extension.eq_ignore_ascii_case("exe")
            ) {
                let name = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                apps.push(App {
                    id: path.to_string_lossy().into_owned(),
                    name,
                    description: "Windows Start Menu".into(),
                    executable: path.to_string_lossy().into_owned(),
                });
            }
        }
    }
    pub fn list(browser: bool) -> Vec<App> {
        let mut apps = Vec::new();
        for root in start_menu_roots() {
            visit(&root, &mut apps);
        }
        if browser {
            apps.retain(|app| {
                let name = app.name.to_ascii_lowercase();
                ["browser", "chrome", "firefox", "edge", "brave", "opera"]
                    .iter()
                    .any(|needle| name.contains(needle))
            });
        }
        apps.sort_by_key(|app| (app.name.to_ascii_lowercase(), app.id.clone()));
        apps.dedup_by(|a, b| a.id.eq_ignore_ascii_case(&b.id));
        apps
    }
    pub fn launch(path: &str, uri: Option<&str>) -> Result<()> {
        let file = wide(OsStr::new(path));
        let parameters = uri.map(|value| wide(OsStr::new(value)));
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                std::ptr::null(),
                file.as_ptr(),
                parameters
                    .as_ref()
                    .map_or(std::ptr::null(), |value| value.as_ptr()),
                std::ptr::null(),
                1,
            )
        };
        if result as usize <= 32 {
            bail!("Windows Shell could not launch {path} (code {:?})", result)
        }
        Ok(())
    }
    pub fn default_uri(uri: &str) -> Result<()> {
        launch(uri, None)
    }
}
impl ApplicationCatalog for NativeCatalog {
    fn applications(&self) -> Result<Vec<App>> {
        #[cfg(target_os = "linux")]
        {
            Ok(linux::list(false))
        }
        #[cfg(not(target_os = "linux"))]
        {
            #[cfg(target_os = "windows")]
            return Ok(windows::list(false));
            #[cfg(not(target_os = "windows"))]
            bail!("Application discovery is not implemented on this platform")
        }
    }
    fn browsers(&self) -> Result<Vec<App>> {
        #[cfg(target_os = "linux")]
        {
            Ok(linux::list(true))
        }
        #[cfg(not(target_os = "linux"))]
        {
            #[cfg(target_os = "windows")]
            return Ok(windows::list(true));
            #[cfg(not(target_os = "windows"))]
            bail!("Browser discovery is not implemented on this platform")
        }
    }
}
pub fn launch(id: &str, uri: Option<&str>) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        linux::launch(id, uri)
    }
    #[cfg(not(target_os = "linux"))]
    {
        #[cfg(target_os = "windows")]
        return windows::launch(id, uri);
        #[cfg(not(target_os = "windows"))]
        {
            let _ = (id, uri);
            bail!("Native application launching is not implemented on this platform")
        }
    }
}
pub fn default_uri(uri: &str) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        linux::default_uri(uri)
    }
    #[cfg(not(target_os = "linux"))]
    {
        #[cfg(target_os = "windows")]
        return windows::default_uri(uri);
        #[cfg(not(target_os = "windows"))]
        {
            let _ = uri;
            bail!("URI launching is not implemented on this platform")
        }
    }
}
pub fn which(program: &str) -> Option<std::path::PathBuf> {
    let p = std::path::Path::new(program);
    if p.is_absolute() {
        return executable(p).then(|| p.to_owned());
    }
    let names = {
        #[cfg(target_os = "windows")]
        {
            if p.extension().is_some() {
                vec![program.to_string()]
            } else {
                vec![
                    format!("{program}.exe"),
                    format!("{program}.cmd"),
                    format!("{program}.bat"),
                ]
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            vec![program.to_string()]
        }
    };
    std::env::split_paths(&std::env::var_os("PATH")?).find_map(|directory| {
        names
            .iter()
            .map(|name| directory.join(name))
            .find(|candidate| executable(candidate))
    })
}
#[cfg(not(target_os = "windows"))]
pub const TERMINALS: &[&str] = &[
    "alacritty",
    "kitty",
    "konsole",
    "gnome-terminal",
    "foot",
    "xterm",
];
#[cfg(target_os = "windows")]
pub const TERMINALS: &[&str] = &["wt.exe", "powershell.exe", "cmd.exe"];

pub fn default_shell() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "cmd.exe"
    }
    #[cfg(not(target_os = "windows"))]
    {
        "/bin/sh"
    }
}

pub fn open_browser(browser: Option<&str>) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        use gio::prelude::*;
        if let Some(id) = browser {
            return linux::launch(id, None);
        }
        let app = gio::AppInfo::default_for_uri_scheme("https")
            .ok_or_else(|| anyhow::anyhow!("No default web browser is configured"))?;
        app.launch(&[], None::<&gio::AppLaunchContext>)?;
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        #[cfg(target_os = "windows")]
        {
            if let Some(path) = browser {
                return windows::launch(path, None);
            }
            windows::default_uri("about:blank")
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = browser;
            bail!("Browser launching is not implemented on this platform")
        }
    }
}

fn executable(p: &std::path::Path) -> bool {
    if !p.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        p.metadata()
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        true
    }
}
