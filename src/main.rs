// ============================================================
//  wezfont - a font picker TUI for WezTerm (primary font + fallback chain)
//
//  Two pages, switch with Tab:
//    1. Primary  - Enter applies the selected font
//    2. Fallback - Enter adds/removes it from the fallback chain
//
//  Every change is written to ~/.wezterm-font.lua, then only the mtime of
//  ~/.wezterm.lua is bumped so WezTerm reloads and the preview pane
//  re-renders with the new font immediately (live preview).
// ============================================================
use std::fs;
use std::io;
use std::path::PathBuf;
use std::process::Command;

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{DefaultTerminal, Frame};

const SAMPLE: &[&str] = &[
    "The quick brown fox jumps over the lazy dog",
    "ABCDEFGHIJKLMNOPQRSTUVWXYZ 0123456789",
    "CJK: 中文测试 你好，世界。，。！？「」",
    "Box:      ─ │ ┌ ┐ └ ┘ █ ▓ ▒ ░ ▏",
    "Symbols:  ◆ ● ★ ☆ ✓ ✗ ❯ ❮ ⚡",
    "Nerd: \u{f07b} \u{e0a0} \u{f121} \u{e5ff} \u{f1b2} \u{f0e7} \u{f07c}",
    "Ligatures:-> => != === >= <= |> |> <|",
];

/// Locate the Windows home directory.
/// When launched from WSL, USERPROFILE can be empty, so fall back to
/// HOMEDRIVE+HOMEPATH, then ask cmd.exe, then the current directory.
fn home() -> PathBuf {
    if let Ok(p) = std::env::var("USERPROFILE") {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    let drive = std::env::var("HOMEDRIVE").unwrap_or_default();
    let path = std::env::var("HOMEPATH").unwrap_or_default();
    if !drive.is_empty() && !path.is_empty() {
        return PathBuf::from(format!("{drive}{path}"));
    }
    // last resort: ask cmd.exe (works through WSL interop)
    if let Ok(out) = Command::new("cmd.exe")
        .args(["/c", "echo", "%USERPROFILE%"])
        .output()
    {
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !s.is_empty() && s != "%USERPROFILE%" {
            return PathBuf::from(s);
        }
    }
    PathBuf::from(".")
}
fn cache_path() -> PathBuf {
    home().join(".wezterm-fontlist.txt")
}
fn font_lua_path() -> PathBuf {
    home().join(".wezterm-font.lua")
}
fn config_path() -> PathBuf {
    home().join(".wezterm.lua")
}

fn default_fallback() -> Vec<String> {
    vec!["Microsoft YaHei".into(), "Cascadia Code".into()]
}

/// Locate wezterm.exe. WEZTERM_EXECUTABLE is a Windows path and can be used
/// directly by this Windows process, both natively and from WSL.
fn find_wezterm_exe() -> PathBuf {
    if let Ok(p) = std::env::var("WEZTERM_EXECUTABLE") {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return pb;
        }
    }
    for guess in [
        r"D:\Program Files\WezTerm\wezterm.exe",
        r"C:\Program Files\WezTerm\wezterm.exe",
    ] {
        let pb = PathBuf::from(guess);
        if pb.exists() {
            return pb;
        }
    }
    PathBuf::from("wezterm.exe")
}

fn scan_fonts() -> io::Result<Vec<String>> {
    let out = Command::new(find_wezterm_exe())
        .args(["ls-fonts", "--list-system"])
        .output()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut list = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.split("wezterm.font(\"").nth(1) {
            if let Some(name) = rest.split('"').next() {
                if !name.is_empty() {
                    list.push(name.to_string());
                }
            }
        }
    }
    list.sort();
    list.dedup();
    Ok(list)
}

fn load_cached() -> Vec<String> {
    fs::read_to_string(cache_path())
        .map(|s| {
            s.lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Extract every double-quoted string from the text
fn quoted(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(start) = rest.find('"') {
        let after = &rest[start + 1..];
        match after.find('"') {
            Some(end) => {
                out.push(after[..end].to_string());
                rest = &after[end + 1..];
            }
            None => break,
        }
    }
    out
}

/// Read the saved file -> (primary, fallback chain)
fn load_saved() -> (Option<String>, Vec<String>) {
    match fs::read_to_string(font_lua_path()) {
        Ok(src) => parse_saved(&src),
        Err(_) => (None, default_fallback()),
    }
}

/// Parse the saved file (table format, or the legacy plain string)
fn parse_saved(src: &str) -> (Option<String>, Vec<String>) {
    let mut primary = None;
    let mut chain: Vec<String> = Vec::new();

    for line in src.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("font =") {
            if let Some(name) = quoted(rest).into_iter().next() {
                primary = Some(name);
            }
        } else if let Some(rest) = line.strip_prefix("fallback =") {
            chain = quoted(rest);
        } else if let Some(rest) = line.strip_prefix("return \"") {
            // legacy format: return "Font Name"
            primary = rest.split('"').next().map(|s| s.to_string());
        }
    }

    if chain.is_empty() {
        chain = default_fallback();
    }
    (primary, chain)
}

/// Write primary + fallback chain, then touch the main config so WezTerm reloads
fn render_saved(primary: Option<&str>, chain: &[String]) -> String {
    let mut body = String::from("-- written by wezfont (delete this file to go back to the config)\nreturn {\n");
    if let Some(p) = primary {
        body.push_str(&format!("  font = {:?},\n", p));
    }
    let items: Vec<String> = chain.iter().map(|c| format!("{:?}", c)).collect();
    body.push_str(&format!("  fallback = {{ {} }},\n}}\n", items.join(", ")));
    body
}

fn save_saved(primary: Option<&str>, chain: &[String]) -> io::Result<()> {
    fs::write(font_lua_path(), render_saved(primary, chain))?;

    let f = fs::OpenOptions::new().write(true).open(config_path())?;
    f.set_times(fs::FileTimes::new().set_modified(std::time::SystemTime::now()))?;
    Ok(())
}

#[derive(PartialEq, Clone, Copy)]
enum Page {
    Primary,
    Fallback,
}

struct App {
    all: Vec<String>,
    filtered: Vec<usize>,
    filter: String,
    sel: usize,
    primary: Option<String>,
    chain: Vec<String>,
    page: Page,
    status: String,
}

impl App {
    fn new() -> Self {
        let mut all = load_cached();
        let mut status = format!("Loaded {} fonts from cache (press r to rescan)", all.len());
        if all.is_empty() {
            match scan_fonts() {
                Ok(list) => {
                    all = list;
                    status = format!("Found {} fonts (cache written)", all.len());
                    let _ = fs::write(cache_path(), all.join("\n") + "\n");
                }
                Err(e) => status = format!("scan failed: {e}"),
            }
        }
        let (primary, chain) = load_saved();
        let mut app = App {
            all,
            filtered: Vec::new(),
            filter: String::new(),
            sel: 0,
            primary,
            chain,
            page: Page::Primary,
            status,
        };
        app.refilter();
        app
    }

    fn refilter(&mut self) {
        let needle = self.filter.to_lowercase();
        self.filtered = self
            .all
            .iter()
            .enumerate()
            .filter(|(_, name)| needle.is_empty() || name.to_lowercase().contains(&needle))
            .map(|(i, _)| i)
            .collect();
        if self.sel >= self.filtered.len() {
            self.sel = self.filtered.len().saturating_sub(1);
        }
    }

    fn selected_name(&self) -> Option<String> {
        self.filtered.get(self.sel).map(|i| self.all[*i].clone())
    }

    fn rescan(&mut self) {
        self.status = "Scanning (about 2s)...".into();
        match scan_fonts() {
            Ok(list) => {
                self.all = list;
                let _ = fs::write(cache_path(), self.all.join("\n") + "\n");
                self.status = format!("Found {} fonts", self.all.len());
                self.refilter();
            }
            Err(e) => self.status = format!("scan failed: {e}"),
        }
    }

    fn persist(&mut self, msg: String) {
        match save_saved(self.primary.as_deref(), &self.chain) {
            Ok(()) => self.status = msg,
            Err(e) => self.status = format!("write failed: {e}"),
        }
    }

    fn choose_primary(&mut self) {
        if let Some(name) = self.selected_name() {
            self.primary = Some(name.clone());
            self.persist(format!("primary -> {name} (WezTerm reloads automatically)"));
        }
    }

    fn toggle_fallback(&mut self) {
        let Some(name) = self.selected_name() else { return };
        if let Some(pos) = self.chain.iter().position(|c| *c == name) {
            self.chain.remove(pos);
            self.persist(format!("removed from fallback chain: {name}"));
        } else {
            self.chain.push(name.clone());
            self.persist(format!("added to fallback chain (#{}): {name}", self.chain.len()));
        }
    }

    fn move_in_chain(&mut self, up: bool) {
        let Some(name) = self.selected_name() else { return };
        let Some(pos) = self.chain.iter().position(|c| *c == name) else {
            self.status = "press Enter first to add it to the chain".into();
            return;
        };
        let new_pos = if up {
            pos.saturating_sub(1)
        } else {
            (pos + 1).min(self.chain.len() - 1)
        };
        if new_pos != pos {
            self.chain.swap(pos, new_pos);
            self.persist(format!("fallback chain: {} <-> {}", pos + 1, new_pos + 1));
        }
    }
}

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.is_empty() {
        std::process::exit(cli(&args));
    }

    let mut terminal = ratatui::init();
    let result = run(&mut terminal);
    ratatui::restore();
    result
}

/// Non-interactive CLI (scripts, WSL)
fn cli(args: &[String]) -> i32 {
    let mut i = 0;
    let mut primary_change: Option<String> = None;
    let mut fallback_change: Option<Vec<String>> = None;
    let mut code = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--help" | "-h" => {
                println!(
                    "wezfont - a font picker for WezTerm
                     
                     No arguments: open the TUI (Tab switches primary / fallback)
                     
                     Command line (scripts, WSL):
                     	--info                     show detected paths, font count, current font and chain
                     	--list                     list available fonts (one per line)
                     	--current                  print the current primary font
                     	--set <font>               set the primary font
                     	--fallback <A,B,...>       set the fallback chain (comma separated)
                     	--scan                     rescan system fonts and refresh the cache
                     	--help                     show this help"
                );
            }
            "--info" => {
                let (primary, chain) = load_saved();
                println!("wezfont {}", env!("CARGO_PKG_VERSION"));
                println!("  home dir     : {}", home().display());
                println!("  font cache   : {}", cache_path().display());
                println!("  saved file   : {}", font_lua_path().display());
                println!("  wezterm      : {}", find_wezterm_exe().display());
                println!("  fonts        : {}", load_cached().len());
                println!(
                    "  primary      : {}",
                    primary.unwrap_or_else(|| "(from config)".into())
                );
                println!("  fallback     : {}", chain.join(" → "));
                if std::env::var("WSL_DISTRO_NAME").is_ok()
                    || fs::read_to_string("/proc/version")
                        .map(|v| v.to_lowercase().contains("microsoft"))
                        .unwrap_or(false)
                {
                    println!("  environment  : WSL (fonts stay Windows-side, config goes to the Windows home)");
                }
            }
            "--list" => {
                let mut list = load_cached();
                if list.is_empty() {
                    match scan_fonts() {
                        Ok(l) => list = l,
                        Err(e) => {
                            eprintln!("scan failed: {e}");
                            code = 1;
                        }
                    }
                }
                for name in list {
                    println!("{name}");
                }
            }
            "--current" => match load_saved().0 {
                Some(p) => println!("{p}"),
                None => println!("(default from config)"),
            },
            "--set" => {
                i += 1;
                match args.get(i) {
                    Some(name) => primary_change = Some(name.clone()),
                    None => {
                        eprintln!("--set needs a font name");
                        code = 2;
                    }
                }
            }
            "--fallback" => {
                i += 1;
                match args.get(i) {
                    Some(list) => {
                        fallback_change = Some(
                            list.split(',')
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty())
                                .collect(),
                        );
                    }
                    None => {
                        eprintln!("--fallback needs a list like 'A,B'");
                        code = 2;
                    }
                }
            }
            "--scan" => match scan_fonts() {
                Ok(list) => {
                    let _ = fs::write(cache_path(), list.join("\n") + "\n");
                    println!("Found {} fonts", list.len());
                }
                Err(e) => {
                    eprintln!("scan failed: {e}");
                    code = 1;
                }
            },
            other => {
                eprintln!("unknown argument: {other} (see --help)");
                code = 2;
            }
        }
        i += 1;
    }

    if primary_change.is_some() || fallback_change.is_some() {
        let (cur_primary, cur_chain) = load_saved();
        let primary = primary_change.or(cur_primary);
        let chain = fallback_change.unwrap_or(cur_chain);
        match save_saved(primary.as_deref(), &chain) {
            Ok(()) => {
                println!(
                    "saved: primary = {}; fallback = {}",
                    primary.as_deref().unwrap_or("(default)"),
                    if chain.is_empty() {
                        "(empty)".to_string()
                    } else {
                        chain.join(" → ")
                    }
                );
            }
            Err(e) => {
                eprintln!("write failed: {e}");
                code = 1;
            }
        }
    }

    code
}

fn run(terminal: &mut DefaultTerminal) -> io::Result<()> {
    let mut app = App::new();
    loop {
        terminal.draw(|f| draw(f, &mut app))?;
        if !event::poll(std::time::Duration::from_millis(250))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        match key.code {
            KeyCode::Tab => {
                app.page = if app.page == Page::Primary {
                    Page::Fallback
                } else {
                    Page::Primary
                };
                app.status = match app.page {
                    Page::Primary => "page: primary font (Enter applies)".into(),
                    Page::Fallback => "page: fallback chain (Enter add/remove, Shift+J/K reorder)".into(),
                };
            }
            KeyCode::Esc => {
                if app.filter.is_empty() {
                    break;
                }
                app.filter.clear();
                app.refilter();
            }
            KeyCode::Char('q') if app.filter.is_empty() => break,
            KeyCode::Down => {
                if app.sel + 1 < app.filtered.len() {
                    app.sel += 1;
                }
            }
            KeyCode::Up => {
                app.sel = app.sel.saturating_sub(1);
            }
            KeyCode::PageDown => {
                app.sel = (app.sel + 15).min(app.filtered.len().saturating_sub(1));
            }
            KeyCode::PageUp => app.sel = app.sel.saturating_sub(15),
            KeyCode::Home => app.sel = 0,
            KeyCode::End => app.sel = app.filtered.len().saturating_sub(1),
            KeyCode::Char('J') => app.move_in_chain(false),
            KeyCode::Char('K') => app.move_in_chain(true),
            KeyCode::Enter => match app.page {
                Page::Primary => app.choose_primary(),
                Page::Fallback => app.toggle_fallback(),
            },
            KeyCode::Char('r') if app.filter.is_empty() => app.rescan(),
            KeyCode::Backspace => {
                app.filter.pop();
                app.refilter();
            }
            KeyCode::Char(c) if c != 'q' || !app.filter.is_empty() => {
                app.filter.push(c);
                app.refilter();
            }
            _ => {}
        }
    }
    Ok(())
}

fn draw(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(3),
            Constraint::Length(3),
        ])
        .split(f.area());

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
        .split(chunks[1]);

    // ---- header ----
    let page_label = match app.page {
        Page::Primary => Span::styled(
            " Primary ",
            Style::default().fg(Color::Black).bg(Color::LightBlue),
        ),
        Page::Fallback => Span::styled(
            " Fallback ",
            Style::default().fg(Color::Black).bg(Color::LightGreen),
        ),
    };
    let title = Paragraph::new(Line::from(vec![
        Span::styled(
            " wezfont ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::LightCyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        page_label,
        Span::raw("  primary: "),
        Span::styled(
            app.primary.clone().unwrap_or_else(|| "(from config)".into()),
            Style::default().fg(Color::LightGreen),
        ),
        Span::raw(format!("   ({}/{})", app.filtered.len(), app.all.len())),
    ]));
    f.render_widget(title, chunks[0]);

    // ---- left: font list ----
    let items: Vec<ListItem> = app
        .filtered
        .iter()
        .map(|i| {
            let name = &app.all[*i];
            let in_chain = app.chain.iter().position(|c| c == name);
            let is_primary = app.primary.as_deref() == Some(name.as_str());
            let mark = match (app.page, is_primary, in_chain) {
                (Page::Primary, true, _) => Span::styled("●", Style::default().fg(Color::LightGreen)),
                (Page::Fallback, _, Some(pos)) => Span::styled(
                    format!("{}", pos + 1),
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::LightGreen)
                        .add_modifier(Modifier::BOLD),
                ),
                _ => Span::raw(" "),
            };
            ListItem::new(Line::from(vec![
                mark,
                Span::raw(" "),
                Span::raw(name.clone()),
            ]))
        })
        .collect();

    let filter = if app.filter.is_empty() {
        "all".to_string()
    } else {
        format!("/{}", app.filter)
    };
    let left_title = match app.page {
        Page::Primary => format!(" Primary ({filter}) "),
        Page::Fallback => format!(" Fallback chain ({filter}) "),
    };
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(left_title))
        .highlight_style(
            Style::default()
                .bg(Color::LightBlue)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");
    let mut state = ListState::default();
    if !app.filtered.is_empty() {
        state.select(Some(app.sel));
    }
    f.render_stateful_widget(list, body[0], &mut state);

    // ---- right: preview + chain ----
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        app.primary.clone().unwrap_or_else(|| "(default)".into()),
        Style::default()
            .fg(Color::LightYellow)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));
    for s in SAMPLE {
        lines.push(Line::from(Span::raw(*s)));
        lines.push(Line::from(""));
    }
    lines.push(Line::from(Span::styled(
        "Fallback chain (tried in order, top first):",
        Style::default().fg(Color::DarkGray),
    )));
    if app.chain.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (empty - only the primary font, missing glyphs show as boxes)",
            Style::default().fg(Color::Red),
        )));
    } else {
        for (i, name) in app.chain.iter().enumerate() {
            lines.push(Line::from(Span::styled(
                format!("  {}. {}", i + 1, name),
                Style::default().fg(Color::Gray),
            )));
        }
    }

    let preview = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" Preview + fallback chain "))
        .wrap(Wrap { trim: false });
    f.render_widget(preview, body[1]);

    // ---- footer ----
    let help = Line::from(vec![
        Span::styled(" Tab ", Style::default().fg(Color::LightCyan)),
        Span::raw("page  "),
        Span::styled("↑↓ ", Style::default().fg(Color::LightBlue)),
        Span::raw("select  "),
        Span::styled("Enter ", Style::default().fg(Color::LightBlue)),
        Span::raw(match app.page {
            Page::Primary => "apply primary font  ",
            Page::Fallback => "add/remove fallback  ",
        }),
        Span::styled("Shift+J/K ", Style::default().fg(Color::LightBlue)),
        Span::raw("reorder chain  "),
        Span::styled("Esc ", Style::default().fg(Color::LightBlue)),
        Span::raw("clear filter/quit  "),
        Span::styled("r ", Style::default().fg(Color::LightBlue)),
        Span::raw("rescan  "),
        Span::styled("q ", Style::default().fg(Color::LightBlue)),
        Span::raw("quit"),
    ]);
    f.render_widget(Paragraph::new(help), chunks[2]);

    let status_area = Rect {
        x: chunks[2].x,
        y: chunks[2].y + 1,
        width: chunks[2].width,
        height: 1,
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!(" {}", app.status),
            Style::default().fg(Color::DarkGray),
        ))),
        status_area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_new_format() {
        let src = "-- x\nreturn {\n  font = \"Mono A\",\n  fallback = { \"YaHei\", \"Cascadia Code\" },\n}\n";
        let (primary, chain) = parse_saved(src);
        assert_eq!(primary.as_deref(), Some("Mono A"));
        assert_eq!(chain, vec!["YaHei".to_string(), "Cascadia Code".to_string()]);
    }

    #[test]
    fn parse_legacy_string() {
        let src = "-- old\nreturn \"Consolas\"\n";
        let (primary, chain) = parse_saved(src);
        assert_eq!(primary.as_deref(), Some("Consolas"));
        assert_eq!(chain, default_fallback());
    }

    #[test]
    fn round_trip() {
        let chain = vec!["Microsoft YaHei".to_string(), "Cascadia Code".to_string()];
        let body = render_saved(Some("0xProto Nerd Font Mono"), &chain);
        let (primary, parsed) = parse_saved(&body);
        assert_eq!(primary.as_deref(), Some("0xProto Nerd Font Mono"));
        assert_eq!(parsed, chain);
    }

    #[test]
    fn empty_chain_falls_back_to_default() {
        let body = render_saved(Some("X"), &[]);
        let (_, chain) = parse_saved(&body);
        assert_eq!(chain, default_fallback());
    }

    #[test]
    fn quoted_extracts_in_order() {
        assert_eq!(
            quoted("a \"one\" b \"two\""),
            vec!["one".to_string(), "two".to_string()]
        );
    }
}
