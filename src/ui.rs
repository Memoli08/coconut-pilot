use crate::input::{InputBackend, NativeInput};
use crate::{
    catalog::{self, ApplicationCatalog, NativeCatalog},
    config::{self, Action, Config},
    platform::{self, NativeSession, SessionIntegration},
    runner,
};
use anyhow::{bail, Context, Result};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    prelude::*,
    widgets::{Block, BorderType, Borders, List, ListItem, ListState, Padding, Paragraph, Wrap},
};
use std::{
    collections::BTreeMap,
    io::{self, IsTerminal},
    path::PathBuf,
};
struct Screen {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
}
impl Screen {
    fn new() -> Result<Self> {
        if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
            bail!("This command needs an interactive terminal. Use coconut status --json or coconut apps --json.")
        }
        enable_raw_mode()?;
        if let Err(e) = execute!(io::stdout(), EnterAlternateScreen) {
            let _ = disable_raw_mode();
            return Err(e.into());
        }
        match Terminal::new(CrosstermBackend::new(io::stdout())) {
            Ok(terminal) => Ok(Self { terminal }),
            Err(e) => {
                let _ = disable_raw_mode();
                let _ = execute!(io::stdout(), LeaveAlternateScreen);
                Err(e.into())
            }
        }
    }
}
impl Drop for Screen {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        let _ = self.terminal.show_cursor();
    }
}
const COCONUT_ART: [&str; 6] = [
    " ██████╗  ██████╗  ██████╗ ██████╗ ███╗   ██╗██╗   ██╗████████╗",
    "██╔════╝ ██╔═══██╗██╔════╝██╔═══██╗████╗  ██║██║   ██║╚══██╔══╝",
    "██║      ██║   ██║██║     ██║   ██║██╔██╗ ██║██║   ██║   ██║   ",
    "██║      ██║   ██║██║     ██║   ██║██║╚██╗██║██║   ██║   ██║   ",
    "╚██████╗ ╚██████╔╝╚██████╗╚██████╔╝██║ ╚████║╚██████╔╝   ██║   ",
    " ╚═════╝  ╚═════╝  ╚═════╝ ╚═════╝ ╚═╝  ╚═══╝ ╚═════╝    ╚═╝   ",
];
const PILOT_ART: [&str; 6] = [
    "██████╗ ██╗██╗      ██████╗ ████████╗",
    "██╔══██╗██║██║     ██╔═══██╗╚══██╔══╝",
    "██████╔╝██║██║     ██║   ██║   ██║   ",
    "██╔═══╝ ██║██║     ██║   ██║   ██║   ",
    "██║     ██║███████╗╚██████╔╝   ██║   ",
    "╚═╝     ╚═╝╚══════╝ ╚═════╝    ╚═╝   ",
];
fn color_at(t: f32, c: &Config) -> Color {
    if std::env::var_os("NO_COLOR").is_some()
        || c.preferences.color == "off"
        || c.preferences.theme == "monochrome"
    {
        return Color::Reset;
    }
    let stops = [
        (121., 82., 56.),
        (199., 149., 109.),
        (152., 121., 185.),
        (87., 156., 245.),
        (156., 120., 220.),
        (232., 132., 188.),
    ];
    let positions = [0.0, 0.40, 0.56, 0.61, 0.80, 1.0];
    let t = t.clamp(0.0, 1.0);
    let i = positions
        .windows(2)
        .position(|pair| t <= pair[1])
        .unwrap_or(4);
    let q = (t - positions[i]) / (positions[i + 1] - positions[i]);
    let (a, b, d) = stops[i];
    let (x, y, z) = stops[i + 1];
    let rgb = (
        (a + (x - a) * q) as u8,
        (b + (y - b) * q) as u8,
        (d + (z - d) * q) as u8,
    );
    let term = std::env::var("TERM").unwrap_or_default();
    let ct = std::env::var("COLORTERM").unwrap_or_default();
    if ct.contains("truecolor") || ct.contains("24bit") {
        Color::Rgb(rgb.0, rgb.1, rgb.2)
    } else if term.contains("256color") {
        Color::Indexed(16 + 36 * (rgb.0 / 51) + 6 * (rgb.1 / 51) + rgb.2 / 51)
    } else if t < 0.5 {
        Color::Yellow
    } else if t < 0.8 {
        Color::Blue
    } else {
        Color::Magenta
    }
}
fn gradient_line(c: &Config, text: String, from: f32, to: f32) -> Line<'static> {
    let width = text.chars().count().max(1);
    Line::from(
        text.chars()
            .enumerate()
            .map(|(index, ch)| {
                let position = from + (to - from) * index as f32 / width as f32;
                Span::styled(ch.to_string(), Style::default().fg(color_at(position, c)))
            })
            .collect::<Vec<_>>(),
    )
}

fn medium_art() -> Vec<String> {
    fn glyph(character: char) -> [&'static str; 5] {
        match character {
            'C' => ["█████", "█    ", "█    ", "█    ", "█████"],
            'O' => [" ███ ", "█   █", "█   █", "█   █", " ███ "],
            'N' => ["█   █", "██  █", "█ █ █", "█  ██", "█   █"],
            'U' => ["█   █", "█   █", "█   █", "█   █", " ███ "],
            'T' => ["█████", "  █  ", "  █  ", "  █  ", "  █  "],
            'P' => ["████ ", "█   █", "████ ", "█    ", "█    "],
            'I' => ["█████", "  █  ", "  █  ", "  █  ", "█████"],
            'L' => ["█    ", "█    ", "█    ", "█    ", "█████"],
            _ => ["     "; 5],
        }
    }
    (0..5)
        .map(|row| {
            "COCONUT PILOT"
                .chars()
                .map(|character| glyph(character)[row])
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

fn banner(c: &Config, width: u16, height: u16) -> Vec<Line<'static>> {
    if c.preferences.banner == "hidden" {
        return vec![];
    }
    let force_compact = c.preferences.banner == "compact";
    let full_width = COCONUT_ART[0].chars().count() + 3 + PILOT_ART[0].chars().count();
    let mut lines = if !force_compact && usize::from(width) >= full_width {
        (0..6)
            .map(|row| {
                gradient_line(
                    c,
                    format!("{}   {}", COCONUT_ART[row], PILOT_ART[row]),
                    0.0,
                    1.0,
                )
            })
            .collect::<Vec<_>>()
    } else if !force_compact && width >= 78 && height < 28 {
        medium_art()
            .into_iter()
            .map(|row| gradient_line(c, row, 0.0, 1.0))
            .collect::<Vec<_>>()
    } else if !force_compact && width >= 74 && height >= 28 {
        let mut rows = COCONUT_ART
            .iter()
            .map(|row| gradient_line(c, (*row).to_string(), 0.0, 0.58))
            .collect::<Vec<_>>();
        rows.extend(
            PILOT_ART
                .iter()
                .map(|row| gradient_line(c, format!("                {row}"), 0.61, 1.0)),
        );
        rows
    } else if width >= 40 {
        vec![gradient_line(
            c,
            "──────────── COCONUT PILOT ────────────".into(),
            0.0,
            1.0,
        )]
    } else {
        vec![gradient_line(c, "COCONUT PILOT".into(), 0.0, 1.0)]
    };
    lines.push(Line::styled(
        "Your keyboard, your shortcuts.",
        Style::default().fg(Color::DarkGray),
    ));
    lines
}
fn style(c: &Config) -> Style {
    match c.preferences.theme.as_str() {
        "light" => Style::default().fg(Color::Black).bg(Color::White),
        "dark" => Style::default().fg(Color::White).bg(Color::Black),
        _ => Style::default(),
    }
}
fn centered(area: Rect, max_width: u16) -> Rect {
    let width = area.width.saturating_sub(2).min(max_width).max(1);
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y,
        width,
        area.height,
    )
}

fn menu_impl(c: &Config, title: &str, items: &[String], _hero: bool) -> Result<Option<usize>> {
    let mut screen = Screen::new()?;
    let mut query = String::new();
    let mut selected = 0;
    let mut state = ListState::default();
    loop {
        let filtered: Vec<_> = items
            .iter()
            .enumerate()
            .filter(|(_, s)| s.to_lowercase().contains(&query.to_lowercase()))
            .collect();
        selected = selected.min(filtered.len().saturating_sub(1));
        state.select((!filtered.is_empty()).then_some(selected));
        screen.terminal.draw(|f| {
            let area = f.area();
            f.render_widget(Block::default().style(style(c)), area);
            // Keep the identity and the action panel in fixed layout slots on every
            // screen. Previously only the welcome screen used the large banner, so
            // entering a menu made the panel jump toward the top of the terminal.
            let lines = banner(c, area.width, area.height);
            let chunks = Layout::vertical([
                Constraint::Length(lines.len() as u16 + 2),
                Constraint::Min(7),
                Constraint::Length(2),
            ])
            .split(area);
            f.render_widget(
                Paragraph::new(lines).alignment(Alignment::Center),
                chunks[0],
            );

            let panel = centered(chunks[1], if c.preferences.compact { 76 } else { 96 });
            let title_height = title.lines().count().clamp(1, 7) as u16;
            let show_search = items.len() > 8 || !query.is_empty();
            let inner = Layout::vertical([
                Constraint::Length(title_height + 1),
                Constraint::Length(u16::from(show_search)),
                Constraint::Min(2),
            ])
            .split(panel.inner(Margin {
                horizontal: 2,
                vertical: 1,
            }));
            f.render_widget(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(color_at(0.47, c)))
                    .padding(Padding::new(1, 1, 0, 0)),
                panel,
            );
            f.render_widget(
                Paragraph::new(title)
                    .style(
                        Style::default()
                            .fg(color_at(0.08, c))
                            .add_modifier(Modifier::BOLD),
                    )
                    .wrap(Wrap { trim: true }),
                inner[0],
            );
            if show_search {
                let search = if query.is_empty() {
                    "Type to filter…".to_string()
                } else {
                    format!("Filter: {query}▏")
                };
                f.render_widget(
                    Paragraph::new(search).style(Style::default().fg(Color::DarkGray)),
                    inner[1],
                );
            }
            let rows = filtered
                .iter()
                .enumerate()
                .map(|(row, (_, item))| {
                    let (label, detail) = item
                        .split_once(" | ")
                        .map_or((item.as_str(), None), |(left, right)| (left, Some(right)));
                    let label_style = if row == selected {
                        Style::default()
                            .fg(color_at(0.68, c))
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };
                    let mut spans = vec![Span::styled(format!("  {label}"), label_style)];
                    if let Some(detail) = detail.filter(|_| !c.preferences.compact) {
                        spans.push(Span::styled(
                            format!("  {detail}"),
                            Style::default().fg(Color::DarkGray),
                        ));
                    }
                    ListItem::new(Line::from(spans))
                })
                .collect::<Vec<_>>();
            f.render_stateful_widget(
                List::new(rows)
                    .highlight_symbol("▸ ")
                    .highlight_style(Style::default().add_modifier(Modifier::BOLD)),
                inner[2],
                &mut state,
            );
            f.render_widget(
                Paragraph::new("↑/↓ navigate   enter select   type to filter   esc back")
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(Color::DarkGray)),
                centered(chunks[2], 96),
            );
        })?;
        if let Event::Key(k) = event::read()? {
            if k.kind != KeyEventKind::Press {
                continue;
            }
            match k.code {
                KeyCode::Esc => return Ok(None),
                KeyCode::Char('c') if k.modifiers.contains(event::KeyModifiers::CONTROL) => {
                    bail!("Cancelled")
                }
                KeyCode::Up => selected = selected.saturating_sub(1),
                KeyCode::Down => selected = (selected + 1).min(filtered.len().saturating_sub(1)),
                KeyCode::PageDown => {
                    selected = (selected + 8).min(filtered.len().saturating_sub(1))
                }
                KeyCode::PageUp => selected = selected.saturating_sub(8),
                KeyCode::Enter => return Ok(filtered.get(selected).map(|(i, _)| *i)),
                KeyCode::Backspace => {
                    query.pop();
                    selected = 0
                }
                KeyCode::Char(ch) => {
                    query.push(ch);
                    selected = 0
                }
                _ => {}
            }
        }
    }
}

pub fn menu(c: &Config, title: &str, items: &[String]) -> Result<Option<usize>> {
    menu_impl(c, title, items, false)
}

fn hero_menu(c: &Config, title: &str, items: &[String]) -> Result<Option<usize>> {
    menu_impl(c, title, items, true)
}
fn choose(c: &Config, title: &str, items: &[&str]) -> Result<Option<usize>> {
    menu(
        c,
        title,
        &items.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
    )
}

fn hero_choose(c: &Config, title: &str, items: &[&str]) -> Result<Option<usize>> {
    hero_menu(
        c,
        title,
        &items.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
    )
}
pub fn message(c: &Config, text: &str) -> Result<()> {
    choose(c, text, &["Continue"])?;
    Ok(())
}
fn prompt(c: &Config, title: &str, initial: &str) -> Result<Option<String>> {
    let mut s = Screen::new()?;
    let mut value = initial.to_string();
    loop {
        s.terminal.draw(|f| {
            let area = f.area();
            f.render_widget(Block::default().style(style(c)), area);
            let lines = banner(c, area.width, area.height);
            let chunks = Layout::vertical([
                Constraint::Length(lines.len() as u16 + 2),
                Constraint::Min(3),
                Constraint::Length(2),
            ])
            .split(area);
            f.render_widget(
                Paragraph::new(lines).alignment(Alignment::Center),
                chunks[0],
            );
            let panel = centered(chunks[1], 96);
            f.render_widget(
                Paragraph::new(format!("{title}\n\n  {value}▏"))
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .border_type(BorderType::Rounded)
                            .border_style(Style::default().fg(color_at(0.47, c)))
                            .padding(Padding::new(1, 1, 1, 1)),
                    )
                    .wrap(Wrap { trim: false }),
                panel,
            );
            f.render_widget(
                Paragraph::new("enter confirm   esc back   ctrl+u clear")
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(Color::DarkGray)),
                centered(chunks[2], 96),
            );
        })?;
        if let Event::Key(k) = event::read()? {
            if k.kind != KeyEventKind::Press {
                continue;
            }
            match k.code {
                KeyCode::Esc => return Ok(None),
                KeyCode::Enter => return Ok(Some(value)),
                KeyCode::Char('c') if k.modifiers.contains(event::KeyModifiers::CONTROL) => {
                    bail!("Cancelled")
                }
                KeyCode::Char('u') if k.modifiers.contains(event::KeyModifiers::CONTROL) => {
                    value.clear()
                }
                KeyCode::Backspace => {
                    value.pop();
                }
                KeyCode::Char(ch) => value.push(ch),
                _ => {}
            }
        }
    }
}
fn required(c: &Config, title: &str, initial: &str) -> Result<String> {
    prompt(c, title, initial)?.context("Cancelled")
}
fn pick_app(c: &Config, browsers: bool) -> Result<Option<String>> {
    let apps = if browsers {
        NativeCatalog.browsers()?
    } else {
        NativeCatalog.applications()?
    };
    let mut rows = vec![if browsers {
        "System default".into()
    } else {
        "Choose an executable…".into()
    }];
    rows.extend(
        apps.iter()
            .map(|a| format!("{} — {} | {}", a.name, a.id, a.description)),
    );
    let choice = menu(
        c,
        if browsers {
            "Choose a browser"
        } else {
            "Choose an application"
        },
        &rows,
    )?
    .context("Cancelled")?;
    Ok(if choice == 0 {
        None
    } else {
        Some(apps[choice - 1].id.clone())
    })
}
fn pick_path(c: &Config, title: &str, directory: bool) -> Result<PathBuf> {
    let mut dir = config::home();
    loop {
        let mut entries: Vec<_> = std::fs::read_dir(&dir)?
            .filter_map(|e| e.ok())
            .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
            .filter(|e| !directory || e.path().is_dir())
            .collect();
        entries.sort_by_key(|e| (!e.path().is_dir(), e.file_name()));
        let mut rows = vec!["Enter a path manually…".into(), "../".into()];
        if directory {
            rows.push("Select this folder".into())
        }
        let offset = rows.len();
        rows.extend(entries.iter().map(|e| {
            format!(
                "{}{}",
                e.file_name().to_string_lossy(),
                if e.path().is_dir() { "/" } else { "" }
            )
        }));
        match menu(c, &format!("{title}\n{}", dir.display()), &rows)?.context("Cancelled")? {
            0 => {
                let p = PathBuf::from(required(c, title, &dir.to_string_lossy())?);
                if (directory && p.is_dir()) || (!directory && p.exists()) {
                    return Ok(p);
                }
                message(c, "That path does not exist.")?
            }
            1 => {
                if let Some(p) = dir.parent() {
                    dir = p.into()
                }
            }
            2 if directory => return Ok(dir),
            i => {
                let p = entries[i - offset].path();
                if p.is_dir() {
                    dir = p
                } else {
                    return Ok(p);
                }
            }
        }
    }
}
fn terminal_preference(c: &mut Config) -> Result<()> {
    let available: Vec<_> = catalog::TERMINALS
        .iter()
        .filter(|s| catalog::which(s).is_some())
        .map(|s| s.to_string())
        .collect();
    if available.is_empty() {
        bail!(
            "Install a supported terminal: {}",
            catalog::TERMINALS.join(", ")
        )
    }
    if let Some(i) = menu(c, "Choose a terminal", &available)? {
        c.preferences.terminal = Some(available[i].clone())
    } else {
        bail!("Cancelled")
    }
    Ok(())
}
fn command_options(
    c: &mut Config,
    program: String,
    args: Vec<String>,
    shell: bool,
) -> Result<Action> {
    let cwd = pick_path(c, "Choose working directory", true)?;
    let terminal_options = if c.preferences.command_terminal {
        ["In a terminal (default)", "In the background"]
    } else {
        ["In the background (default)", "In a terminal"]
    };
    let selection =
        choose(c, "Where should the command run?", &terminal_options)?.context("Cancelled")?;
    let terminal = if selection == 0 {
        c.preferences.command_terminal
    } else {
        !c.preferences.command_terminal
    };
    if terminal && c.preferences.terminal.is_none() {
        terminal_preference(c)?
    }
    let repeat_options = if c.preferences.command_single {
        ["Ignore another press (default)", "Start another instance"]
    } else {
        ["Start another instance (default)", "Ignore another press"]
    };
    let selection =
        choose(c, "If this command is already running", &repeat_options)?.context("Cancelled")?;
    let single = if selection == 0 {
        c.preferences.command_single
    } else {
        !c.preferences.command_single
    };
    let action = if shell {
        Action::ShellCommand {
            shell: program,
            command: args.first().cloned().unwrap_or_default(),
            cwd,
            terminal,
            single,
        }
    } else {
        let mut env = BTreeMap::new();
        loop {
            if choose(c, "Environment variables", &["Continue", "Add a variable"])?
                .context("Cancelled")?
                == 0
            {
                break;
            }
            let name = required(c, "Variable name", "")?;
            if name.is_empty() || name.contains('=') || name.contains('\0') {
                bail!("Invalid variable name")
            }
            let value = required(
                c,
                "Variable value (stored in your private configuration)",
                "",
            )?;
            env.insert(name, value);
        }
        Action::Executable {
            program,
            args,
            cwd,
            terminal,
            env,
            single,
        }
    };
    runner::validate(&action)?;
    Ok(action)
}
fn website(c: &Config, initial: &str) -> Result<Action> {
    let browser = if let Some(id) = &c.preferences.browser {
        match choose(
            c,
            &format!("Browser preference: {id}"),
            &["Use preferred browser", "Choose another browser"],
        )?
        .context("Cancelled")?
        {
            0 => Some(id.clone()),
            _ => pick_app(c, true)?,
        }
    } else {
        pick_app(c, true)?
    };
    let url = loop {
        let raw = required(c, "Website address (HTTP or HTTPS)", initial)?;
        match config::normalize_url(&raw) {
            Ok(u) => break u,
            Err(e) => message(c, &e.to_string())?,
        }
    };
    message(
        c,
        &format!(
            "Website: {url}\nBrowser: {}",
            browser.as_deref().unwrap_or("System default")
        ),
    )?;
    Ok(Action::Website { url, browser })
}
fn new_action(c: &mut Config) -> Result<Action> {
    let choice = choose(
        c,
        "Choose an action",
        &[
            "Open an application | Pick from apps installed on this computer",
            "Open a website | Choose a browser and enter any HTTP/HTTPS address",
            "Open a terminal | Launch your preferred terminal emulator",
            "Open settings | Pick the settings application for your desktop",
            "Open a file or folder | Browse to a local path",
            "Home folder | Open your files immediately",
            "Web browser | Open a selected browser without a URL",
            "GitHub | Ready-made website action",
            "YouTube | Ready-made website action",
            "Wikipedia | Ready-made website action",
            "Localhost | Open a local development server",
            "npm run start | Run an npm project in its directory",
            "npm run dev | Run an npm development server",
            "Run an executable | Keep the program and arguments safely separated",
            "Run a shell command | Explicitly execute through a chosen shell",
        ],
    )?
    .context("Cancelled")?;
    match choice {
        0 | 3 => match pick_app(c, false)? {
            Some(id) => Ok(Action::Application { id }),
            None => {
                let p = pick_path(c, "Choose an executable", false)?;
                Ok(Action::Executable {
                    program: p.to_string_lossy().into(),
                    args: vec![],
                    cwd: config::home(),
                    terminal: false,
                    env: Default::default(),
                    single: false,
                })
            }
        },
        1 => website(c, "https://"),
        2 => {
            terminal_preference(c)?;
            Ok(Action::Terminal)
        }
        4 => {
            let folder = choose(c, "Open a path", &["Folder", "File"])?.context("Cancelled")? == 0;
            Ok(Action::OpenPath {
                path: pick_path(c, "Choose a path", folder)?,
            })
        }
        5 => Ok(Action::OpenPath {
            path: config::home(),
        }),
        6 => Ok(Action::Browser {
            browser: pick_app(c, true)?,
        }),
        7 => website(c, "https://github.com"),
        8 => website(c, "https://www.youtube.com"),
        9 => website(c, "https://www.wikipedia.org"),
        10 => website(c, "http://localhost:3000"),
        11 | 12 => command_options(
            c,
            "npm".into(),
            vec![
                "run".into(),
                if choice == 11 { "start" } else { "dev" }.into(),
            ],
            false,
        ),
        13 => {
            let program = required(c, "Executable name or absolute path", "")?;
            let mut args = vec![];
            loop {
                let arg = required(c, "Add one argument (empty to finish)", "")?;
                if arg.is_empty() {
                    break;
                }
                args.push(arg);
            }
            command_options(c, program, args, false)
        }
        _ => {
            let shell = required(c, "Shell executable", catalog::default_shell())?;
            let command = required(c, "Shell command", "")?;
            command_options(c, shell, vec![command], true)
        }
    }
}
fn save_action(c: &mut Config) -> Result<bool> {
    let a = new_action(c)?;
    let name = required(c, "Name this action", "My action")?;
    if name.trim().is_empty() {
        bail!("Action name cannot be empty")
    }
    if c.actions.contains_key(&name)
        && choose(c, "Replace existing action?", &["Cancel", "Replace"])?.unwrap_or(0) != 1
    {
        return Ok(false);
    }
    let preview = serde_json::to_string_pretty(&a)?;
    loop {
        match choose(
            c,
            &format!("Review action\n{preview}"),
            &["Save and activate", "Test action", "Cancel"],
        )? {
            Some(0) => {
                c.actions.insert(name.clone(), a);
                c.active = Some(name);
                return Ok(true);
            }
            Some(1) => {
                use runner::ActionLauncher;
                match runner::NativeLauncher.launch(&a, c) {
                    Ok(()) => message(c, "Launch requested. Check the application or terminal.")?,
                    Err(e) => message(c, &format!("Launch failed: {e:#}"))?,
                }
            }
            _ => return Ok(false),
        }
    }
}
pub fn actions(c: &mut Config) -> Result<()> {
    loop {
        let names: Vec<_> = c.actions.keys().cloned().collect();
        let mut rows = vec!["Create an action".into()];
        rows.extend(names.iter().map(|n| {
            format!(
                "{}{}",
                if c.active.as_ref() == Some(n) {
                    "● "
                } else {
                    ""
                },
                n
            )
        }));
        let Some(i) = menu(c, "Saved actions", &rows)? else {
            return Ok(());
        };
        if i == 0 {
            match save_action(c) {
                Ok(true) => c.save()?,
                Ok(false) => {}
                Err(e) => message(c, &format!("{e:#}"))?,
            }
            continue;
        }
        let name = &names[i - 1];
        match choose(
            c,
            name,
            &["Activate", "Edit / replace", "Duplicate", "Delete"],
        )? {
            Some(0) => c.active = Some(name.clone()),
            Some(1) => {
                if let Some(a) = edit_action(c, &c.actions[name].clone())? {
                    c.actions.insert(name.clone(), a);
                }
            }
            Some(2) => {
                let new = required(c, "New action name", &format!("{name} copy"))?;
                if new.trim().is_empty() || c.actions.contains_key(&new) {
                    message(c, "Choose a unique, nonempty name")?;
                    continue;
                }
                c.actions.insert(new, c.actions[name].clone());
            }
            Some(3)
                if choose(c, "Delete this action?", &["Cancel", "Delete"])?.unwrap_or(0) == 1 =>
            {
                c.actions.remove(name);
                if c.active.as_ref() == Some(name) {
                    c.active = None;
                    c.enabled = false;
                }
            }
            _ => {}
        }
        c.save()?
    }
}
pub fn preferences(c: &mut Config) -> Result<()> {
    loop {
        let Some(i) = choose(
            c,
            "Preferences",
            &[
                "Theme",
                "Banner",
                "List density",
                "Color",
                "Default browser",
                "Preferred terminal",
                "Start at login",
                "Notifications",
                "Default command visibility",
                "Default repeat behavior",
            ],
        )?
        else {
            return Ok(());
        };
        match i {
            0 => {
                if let Some(i) = choose(
                    c,
                    "Theme",
                    &["Coconut gradient", "Dark", "Light", "Monochrome"],
                )? {
                    c.preferences.theme = ["coconut", "dark", "light", "monochrome"][i].into()
                }
            }
            1 => {
                if let Some(i) = choose(c, "Banner", &["Automatic large", "Compact", "Hidden"])? {
                    c.preferences.banner = ["auto", "compact", "hidden"][i].into()
                }
            }
            2 => {
                c.preferences.compact =
                    choose(c, "List density", &["Comfortable", "Compact"])?.unwrap_or(0) == 1
            }
            3 => {
                if let Some(i) = choose(c, "Color", &["Automatic", "On", "Off"])? {
                    c.preferences.color = ["auto", "on", "off"][i].into()
                }
            }
            4 => c.preferences.browser = pick_app(c, true)?,
            5 => terminal_preference(c)?,
            6 => {
                c.preferences.autostart =
                    choose(c, "Start at login", &["On", "Off"])?.unwrap_or(0) == 0;
                NativeSession.autostart(c.preferences.autostart)?
            }
            7 => {
                if let Some(i) = choose(c, "Notifications", &["Errors only", "All results", "Off"])?
                {
                    c.preferences.notifications = ["errors", "all", "off"][i].into()
                }
            }
            8 => {
                if let Some(i) =
                    choose(c, "Default command visibility", &["Terminal", "Background"])?
                {
                    c.preferences.command_terminal = i == 0;
                }
            }
            _ => {
                if let Some(i) = choose(
                    c,
                    "Default repeat behavior",
                    &["Ignore while running", "Start another instance"],
                )? {
                    c.preferences.command_single = i == 0;
                }
            }
        }
        c.save()?
    }
}
fn detect(c: &Config) -> Result<config::Binding> {
    let mut screen = Screen::new()?;
    let draw = |screen: &mut Screen, status: &str| -> Result<()> {
        screen.terminal.draw(|frame| {
            let area = frame.area();
            frame.render_widget(Block::default().style(style(c)), area);
            let lines = banner(c, area.width, area.height);
            let chunks = Layout::vertical([
                Constraint::Length(lines.len() as u16 + 2),
                Constraint::Min(8),
                Constraint::Length(2),
            ])
            .split(area);
            frame.render_widget(
                Paragraph::new(lines).alignment(Alignment::Center),
                chunks[0],
            );
            frame.render_widget(
                Paragraph::new(format!(
                    "DETECT YOUR COPILOT KEY\n\n{status}\n\nCoconut listens for at most 60 seconds and does not save raw keyboard events."
                ))
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true })
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(Style::default().fg(color_at(0.47, c)))
                        .padding(Padding::new(2, 2, 2, 1)),
                ),
                centered(chunks[1], 84),
            );
            frame.render_widget(
                Paragraph::new("emergency stop: backspace + escape + enter")
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(Color::DarkGray)),
                centered(chunks[2], 84),
            );
        })?;
        Ok(())
    };
    draw(
        &mut screen,
        "Release every key, then press and release Copilot once.",
    )?;
    let binding = NativeInput.learn_with_updates(|name, binding| {
        let _ = draw(
            &mut screen,
            &format!(
                "Detected {name}\n{}\n\nRelease it, then press the same key once more to confirm.",
                crate::input::describe(&binding.keys)
            ),
        );
    });
    drop(screen);
    binding
}
pub fn setup(c: &mut Config) -> Result<()> {
    if hero_choose(
        c,
        "Turn the unused Copilot key into something useful.",
        &["Start setup", "Exit"],
    )?
    .unwrap_or(1)
        != 0
    {
        return Ok(());
    }
    crate::platform::input_platform_preflight()?;
    let service_problem = crate::service::probe().err();
    if service_problem.is_some() {
        let explanation = service_problem
            .map(|error| format!("Current service check: {error}"))
            .unwrap_or_default();
        if choose(c,&format!("Install or repair the system input service?\n{explanation}\n\nThe service handles keyboard events only. Actions always run as your user."),&["Install / repair","Cancel"])?.unwrap_or(1)!=0{return Ok(())}
        if !crate::platform::request_system_install()? {
            bail!("System installation failed")
        }
        crate::service::probe()
            .context("The repaired input service did not pass its health check")?;
    }
    let binding = detect(c)?;
    message(c,&format!("Confirmed: {}\nKeys: {}\nAn identical manually pressed chord also triggers this action.",binding.device,crate::input::describe(&binding.keys)))?;
    let mut draft = c.clone();
    draft.binding = Some(binding);
    draft.enabled = false;
    if !save_action(&mut draft)? {
        return Ok(());
    }
    if draft.active.is_none() {
        return Ok(());
    }
    let startup_message = if cfg!(target_os = "windows") {
        "Save this key mapping and enable it?\nLogin startup is enabled by default through your current-user Windows startup entry."
    } else {
        "Save this key mapping and enable it?\nLogin startup is enabled by default. On Hyprland, Coconut adds\na marked startup block to the standard config and keeps a backup."
    };
    if choose(&draft, startup_message, &["Save and enable", "Cancel"])?.unwrap_or(1) == 0 {
        draft.enabled = true;
        draft.save()?;
        NativeSession.autostart(draft.preferences.autostart)?;
        platform::ensure_agent()?;
        *c = draft;
        message(c,"Ready. Press Copilot to try your action.\nEmergency stop: Backspace + Escape + Enter.\nClosing this interface keeps the mapping active.")?
    }
    Ok(())
}
pub fn uninstall(c: &Config) -> Result<()> {
    if choose(
        c,
        "Remove Coconut system and login integration?",
        &["Cancel", "Remove integration"],
    )?
    .unwrap_or(0)
        != 1
    {
        return Ok(());
    }
    let mut disabled = c.clone();
    disabled.enabled = false;
    disabled.preferences.autostart = false;
    disabled.save()?;
    NativeSession.autostart(false)?;
    if !crate::platform::request_system_uninstall()? {
        bail!("System removal failed")
    };
    if choose(
        c,
        "Personal configuration",
        &["Keep saved actions and preferences", "Delete configuration"],
    )?
    .unwrap_or(0)
        == 1
    {
        std::fs::remove_file(config::config_path())?;
    }
    Ok(())
}
pub fn run() -> Result<()> {
    let mut c = Config::load()?;
    if c.binding.is_none() {
        setup(&mut c)?;
        c = Config::load()?;
        if c.binding.is_none() {
            return Ok(());
        }
    }
    loop {
        c = Config::load()?;
        let title = format!(
            "{}\nCopilot key  →  {}",
            if c.enabled {
                "● COCONUT IS READY"
            } else {
                "○ COCONUT IS PAUSED"
            },
            c.active.as_deref().unwrap_or("None")
        );
        let Some(i) = hero_choose(
            &c,
            &title,
            &[
                "Change action | Create and activate a new action",
                "Saved actions | Activate, edit, duplicate, or delete",
                "Test action | Run the active action now",
                "Detect key again | Learn a different physical key",
                "Pause / enable | Temporarily toggle the key mapping",
                "Preferences | Appearance, browser, terminal, and behavior",
                "Diagnostics | Inspect the service, session, and keyboards",
                "Uninstall integration | Remove system and login components",
                "Exit | Keep the background mapping running",
            ],
        )?
        else {
            return Ok(());
        };
        let result = match i {
            0 => save_action(&mut c).and_then(|saved| if saved { c.save() } else { Ok(()) }),
            1 => actions(&mut c),
            2 => runner::run_active(&c).and_then(|_| message(&c, "Launch requested.")),
            3 => detect(&c).and_then(|b| {
                c.binding = Some(b);
                c.save()
            }),
            4 => {
                c.enabled = !c.enabled;
                c.save().and_then(|_| platform::ensure_agent())
            }
            5 => preferences(&mut c),
            6 => message(&c, &crate::diagnostics()),
            7 => uninstall(&c),
            _ => return Ok(()),
        };
        if let Err(e) = result {
            message(&c, &format!("{e:#}"))?
        }
    }
}

fn edit_action(c: &mut Config, original: &Action) -> Result<Option<Action>> {
    let mut draft = original.clone();
    loop {
        let mut fields = vec![
            "Save changes".to_string(),
            "Replace action type".to_string(),
        ];
        match &draft {
            Action::Website { .. } => {
                fields.extend(["Website address", "Browser"].map(String::from))
            }
            Action::Browser { .. } => fields.push("Browser".into()),
            Action::Application { .. } => fields.push("Application".into()),
            Action::OpenPath { .. } => fields.push("File or folder path".into()),
            Action::Terminal => fields.push("Preferred terminal".into()),
            Action::Executable { .. } => fields.extend(
                [
                    "Executable",
                    "Arguments (JSON array)",
                    "Working directory",
                    "Show in terminal",
                    "Ignore while running",
                    "Environment (JSON object)",
                ]
                .map(String::from),
            ),
            Action::ShellCommand { .. } => fields.extend(
                [
                    "Shell",
                    "Command",
                    "Working directory",
                    "Show in terminal",
                    "Ignore while running",
                ]
                .map(String::from),
            ),
        }
        let Some(i) = menu(
            c,
            &format!("Edit action\n{}", serde_json::to_string_pretty(&draft)?),
            &fields,
        )?
        else {
            return Ok(None);
        };
        if i == 0 {
            runner::validate(&draft)?;
            return Ok(Some(draft));
        }
        if i == 1 {
            draft = new_action(c)?;
            continue;
        }
        match &mut draft {
            Action::Website { url, browser } => {
                if i == 2 {
                    *url = config::normalize_url(&required(c, "Website address", url)?)?
                } else {
                    *browser = pick_app(c, true)?
                }
            }
            Action::Browser { browser } => *browser = pick_app(c, true)?,
            Action::Application { id } => {
                *id = pick_app(c, false)?
                    .context("Use Replace action type to choose an executable")?;
            }
            Action::OpenPath { path } => {
                *path = PathBuf::from(required(c, "File or folder", &path.to_string_lossy())?);
                if !path.exists() {
                    bail!("Selected path does not exist")
                }
            }
            Action::Terminal => terminal_preference(c)?,
            Action::Executable {
                program,
                args,
                cwd,
                terminal,
                single,
                env,
            } => match i {
                2 => *program = required(c, "Executable", program)?,
                3 => {
                    *args = serde_json::from_str(&required(
                        c,
                        "Arguments as a JSON array",
                        &serde_json::to_string(args)?,
                    )?)?
                }
                4 => {
                    *cwd = PathBuf::from(required(c, "Working directory", &cwd.to_string_lossy())?)
                }
                5 => *terminal = !*terminal,
                6 => *single = !*single,
                _ => {
                    *env = serde_json::from_str(&required(
                        c,
                        "Environment as a JSON object",
                        &serde_json::to_string(env)?,
                    )?)?
                }
            },
            Action::ShellCommand {
                shell,
                command,
                cwd,
                terminal,
                single,
            } => match i {
                2 => *shell = required(c, "Shell", shell)?,
                3 => *command = required(c, "Shell command", command)?,
                4 => {
                    *cwd = PathBuf::from(required(c, "Working directory", &cwd.to_string_lossy())?)
                }
                5 => *terminal = !*terminal,
                _ => *single = !*single,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn responsive_banner() {
        let c = Config::default();
        let tiny = banner(&c, 32, 24);
        assert_eq!(tiny[0].to_string(), "COCONUT PILOT");
        let medium = banner(&c, 80, 24);
        assert_eq!(medium.len(), 6);
        assert!(medium.iter().all(|line| line.width() <= 80));
        let stacked = banner(&c, 80, 36);
        assert_eq!(stacked.len(), 13);
        assert!(stacked.iter().all(|line| line.width() <= 80));
        let large = banner(&c, 120, 36);
        assert_eq!(large.len(), 7);
        assert!(large.iter().all(|line| line.width() <= 120));
    }
    #[test]
    fn hidden_banner() {
        let mut c = Config::default();
        c.preferences.banner = "hidden".into();
        assert!(banner(&c, 120, 36).is_empty());
    }
}
