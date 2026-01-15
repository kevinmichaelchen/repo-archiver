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
cargo run -- --years 5
```

## Controls

| Key | Action |
|-----|--------|
| `↑` / `k` | Move up |
| `↓` / `j` | Move down |
| `Space` / `Tab` | Toggle selection |
| `Enter` | Confirm selection |
| `y` | Confirm archive (in modal) |
| `n` / `Esc` | Cancel |
| `q` | Quit |

## Dependencies

- [gh](https://cli.github.com/) - GitHub CLI (must be installed and authenticated)

## How it works

1. Fetches your non-archived source repos created before the cutoff date
2. Displays an interactive table with repo name, created date, last push, and description
3. Select multiple repos using Space/Tab
4. Press Enter to show confirmation modal
5. Archives all selected repos in batch with live status indicators
