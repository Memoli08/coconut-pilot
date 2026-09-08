use std::{fs, process::Command};
fn coconut() -> Command {
    Command::new(env!("CARGO_BIN_EXE_coconut"))
}
#[test]
fn public_command_is_coconut() {
    let o = coconut().arg("--help").output().unwrap();
    assert!(o.status.success());
    let s = String::from_utf8_lossy(&o.stdout);
    assert!(s.contains("Usage: coconut"));
    assert!(s.contains("browsers"));
    assert!(!s.contains("input-service"));
}
#[test]
fn absent_config_does_not_write() {
    let d = tempfile::tempdir().unwrap();
    let o = coconut()
        .args(["status", "--json"])
        .env("XDG_CONFIG_HOME", d.path())
        .output()
        .unwrap();
    assert!(o.status.success());
    let j: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(j["enabled"], false);
    assert!(j["service"]["healthy"].is_boolean());
    assert!(!d.path().join("coconut").exists());
}
#[test]
fn invalid_config_is_preserved() {
    let d = tempfile::tempdir().unwrap();
    let config_dir = if cfg!(target_os = "windows") {
        d.path().join("CoconutPilot")
    } else {
        d.path().join("coconut")
    };
    fs::create_dir(&config_dir).unwrap();
    let p = config_dir.join("config.toml");
    fs::write(&p, "version = 999\n").unwrap();
    let o = coconut()
        .arg("disable")
        .env("XDG_CONFIG_HOME", d.path())
        .env("APPDATA", d.path())
        .output()
        .unwrap();
    assert!(!o.status.success());
    assert_eq!(fs::read_to_string(p).unwrap(), "version = 999\n");
}
#[test]
fn setup_without_terminal_fails_without_mutating() {
    let d = tempfile::tempdir().unwrap();
    let o = coconut()
        .arg("setup")
        .env("XDG_CONFIG_HOME", d.path())
        .output()
        .unwrap();
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("interactive terminal"));
    assert!(!d.path().join("coconut").exists());
}
#[cfg(target_os = "linux")]
#[test]
fn desktop_catalog_respects_hidden_and_user_override() {
    let d = tempfile::tempdir().unwrap();
    let user = d.path().join("user/applications");
    let system = d.path().join("system/applications");
    fs::create_dir_all(&user).unwrap();
    fs::create_dir_all(&system).unwrap();
    let entry = |name: &str, extra: &str| {
        format!("[Desktop Entry]\nType=Application\nName={name}\nExec=/bin/true\n{extra}\n")
    };
    fs::write(system.join("fixture.desktop"), entry("System Name", "")).unwrap();
    fs::write(user.join("fixture.desktop"), entry("User Name", "")).unwrap();
    fs::write(user.join("hidden.desktop"), entry("Hidden", "Hidden=true")).unwrap();
    fs::write(
        user.join("nodisplay.desktop"),
        entry("No display", "NoDisplay=true"),
    )
    .unwrap();
    fs::write(
        user.join("browser.desktop"),
        entry(
            "Fixture Browser",
            "MimeType=x-scheme-handler/https;x-scheme-handler/http;",
        ),
    )
    .unwrap();
    fs::write(user.join("mimeinfo.cache"),"[MIME Cache]\nx-scheme-handler/https=browser.desktop;\nx-scheme-handler/http=browser.desktop;\n").unwrap();
    let invoke = |sub: &str| {
        coconut()
            .args([sub, "--json"])
            .env("XDG_DATA_HOME", d.path().join("user"))
            .env("XDG_DATA_DIRS", d.path().join("system"))
            .env("XDG_CONFIG_HOME", d.path().join("config"))
            .output()
            .unwrap()
    };
    let o = invoke("apps");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let apps: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    let apps = apps.as_array().unwrap();
    assert!(apps.iter().any(|a| a["name"] == "User Name"));
    assert!(!apps
        .iter()
        .any(|a| a["name"] == "System Name" || a["name"] == "Hidden" || a["name"] == "No display"));
    let o = invoke("browsers");
    let apps: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(apps.as_array().unwrap().len(), 1);
    assert_eq!(apps[0]["id"], "browser.desktop");
}
#[cfg(target_os = "linux")]
#[test]
fn task_preserves_arguments_cwd_and_single_instance() {
    let d = tempfile::tempdir().unwrap();
    let cwd = d.path().join("Türkçe proje");
    fs::create_dir(&cwd).unwrap();
    let task = d.path().join("task.json");
    let action = serde_json::json!({"type":"executable","program":"/bin/sh","args":["-c","printf '%s' \"$1\" >> result; sleep 1","worker","literal $(not-run) & spaces"],"cwd":cwd,"terminal":false,"env":{},"single":true});
    fs::write(&task, serde_json::to_vec(&action).unwrap()).unwrap();
    let mut first = coconut().arg("task-worker").arg(&task).spawn().unwrap();
    for _ in 0..100 {
        if cwd.join("result").exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(cwd.join("result").exists());
    let second = coconut().arg("task-worker").arg(&task).output().unwrap();
    assert!(second.status.success());
    assert!(String::from_utf8_lossy(&second.stdout).contains("already running"));
    assert!(first.wait().unwrap().success());
    assert_eq!(
        fs::read_to_string(cwd.join("result")).unwrap(),
        "literal $(not-run) & spaces"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn website_uses_selected_browser_and_preserves_url() {
    let d = tempfile::tempdir().unwrap();
    let apps = d.path().join("data/applications");
    let cfg = d.path().join("config/coconut");
    fs::create_dir_all(&apps).unwrap();
    fs::create_dir_all(&cfg).unwrap();
    let script = d.path().join("browser.sh");
    let output = d.path().join("received-url");
    fs::write(
        &script,
        format!("#!/bin/sh\nprintf '%s' \"$1\" > '{}'\n", output.display()),
    )
    .unwrap();
    fs::write(apps.join("chosen.desktop"),format!("[Desktop Entry]\nType=Application\nName=Chosen test browser\nExec=/bin/sh {} %u\nMimeType=x-scheme-handler/https;\n",script.display())).unwrap();
    let url = "https://example.test/?q=%24%28not-run%29&x=1#frag";
    fs::write(cfg.join("config.toml"),format!("version = 1\nactive = 'site'\n[actions.site]\ntype = 'website'\nurl = '{url}'\nbrowser = 'chosen.desktop'\n")).unwrap();
    let o = coconut()
        .arg("test")
        .env("XDG_CONFIG_HOME", d.path().join("config"))
        .env("XDG_DATA_HOME", d.path().join("data"))
        .env("XDG_DATA_DIRS", d.path().join("empty"))
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    for _ in 0..100 {
        if output.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(fs::read_to_string(output).unwrap(), url);
}
