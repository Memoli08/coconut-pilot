use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    Application {
        id: String,
    },
    Browser {
        browser: Option<String>,
    },
    Website {
        url: String,
        browser: Option<String>,
    },
    OpenPath {
        path: PathBuf,
    },
    Executable {
        program: String,
        args: Vec<String>,
        cwd: PathBuf,
        terminal: bool,
        #[serde(default)]
        env: BTreeMap<String, String>,
        #[serde(default = "yes")]
        single: bool,
    },
    ShellCommand {
        shell: String,
        command: String,
        cwd: PathBuf,
        terminal: bool,
        #[serde(default = "yes")]
        single: bool,
    },
    Terminal,
}
fn yes() -> bool {
    true
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Binding {
    pub device: String,
    pub keys: Vec<u16>,
    pub trigger: u16,
}
impl Binding {
    pub fn valid(&self) -> bool {
        !self.device.is_empty()
            && self.device.len() < 1024
            && !self.keys.is_empty()
            && self.keys.len() <= 5
            && self.keys.contains(&self.trigger)
            && !crate::input::modifier(self.trigger)
            && self
                .keys
                .iter()
                .all(|k| *k <= 0x2ff && (*k == self.trigger || crate::input::modifier(*k)))
            && self
                .keys
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                == self.keys.len()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub theme: String,
    pub banner: String,
    pub compact: bool,
    pub color: String,
    pub browser: Option<String>,
    pub terminal: Option<String>,
    pub autostart: bool,
    pub notifications: String,
    pub command_terminal: bool,
    pub command_single: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: "coconut".into(),
            banner: "auto".into(),
            compact: false,
            color: "auto".into(),
            browser: None,
            terminal: None,
            autostart: true,
            notifications: "errors".into(),
            command_terminal: true,
            command_single: true,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub version: u32,
    pub enabled: bool,
    pub binding: Option<Binding>,
    pub active: Option<String>,
    pub actions: BTreeMap<String, Action>,
    pub preferences: Preferences,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            enabled: false,
            binding: None,
            active: None,
            actions: BTreeMap::new(),
            preferences: Preferences::default(),
        }
    }
}
pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| directories::BaseDirs::new().map(|d| d.home_dir().to_owned()))
        .unwrap_or_else(|| PathBuf::from("."))
}
pub fn config_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(home)
            .join("CoconutPilot")
    }
    #[cfg(not(target_os = "windows"))]
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"))
        .join("coconut")
}
pub fn state_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(home)
            .join("CoconutPilot")
    }
    #[cfg(not(target_os = "windows"))]
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".local/state"))
        .join("coconut")
}
pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}
impl Config {
    pub fn load() -> Result<Self> {
        let p = config_path();
        if !p.exists() {
            return Ok(Self::default());
        }
        let c: Self = toml::from_str(&fs::read_to_string(&p)?)
            .context("Invalid configuration; existing file was not changed")?;
        c.validate()?;
        Ok(c)
    }
    pub fn validate(&self) -> Result<()> {
        if self.version != 1 {
            bail!("Unsupported configuration version {}", self.version)
        }
        if self.enabled && (self.binding.is_none() || self.active.is_none()) {
            bail!("Enabled mapping needs a learned key and an active action")
        }
        if let Some(n) = &self.active {
            if !self.actions.contains_key(n) {
                bail!("Active action does not exist")
            }
        }
        if let Some(b) = &self.binding {
            if !b.valid() {
                bail!("Invalid keyboard binding")
            }
        }
        Ok(())
    }
    pub fn save(&self) -> Result<()> {
        self.validate()?;
        atomic_write(&config_path(), toml::to_string_pretty(self)?.as_bytes())
    }
}
pub fn atomic_write(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let parent = path.parent().context("Missing parent")?;
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(".coconut-{}.tmp", std::process::id()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut f = options.open(&temp)?;
    let result = (|| -> Result<()> {
        f.write_all(bytes)?;
        f.sync_all()?;
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}
pub fn normalize_url(input: &str) -> Result<String> {
    let s = input.trim();
    if s.chars().any(char::is_control) {
        bail!("Website addresses cannot contain control characters")
    }
    if s.is_empty() {
        bail!("Enter a website address")
    }
    let candidate = if s.contains("://") {
        s.to_owned()
    } else {
        format!("https://{s}")
    };
    let u = url::Url::parse(&candidate).context("Invalid website address")?;
    if !matches!(u.scheme(), "http" | "https")
        || u.host_str().is_none()
        || !u.username().is_empty()
        || u.password().is_some()
    {
        bail!("Use an HTTP or HTTPS URL without embedded credentials")
    }
    Ok(u.to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn urls() {
        assert_eq!(
            normalize_url("example.com").unwrap(),
            "https://example.com/"
        );
        assert_eq!(
            normalize_url("http://localhost:3000/a?q=1#x").unwrap(),
            "http://localhost:3000/a?q=1#x"
        );
        assert!(normalize_url("file:///etc/passwd").is_err());
        assert!(normalize_url("https://user:pass@example.com").is_err());
        assert!(normalize_url("").is_err());
    }
    #[test]
    fn roundtrip() {
        let mut c = Config::default();
        c.actions.insert(
            "site".into(),
            Action::Website {
                url: "https://example.com/".into(),
                browser: None,
            },
        );
        c.active = Some("site".into());
        let d: Config = toml::from_str(&toml::to_string(&c).unwrap()).unwrap();
        d.validate().unwrap();
        assert_eq!(c.actions, d.actions);
    }
    #[test]
    fn atomic() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("config");
        atomic_write(&p, b"one").unwrap();
        atomic_write(&p, b"two").unwrap();
        assert_eq!(fs::read(p).unwrap(), b"two");
    }
}
