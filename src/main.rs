//! Herdr Overview: one popup with the agents that need you and every open todo.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

use ratatui::{
    DefaultTerminal, Frame,
    crossterm::event::{self, Event, KeyCode, KeyEventKind},
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, List, ListItem, ListState, Paragraph},
};
use serde_json::Value;

/// Ordered by urgency: what needs you first.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Status {
    Blocked,
    Done,
    Working,
    Idle,
    Unknown,
}

impl Status {
    const ALL: [Status; 5] = [
        Status::Blocked,
        Status::Done,
        Status::Working,
        Status::Idle,
        Status::Unknown,
    ];

    fn parse(s: &str) -> Self {
        match s {
            "blocked" => Status::Blocked,
            "done" => Status::Done,
            "working" => Status::Working,
            "idle" => Status::Idle,
            _ => Status::Unknown,
        }
    }

    // Same symbols and colors as Herdr's sidebar.
    fn icon(self) -> &'static str {
        match self {
            Status::Blocked => "×",
            Status::Done => "✓",
            Status::Working => "◐",
            Status::Idle => "○",
            Status::Unknown => "·",
        }
    }

    fn color(self) -> Color {
        match self {
            Status::Blocked => Color::Red,
            Status::Done => Color::Cyan,
            Status::Working => Color::Yellow,
            Status::Idle => Color::Green,
            Status::Unknown => Color::DarkGray,
        }
    }
}

#[derive(Debug)]
struct Agent {
    status: Status,
    place: String,
    title: String,
    pane_id: String,
}

#[derive(Debug, PartialEq)]
struct Todo {
    group: String,
    text: String,
}

const GROUP_COLORS: [Color; 8] = [
    Color::Magenta,
    Color::Blue,
    Color::Cyan,
    Color::Green,
    Color::Yellow,
    Color::Red,
    Color::LightMagenta,
    Color::LightBlue,
];

fn herdr_bin() -> String {
    std::env::var("HERDR_BIN_PATH")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "herdr".into())
}

fn herdr(args: &[&str]) -> Option<Value> {
    let out = Command::new(herdr_bin()).args(args).output().ok()?;
    let value: Value = serde_json::from_slice(&out.stdout).ok()?;
    value.get("result").cloned()
}

fn labels(result: Option<Value>, list: &str, id: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for item in result
        .as_ref()
        .and_then(|r| r.get(list))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let (Some(k), Some(v)) = (item[id].as_str(), item["label"].as_str()) {
            map.insert(k.to_string(), v.to_string());
        }
    }
    map
}

fn parse_agents(
    agents: &Value,
    workspaces: &HashMap<String, String>,
    tabs: &HashMap<String, String>,
) -> Vec<Agent> {
    let mut list: Vec<Agent> = agents["agents"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|a| {
            let s = |k: &str| a[k].as_str().unwrap_or_default().to_string();
            let ws = workspaces
                .get(&s("workspace_id"))
                .cloned()
                .unwrap_or_default();
            let tab = tabs.get(&s("tab_id")).cloned().unwrap_or_default();
            Agent {
                status: Status::parse(&s("agent_status")),
                place: format!("{ws} › {tab}"),
                title: s("terminal_title_stripped"),
                pane_id: s("pane_id"),
            }
        })
        .collect();
    list.sort_by(|a, b| (a.status, &a.place).cmp(&(b.status, &b.place)));
    list
}

fn load_agents() -> Vec<Agent> {
    let workspaces = labels(herdr(&["workspace", "list"]), "workspaces", "workspace_id");
    let tabs = labels(herdr(&["tab", "list"]), "tabs", "tab_id");
    herdr(&["agent", "list"])
        .map(|a| parse_agents(&a, &workspaces, &tabs))
        .unwrap_or_default()
}

/// An open Markdown checkbox. The group is its `#todo/<group>` tag, else "other".
fn parse_todo(line: &str) -> Option<Todo> {
    let line = line.trim_start();
    let rest = line
        .strip_prefix("- [ ] ")
        .or_else(|| line.strip_prefix("* [ ] "))?;
    let mut group = "other".to_string();
    let mut words = Vec::new();
    for word in rest.split_whitespace() {
        match word.strip_prefix("#todo/") {
            Some(tag) => group = tag.to_string(),
            None => words.push(word),
        }
    }
    let text = words.join(" ").replace("**", "");
    (!text.is_empty()).then_some(Todo { group, text })
}

/// Open todos in a Markdown file, skipping examples inside fenced code blocks.
fn parse_file(text: &str) -> Vec<Todo> {
    let mut in_code = false;
    text.lines()
        .filter(|line| {
            if line.trim_start().starts_with("```") {
                in_code = !in_code;
            }
            !in_code
        })
        .filter_map(parse_todo)
        .collect()
}

fn scan(dir: &Path, todos: &mut Vec<Todo>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        if path.is_dir() {
            scan(&path, todos);
        } else if path.extension().is_some_and(|e| e == "md")
            && let Ok(text) = fs::read_to_string(&path)
        {
            todos.extend(parse_file(&text));
        }
    }
}

fn config_path() -> Option<PathBuf> {
    std::env::var("HERDR_PLUGIN_CONFIG_DIR")
        .ok()
        .map(|d| PathBuf::from(d).join("config.toml"))
}

/// `[todos] dirs = [...]` from the plugin's config.toml, with `~` expanded.
fn todo_dirs(config: &str) -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    config
        .parse::<toml::Table>()
        .ok()
        .and_then(|t| t.get("todos")?.get("dirs")?.as_array().cloned())
        .into_iter()
        .flatten()
        .filter_map(|v| {
            v.as_str().map(|s| match s.strip_prefix("~/") {
                Some(rest) => Path::new(&home).join(rest),
                None => PathBuf::from(s),
            })
        })
        .collect()
}

/// Groups with their counts, biggest first.
fn groups(todos: &[Todo]) -> Vec<(String, usize)> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for t in todos {
        *counts.entry(&t.group).or_default() += 1;
    }
    let mut list: Vec<_> = counts
        .into_iter()
        .map(|(g, n)| (g.to_string(), n))
        .collect();
    list.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    list
}

#[derive(PartialEq)]
enum Focus {
    Agents,
    Todos,
}

struct App {
    agents: Vec<Agent>,
    todos: Vec<Todo>,
    groups: Vec<(String, usize)>,
    /// 0 shows every group, n shows `groups[n - 1]`.
    group: usize,
    focus: Focus,
    agent_list: ListState,
    todo_list: ListState,
    configured: bool,
}

impl App {
    fn load() -> Self {
        let config = config_path()
            .and_then(|p| fs::read_to_string(p).ok())
            .unwrap_or_default();
        let dirs = todo_dirs(&config);
        let mut todos = Vec::new();
        for dir in &dirs {
            scan(dir, &mut todos);
        }
        let mut app = App {
            agents: load_agents(),
            groups: groups(&todos),
            todos,
            group: 0,
            focus: Focus::Agents,
            agent_list: ListState::default().with_selected(Some(0)),
            todo_list: ListState::default().with_selected(Some(0)),
            configured: !dirs.is_empty(),
        };
        if app.agents.is_empty() {
            app.focus = Focus::Todos;
        }
        app
    }

    fn color_of(&self, group: &str) -> Color {
        let i = self
            .groups
            .iter()
            .position(|(g, _)| g == group)
            .unwrap_or(0);
        GROUP_COLORS[i % GROUP_COLORS.len()]
    }

    fn visible_todos(&self) -> Vec<&Todo> {
        match self.group {
            0 => self.todos.iter().collect(),
            n => self
                .todos
                .iter()
                .filter(|t| t.group == self.groups[n - 1].0)
                .collect(),
        }
    }

    fn step(&mut self, delta: isize) {
        let (state, len) = match self.focus {
            Focus::Agents => (&mut self.agent_list, self.agents.len()),
            Focus::Todos => {
                let len = self.visible_todos().len();
                (&mut self.todo_list, len)
            }
        };
        if len > 0 {
            let i = state.selected().unwrap_or(0) as isize + delta;
            state.select(Some(i.clamp(0, len as isize - 1) as usize));
        }
    }

    fn cycle_group(&mut self, delta: isize) {
        let n = self.groups.len() as isize + 1;
        self.group = (self.group as isize + delta).rem_euclid(n) as usize;
        self.todo_list.select(Some(0));
        self.focus = Focus::Todos;
    }
}

fn main() -> std::io::Result<()> {
    let mut terminal = ratatui::init();
    let result = run(&mut terminal);
    ratatui::restore();
    result
}

fn run(terminal: &mut DefaultTerminal) -> std::io::Result<()> {
    let mut app = App::load();
    let mut refreshed = Instant::now();
    loop {
        terminal.draw(|f| draw(f, &mut app))?;
        if refreshed.elapsed() > Duration::from_secs(2) {
            app.agents = load_agents();
            refreshed = Instant::now();
        }
        if !event::poll(Duration::from_millis(500))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
            KeyCode::Tab | KeyCode::BackTab => {
                app.focus = match app.focus {
                    Focus::Agents => Focus::Todos,
                    Focus::Todos => Focus::Agents,
                }
            }
            KeyCode::Char('j') | KeyCode::Down => app.step(1),
            KeyCode::Char('k') | KeyCode::Up => app.step(-1),
            KeyCode::Char('l') | KeyCode::Right => app.cycle_group(1),
            KeyCode::Char('h') | KeyCode::Left => app.cycle_group(-1),
            KeyCode::Char('r') => app = App::load(),
            KeyCode::Enter if app.focus == Focus::Agents => {
                if let Some(a) = app.agent_list.selected().and_then(|i| app.agents.get(i)) {
                    herdr(&["agent", "focus", &a.pane_id]);
                    return Ok(());
                }
            }
            _ => {}
        }
    }
}

fn panel(title: &str, focused: bool) -> Block<'_> {
    let color = if focused {
        Color::White
    } else {
        Color::DarkGray
    };
    Block::bordered()
        .title(format!(" {title} "))
        .border_style(Style::new().fg(color))
}

fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let [header, body] = Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(area);

    let mut spans = vec![Span::raw(" ")];
    for status in Status::ALL {
        let n = app.agents.iter().filter(|a| a.status == status).count();
        if n > 0 {
            spans.push(Span::styled(
                format!("{} {n}   ", status.icon()),
                Style::new().fg(status.color()).bold(),
            ));
        }
    }
    spans.push(Span::styled(
        format!("☐ {}", app.todos.len()),
        Style::new().bold(),
    ));
    f.render_widget(Line::from(spans), header);

    // A phone-width client stacks the panels instead of splitting them.
    let [left, right] = if area.width < 80 {
        let h = (app.agents.len() as u16 + 2).min(body.height / 2);
        Layout::vertical([Constraint::Length(h), Constraint::Fill(1)]).areas(body)
    } else {
        Layout::horizontal([Constraint::Percentage(50), Constraint::Fill(1)]).areas(body)
    };
    draw_agents(f, app, left);
    draw_todos(f, app, right);
}

fn draw_agents(f: &mut Frame, app: &mut App, area: Rect) {
    let items: Vec<ListItem> = app
        .agents
        .iter()
        .map(|a| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{} ", a.status.icon()),
                    Style::new().fg(a.status.color()),
                ),
                Span::raw(a.place.clone()).bold(),
                Span::raw(format!("  {}", a.title)).dark_gray(),
            ]))
        })
        .collect();
    let list = List::new(items)
        .block(panel("agents", app.focus == Focus::Agents))
        .highlight_style(Style::new().add_modifier(Modifier::REVERSED));
    f.render_stateful_widget(list, area, &mut app.agent_list);
}

fn draw_todos(f: &mut Frame, app: &mut App, area: Rect) {
    let block = panel("todo", app.focus == Focus::Todos);
    let inner = block.inner(area);
    f.render_widget(block, area);

    if !app.configured {
        let hint = format!(
            "set [todos] dirs in {}",
            config_path()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        );
        f.render_widget(Paragraph::new(hint).dark_gray(), inner);
        return;
    }

    let bars_height = (app.groups.len() as u16 + 1).min(inner.height / 3);
    let [bars, _, items] = Layout::vertical([
        Constraint::Length(bars_height),
        Constraint::Length(1),
        Constraint::Fill(1),
    ])
    .areas(inner);

    let max = app.groups.first().map(|g| g.1).unwrap_or(1).max(1);
    let name_width = app
        .groups
        .iter()
        .map(|g| g.0.len())
        .max()
        .unwrap_or(3)
        .max(3);
    let bar_width = (bars.width as usize).saturating_sub(name_width + 8).max(1);
    let all = std::iter::once(("all".to_string(), app.todos.len(), Color::White));
    let rows: Vec<Line> = all
        .chain(
            app.groups
                .iter()
                .map(|(g, n)| (g.clone(), *n, app.color_of(g))),
        )
        .enumerate()
        .map(|(i, (name, n, color))| {
            let len = if i == 0 {
                0
            } else {
                (n * bar_width).div_ceil(max)
            };
            let mut line = Line::from(vec![
                Span::raw(format!("{name:>name_width$} ")),
                Span::styled("▰".repeat(len), Style::new().fg(color)),
                Span::raw(format!(" {n}")).dark_gray(),
            ]);
            if i == app.group {
                line = line.add_modifier(Modifier::REVERSED);
            }
            line
        })
        .collect();
    let skip = app.group.saturating_sub(bars_height as usize - 1);
    f.render_widget(Paragraph::new(rows).scroll((skip as u16, 0)), bars);

    let list_items: Vec<ListItem> = app
        .visible_todos()
        .iter()
        .map(|t| {
            ListItem::new(Line::from(vec![
                Span::styled("☐ ", Style::new().fg(app.color_of(&t.group))),
                Span::raw(t.text.clone()),
            ]))
        })
        .collect();
    let mut list = List::new(list_items);
    if app.focus == Focus::Todos {
        list = list.highlight_style(Style::new().add_modifier(Modifier::REVERSED));
    }
    f.render_stateful_widget(list, items, &mut app.todo_list);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_todo_reads_open_checkboxes_and_their_group() {
        assert_eq!(
            parse_todo("  - [ ] **Book** the dentist #todo/home (2026-08-26)"),
            Some(Todo {
                group: "home".into(),
                text: "Book the dentist (2026-08-26)".into()
            })
        );
        assert_eq!(
            parse_todo("* [ ] Untagged task"),
            Some(Todo {
                group: "other".into(),
                text: "Untagged task".into()
            })
        );
        assert_eq!(parse_todo("- [x] Finished"), None);
        assert_eq!(parse_todo("- [ ] #todo/home"), None);
        assert_eq!(parse_todo("plain text"), None);
    }

    #[test]
    fn parse_file_skips_code_blocks() {
        let text = "- [ ] Real\n```markdown\n- [ ] Example #todo/<section>\n```\n- [ ] Also real";
        let texts: Vec<_> = parse_file(text).into_iter().map(|t| t.text).collect();
        assert_eq!(texts, vec!["Real", "Also real"]);
    }

    #[test]
    fn groups_sort_biggest_first() {
        let todo = |g: &str| Todo {
            group: g.into(),
            text: "x".into(),
        };
        let todos = [todo("b"), todo("a"), todo("a"), todo("c")];
        assert_eq!(
            groups(&todos),
            vec![("a".into(), 2), ("b".into(), 1), ("c".into(), 1)]
        );
    }

    #[test]
    fn todo_dirs_expands_home() {
        // SAFETY: tests in this crate do not read HOME concurrently.
        unsafe { std::env::set_var("HOME", "/home/u") };
        assert_eq!(
            todo_dirs("[todos]\ndirs = [\"~/notes\", \"/srv/a~b\"]"),
            vec![PathBuf::from("/home/u/notes"), PathBuf::from("/srv/a~b")]
        );
        assert!(todo_dirs("").is_empty());
    }

    #[test]
    fn agents_sort_by_urgency_with_workspace_and_tab_labels() {
        let agents = json!({"agents": [
            {"agent_status": "idle", "workspace_id": "w1", "tab_id": "t1", "pane_id": "p1", "terminal_title_stripped": "a"},
            {"agent_status": "blocked", "workspace_id": "w1", "tab_id": "t2", "pane_id": "p2", "terminal_title_stripped": "b"},
            {"agent_status": "done", "workspace_id": "w2", "tab_id": "t3", "pane_id": "p3", "terminal_title_stripped": "c"},
        ]});
        let workspaces = HashMap::from([("w1".into(), "api".into()), ("w2".into(), "web".into())]);
        let tabs = HashMap::from([
            ("t1".into(), "main".into()),
            ("t2".into(), "fix".into()),
            ("t3".into(), "docs".into()),
        ]);
        let list = parse_agents(&agents, &workspaces, &tabs);
        let order: Vec<_> = list
            .iter()
            .map(|a| (a.status, a.place.as_str(), a.pane_id.as_str()))
            .collect();
        assert_eq!(
            order,
            vec![
                (Status::Blocked, "api › fix", "p2"),
                (Status::Done, "web › docs", "p3"),
                (Status::Idle, "api › main", "p1"),
            ]
        );
    }
}
