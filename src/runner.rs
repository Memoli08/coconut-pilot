use crate::{
    catalog,
    config::{self, Action, Config},
};
use anyhow::{bail, Context, Result};
use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
};
pub trait ActionLauncher {
    fn launch(&self, action: &Action, config: &Config) -> Result<()>;
}
pub struct NativeLauncher;
impl ActionLauncher for NativeLauncher {
    fn launch(&self, action: &Action, c: &Config) -> Result<()> {
        match action {
            Action::Browser { browser } => catalog::open_browser(browser.as_deref()),
            Action::Application { id } => catalog::launch(id, None),
            Action::Website { url, browser } => {
                let u = config::normalize_url(url)?;
                match browser {
                    Some(id) => catalog::launch(id, Some(&u)),
                    None => catalog::default_uri(&u),
                }
            }
            Action::OpenPath { path } => {
                let p =
                    fs::canonicalize(path).context("Selected file or folder no longer exists")?;
                let u = url::Url::from_file_path(p)
                    .map_err(|_| anyhow::anyhow!("Invalid file path"))?;
                catalog::default_uri(u.as_str())
            }
            Action::Terminal => {
                let t = terminal(c)?;
                reap(Command::new(t).spawn()?);
                Ok(())
            }
            Action::Executable { .. } | Action::ShellCommand { .. } => launch_task(action, c),
        }
    }
}
pub fn terminal(c: &Config) -> Result<String> {
    let name = c
        .preferences
        .terminal
        .clone()
        .or_else(|| {
            catalog::TERMINALS
                .iter()
                .find(|p| catalog::which(p).is_some())
                .map(|s| s.to_string())
        })
        .context("No supported terminal found. Select one in Preferences.")?;
    if !catalog::TERMINALS.contains(&name.as_str()) {
        bail!("Unsupported terminal adapter: {name}")
    }
    catalog::which(&name).context("Selected terminal is no longer installed")?;
    Ok(name)
}
pub fn validate(a: &Action) -> Result<()> {
    match a {
        Action::Executable { program, cwd, .. } => {
            catalog::which(program).with_context(||format!("Cannot find {program} in the desktop session PATH. Select an absolute executable path."))?;
            if !cwd.is_dir() {
                bail!("Working directory does not exist: {}", cwd.display())
            }
        }
        Action::ShellCommand {
            shell,
            cwd,
            command,
            ..
        } => {
            catalog::which(shell).context("Shell not found")?;
            if !cwd.is_dir() || command.trim().is_empty() {
                bail!("Provide an existing working directory and a command")
            }
        }
        Action::Website { url, .. } => {
            config::normalize_url(url)?;
        }
        _ => {}
    }
    Ok(())
}
fn launch_task(a: &Action, c: &Config) -> Result<()> {
    validate(a)?;
    let terminal_mode = match a {
        Action::Executable { terminal, .. } | Action::ShellCommand { terminal, .. } => *terminal,
        _ => false,
    };
    let dir = config::state_dir().join("tasks");
    fs::create_dir_all(&dir)?;
    let bytes = serde_json::to_vec(a)?;
    let hash = stable_hash(&bytes);
    let path = dir.join(format!("{hash:016x}.json"));
    config::atomic_write(&path, &bytes)?;
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        let single = match a {
            Action::Executable { single, .. } | Action::ShellCommand { single, .. } => *single,
            _ => false,
        };
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path.with_extension("lock"))?;
        if single && unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Ok(());
        }
    }

    let exe = std::env::current_exe()?;
    let mut cmd = if terminal_mode {
        let t = terminal(c)?;
        let mut cmd = Command::new(&t);
        #[cfg(target_os = "windows")]
        {
            match t.as_str() {
                "wt.exe" => {
                    cmd.args(["new-tab", "--"]);
                    cmd.arg(&exe).arg("task-worker").arg(&path);
                }
                "powershell.exe" => {
                    let quote =
                        |value: &std::path::Path| value.to_string_lossy().replace('\'', "''");
                    cmd.args(["-NoExit", "-Command"]);
                    cmd.arg(format!(
                        "& '{}' task-worker '{}'",
                        quote(&exe),
                        quote(&path)
                    ));
                }
                "cmd.exe" => {
                    cmd.arg("/K");
                    cmd.arg(&exe).arg("task-worker").arg(&path);
                }
                _ => {
                    cmd.arg(&exe).arg("task-worker").arg(&path);
                }
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            match t.as_str() {
                "gnome-terminal" => {
                    cmd.args(["--wait", "--"]);
                }
                "kitty" | "foot" => {}
                _ => {
                    cmd.arg("-e");
                }
            }
            cmd.arg(&exe);
        }
        cmd
    } else {
        Command::new(&exe)
    };
    #[cfg(not(target_os = "windows"))]
    cmd.arg("task-worker").arg(&path);
    #[cfg(target_os = "windows")]
    if !terminal_mode {
        cmd.arg("task-worker").arg(&path);
    }
    if !terminal_mode {
        fs::create_dir_all(config::state_dir())?;
        let log = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(config::state_dir().join("commands.log"))?;
        cmd.stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log);
    }
    reap(cmd.spawn()?);
    Ok(())
}
fn stable_hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
    })
}
pub fn worker(path: PathBuf) -> Result<()> {
    let bytes = fs::read(&path)?;
    let a: Action = serde_json::from_slice(&bytes)?;
    validate(&a)?;
    let single = match &a {
        Action::Executable { single, .. } | Action::ShellCommand { single, .. } => *single,
        _ => bail!("Invalid task"),
    };
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path.with_extension("lock"))?;
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        if single && unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            println!("This command is already running.");
            return Ok(());
        }
    }
    #[cfg(not(unix))]
    {
        #[cfg(target_os = "windows")]
        let _lock = if single {
            use windows_sys::Win32::{
                Foundation::{GetLastError, ERROR_ALREADY_EXISTS},
                System::Threading::CreateMutexW,
            };
            let name: Vec<u16> = format!("Local\\CoconutPilotTask-{:016x}\0", stable_hash(&bytes))
                .encode_utf16()
                .collect();
            let lock = unsafe { CreateMutexW(std::ptr::null(), 1, name.as_ptr()) };
            if lock.is_null() || unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
                println!("This command is already running.");
                return Ok(());
            }
            Some(lock)
        } else {
            None
        };
        #[cfg(not(target_os = "windows"))]
        if single {
            bail!("Task locking is not implemented on this platform")
        }
    }
    let mut cmd = match a {
        Action::Executable {
            program,
            args,
            cwd,
            env,
            ..
        } => {
            let mut c = Command::new(program);
            c.args(args).current_dir(cwd).envs(env);
            c
        }
        Action::ShellCommand {
            shell,
            command,
            cwd,
            ..
        } => {
            let mut c = Command::new(shell);
            #[cfg(target_os = "windows")]
            {
                let shell_name = c.get_program().to_string_lossy().to_ascii_lowercase();
                c.arg(if shell_name.ends_with("cmd.exe") {
                    "/C"
                } else {
                    "-Command"
                });
            }
            #[cfg(not(target_os = "windows"))]
            c.arg("-c");
            c.arg(command).current_dir(cwd);
            c
        }
        _ => unreachable!(),
    };
    let status = cmd.status()?;
    drop(lock);
    if !status.success() {
        bail!("Command exited with {status}")
    }
    Ok(())
}
pub fn run_active(c: &Config) -> Result<()> {
    let name = c
        .active
        .as_ref()
        .context("No active action; run coconut setup")?;
    let action = c.actions.get(name).context("Active action is missing")?;
    NativeLauncher.launch(action, c)
}
fn reap(mut child: std::process::Child) {
    std::thread::spawn(move || {
        let _ = child.wait();
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn key_stable() {
        assert_eq!(stable_hash(b"hello"), stable_hash(b"hello"));
        assert_ne!(stable_hash(b"hello"), stable_hash(b"world"));
    }
    #[test]
    fn missing_program() {
        let a = Action::Executable {
            program: "/nonexistent/coconut-program".into(),
            args: vec![],
            cwd: std::env::temp_dir(),
            terminal: false,
            env: Default::default(),
            single: true,
        };
        assert!(validate(&a).is_err());
    }
}
