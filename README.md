# repo-archiver

Interactive TUI to archive old GitHub repos, built with Rust and [Ratatui](https://ratatui.rs/).

## Usage

```bash
# Build
cargo build --release

# Interactive mode - prompts for age selection
cargo run

# Dry run - prompts for age, then shows what would be archived
cargo run -- --dry-run

# Skip age picker by specifying directly (5 years)
cargo run -- --age 5y

# Skip age picker by specifying directly (6 months)
cargo run -- --age 6m
```

## Controls

### Age picker (if --age not provided)
| Key | Action |
|-----|--------|
| `↑` / `k` | Move up |
| `↓` / `j` | Move down |
| `Enter` | Confirm selection |
| `q` / `Esc` | Quit |

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
