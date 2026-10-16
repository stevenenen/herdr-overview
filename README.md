# Herdr Overview

One popup for a [Herdr](https://herdr.dev) session: which agents need you, and every open todo
in your notes.

```
 Agents   × 1  ✓ 2  ◐ 1  ○ 2             │ Todo   ☐ 24
─────────────────────────────────────────│─────────────────────────────────
 > / to filter                        6  │ > pass▏                     1/24
─────────────────────────────────────────│─────────────────────────────────
      Tab         Workspace Title        │   all    24
 1. × api         auth      Fix login t… │   work  ▰▰▰▰▰▰▰▰▰▰▰▰▰▰ 14
 2. ✓ web         docs      Update READ… │   home  ▰▰▰▰▰▰▰ 7
 3. ✓ web         main      Claude Code  │   other ▰▰ 3
 4. ◐ api         main      Claude Code  │
 5. ○ cli         main      Claude Code  │        Todo                Group
 6. ○ cli         release   Claude Code  │ 1. ☐ ↗ Renew the passport  home
                                         │ 2. ☒   Review the notes    work
─────────────────────────────────────────│─────────────────────────────────
  j/k ↑/↓ move  g/G top/bottom           │  j/k ↑/↓ move  h/l ←/→ group
  Enter go to agent  Tab to todos  …     │  Enter Claude tab  Space cross out  …
```

- **Agents** are sorted most recently focused first; agents you have not focused yet follow,
  by urgency: blocked, done, working, idle. Same symbols and colors as
  the Herdr sidebar. The list refreshes every 2 seconds. Enter jumps to the agent.
- **Todos** are every open Markdown checkbox (`- [ ] ...`) in the folders you configure.
  Code blocks are skipped. They are grouped two levels deep, from where the note lives:
  - the **area** (the bars, `h`/`l` to pick one) is the first word of the top folder, so
    `Work/…` is `work`. A note directly in the scanned folder uses its `#todo/<area>` tag,
    else `other`.
  - the **topic** (a `▸` header in the list) is the folder or note one level down:
    `Work/Launch/plan.md` is topic `Launch`. A `TODO.md` right in an area folder is `inbox`.
- The topic you used most recently comes first, then the biggest. Inside a topic the todo you
  opened or crossed most recently leads.
- **Enter on a todo** opens a new tab running `claude "<todo>"` and remembers the link (`↗`).
  Enter again goes back to that tab instead of starting another; if the tab was closed, a
  new one opens.
- **Space** crosses a todo out (`☒`), Space again undoes it. This only marks it in the
  overview; your notes are never written to.
- Links, cross-outs and focus times live in the plugin's state directory
  (`$HERDR_PLUGIN_STATE_DIR`). A `pane.focused` hook records when you last focused each pane.
- **`/`** filters the panel you are in, like Herdr Recent Navigator: type to narrow the list
  (every word must match), Esc stops typing and keeps the filter, Esc again clears it.
- Each panel's footer lists every key it takes. On a narrow client, such as Herdr on a phone,
  the two panels stack.

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
| `/` | Filter this panel; typed keys go to the filter |
| `j` / `k`, `↑` / `↓` | Move |
| `g` / `G`, `Home` / `End` | Top / bottom |
| `Tab` | Switch between agents and todos |
| `h` / `l`, `←` / `→` | Previous / next todo group (Todo panel) |
| `Enter` on an agent | Jump to it |
| `Enter` on a todo | Open or return to its Claude tab |
| `Space` on a todo | Cross out / undo |
| `r` | Reload |
| `Esc` | Stop typing, then clear the filter, then close |
| `q` | Close |

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```
