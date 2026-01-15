# repo-archiver

Interactive TUI to archive old GitHub repos, built with Rust and [Ratatui](https://ratatui.rs/).

## Usage

```bash
# Build
cargo build --release

# Dry run - see what would be archived (repos older than 8 years)
cargo run -- --dry-run

# Archive repos older than 8 years (default)
cargo run

# Archive repos older than 5 years
cargo run -- --age 5y

# Archive repos older than 6 months
cargo run -- --age 6m
```

## Controls

### Selection mode
| Key | Action |
|-----|--------|
| `↑` / `k` | Move up |
| `↓` / `j` | Move down |
| `Space` / `Tab` | Toggle selection |
| `Enter` | Open confirmation modal |
| `q` | Quit |

### Confirmation modal
| Key | Action |
|-----|--------|
| `←` / `→` | Switch between Cancel/Continue |
| `Tab` | Toggle button |
| `Enter` | Select highlighted button |
| `Esc` | Cancel |

### During archiving
| Key | Action |
|-----|--------|
| `↑` / `k` | Scroll up |
| `↓` / `j` | Scroll down |
| `q` | Quit |

## Dependencies

- [gh](https://cli.github.com/) - GitHub CLI (must be installed and authenticated)

## How it works

1. Fetches your non-archived source repos created before the cutoff date
2. Displays an interactive table with repo name, created date, last push, and description
3. Select multiple repos using Space/Tab
4. Press Enter to show confirmation modal
5. Archives all selected repos in batch with live status indicators
