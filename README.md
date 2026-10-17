# Herdr Overview

One popup for a [Herdr](https://herdr.dev) session: which agents need you, and every open todo
in your notes. While it is open, agents and todos
refresh every 2 seconds without moving your selection.

```
 Agents   × 1  ✓ 2  ◐ 1  ○ 2
──────────────────────────────────────────────────────────────────────
 > / to filter                                                      6
──────────────────────────────────────────────────────────────────────
      Tab                       Workspace
 1. × Fix login test            api
 2. ✓ Update README             web
 3. ◐ main                      api
──────────────────────────────────────────────────────────────────────
  u open GitHub link  / filter  j/k ↑/↓ move  Enter go to agent  …
 Todo   ☐ 24
──────────────────────────────────────────────────────────────────────
 > pass▏                                                         1/24
──────────────────────────────────────────────────────────────────────
   all    24
   work  ▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰ 14
   home  ▰▰▰▰▰▰▰▰▰▰▰▰▰ 7

 ▸ Passport  home  1
 1. ☐ ↗ Renew the passport
──────────────────────────────────────────────────────────────────────
  / filter  h/l ←/→ group  Enter Claude tab  Space cross out  …
```

- **Agents** are sorted most recently focused first; agents you have not focused yet follow,
  by urgency: blocked, done, working, idle. Same symbols and colors as
  the Herdr sidebar. The overview starts on the most recent agent other than the one you are
  in, so `prefix+a` then Enter jumps back to it. The one you are in is drawn in blue. Enter jumps to the agent.
- **Todos** are every open Markdown checkbox (`- [ ] ...`) in the folders you configure.
  Code blocks are skipped. They are grouped two levels deep, from where the note lives:
  - the **area** (the bars, `h`/`l` to pick one) is the first word of the top folder, so
    `Work/…` is `work`. A note directly in the scanned folder uses its `#todo/<area>` tag,
    else `other`.
  - the **topic** (a `▸` header in the list) is the folder or note one level down:
    `Work/Launch/plan.md` is topic `Launch`. A `TODO.md` right in an area folder is `inbox`.
- Areas and topics are listed by name, each topic in note order. Nothing you press moves a todo.
- **Enter on a todo** picks where to work on it:
  - a GitHub issue or pull request link in the todo opens that worktree, like `u`;
  - a topic named after a repo in `[repos] dirs` (`agentic-development` finds
    `acme-agentic-development`) opens a worktree there on branch `todo-<words>`, Claude
    started on the todo. Enter again reopens it;
  - anything else gets a Claude tab, as `t` does.
- **`t` on a todo** opens a new tab running `claude "<todo>"` and remembers the link (`↗`).
  `t` again goes back to that tab; if the tab was closed, a new one opens.
- **Space** crosses a todo out (`☒`), Space again undoes it. This only marks it in the
  overview; the note is left alone.
- **`o`** opens the todo's note where it lives. A note inside an Obsidian vault (one Obsidian
  knows about) opens in Obsidian. Any other note opens in `$VISUAL`, `$EDITOR` or `vi` at the
  todo's line, and the overview comes back, reloaded, when you close the editor.
- **`a`** adds a todo right below the selected one, in the same note, with the same indent and
  `#todo/` tag. Pick any todo in the place you want it, press `a`, type, Enter. This is the
  only key that writes to your notes.
- Links, cross-outs and focus times live in the plugin's state directory
  (`$HERDR_PLUGIN_STATE_DIR`). A `pane.focused` hook records when you last focused each pane.
- **`u`** opens a GitHub issue or pull request link: paste it, press Enter. The overview finds
  your clone of that repo, then opens a Herdr worktree for it and starts `claude "<link>"`:
  - an issue gets a new branch `issue-<n>-<title words>` from the latest default branch;
  - a pull request checks out its own branch.
  The workspace and tab are labeled `#<n> <title>`. Pasting the same link again goes back to
  that worktree. Needs `gh`, signed in to an account that can see the repo.
- **`/`** filters the panel you are in, like Herdr Recent Navigator: type to narrow the list
  (every word must match), Esc stops typing and keeps the filter, Esc again clears it.
- Agents sit on top and todos below, both full width. Each panel's footer lists every key
  it takes.

## Install

```sh
herdr plugin install stevenenen/herdr-overview
```

Needs Rust (`cargo`); Herdr builds the plugin on install.

Then tell it where your todos live. It has no default, so until you do, the Todo panel
shows where to put the setting. Create `config.toml` in the plugin's config directory:

```sh
cat > "$(herdr plugin config-dir stevenenen.herdr-overview)/config.toml" <<'TOML'
[todos]
dirs = ["~/notes"]          # any folders of Markdown files, scanned recursively
TOML
```

For `u`, also list the folders that hold your git clones (each clone directly inside):

```toml
[repos]
dirs = ["~/code"]
```

Any notes app that stores plain Markdown works: an Obsidian vault, a Logseq graph, a folder
of `.md` files.

Bind a key in `~/.config/herdr/config.toml`:

```toml
[[keys.command]]
key = "prefix+a"
type = "plugin_action"
command = "stevenenen.herdr-overview.open"
description = "Overview: agents and todos"
```

## Keys

| Key | Action |
| --- | --- |
| `u` | Open a GitHub issue or pull request link as a worktree (Agents panel) |
| `/` | Filter this panel; typed keys go to the filter |
| `j` / `k`, `↑` / `↓` | Move |
| `g` / `G`, `Home` / `End` | Top / bottom |
| `Tab` | Switch between agents and todos |
| `h` / `l`, `←` / `→` | Previous / next todo group (Todo panel); the overview reopens on it |
| `Enter` on an agent | Jump to it |
| `Enter` on a todo | Its issue worktree, its repo's worktree, or else its Claude tab |
| `t` on a todo | Open or return to its Claude tab |
| `Space` on a todo | Cross out / undo |
| `o` on a todo | Open its note: in Obsidian when it is in a vault, else in your editor |
| `a` on a todo | Add a todo below it, in the same note |
| `Esc` | Stop typing, then clear the filter, then close |
| `q` | Close |

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```
