mod catalog;
mod config;
mod input;
mod platform;
mod runner;
mod service;
mod ui;
use anyhow::{Context, Result};
use catalog::ApplicationCatalog;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
fn last_trigger_timing() -> serde_json::Value {
    std::fs::read(config::state_dir().join("last-trigger.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or(serde_json::Value::Null)
}
fn timing_text(timing: &serde_json::Value) -> String {
    match (
        timing
            .get("input_to_agent_ms")
            .and_then(serde_json::Value::as_u64),
        timing
            .get("launch_request_ms")
            .and_then(serde_json::Value::as_u64),
    ) {
        (Some(input), Some(launch)) => {
            format!("{input} ms input to agent; {launch} ms launch request")
        }
        _ => "No measured Copilot press yet".into(),
    }
}
#[derive(Parser)]
#[command(
    name = "coconut",
    version,
    about = "COCONUT PILOT — Your keyboard, your shortcuts."
)]
struct Cli {
    #[arg(long, global = true)]
    no_color: bool,
    #[command(subcommand)]
    command: Option<Commands>,
}
#[derive(Subcommand)]
enum Commands {
    Setup,
    Apps {
        #[arg(long)]
        json: bool,
    },
    Browsers {
        #[arg(long)]
        json: bool,
    },
    Actions,
    Preferences,
    Status {
        #[arg(long)]
        json: bool,
    },
    Test,
    Detect {
        #[arg(long)]
        json: bool,
    },
    Enable,
    Disable,
    Doctor {
        #[arg(long)]
        json: bool,
    },
    Uninstall,
    #[command(hide = true)]
    Agent,
    #[command(hide = true)]
    InputService,
    #[command(hide = true)]
    InputSelfTest,
    #[command(hide = true)]
    SystemInstall,
    #[command(hide = true)]
    SystemUninstall,
    #[command(hide = true)]
    TaskWorker {
        path: PathBuf,
    },
}
fn diagnostics() -> String {
    let config = config::Config::load();
    let (service_health, keyboards) = match service::probe() {
        Ok(devices) => (
            format!("ready ({} keyboard input nodes)", devices.len()),
            devices
                .iter()
                .map(|device| {
                    let mut features = device.copilot_keys.clone();
                    if device.composite {
                        features.push("composite HID".into());
                    }
                    if features.is_empty() {
                        features.push("standard keyboard".into());
                    }
                    format!(
                        "  • {} ({}) [{}]",
                        device.name,
                        device.path,
                        features.join(", ")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        Err(error) => (format!("failed: {error}"), "  • unavailable".into()),
    };
    let service_logs = if cfg!(target_os = "linux") {
        "Service logs: journalctl -u coconut-input.service".to_string()
    } else {
        "Service logs: Windows user-session agent".to_string()
    };
    format!("Platform: {}\nDesktop: {}\nSession: {} ({})\nInput service: {}\nPlatform prerequisites:\n{}\nKeyboard candidates:\n{}\nConfiguration: {}\nConfig valid: {}\nLast Copilot timing: {}\nNode/npm: {}\nTerminal candidates: {}\nEmergency stop: Backspace + Escape + Enter\n{}\nAgent log: {}\nCommands log: {}",std::env::consts::OS,std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default(),std::env::var("XDG_SESSION_TYPE").unwrap_or_default(),std::env::var("XDG_SESSION_ID").unwrap_or_else(|_|"no session id".into()),service_health,platform::compatibility_report().join("\n"),keyboards,config::config_path().display(),config.is_ok(),timing_text(&last_trigger_timing()),catalog::which("npm").map(|p|p.display().to_string()).unwrap_or_else(||"not in PATH".into()),catalog::TERMINALS.iter().filter(|t|catalog::which(t).is_some()).copied().collect::<Vec<_>>().join(", "),service_logs,config::state_dir().join("agent.log").display(),config::state_dir().join("commands.log").display())
}
fn run() -> Result<()> {
    let cli = Cli::parse();
    if cli.no_color {
        std::env::set_var("NO_COLOR", "1")
    };
    match cli.command {
        None => ui::run(),
        Some(Commands::Setup) => ui::setup(&mut config::Config::load()?),
        Some(Commands::Apps { json }) | Some(Commands::Browsers { json }) => {
            let browsers = matches!(cli.command, Some(Commands::Browsers { .. }));
            let apps = if browsers {
                catalog::NativeCatalog.browsers()?
            } else {
                catalog::NativeCatalog.applications()?
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&apps)?)
            } else {
                for a in apps {
                    println!("{}\t{}", a.name, a.id)
                }
            }
            Ok(())
        }
        Some(Commands::Actions) => ui::actions(&mut config::Config::load()?),
        Some(Commands::Preferences) => ui::preferences(&mut config::Config::load()?),
        Some(Commands::Status { json }) => {
            let c = config::Config::load()?;
            let service = service::probe()
                .map(|devices| serde_json::json!({"healthy":true,"keyboards":devices}))
                .unwrap_or_else(
                    |error| serde_json::json!({"healthy":false,"error":error.to_string()}),
                );
            let last_trigger = last_trigger_timing();
            let value = serde_json::json!({"enabled":c.enabled,"active_action":c.active,"binding":c.binding,"service":service,"last_trigger":last_trigger});
            if json {
                println!("{}", serde_json::to_string_pretty(&value)?)
            } else {
                println!(
                    "{}\nActive action: {}\nService: {}\nLast Copilot timing: {}",
                    if c.enabled { "Enabled" } else { "Paused" },
                    c.active.as_deref().unwrap_or("None"),
                    if value["service"]["healthy"] == true {
                        "ready"
                    } else {
                        "not ready"
                    },
                    timing_text(&value["last_trigger"])
                )
            }
            Ok(())
        }
        Some(Commands::Test) => runner::run_active(&config::Config::load()?),
        Some(Commands::Detect { json }) => {
            let binding = service::learn_with_updates(|name, binding| {
                if !json {
                    eprintln!(
                        "Detected {} on {name}; press the same key again to confirm.",
                        input::describe(&binding.keys)
                    );
                }
            })?;
            if json {
                println!("{}", serde_json::to_string_pretty(&binding)?)
            } else {
                println!(
                    "Confirmed {} on {}",
                    input::describe(&binding.keys),
                    binding.device
                )
            }
            Ok(())
        }
        Some(Commands::Enable) => {
            let mut c = config::Config::load()?;
            c.binding.as_ref().context("Run coconut setup first")?;
            c.active.as_ref().context("Select an action first")?;
            c.enabled = true;
            c.save()?;
            platform::ensure_agent()?;
            println!("Enabled");
            Ok(())
        }
        Some(Commands::Disable) => {
            let mut c = config::Config::load()?;
            c.enabled = false;
            c.save()?;
            println!("Paused");
            Ok(())
        }
        Some(Commands::Doctor { json }) => {
            if json {
                println!("{}", serde_json::json!({"diagnostics":diagnostics()}))
            } else {
                println!("{}", diagnostics())
            }
            Ok(())
        }
        Some(Commands::Uninstall) => ui::uninstall(&config::Config::load()?),
        Some(Commands::Agent) => service::agent(),
        Some(Commands::InputService) => service::run(),
        Some(Commands::InputSelfTest) => service::self_test_input(),
        Some(Commands::SystemInstall) => platform::install(),
        Some(Commands::SystemUninstall) => platform::uninstall_system(),
        Some(Commands::TaskWorker { path }) => runner::worker(path),
    }
}
fn main() {
    if let Err(e) = run() {
        eprintln!("coconut: {e:#}");
        std::process::exit(1)
    }
}
