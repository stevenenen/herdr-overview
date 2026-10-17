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
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};
use serde_json::Value;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

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
            Status::Blocked => palette::RED,
            Status::Done => palette::TEAL,
            Status::Working => palette::YELLOW,
            Status::Idle => palette::GREEN,
            Status::Unknown => palette::SUBTEXT,
        }
    }
}

#[derive(Debug)]
struct Agent {
    status: Status,
    workspace: String,
    tab: String,
    title: String,
    pane_id: String,
}

#[derive(Debug, PartialEq, Default, Clone)]
struct Todo {
    group: String,
    /// The project it belongs to: the folder or note one level below the area folder.
    topic: String,
    text: String,
    /// The note it came from, and its 1-based line there.
    file: PathBuf,
    line: usize,
}

const GROUP_COLORS: [Color; 6] = [
    palette::MAUVE,
    palette::ACCENT,
    palette::PEACH,
    palette::GREEN,
    palette::TEAL,
    palette::YELLOW,
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

/// Most recently focused first; agents never focused follow, by urgency.
fn parse_agents(
    agents: &Value,
    workspaces: &HashMap<String, String>,
    tabs: &HashMap<String, String>,
    focused: &HashMap<String, u64>,
) -> Vec<Agent> {
    let mut list: Vec<Agent> = agents["agents"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|a| {
            let s = |k: &str| a[k].as_str().unwrap_or_default().to_string();
            let label =
                |map: &HashMap<String, String>, k| map.get(&s(k)).cloned().unwrap_or_default();
            Agent {
                status: Status::parse(&s("agent_status")),
                workspace: label(workspaces, "workspace_id"),
                tab: label(tabs, "tab_id"),
                title: s("terminal_title_stripped"),
                pane_id: s("pane_id"),
            }
        })
        .collect();
    let last = |a: &Agent| std::cmp::Reverse(focused.get(&a.pane_id).copied().unwrap_or(0));
    list.sort_by(|a, b| {
        (last(a), a.status, &a.workspace, &a.tab).cmp(&(last(b), b.status, &b.workspace, &b.tab))
    });
    list
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// When each pane was last focused, in `$HERDR_PLUGIN_STATE_DIR/focus.json`.
fn focus_path() -> Option<PathBuf> {
    std::env::var("HERDR_PLUGIN_STATE_DIR")
        .ok()
        .map(|d| PathBuf::from(d).join("focus.json"))
}

fn load_focus() -> HashMap<String, u64> {
    focus_path()
        .and_then(|p| fs::read(p).ok())
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| {
            v.as_object().map(|o| {
                o.iter()
                    .filter_map(|(k, v)| Some((k.clone(), v.as_u64()?)))
                    .collect()
            })
        })
        .unwrap_or_default()
}

/// The `pane.focused` hook: stamp the pane with the current time.
// ponytail: unlocked read-modify-write; two focus events in the same instant can drop one stamp.
fn track_focus() {
    let Some(pane) = std::env::var("HERDR_PLUGIN_EVENT_JSON")
        .ok()
        .and_then(|e| serde_json::from_str::<Value>(&e).ok())
        .and_then(|v| v["data"]["pane_id"].as_str().map(str::to_string))
    else {
        return;
    };
    let mut focus = load_focus();
    focus.insert(pane, now_millis());
    if let Some(p) = focus_path() {
        let _ = fs::write(p, serde_json::to_string(&focus).unwrap_or_default());
    }
}

fn load_agents() -> Vec<Agent> {
    let workspaces = labels(herdr(&["workspace", "list"]), "workspaces", "workspace_id");
    let tabs = labels(herdr(&["tab", "list"]), "tabs", "tab_id");
    let focused = load_focus();
    herdr(&["agent", "list"])
        .map(|a| parse_agents(&a, &workspaces, &tabs, &focused))
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
    (!text.is_empty()).then_some(Todo {
        group,
        text,
        ..Todo::default()
    })
}

/// Open todos in a Markdown file, skipping examples inside fenced code blocks.
fn parse_file(text: &str) -> Vec<Todo> {
    let mut in_code = false;
    text.lines()
        .enumerate()
        .filter(|(_, line)| {
            if line.trim_start().starts_with("```") {
                in_code = !in_code;
            }
            !in_code
        })
        .filter_map(|(i, line)| {
            let mut todo = parse_todo(line)?;
            todo.line = i + 1;
            Some(todo)
        })
        .collect()
}

/// Area and topic from a note's path below the scanned folder. The area is the first word of
/// the top folder (`Houses/…` → `houses`); the topic is the next folder, or the note itself.
/// A `#todo/<tag>` only names the area for notes that sit directly in the scanned folder.
fn place(relative: &Path, tag: &str) -> (String, String) {
    let parts: Vec<String> = relative
        .with_extension("")
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    let first_word = |s: &str| s.split_whitespace().next().unwrap_or(s).to_lowercase();
    match parts.as_slice() {
        [note] => (tag.to_string(), note.clone()),
        [area, note] if note.eq_ignore_ascii_case("todo") => (first_word(area), "inbox".into()),
        [area, topic, ..] => (first_word(area), topic.clone()),
        [] => (tag.to_string(), String::new()),
    }
}

fn scan(root: &Path, dir: &Path, todos: &mut Vec<Todo>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        if path.is_dir() {
            scan(root, &path, todos);
        } else if path.extension().is_some_and(|e| e == "md")
            && let Ok(text) = fs::read_to_string(&path)
        {
            let relative = path.strip_prefix(root).unwrap_or(&path);
            todos.extend(parse_file(&text).into_iter().map(|mut t| {
                (t.group, t.topic) = place(relative, &t.group);
                t.file = path.clone();
                t
            }));
        }
    }
}

fn config_path() -> Option<PathBuf> {
    std::env::var("HERDR_PLUGIN_CONFIG_DIR")
        .ok()
        .map(|d| PathBuf::from(d).join("config.toml"))
}

/// `[<section>] dirs = [...]` from the plugin's config.toml, with `~` expanded.
fn config_dirs(config: &str, section: &str) -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    config
        .parse::<toml::Table>()
        .ok()
        .and_then(|t| t.get(section)?.get("dirs")?.as_array().cloned())
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

/// What the overview remembers per todo: its Claude tab and whether it is crossed out.
/// Keyed by the todo text, stored in `$HERDR_PLUGIN_STATE_DIR/state.json`.
#[derive(Default)]
struct Memory {
    tabs: HashMap<String, String>,
    crossed: Vec<String>,
    /// When each todo was last opened or crossed, so recent ones sort first.
    used: HashMap<String, u64>,
}

impl Memory {
    fn path() -> Option<PathBuf> {
        std::env::var("HERDR_PLUGIN_STATE_DIR")
            .ok()
            .map(|d| PathBuf::from(d).join("state.json"))
    }

    fn from_json(v: &Value) -> Self {
        let tabs = v["tabs"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
            .collect();
        let crossed = v["crossed"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| Some(v.as_str()?.to_string()))
            .collect();
        let used = v["used"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(k, v)| Some((k.clone(), v.as_u64()?)))
            .collect();
        Memory {
            tabs,
            crossed,
            used,
        }
    }

    fn load() -> Self {
        Self::path()
            .and_then(|p| fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .map(|v| Self::from_json(&v))
            .unwrap_or_default()
    }

    fn save(&self) {
        if let Some(p) = Self::path() {
            let v = serde_json::json!({ "tabs": self.tabs, "crossed": self.crossed, "used": self.used });
            let _ = fs::write(p, v.to_string());
        }
    }

    fn touch(&mut self, text: &str) {
        self.used.insert(text.to_string(), now_millis());
    }

    fn toggle_crossed(&mut self, text: &str) {
        self.touch(text);
        match self.crossed.iter().position(|t| t == text) {
            Some(i) => {
                self.crossed.remove(i);
            }
            None => self.crossed.push(text.to_string()),
        }
    }
}

/// `pane run` types into the shell, so the todo goes in single quotes.
fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// Focuses the todo's Claude tab, or opens one when it has none (or it was closed).
fn open_todo(memory: &mut Memory, todo: &Todo) {
    if let Some(tab) = memory.tabs.get(&todo.text)
        && herdr(&["tab", "focus", tab]).is_some()
    {
        memory.touch(&todo.text);
        memory.save();
        return;
    }
    let label: String = todo.text.chars().take(30).collect();
    let Some(created) = herdr(&["tab", "create", "--label", &label, "--focus"]) else {
        return;
    };
    let (Some(tab), Some(pane)) = (
        created["tab"]["tab_id"].as_str(),
        created["root_pane"]["pane_id"].as_str(),
    ) else {
        return;
    };
    let _ = Command::new(herdr_bin())
        .args([
            "pane",
            "run",
            pane,
            &format!("claude {}", shell_quote(&todo.text)),
        ])
        .output();
    memory.tabs.insert(todo.text.clone(), tab.to_string());
    memory.touch(&todo.text);
    memory.save();
}

/// An issue or pull request link: `https://github.com/<owner>/<repo>/(issues|pull)/<n>...`.
#[derive(Debug, PartialEq)]
struct GithubLink {
    repo: String,
    pull: bool,
    number: u64,
}

fn parse_link(url: &str) -> Option<GithubLink> {
    let url = url.trim();
    let url = url.split_once("://").map_or(url, |(_, rest)| rest);
    let url = url.strip_prefix("www.").unwrap_or(url);
    let mut parts = url.strip_prefix("github.com/")?.split('/');
    let (owner, repo, kind, number) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    let digits: String = number.chars().take_while(char::is_ascii_digit).collect();
    Some(GithubLink {
        repo: format!("{owner}/{repo}").to_lowercase(),
        pull: match kind {
            "issues" => false,
            "pull" => true,
            _ => return None,
        },
        number: digits.parse().ok()?,
    })
}

/// `owner/repo` from a GitHub remote URL, over HTTPS or SSH (any `github.com*` host alias).
fn remote_repo(remote: &str) -> Option<String> {
    let remote = remote.trim();
    if !remote.contains("github.com") {
        return None;
    }
    let path = match remote.split_once("://") {
        Some((_, rest)) => rest.split_once('/')?.1,
        None => remote.split_once(':')?.1,
    };
    Some(
        path.trim_end_matches('/')
            .trim_end_matches(".git")
            .to_lowercase(),
    )
}

/// The main clone of `repo` among the folders directly inside `dirs`.
fn find_clone(dirs: &[PathBuf], repo: &str) -> Option<PathBuf> {
    dirs.iter()
        .filter_map(|d| fs::read_dir(d).ok())
        .flatten()
        .flatten()
        .map(|e| e.path())
        // A main clone has a `.git` folder; a linked worktree only has a `.git` file.
        .filter(|p| p.join(".git").is_dir())
        .find(|p| {
            git(p, &["remote", "get-url", "origin"])
                .and_then(|url| remote_repo(&url))
                .as_deref()
                == Some(repo)
        })
}

/// Runs git in `dir`. Credential prompts are off: they would draw over the popup.
fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// `issue-874-prove-a-customer-can-only-see`: the number plus the title's first words.
fn branch_name(number: u64, title: &str) -> String {
    let mut name = format!("issue-{number}");
    let lower = title.to_lowercase();
    for word in lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
    {
        if name.len() + word.len() + 1 > 48 {
            break;
        }
        name = format!("{name}-{word}");
    }
    name
}

/// Herdr's JSON result, or its error message.
fn herdr_result(args: &[&str]) -> Result<Value, String> {
    let out = Command::new(herdr_bin())
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    let value: Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
    if out.status.success()
        && let Some(result) = value.get("result")
    {
        return Ok(result.clone());
    }
    Err(value["error"]["message"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| String::from_utf8_lossy(&out.stderr).trim().to_string()))
}

/// Where Herdr puts a new worktree: `<worktrees.directory>/<repo folder>/<branch slug>`.
/// Opening by this path still finds it after the branch inside has been switched.
fn worktree_path(clone: &Path, branch: &str) -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    let configured = std::env::var("HERDR_CONFIG_PATH")
        .ok()
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(&home).join(".config/herdr/config.toml"));
    let dir = fs::read_to_string(configured)
        .ok()
        .and_then(|c| c.parse::<toml::Table>().ok())
        .and_then(|t| Some(t.get("worktrees")?.get("directory")?.as_str()?.to_string()))
        .unwrap_or_else(|| "~/.herdr/worktrees".into());
    let dir = match dir.strip_prefix("~/") {
        Some(rest) => Path::new(&home).join(rest),
        None => PathBuf::from(dir),
    };
    let repo = clone.file_name().unwrap_or_default();
    dir.join(repo).join(branch.replace('/', "-"))
}

/// The open workspace of the clone itself. Herdr only starts worktrees from the repo's own
/// workspace, and resolving `--cwd` can land on a linked worktree of it instead.
fn source_workspace(clone: &Path) -> Option<String> {
    let clone = fs::canonicalize(clone).ok()?;
    herdr(&["workspace", "list"])?["workspaces"]
        .as_array()?
        .iter()
        .find(|w| {
            let wt = &w["worktree"];
            wt["is_linked_worktree"] == false
                && wt["checkout_path"]
                    .as_str()
                    .and_then(|p| fs::canonicalize(p).ok())
                    .as_ref()
                    == Some(&clone)
        })
        .and_then(|w| w["workspace_id"].as_str().map(str::to_string))
}

/// Opens the worktree for an issue or pull request link, with Claude started on it.
/// Pasting the same link again reopens that worktree instead of creating another.
fn open_link(url: &str, repo_dirs: &[PathBuf]) -> Result<(), String> {
    let link = parse_link(url).ok_or("not a GitHub issue or pull request link")?;
    if repo_dirs.is_empty() {
        return Err(format!(
            "set [repos] dirs in {}",
            config_path()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        ));
    }
    let clone = find_clone(repo_dirs, &link.repo)
        .ok_or_else(|| format!("no clone of {} in [repos] dirs", link.repo))?;
    let (kind, fields) = if link.pull {
        ("pr", "title,headRefName")
    } else {
        ("issue", "title")
    };
    let number = link.number.to_string();
    let out = Command::new("gh")
        .args([kind, "view", &number, "-R", &link.repo, "--json", fields])
        .output()
        .map_err(|e| format!("gh: {e}"))?;
    let info: Value = serde_json::from_slice(&out.stdout)
        .map_err(|_| format!("gh: {}", String::from_utf8_lossy(&out.stderr).trim()))?;
    let title = info["title"].as_str().unwrap_or_default();
    let branch = match info["headRefName"].as_str() {
        Some(head) => head.to_string(),
        None => branch_name(link.number, title),
    };
    let label: String = format!("#{} {title}", link.number)
        .chars()
        .take(40)
        .collect();
    let cwd = clone.to_string_lossy().to_string();
    let (flag, value) = match source_workspace(&clone) {
        Some(id) => ("--workspace", id),
        None => ("--cwd", cwd),
    };
    let source = [flag, value.as_str()];

    let path = worktree_path(&clone, &branch);
    if path.exists() {
        let path = path.to_string_lossy();
        let reopen = [
            "worktree", "open", source[0], source[1], "--path", &path, "--label", &label, "--focus",
        ];
        return herdr_result(&reopen).map(|_| ());
    }
    let base = if link.pull {
        git(
            &clone,
            &["fetch", "origin", &format!("pull/{number}/head:{branch}")],
        );
        None
    } else {
        let base = git(
            &clone,
            &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
        )
        .unwrap_or_else(|| "origin/main".into());
        git(
            &clone,
            &["fetch", "origin", base.trim_start_matches("origin/")],
        );
        Some(base)
    };
    let mut args = vec![
        "worktree", "create", source[0], source[1], "--branch", &branch, "--label", &label,
        "--focus",
    ];
    if let Some(base) = &base {
        args.extend(["--base", base]);
    }
    let created = herdr_result(&args)?;
    if let Some(tab) = created["tab"]["tab_id"].as_str() {
        herdr(&["tab", "rename", tab, &label]);
    }
    if let Some(pane) = created["root_pane"]["pane_id"].as_str() {
        let _ = Command::new(herdr_bin())
            .args([
                "pane",
                "run",
                pane,
                &format!("claude {}", shell_quote(url.trim())),
            ])
            .output();
    }
    Ok(())
}

/// Inserts `- [ ] <text>` right after `after`'s line in its note, with the same indent and
/// `#todo/` tag, and returns the new file contents.
fn insert_todo(note: &str, after: &Todo, text: &str) -> String {
    let mut lines: Vec<&str> = note.lines().collect();
    let anchor = lines
        .get(after.line.saturating_sub(1))
        .copied()
        .unwrap_or("");
    let indent: String = anchor.chars().take_while(|c| c.is_whitespace()).collect();
    let bullet = if anchor.trim_start().starts_with('*') {
        "*"
    } else {
        "-"
    };
    let tag = anchor
        .split_whitespace()
        .find(|w| w.starts_with("#todo/"))
        .map(|t| format!(" {t}"))
        .unwrap_or_default();
    let new = format!("{indent}{bullet} [ ] {}{tag}", text.trim());
    lines.insert(after.line.min(lines.len()), &new);
    let mut out = lines.join("\n");
    if note.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// The Obsidian vaults this machine knows, from Obsidian's own `obsidian.json`.
fn obsidian_vaults() -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    let config = if cfg!(target_os = "macos") {
        "Library/Application Support/obsidian/obsidian.json"
    } else {
        ".config/obsidian/obsidian.json"
    };
    fs::read(Path::new(&home).join(config))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| {
            v["vaults"].as_object().map(|vaults| {
                vaults
                    .values()
                    .filter_map(|v| v["path"].as_str().map(PathBuf::from))
                    .collect()
            })
        })
        .unwrap_or_default()
}

/// `obsidian://open?vault=<name>&file=<path in vault>` when `file` sits in one of `vaults`.
fn obsidian_url(file: &Path, vaults: &[PathBuf]) -> Option<String> {
    let (vault, inside) = vaults
        .iter()
        .find_map(|v| Some((v, file.strip_prefix(v).ok()?)))?;
    let name = vault.file_name()?.to_string_lossy();
    let inside = inside.with_extension("");
    Some(format!(
        "obsidian://open?vault={}&file={}",
        percent_encode(&name),
        percent_encode(&inside.to_string_lossy())
    ))
}

fn percent_encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Opens a note that lives in an Obsidian vault in Obsidian. Returns false when it does not.
fn open_in_obsidian(todo: &Todo) -> bool {
    let Some(url) = obsidian_url(&todo.file, &obsidian_vaults()) else {
        return false;
    };
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    Command::new(opener)
        .arg(url)
        .status()
        .is_ok_and(|s| s.success())
}

/// Opens `todo`'s note at its line in `$VISUAL`, `$EDITOR` or `vi`, and waits for it.
fn edit_source(todo: &Todo) {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .ok()
        .filter(|e| !e.is_empty())
        .unwrap_or_else(|| "vi".into());
    // `+<line>` is understood by vi, vim, nvim, nano, micro, helix and emacs.
    let _ = Command::new("sh")
        .arg("-c")
        .arg(format!("{editor} \"+$1\" \"$2\""))
        .arg("sh")
        .arg(todo.line.to_string())
        .arg(&todo.file)
        .status();
}

/// Groups with their counts, biggest first.
/// Case-insensitive: every word of the filter appears in one of the fields.
fn matches(filter: &str, fields: &[&str]) -> bool {
    let haystack = fields.join(" ").to_lowercase();
    filter
        .to_lowercase()
        .split_whitespace()
        .all(|word| haystack.contains(word))
}

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
    /// Scroll position of the todo list, which also holds topic header rows.
    todo_offset: usize,
    configured: bool,
    memory: Memory,
    /// Per-panel `/` filters, and whether keys are typing into the focused one.
    agent_filter: String,
    todo_filter: String,
    typing: bool,
    /// The GitHub link being typed after `u`, and the last message about it.
    link: Option<String>,
    notice: Option<String>,
    repo_dirs: Vec<PathBuf>,
    /// The todo being typed after `a`, added below the selected one.
    adding: Option<String>,
}

impl App {
    fn load() -> Self {
        let config = config_path()
            .and_then(|p| fs::read_to_string(p).ok())
            .unwrap_or_default();
        let dirs = config_dirs(&config, "todos");
        let mut todos = Vec::new();
        for dir in &dirs {
            scan(dir, dir, &mut todos);
        }
        let mut app = App {
            agents: load_agents(),
            groups: groups(&todos),
            todos,
            group: 0,
            focus: Focus::Agents,
            agent_list: ListState::default().with_selected(Some(0)),
            todo_list: ListState::default().with_selected(Some(0)),
            todo_offset: 0,
            configured: !dirs.is_empty(),
            memory: Memory::load(),
            agent_filter: String::new(),
            todo_filter: String::new(),
            typing: false,
            link: None,
            notice: None,
            repo_dirs: config_dirs(&config, "repos"),
            adding: None,
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

    fn visible_agents(&self) -> Vec<&Agent> {
        self.agents
            .iter()
            .filter(|a| matches(&self.agent_filter, &[&a.workspace, &a.tab, &a.title]))
            .collect()
    }

    /// Grouped by topic. The topic used most recently comes first, then the biggest; inside a
    /// topic the most recently opened or crossed todo leads and the rest keep their note order.
    fn visible_todos(&self) -> Vec<&Todo> {
        let used = |t: &Todo| self.memory.used.get(&t.text).copied().unwrap_or(0);
        let list: Vec<&Todo> = self
            .todos
            .iter()
            .filter(|t| self.group == 0 || t.group == self.groups[self.group - 1].0)
            .filter(|t| matches(&self.todo_filter, &[&t.text, &t.group, &t.topic]))
            .collect();
        let mut topics: HashMap<(&str, &str), (u64, usize)> = HashMap::new();
        for t in &list {
            let e = topics.entry((&t.group, &t.topic)).or_default();
            e.0 = e.0.max(used(t));
            e.1 += 1;
        }
        let mut list = list;
        list.sort_by_key(|t| {
            let (last, size) = topics[&(t.group.as_str(), t.topic.as_str())];
            (
                std::cmp::Reverse(last),
                std::cmp::Reverse(size),
                (t.group.clone(), t.topic.clone()),
                std::cmp::Reverse(used(t)),
            )
        });
        list
    }

    fn filter_mut(&mut self) -> &mut String {
        match self.focus {
            Focus::Agents => &mut self.agent_filter,
            Focus::Todos => &mut self.todo_filter,
        }
    }

    fn list_mut(&mut self) -> &mut ListState {
        match self.focus {
            Focus::Agents => &mut self.agent_list,
            Focus::Todos => &mut self.todo_list,
        }
    }

    /// Esc steps back one layer: stop typing, then clear the filter, then close.
    fn back_out(&mut self) -> bool {
        if self.typing {
            self.typing = false;
        } else if !self.filter_mut().is_empty() {
            self.filter_mut().clear();
            self.list_mut().select(Some(0));
        } else {
            return true;
        }
        false
    }

    fn step(&mut self, delta: isize) {
        let (state, len) = match self.focus {
            Focus::Agents => {
                let len = self.visible_agents().len();
                (&mut self.agent_list, len)
            }
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

    /// Re-reads agents and notes, keeping filters, focus and the selected position.
    fn reload(&mut self) {
        let mut fresh = App::load();
        fresh.agent_filter = std::mem::take(&mut self.agent_filter);
        fresh.todo_filter = std::mem::take(&mut self.todo_filter);
        fresh.focus = std::mem::replace(&mut self.focus, Focus::Agents);
        fresh.group = self.group.min(fresh.groups.len());
        fresh.notice = self.notice.take();
        fresh.agent_list.select(self.agent_list.selected());
        fresh.todo_list.select(self.todo_list.selected());
        *self = fresh;
    }

    fn selected_todo(&self) -> Option<&Todo> {
        let i = self.todo_list.selected()?;
        self.visible_todos().get(i).copied()
    }

    fn cycle_group(&mut self, delta: isize) {
        let n = self.groups.len() as isize + 1;
        self.group = (self.group as isize + delta).rem_euclid(n) as usize;
        self.todo_list.select(Some(0));
    }
}

fn main() -> std::io::Result<()> {
    if std::env::args().nth(1).as_deref() == Some("track") {
        track_focus();
        return Ok(());
    }
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
        if let Some(text) = app.adding.as_mut() {
            match key.code {
                KeyCode::Char(c) => text.push(c),
                KeyCode::Backspace => {
                    text.pop();
                }
                KeyCode::Esc => app.adding = None,
                KeyCode::Enter => {
                    let text = app.adding.take().unwrap_or_default();
                    if let Some(after) = app.selected_todo().cloned()
                        && !text.trim().is_empty()
                    {
                        let written = fs::read_to_string(&after.file)
                            .map(|note| insert_todo(&note, &after, &text))
                            .and_then(|note| fs::write(&after.file, note));
                        app.notice = Some(match written {
                            Ok(()) => format!("added to {}", after.file.display()),
                            Err(e) => format!("could not add: {e}"),
                        });
                        app.reload();
                    }
                }
                _ => {}
            }
            continue;
        }
        if let Some(link) = app.link.as_mut() {
            match key.code {
                KeyCode::Char(c) => link.push(c),
                KeyCode::Backspace => {
                    link.pop();
                }
                KeyCode::Esc => app.link = None,
                KeyCode::Enter => {
                    let url = app.link.take().unwrap_or_default();
                    app.notice = Some(format!("opening {}…", url.trim()));
                    terminal.draw(|f| draw(f, &mut app))?;
                    match open_link(&url, &app.repo_dirs) {
                        Ok(()) => return Ok(()),
                        Err(e) => app.notice = Some(e),
                    }
                }
                _ => {}
            }
            continue;
        }
        // While typing a filter, printable keys go to it, so j/k/q can be searched.
        if app.typing {
            match key.code {
                KeyCode::Char(c) => {
                    app.filter_mut().push(c);
                    app.list_mut().select(Some(0));
                    continue;
                }
                KeyCode::Backspace => {
                    app.filter_mut().pop();
                    app.list_mut().select(Some(0));
                    continue;
                }
                _ => {}
            }
        }
        match key.code {
            KeyCode::Esc => {
                if app.back_out() {
                    return Ok(());
                }
            }
            KeyCode::Char('q') => return Ok(()),
            KeyCode::Char('/') => app.typing = true,
            KeyCode::Char('u') => {
                app.link = Some(String::new());
                app.notice = None;
            }
            KeyCode::Tab | KeyCode::BackTab => {
                app.typing = false;
                app.focus = match app.focus {
                    Focus::Agents => Focus::Todos,
                    Focus::Todos => Focus::Agents,
                }
            }
            KeyCode::Char('j') | KeyCode::Down => app.step(1),
            KeyCode::Char('k') | KeyCode::Up => app.step(-1),
            KeyCode::Char('l') | KeyCode::Right if app.focus == Focus::Todos => {
                app.typing = false;
                app.cycle_group(1)
            }
            KeyCode::Char('h') | KeyCode::Left if app.focus == Focus::Todos => {
                app.typing = false;
                app.cycle_group(-1)
            }
            KeyCode::Char('r') => app.reload(),
            KeyCode::Char('o') if app.focus == Focus::Todos => {
                if let Some(todo) = app.selected_todo().cloned() {
                    if open_in_obsidian(&todo) {
                        app.notice = Some(format!("opened {} in Obsidian", todo.topic));
                    } else {
                        ratatui::restore();
                        edit_source(&todo);
                        *terminal = ratatui::init();
                        app.reload();
                    }
                }
            }
            KeyCode::Char('a') if app.focus == Focus::Todos => {
                if app.selected_todo().is_some() {
                    app.adding = Some(String::new());
                    app.notice = None;
                }
            }
            KeyCode::Char('g') | KeyCode::Home => app.step(isize::MIN / 2),
            KeyCode::Char('G') | KeyCode::End => app.step(isize::MAX / 2),
            KeyCode::Enter if app.focus == Focus::Agents => {
                let agents = app.visible_agents();
                if let Some(a) = app.agent_list.selected().and_then(|i| agents.get(i)) {
                    herdr(&["agent", "focus", &a.pane_id]);
                    return Ok(());
                }
            }
            KeyCode::Enter => {
                if let Some(todo) = app.selected_todo() {
                    let todo = todo.clone();
                    open_todo(&mut app.memory, &todo);
                    return Ok(());
                }
            }
            KeyCode::Char(' ') if app.focus == Focus::Todos => {
                if let Some(text) = app.selected_todo().map(|t| t.text.clone()) {
                    app.memory.toggle_crossed(&text);
                    app.memory.save();
                    // It just moved to the top; keep it selected.
                    app.todo_list.select(Some(0));
                }
            }
            _ => {}
        }
    }
}

/// Tokyo Night, the same dark palette Herdr Recent Navigator uses.
// ponytail: dark palette only; read Herdr's theme here if a light one is needed.
mod palette {
    use ratatui::style::Color;
    pub const BG: Color = Color::Rgb(26, 27, 38);
    pub const LINE: Color = Color::Rgb(36, 40, 59);
    pub const SELECTED: Color = Color::Rgb(65, 72, 104);
    pub const TEXT: Color = Color::Rgb(192, 202, 245);
    pub const SUBTEXT: Color = Color::Rgb(169, 177, 214);
    pub const ACCENT: Color = Color::Rgb(122, 162, 247);
    pub const RED: Color = Color::Rgb(247, 118, 142);
    pub const YELLOW: Color = Color::Rgb(224, 175, 104);
    pub const GREEN: Color = Color::Rgb(158, 206, 106);
    pub const TEAL: Color = Color::Rgb(125, 207, 255);
    pub const MAUVE: Color = Color::Rgb(187, 154, 247);
    pub const PEACH: Color = Color::Rgb(255, 158, 100);
}

/// Truncates to `width` columns with an ellipsis, then pads to exactly `width`.
fn fit(text: &str, width: usize) -> String {
    if text.width() <= width {
        return format!(
            "{text:<width$}",
            width = width - text.width() + text.chars().count()
        );
    }
    let mut out = String::new();
    for c in text.chars() {
        if out.width() + c.width().unwrap_or(0) + 1 > width {
            break;
        }
        out.push(c);
    }
    out.push('…');
    let pad = width.saturating_sub(out.width());
    out + &" ".repeat(pad)
}

/// A section title like a Recent Navigator tab: filled when focused.
fn title(text: &str, focused: bool) -> Span<'static> {
    let style = if focused {
        Style::new().fg(palette::BG).bg(palette::ACCENT).bold()
    } else {
        Style::new().fg(palette::ACCENT)
    };
    Span::styled(format!(" {text} "), style)
}

fn header(text: &str) -> Span<'static> {
    Span::styled(text.to_string(), Style::new().fg(palette::SUBTEXT).bold())
}

fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    f.render_widget(
        Block::new().style(Style::new().bg(palette::BG).fg(palette::TEXT)),
        area,
    );
    let bar = app.link.is_some() || app.adding.is_some() || app.notice.is_some();
    let [bar_area, body] = Layout::vertical([
        Constraint::Length(if bar { 2 } else { 0 }),
        Constraint::Fill(1),
    ])
    .areas(area);
    if bar {
        let line = match (&app.link, &app.adding) {
            (_, Some(text)) => Line::from(vec![
                Span::styled(
                    " New todo ",
                    Style::new().fg(palette::BG).bg(palette::GREEN).bold(),
                ),
                Span::styled(format!(" {text}"), Style::new().fg(palette::TEXT)),
                Span::styled("▏", Style::new().fg(palette::GREEN)),
                Span::styled(
                    "   Enter add below the selected todo  Esc cancel",
                    Style::new().fg(palette::SUBTEXT),
                ),
            ]),
            (Some(link), None) => Line::from(vec![
                Span::styled(
                    " GitHub link ",
                    Style::new().fg(palette::BG).bg(palette::ACCENT).bold(),
                ),
                Span::styled(format!(" {link}"), Style::new().fg(palette::TEXT)),
                Span::styled("▏", Style::new().fg(palette::ACCENT)),
                Span::styled(
                    "   Enter open  Esc cancel",
                    Style::new().fg(palette::SUBTEXT),
                ),
            ]),
            (None, None) => Line::from(Span::styled(
                format!(" {}", app.notice.as_deref().unwrap_or_default()),
                Style::new().fg(palette::YELLOW),
            )),
        };
        f.render_widget(
            Paragraph::new(line).block(
                Block::new()
                    .borders(Borders::BOTTOM)
                    .border_style(Style::new().fg(palette::LINE)),
            ),
            bar_area,
        );
    }

    // Agents on top, todos below, both full width so tab and title have room.
    let h = (app.agents.len() as u16 + 7).min(body.height / 2);
    let [left, right] = Layout::vertical([Constraint::Length(h), Constraint::Fill(1)]).areas(body);
    let [left, left_keys] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(hint_lines(AGENT_KEYS, left.width).len() as u16 + 1),
    ])
    .areas(left);
    let [right, right_keys] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(hint_lines(TODO_KEYS, right.width).len() as u16 + 1),
    ])
    .areas(right);
    draw_agents(f, app, left);
    draw_todos(f, app, right);
    draw_hints(f, left_keys, AGENT_KEYS);
    draw_hints(f, right_keys, TODO_KEYS);
}

/// Title strip plus a column header, as in Recent Navigator. Returns the list area.
/// Recent Navigator's search line: ` > query` with the match count on the right.
fn filter_line(
    filter: &str,
    typing: bool,
    matched: usize,
    total: usize,
    width: u16,
) -> Line<'static> {
    let (text, style) = match (filter.is_empty(), typing) {
        (true, false) => ("/ to filter".to_string(), Style::new().fg(palette::SUBTEXT)),
        (true, true) => (String::new(), Style::new()),
        (false, _) => (filter.to_string(), Style::new().fg(palette::TEXT)),
    };
    let prompt = if typing {
        palette::ACCENT
    } else {
        palette::SUBTEXT
    };
    let count = if filter.is_empty() {
        total.to_string()
    } else {
        format!("{matched}/{total}")
    };
    let cursor = if typing { "▏" } else { "" };
    let pad = (width as usize).saturating_sub(3 + text.width() + cursor.width() + count.len() + 2);
    Line::from(vec![
        Span::styled(" > ", Style::new().fg(prompt)),
        Span::styled(text, style),
        Span::styled(cursor, Style::new().fg(palette::ACCENT)),
        Span::raw(" ".repeat(pad)),
        Span::styled(count, Style::new().fg(palette::SUBTEXT)),
    ])
}

fn section(f: &mut Frame, area: Rect, top: Line, search: Line, columns: Line) -> Rect {
    let [strip, search_area, head, list] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Fill(1),
    ])
    .areas(area);
    f.render_widget(
        Paragraph::new(search).block(
            Block::new()
                .borders(Borders::BOTTOM)
                .border_style(Style::new().fg(palette::LINE)),
        ),
        search_area,
    );
    f.render_widget(
        Paragraph::new(top).block(
            Block::new()
                .borders(Borders::BOTTOM)
                .border_style(Style::new().fg(palette::LINE)),
        ),
        strip,
    );
    f.render_widget(Paragraph::new(columns), head);
    list
}

fn number(i: usize) -> Span<'static> {
    Span::styled(format!("{:>2}. ", i + 1), Style::new().fg(palette::SUBTEXT))
}

fn draw_agents(f: &mut Frame, app: &mut App, area: Rect) {
    let mut top = vec![title("Agents", app.focus == Focus::Agents), Span::raw("  ")];
    for status in Status::ALL {
        let n = app.agents.iter().filter(|a| a.status == status).count();
        if n > 0 {
            top.push(Span::styled(
                format!("{} {n}  ", status.icon()),
                Style::new().fg(status.color()).bold(),
            ));
        }
    }
    // Index, icon, then tab, workspace and title columns.
    // Each column keeps a two-space gap after it, so a full one never runs into the next.
    let rest = (area.width as usize).saturating_sub(6 + 4);
    let (tab_w, ws_w) = (rest * 4 / 10, rest * 25 / 100);
    let title_w = rest - ws_w - tab_w;
    let gap = || Span::raw("  ");
    let columns = Line::from(vec![
        Span::raw("      "),
        header(&fit("Tab", tab_w)),
        gap(),
        header(&fit("Workspace", ws_w)),
        gap(),
        header(&fit("Title", title_w)),
    ]);
    let agents = app.visible_agents();
    let typing = app.typing && app.focus == Focus::Agents;
    let search = filter_line(
        &app.agent_filter,
        typing,
        agents.len(),
        app.agents.len(),
        area.width,
    );
    let list_area = section(f, area, Line::from(top), search, columns);

    let items: Vec<ListItem> = agents
        .iter()
        .enumerate()
        .map(|(i, a)| {
            ListItem::new(Line::from(vec![
                number(i),
                Span::styled(
                    format!("{} ", a.status.icon()),
                    Style::new().fg(a.status.color()),
                ),
                Span::styled(fit(&a.tab, tab_w), Style::new().fg(palette::TEXT).bold()),
                gap(),
                Span::styled(fit(&a.workspace, ws_w), Style::new().fg(palette::TEXT)),
                gap(),
                Span::styled(fit(&a.title, title_w), Style::new().fg(palette::SUBTEXT)),
            ]))
        })
        .collect();
    let mut list = List::new(items);
    if app.focus == Focus::Agents {
        list = list.highlight_style(Style::new().bg(palette::SELECTED));
    }
    f.render_stateful_widget(list, list_area, &mut app.agent_list);
}

fn draw_todos(f: &mut Frame, app: &mut App, area: Rect) {
    let top = Line::from(vec![
        title("Todo", app.focus == Focus::Todos),
        Span::styled(
            format!("  ☐ {}", app.todos.len()),
            Style::new().fg(palette::TEXT).bold(),
        ),
    ]);
    if !app.configured {
        let list_area = section(f, area, top, Line::default(), Line::default());
        let hint = format!(
            "  set [todos] dirs in {}",
            config_path()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        );
        f.render_widget(Paragraph::new(hint).fg(palette::SUBTEXT), list_area);
        return;
    }

    // One bar per group, the selected group filled like a tab.
    let name_w = app
        .groups
        .iter()
        .map(|g| g.0.width())
        .max()
        .unwrap_or(3)
        .max(3);
    let bars_h = (app.groups.len() as u16 + 1).min(area.height / 3);
    let [bars, list_part] =
        Layout::vertical([Constraint::Length(bars_h + 5), Constraint::Fill(1)]).areas(area);
    let todos = app.visible_todos();
    let typing = app.typing && app.focus == Focus::Todos;
    let search = filter_line(
        &app.todo_filter,
        typing,
        todos.len(),
        app.todos.len(),
        area.width,
    );
    let bars_area = section(f, bars, top, search, Line::default());
    let max = app.groups.first().map(|g| g.1).unwrap_or(1).max(1);
    let bar_w = (area.width as usize).saturating_sub(name_w + 10).max(1);
    let all = std::iter::once(("all".to_string(), app.todos.len(), palette::TEXT));
    let rows: Vec<Line> = all
        .chain(
            app.groups
                .iter()
                .map(|(g, n)| (g.clone(), *n, app.color_of(g))),
        )
        .enumerate()
        .map(|(i, (name, n, color))| {
            let len = if i == 0 { 0 } else { (n * bar_w).div_ceil(max) };
            let selected = i == app.group;
            let name_style = if selected {
                Style::new().fg(palette::BG).bg(color).bold()
            } else {
                Style::new().fg(color)
            };
            Line::from(vec![
                Span::raw("  "),
                Span::styled(format!(" {name:<name_w$} "), name_style),
                Span::raw(" "),
                Span::styled("▰".repeat(len), Style::new().fg(color)),
                Span::styled(format!(" {n}"), Style::new().fg(palette::TEXT)),
            ])
        })
        .collect();
    let skip = app.group.saturating_sub(bars_area.height as usize - 1);
    f.render_widget(Paragraph::new(rows).scroll((skip as u16, 0)), bars_area);

    let [_, head, list_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Fill(1),
    ])
    .areas(list_part);
    let text_w = (area.width as usize).saturating_sub(8);
    f.render_widget(
        Line::from(vec![Span::raw("        "), header(&fit("Todo", text_w))]),
        head,
    );
    let mut items: Vec<ListItem> = Vec::new();
    let mut selected_row = 0;
    for (i, t) in todos.iter().enumerate() {
        if i == 0 || (&t.group, &t.topic) != (&todos[i - 1].group, &todos[i - 1].topic) {
            let n = todos[i..]
                .iter()
                .take_while(|o| (&o.group, &o.topic) == (&t.group, &t.topic))
                .count();
            let color = app.color_of(&t.group);
            items.push(ListItem::new(Line::from(vec![
                Span::styled(format!(" ▸ {}", t.topic), Style::new().fg(color).bold()),
                Span::styled(format!("  {}", t.group), Style::new().fg(color)),
                Span::styled(format!("  {n}"), Style::new().fg(palette::SUBTEXT)),
            ])));
        }
        if app.todo_list.selected() == Some(i) {
            selected_row = items.len();
        }
        items.push({
            let color = app.color_of(&t.group);
            let crossed = app.memory.crossed.contains(&t.text);
            let (box_icon, text_style) = if crossed {
                ("☒ ", Style::new().fg(palette::SUBTEXT).crossed_out())
            } else {
                ("☐ ", Style::new().fg(palette::TEXT))
            };
            let link = if app.memory.tabs.contains_key(&t.text) {
                "↗ "
            } else {
                "  "
            };
            ListItem::new(Line::from(vec![
                number(i),
                Span::styled(box_icon, Style::new().fg(color)),
                Span::styled(link, Style::new().fg(palette::ACCENT)),
                Span::styled(fit(&t.text, text_w), text_style),
            ]))
        });
    }
    let mut list = List::new(items);
    if app.focus == Focus::Todos {
        list = list.highlight_style(Style::new().bg(palette::SELECTED));
    }
    // Selection counts todos only; the list widget counts header rows too.
    let mut state = ListState::default()
        .with_offset(app.todo_offset)
        .with_selected(Some(selected_row));
    f.render_stateful_widget(list, list_area, &mut state);
    app.todo_offset = state.offset();
}

const AGENT_KEYS: &[(&str, &str)] = &[
    ("u", "open GitHub link"),
    ("/", "filter"),
    ("j/k ↑/↓", "move"),
    ("g/G", "top/bottom"),
    ("Enter", "go to agent"),
    ("Tab", "to todos"),
    ("r", "reload"),
    ("Esc", "back / close"),
    ("q", "close"),
];

const TODO_KEYS: &[(&str, &str)] = &[
    ("u", "open GitHub link"),
    ("/", "filter"),
    ("j/k ↑/↓", "move"),
    ("g/G", "top/bottom"),
    ("h/l ←/→", "group"),
    ("Enter", "Claude tab"),
    ("Space", "cross out"),
    ("o", "open note"),
    ("a", "add todo below"),
    ("Tab", "to agents"),
    ("r", "reload"),
    ("Esc", "back / close"),
    ("q", "close"),
];

/// Every key the panel takes, wrapped over as many lines as `width` needs.
fn hint_lines(keys: &[(&'static str, &'static str)], width: u16) -> Vec<Line<'static>> {
    let mut lines = vec![Line::default()];
    let mut used = 0;
    for (key, label) in keys {
        let w = key.width() + label.width() + 3;
        if used + w > width as usize && used > 0 {
            lines.push(Line::default());
            used = 0;
        }
        let line = lines.last_mut().unwrap();
        line.push_span(Span::raw("  "));
        line.push_span(Span::styled(*key, Style::new().fg(palette::ACCENT)));
        line.push_span(Span::raw(" "));
        line.push_span(Span::styled(*label, Style::new().fg(palette::SUBTEXT)));
        used += w;
    }
    lines
}

fn draw_hints(f: &mut Frame, area: Rect, keys: &[(&'static str, &'static str)]) {
    let lines = hint_lines(keys, area.width);
    f.render_widget(
        Paragraph::new(lines).block(
            Block::new()
                .borders(Borders::TOP)
                .border_style(Style::new().fg(palette::LINE)),
        ),
        area,
    );
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
                topic: String::new(),
                text: "Book the dentist (2026-08-26)".into(),
                ..Todo::default()
            })
        );
        assert_eq!(
            parse_todo("* [ ] Untagged task"),
            Some(Todo {
                group: "other".into(),
                topic: String::new(),
                text: "Untagged task".into(),
                ..Todo::default()
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
    fn memory_round_trips_and_toggles_crossed() {
        let mut m = Memory::from_json(&json!({"tabs": {"Book it": "w1:t2"}, "crossed": ["Done"]}));
        assert_eq!(m.tabs.get("Book it").map(String::as_str), Some("w1:t2"));
        m.toggle_crossed("Done");
        m.toggle_crossed("New");
        assert_eq!(m.crossed, vec!["New"]);
    }

    #[test]
    fn shell_quote_keeps_quotes_and_dollars_literal() {
        assert_eq!(shell_quote("it's $HOME"), r"'it'\''s $HOME'");
    }

    #[test]
    fn agents_sort_most_recently_focused_first() {
        let agents = json!({"agents": [
            {"agent_status": "blocked", "pane_id": "p1"},
            {"agent_status": "idle", "pane_id": "p2"},
            {"agent_status": "done", "pane_id": "p3"},
        ]});
        let focused = HashMap::from([("p2".into(), 200), ("p3".into(), 100)]);
        let list = parse_agents(&agents, &HashMap::new(), &HashMap::new(), &focused);
        let order: Vec<_> = list.iter().map(|a| a.pane_id.as_str()).collect();
        assert_eq!(order, vec!["p2", "p3", "p1"]);
    }

    #[test]
    fn todos_sort_most_recently_used_first() {
        let todo = |t: &str| Todo {
            group: "g".into(),
            topic: "x".into(),
            text: t.into(),
            ..Todo::default()
        };
        let mut app = App {
            agents: vec![],
            todos: vec![todo("a"), todo("b"), todo("c")],
            groups: vec![],
            group: 0,
            focus: Focus::Todos,
            agent_list: ListState::default(),
            todo_list: ListState::default(),
            todo_offset: 0,
            configured: true,
            memory: Memory::default(),
            agent_filter: String::new(),
            todo_filter: String::new(),
            typing: false,
            link: None,
            notice: None,
            repo_dirs: vec![],
            adding: None,
        };
        app.memory.used = HashMap::from([("c".into(), 200), ("b".into(), 100)]);
        let order: Vec<_> = app
            .visible_todos()
            .iter()
            .map(|t| t.text.as_str())
            .collect();
        assert_eq!(order, vec!["c", "b", "a"]);
    }

    #[test]
    fn place_takes_area_from_top_folder_and_topic_from_the_next_level() {
        let p = |s: &str, tag: &str| place(Path::new(s), tag);
        let pair = |a: &str, b: &str| (a.to_string(), b.to_string());
        assert_eq!(
            p("Houses/Main Street 1/deed/fixes.md", "home"),
            pair("houses", "Main Street 1")
        );
        assert_eq!(p("Work/Onboarding.md", "other"), pair("work", "Onboarding"));
        assert_eq!(p("Work/TODO.md", "work"), pair("work", "inbox"));
        assert_eq!(p("Inbox.md", "home"), pair("home", "Inbox"));
    }

    #[test]
    fn todos_group_by_topic_most_recent_topic_first() {
        let todo = |topic: &str, t: &str| Todo {
            group: "g".into(),
            topic: topic.into(),
            text: t.into(),
            ..Todo::default()
        };
        let mut app = App {
            agents: vec![],
            todos: vec![
                todo("big", "a"),
                todo("small", "b"),
                todo("big", "c"),
                todo("big", "d"),
            ],
            groups: vec![],
            group: 0,
            focus: Focus::Todos,
            agent_list: ListState::default(),
            todo_list: ListState::default(),
            todo_offset: 0,
            configured: true,
            memory: Memory::default(),
            agent_filter: String::new(),
            todo_filter: String::new(),
            typing: false,
            link: None,
            notice: None,
            repo_dirs: vec![],
            adding: None,
        };
        let order = |app: &App| -> Vec<String> {
            app.visible_todos().iter().map(|t| t.text.clone()).collect()
        };
        assert_eq!(order(&app), vec!["a", "c", "d", "b"]);
        app.memory.used = HashMap::from([("b".into(), 1)]);
        assert_eq!(order(&app), vec!["b", "a", "c", "d"]);
    }

    #[test]
    fn parse_link_reads_issues_and_pull_requests() {
        let link = |repo: &str, pull, number| {
            Some(GithubLink {
                repo: repo.into(),
                pull,
                number,
            })
        };
        let p = parse_link;
        assert_eq!(
            p("https://github.com/Acme/Api/issues/874"),
            link("acme/api", false, 874)
        );
        assert_eq!(
            p(" github.com/acme/api/pull/12/files "),
            link("acme/api", true, 12)
        );
        assert_eq!(
            p("https://github.com/acme/api/issues/9#issuecomment-1"),
            link("acme/api", false, 9)
        );
        assert_eq!(p("https://github.com/acme/api/tree/main"), None);
        assert_eq!(p("https://gitlab.com/acme/api/issues/1"), None);
    }

    #[test]
    fn remote_repo_handles_https_ssh_and_host_aliases() {
        let r = remote_repo;
        assert_eq!(
            r("https://github.com/Acme/Api.git"),
            Some("acme/api".into())
        );
        assert_eq!(r("git@github.com:acme/api.git"), Some("acme/api".into()));
        assert_eq!(r("git@github.com-work:acme/api"), Some("acme/api".into()));
        assert_eq!(
            r("ssh://git@github.com/acme/api.git"),
            Some("acme/api".into())
        );
        assert_eq!(r("git@gitlab.com:acme/api.git"), None);
    }

    #[test]
    fn branch_name_is_number_plus_title_words_within_48_chars() {
        assert_eq!(
            branch_name(
                874,
                "Prove a customer can only see and change their own data"
            ),
            "issue-874-prove-a-customer-can-only-see-and"
        );
        assert_eq!(
            branch_name(5, "Fix: login (SSO) — retry!"),
            "issue-5-fix-login-sso-retry"
        );
        assert!(branch_name(1, &"word ".repeat(40)).len() <= 48);
    }

    #[test]
    fn worktree_path_follows_herdr_layout() {
        // SAFETY: tests in this crate do not read these variables concurrently.
        unsafe {
            std::env::set_var("HOME", "/home/u");
            std::env::set_var("HERDR_CONFIG_PATH", "/nonexistent/config.toml");
        }
        assert_eq!(
            worktree_path(Path::new("/src/api"), "feat/login"),
            PathBuf::from("/home/u/.herdr/worktrees/api/feat-login")
        );
    }

    #[test]
    fn parse_file_records_each_todo_line() {
        let lines: Vec<_> = parse_file("# Title\n- [ ] One\n\n  - [ ] Two")
            .into_iter()
            .map(|t| t.line)
            .collect();
        assert_eq!(lines, vec![2, 4]);
    }

    #[test]
    fn insert_todo_copies_indent_bullet_and_tag() {
        let note = "# Plan\n  * [ ] First #todo/work (2026-01-01)\n- [ ] Last\n";
        let after = Todo {
            line: 2,
            ..Todo::default()
        };
        assert_eq!(
            insert_todo(note, &after, " Second "),
            "# Plan\n  * [ ] First #todo/work (2026-01-01)\n  * [ ] Second #todo/work\n- [ ] Last\n"
        );
    }

    #[test]
    fn obsidian_url_names_vault_and_note_without_extension() {
        let vaults = [PathBuf::from("/notes/My Vault")];
        assert_eq!(
            obsidian_url(Path::new("/notes/My Vault/Work/Plan — v2.md"), &vaults),
            Some("obsidian://open?vault=My%20Vault&file=Work%2FPlan%20%E2%80%94%20v2".into())
        );
        assert_eq!(obsidian_url(Path::new("/elsewhere/a.md"), &vaults), None);
    }

    #[test]
    fn matches_every_word_case_insensitively_across_fields() {
        assert!(matches("", &["anything"]));
        assert!(matches("API fix", &["api", "fix login"]));
        assert!(!matches("api deploy", &["api", "fix login"]));
    }

    #[test]
    fn esc_stops_typing_then_clears_filter_then_closes() {
        let mut app = App {
            agents: vec![],
            todos: vec![],
            groups: vec![],
            group: 0,
            focus: Focus::Todos,
            agent_list: ListState::default(),
            todo_list: ListState::default(),
            todo_offset: 0,
            configured: true,
            memory: Memory::default(),
            agent_filter: String::new(),
            todo_filter: "dentist".into(),
            typing: true,
            link: None,
            notice: None,
            repo_dirs: vec![],
            adding: None,
        };
        assert!(!app.back_out());
        assert!(!app.typing);
        assert_eq!(app.todo_filter, "dentist");
        assert!(!app.back_out());
        assert!(app.todo_filter.is_empty());
        assert!(app.back_out());
    }

    #[test]
    fn fit_truncates_and_pads_by_display_width() {
        assert_eq!(fit("abc", 5), "abc  ");
        assert_eq!(fit("abcdef", 4), "abc…");
        assert_eq!(fit("日本語", 4), "日… ");
    }

    #[test]
    fn groups_sort_biggest_first() {
        let todo = |g: &str| Todo {
            group: g.into(),
            topic: String::new(),
            text: "x".into(),
            ..Todo::default()
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
            config_dirs("[todos]\ndirs = [\"~/notes\", \"/srv/a~b\"]", "todos"),
            vec![PathBuf::from("/home/u/notes"), PathBuf::from("/srv/a~b")]
        );
        assert!(config_dirs("", "todos").is_empty());
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
        let list = parse_agents(&agents, &workspaces, &tabs, &HashMap::new());
        let order: Vec<_> = list
            .iter()
            .map(|a| {
                (
                    a.status,
                    format!("{} › {}", a.workspace, a.tab),
                    a.pane_id.as_str(),
                )
            })
            .collect();
        assert_eq!(
            order,
            vec![
                (Status::Blocked, "api › fix".to_string(), "p2"),
                (Status::Done, "web › docs".to_string(), "p3"),
                (Status::Idle, "api › main".to_string(), "p1"),
            ]
        );
    }
}
