// ============================================================
//  wezfont —— wezterm 字体选择 TUI（主字体 + fallback 链）
//
//  两个页面，Tab 切换：
//    1. 主字体  —— 选中回车即可应用
//    2. Fallback —— 回车把字体加入/移出回退链（中文、符号回退）
//
//  任何改动都会写 ~/.wezterm-font.lua，并只更新 ~/.wezterm.lua 的 mtime，
//  wezterm 监听到配置变化会自动重载 —— 右边样张当场变成新字体，即实时预览。
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
    "中文测试：你好，世界。全角标点：，。！？「」",
    "框线: ─ │ ┌ ┐ └ ┘ █ ▓ ▒ ░ ▏",
    "符号: ◆ ● ★ ☆ ✓ ✗ ❯ ❮ ⚡",
    "Nerd: \u{f07b} \u{e0a0} \u{f121} \u{e5ff} \u{f1b2} \u{f0e7} \u{f07c}",
    "连字: -> => != === >= <= |> |> <|",
];

/// 找 Windows 用户家目录。
/// 从 WSL 启动时 USERPROFILE 可能为空（WSL 不会设置它），
/// 依次退：USERPROFILE → HOMEDRIVE+HOMEPATH → 问一下 cmd.exe → 当前目录。
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
    // 最后手段：问 cmd.exe（WSL interop 下能拿到 Windows 侧的值）
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

/// 找 wezterm 可执行文件：从 WSL 启动时 WEZTERM_EXECUTABLE 是 Windows 路径，
/// Windows 进程可以直接用；原生运行时同样适用。
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

/// 从一段文本里抽出所有双引号字符串
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

/// 读保存文件 → (主字体, fallback 链)
fn load_saved() -> (Option<String>, Vec<String>) {
    match fs::read_to_string(font_lua_path()) {
        Ok(src) => parse_saved(&src),
        Err(_) => (None, default_fallback()),
    }
}

/// 解析保存文件的文本（新格式 table / 老格式字符串）
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
            // 老格式：return "字体名"
            primary = rest.split('"').next().map(|s| s.to_string());
        }
    }

    if chain.is_empty() {
        chain = default_fallback();
    }
    (primary, chain)
}

/// 写主字体 + fallback 链，并只更新主配置的 mtime 触发 wezterm 重载
fn render_saved(primary: Option<&str>, chain: &[String]) -> String {
    let mut body = String::from("-- 由 wezfont 写入（删掉本文件即恢复配置里的字体）\nreturn {\n");
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
        let mut status = format!("已从缓存加载 {} 个字体（按 r 重新扫描）", all.len());
        if all.is_empty() {
            match scan_fonts() {
                Ok(list) => {
                    all = list;
                    status = format!("扫描到 {} 个字体（已写入缓存）", all.len());
                    let _ = fs::write(cache_path(), all.join("\n") + "\n");
                }
                Err(e) => status = format!("扫描失败: {e}"),
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
        self.status = "正在扫描（约 2 秒）...".into();
        match scan_fonts() {
            Ok(list) => {
                self.all = list;
                let _ = fs::write(cache_path(), self.all.join("\n") + "\n");
                self.status = format!("扫描到 {} 个字体", self.all.len());
                self.refilter();
            }
            Err(e) => self.status = format!("扫描失败: {e}"),
        }
    }

    fn persist(&mut self, msg: String) {
        match save_saved(self.primary.as_deref(), &self.chain) {
            Ok(()) => self.status = msg,
            Err(e) => self.status = format!("写入失败: {e}"),
        }
    }

    fn choose_primary(&mut self) {
        if let Some(name) = self.selected_name() {
            self.primary = Some(name.clone());
            self.persist(format!("主字体 → {name}（wezterm 会自动重载）"));
        }
    }

    fn toggle_fallback(&mut self) {
        let Some(name) = self.selected_name() else { return };
        if let Some(pos) = self.chain.iter().position(|c| *c == name) {
            self.chain.remove(pos);
            self.persist(format!("已从回退链移除：{name}"));
        } else {
            self.chain.push(name.clone());
            self.persist(format!("已加入回退链（第 {} 位）：{name}", self.chain.len()));
        }
    }

    fn move_in_chain(&mut self, up: bool) {
        let Some(name) = self.selected_name() else { return };
        let Some(pos) = self.chain.iter().position(|c| *c == name) else {
            self.status = "先按 Enter 把它加入回退链".into();
            return;
        };
        let new_pos = if up {
            pos.saturating_sub(1)
        } else {
            (pos + 1).min(self.chain.len() - 1)
        };
        if new_pos != pos {
            self.chain.swap(pos, new_pos);
            self.persist(format!("回退链第 {} 位 ↔ 第 {} 位", pos + 1, new_pos + 1));
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

/// 非交互（脚本 / WSL）用的命令行接口
fn cli(args: &[String]) -> i32 {
    let mut i = 0;
    let mut primary_change: Option<String> = None;
    let mut fallback_change: Option<Vec<String>> = None;
    let mut code = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--help" | "-h" => {
                println!(
                    "wezfont —— wezterm 字体选择器\n\
                     \n\
                     不带参数运行：打开 TUI（Tab 切主字体 / Fallback 页）\n\
                     \n\
                     命令行模式（适合脚本 / WSL）：\n\
                     \t--info                     显示检测到的路径、字体数、当前主字体和回退链\n\
                     \t--list                     列出可用字体（一行一个）\n\
                     \t--current                  打印当前主字体\n\
                     \t--set <字体名>             设主字体\n\
                     \t--fallback <A,B,...>       设回退链（逗号分隔，引号包住）\n\
                     \t--scan                     重新扫描系统字体并更新缓存\n\
                     \t--help                     显示本帮助"
                );
            }
            "--info" => {
                let (primary, chain) = load_saved();
                println!("wezfont {}", env!("CARGO_PKG_VERSION"));
                println!("  家目录       : {}", home().display());
                println!("  字体缓存     : {}", cache_path().display());
                println!("  保存文件     : {}", font_lua_path().display());
                println!("  wezterm      : {}", find_wezterm_exe().display());
                println!("  字体数       : {}", load_cached().len());
                println!(
                    "  主字体       : {}",
                    primary.unwrap_or_else(|| "(配置里的默认)".into())
                );
                println!("  回退链       : {}", chain.join(" → "));
                if std::env::var("WSL_DISTRO_NAME").is_ok()
                    || fs::read_to_string("/proc/version")
                        .map(|v| v.to_lowercase().contains("microsoft"))
                        .unwrap_or(false)
                {
                    println!("  运行环境     : WSL（字体依然是 Windows 侧的，配置写回 Windows 家目录）");
                }
            }
            "--list" => {
                let mut list = load_cached();
                if list.is_empty() {
                    match scan_fonts() {
                        Ok(l) => list = l,
                        Err(e) => {
                            eprintln!("扫描失败: {e}");
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
                None => println!("(配置里的默认字体)"),
            },
            "--set" => {
                i += 1;
                match args.get(i) {
                    Some(name) => primary_change = Some(name.clone()),
                    None => {
                        eprintln!("--set 需要字体名");
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
                        eprintln!("--fallback 需要 'A,B' 这样的字体名列表");
                        code = 2;
                    }
                }
            }
            "--scan" => match scan_fonts() {
                Ok(list) => {
                    let _ = fs::write(cache_path(), list.join("\n") + "\n");
                    println!("已扫描到 {} 个字体", list.len());
                }
                Err(e) => {
                    eprintln!("扫描失败: {e}");
                    code = 1;
                }
            },
            other => {
                eprintln!("未知参数: {other}（用 --help 看用法）");
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
                    "已保存：主字体 = {}；回退链 = {}",
                    primary.as_deref().unwrap_or("(默认)"),
                    if chain.is_empty() {
                        "(空)".to_string()
                    } else {
                        chain.join(" → ")
                    }
                );
            }
            Err(e) => {
                eprintln!("写入失败: {e}");
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
                    Page::Primary => "页面：主字体（Enter 应用）".into(),
                    Page::Fallback => "页面：Fallback 回退链（Enter 加入/移除，Shift+J/K 调顺序）".into(),
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

    // ---- 顶部：标题 + 当前状态 ----
    let page_label = match app.page {
        Page::Primary => Span::styled(
            " 主字体 ",
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
        Span::raw("  主字体: "),
        Span::styled(
            app.primary.clone().unwrap_or_else(|| "(配置里的默认)".into()),
            Style::default().fg(Color::LightGreen),
        ),
        Span::raw(format!("   ({}/{})", app.filtered.len(), app.all.len())),
    ]));
    f.render_widget(title, chunks[0]);

    // ---- 左：字体列表 ----
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
        "全部".to_string()
    } else {
        format!("/{}", app.filter)
    };
    let left_title = match app.page {
        Page::Primary => format!(" 主字体 ({filter}) "),
        Page::Fallback => format!(" Fallback 回退链 ({filter}) "),
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

    // ---- 右：样张 + 回退链 ----
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        app.primary.clone().unwrap_or_else(|| "(默认字体)".into()),
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
        "回退链（按顺序尝试，越前优先级越高）:",
        Style::default().fg(Color::DarkGray),
    )));
    if app.chain.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (空 —— 只有主字体，缺字会显示方块)",
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
        .block(Block::default().borders(Borders::ALL).title(" 样张 + 回退链 "))
        .wrap(Wrap { trim: false });
    f.render_widget(preview, body[1]);

    // ---- 底部：帮助 + 状态 ----
    let help = Line::from(vec![
        Span::styled(" Tab ", Style::default().fg(Color::LightCyan)),
        Span::raw("切页  "),
        Span::styled("↑↓ ", Style::default().fg(Color::LightBlue)),
        Span::raw("选择  "),
        Span::styled("Enter ", Style::default().fg(Color::LightBlue)),
        Span::raw(match app.page {
            Page::Primary => "应用主字体  ",
            Page::Fallback => "加入/移除回退  ",
        }),
        Span::styled("Shift+J/K ", Style::default().fg(Color::LightBlue)),
        Span::raw("调回退顺序  "),
        Span::styled("Esc ", Style::default().fg(Color::LightBlue)),
        Span::raw("清过滤/退出  "),
        Span::styled("r ", Style::default().fg(Color::LightBlue)),
        Span::raw("重扫  "),
        Span::styled("q ", Style::default().fg(Color::LightBlue)),
        Span::raw("退出"),
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
