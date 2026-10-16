# Herdr Overview

One popup for a [Herdr](https://herdr.dev) session: which agents need you, and every open todo
in your notes.

```
 × 1   ✓ 2   ◐ 3   ○ 5   ☐ 24
┌ agents ──────────────────────┐┌ todo ─────────────────────────┐
│× api › auth   Fix login test ││  all  24                      │
│✓ web › docs   Update README  ││ work ▰▰▰▰▰▰▰▰▰▰▰▰▰▰ 14         │
│✓ web › main   Claude Code    ││ home ▰▰▰▰▰▰▰ 7                 │
│◐ api › main   Claude Code    ││other ▰▰ 3                     │
│○ cli › main   Claude Code    ││                               │
│                              ││☐ Renew the passport           │
│                              ││☐ Review the release notes     │
└──────────────────────────────┘└───────────────────────────────┘
```

- **Agents** are sorted by urgency: blocked, done, working, idle. Same symbols and colors as
  the Herdr sidebar. The list refreshes every 2 seconds. Enter jumps to the agent.
- **Todos** are every open Markdown checkbox (`- [ ] ...`) in the folders you configure.
  A `#todo/<group>` tag puts it in a group; untagged ones go under `other`. Code blocks are
  skipped. The bars show how many each group has.
- On a narrow client, such as Herdr on a phone, the two panels stack.

## Install

```sh
herdr plugin install stevenenen/herdr-overview
```

Needs Rust (`cargo`); Herdr builds the plugin on install.

Point it at your notes in `$(herdr plugin config-dir stevenenen.herdr-overview)/config.toml`:

```toml
[todos]
dirs = ["~/notes"]
```

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
| `j` / `k`, arrows | Move |
| `Tab` | Switch between agents and todos |
| `h` / `l` | Previous / next todo group |
| `Enter` | Jump to the selected agent |
| `r` | Reload todos |
| `q` / `Esc` | Close |

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```
