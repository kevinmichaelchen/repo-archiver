use anyhow::{Context, Result};
use chrono::{Datelike, NaiveDate, Utc};
use clap::Parser;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState},
};
use serde::Deserialize;
use std::{
    io,
    process::Command,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

#[derive(Parser)]
#[command(name = "repo-archiver")]
#[command(about = "Interactive CLI to archive old GitHub repos")]
struct Args {
    /// Dry run - show what would be archived without making changes
    #[arg(long)]
    dry_run: bool,

    /// Archive repos older than this age (e.g., "8y" for 8 years, "6m" for 6 months)
    /// If not provided, an interactive picker will be shown.
    #[arg(long)]
    age: Option<String>,
}

#[derive(Debug, Clone, Copy)]
enum Age {
    Months(u32),
    Years(u32),
}

impl Age {
    fn parse(s: &str) -> Result<Self> {
        let s = s.trim().to_lowercase();
        if s.is_empty() {
            anyhow::bail!("Age cannot be empty");
        }

        let (num_str, unit) = s.split_at(s.len() - 1);
        let num: u32 = num_str
            .parse()
            .with_context(|| format!("Invalid number in age: {num_str}"))?;

        match unit {
            "y" => Ok(Self::Years(num)),
            "m" => Ok(Self::Months(num)),
            _ => anyhow::bail!("Invalid age unit '{unit}'. Use 'y' for years or 'm' for months (e.g., '8y', '6m')"),
        }
    }

    fn cutoff_date(self) -> NaiveDate {
        let today = Utc::now().date_naive();
        match self {
            Self::Years(y) => today
                .with_year(today.year() - y as i32)
                .unwrap_or(today),
            Self::Months(m) => today - chrono::Months::new(m),
        }
    }

    fn display(self) -> String {
        match self {
            Self::Years(y) => format!("{y} year{}", if y == 1 { "" } else { "s" }),
            Self::Months(m) => format!("{m} month{}", if m == 1 { "" } else { "s" }),
        }
    }

    fn cutoff_display(self) -> String {
        self.cutoff_date().format("%b %d, %Y").to_string()
    }
}

#[derive(Clone, Copy, PartialEq)]
enum AgeUnit {
    Months,
    Years,
}

#[derive(Clone, Copy)]
struct AgePicker {
    value: u32,
    unit: AgeUnit,
}

impl AgePicker {
    fn new() -> Self {
        Self {
            value: 2,
            unit: AgeUnit::Years,
        }
    }

    fn increment(&mut self) {
        let max = match self.unit {
            AgeUnit::Months => 11,
            AgeUnit::Years => 10,
        };
        if self.value < max {
            self.value += 1;
        }
    }

    fn decrement(&mut self) {
        if self.value > 1 {
            self.value -= 1;
        }
    }

    fn toggle_unit(&mut self) {
        self.unit = match self.unit {
            AgeUnit::Months => AgeUnit::Years,
            AgeUnit::Years => AgeUnit::Months,
        };
        // Clamp value to valid range
        let max = match self.unit {
            AgeUnit::Months => 11,
            AgeUnit::Years => 10,
        };
        if self.value > max {
            self.value = max;
        }
    }

    fn to_age(self) -> Age {
        match self.unit {
            AgeUnit::Months => Age::Months(self.value),
            AgeUnit::Years => Age::Years(self.value),
        }
    }

    const fn unit_str(self) -> &'static str {
        match self.unit {
            AgeUnit::Months => "months",
            AgeUnit::Years => "years",
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Repo {
    name: String,
    created_at: String,
    pushed_at: String,
    description: Option<String>,
}

#[derive(Clone, PartialEq)]
enum RepoStatus {
    Idle,
    Pending,
    Archiving,
    Done,
    Failed(String),
}

struct App {
    repos: Vec<Repo>,
    statuses: Vec<RepoStatus>,
    state: TableState,
    selected: Vec<bool>,
    mode: Mode,
    dry_run: bool,
    spinner_tick: usize,
    last_tick: Instant,
    modal_button: usize, // 0 = Cancel, 1 = Continue
}

#[derive(PartialEq)]
enum Mode {
    Selecting,
    ConfirmModal,
    Archiving,
    Done,
}

const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

impl App {
    fn new(repos: Vec<Repo>, dry_run: bool) -> Self {
        let len = repos.len();
        let mut state = TableState::default();
        if !repos.is_empty() {
            state.select(Some(0));
        }
        Self {
            repos,
            statuses: vec![RepoStatus::Idle; len],
            state,
            selected: vec![false; len],
            mode: Mode::Selecting,
            dry_run,
            spinner_tick: 0,
            last_tick: Instant::now(),
            modal_button: 1, // Default to "Continue"
        }
    }

    fn next(&mut self) {
        if self.repos.is_empty() {
            return;
        }
        let i = match self.state.selected() {
            Some(i) => (i + 1) % self.repos.len(),
            None => 0,
        };
        self.state.select(Some(i));
    }

    fn previous(&mut self) {
        if self.repos.is_empty() {
            return;
        }
        let i = match self.state.selected() {
            Some(i) => {
                if i == 0 {
                    self.repos.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }

    fn toggle_selection(&mut self) {
        if let Some(i) = self.state.selected() {
            self.selected[i] = !self.selected[i];
        }
    }

    fn selected_count(&self) -> usize {
        self.selected.iter().filter(|&&s| s).count()
    }

    fn tick_spinner(&mut self) {
        if self.last_tick.elapsed() >= Duration::from_millis(80) {
            self.spinner_tick = (self.spinner_tick + 1) % SPINNER_FRAMES.len();
            self.last_tick = Instant::now();
        }
    }

    fn spinner(&self) -> &'static str {
        SPINNER_FRAMES[self.spinner_tick]
    }

    fn mark_selected_as_pending(&mut self) {
        for (i, selected) in self.selected.iter().enumerate() {
            if *selected {
                self.statuses[i] = RepoStatus::Pending;
            }
        }
    }

    fn is_all_done(&self) -> bool {
        self.statuses.iter().enumerate().all(|(i, status)| {
            !self.selected[i]
                || matches!(status, RepoStatus::Done | RepoStatus::Failed(_))
        })
    }
}

#[derive(Debug)]
enum ArchiveResult {
    Started(usize),
    Done(usize),
    Failed(usize, String),
}

fn fetch_repos(age: Age) -> Result<Vec<Repo>> {
    let cutoff = age.cutoff_date();

    let output = Command::new("gh")
        .args([
            "repo",
            "list",
            "--source",
            "--no-archived",
            "--limit",
            "200",
            "--json",
            "name,createdAt,description,pushedAt",
        ])
        .output()
        .context("Failed to run gh CLI. Is it installed?")?;

    if !output.status.success() {
        anyhow::bail!(
            "gh command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let repos: Vec<Repo> = serde_json::from_slice(&output.stdout)?;

    let mut filtered: Vec<Repo> = repos
        .into_iter()
        .filter(|r| {
            let created = &r.created_at[..10];
            NaiveDate::parse_from_str(created, "%Y-%m-%d")
                .map(|d| d < cutoff)
                .unwrap_or(false)
        })
        .collect();

    filtered.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    Ok(filtered)
}

fn main() -> Result<()> {
    let args = Args::parse();

    // Parse age from CLI or show interactive picker
    let age = if let Some(age_str) = &args.age {
        Age::parse(age_str)?
    } else {
        // Launch TUI for age selection
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let age_result = run_age_picker(&mut terminal);

        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        terminal.show_cursor()?;

        match age_result? {
            Some(age) => age,
            None => return Ok(()), // User cancelled
        }
    };

    println!("Finding repos older than {}...", age.display());
    let repos = fetch_repos(age)?;

    if repos.is_empty() {
        println!("No repos found older than {}.", age.display());
        return Ok(());
    }

    println!("Found {} repos. Launching TUI...", repos.len());

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(repos, args.dry_run);
    let res = run_app(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        eprintln!("Error: {err:?}");
    }

    Ok(())
}

fn run_age_picker<B: Backend>(terminal: &mut Terminal<B>) -> Result<Option<Age>> {
    let mut picker = AgePicker::new();

    loop {
        let age = picker.to_age();

        terminal.draw(|f| {
            let area = f.area();

            // Center the picker
            let picker_width = 44;
            let picker_height = 9;
            let picker_area = Rect {
                x: area.width.saturating_sub(picker_width) / 2,
                y: area.height.saturating_sub(picker_height) / 2,
                width: picker_width.min(area.width),
                height: picker_height.min(area.height),
            };

            // Build the stepper display
            let value_display = Line::from(vec![
                Span::styled("  ◀  ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!(" {} ", picker.value),
                    Style::default().fg(Color::Cyan).bold(),
                ),
                Span::styled(
                    format!(" {} ", picker.unit_str()),
                    Style::default().fg(Color::White),
                ),
                Span::styled("  ▶  ", Style::default().fg(Color::DarkGray)),
            ]);

            let lines = vec![
                Line::from(""),
                Line::from("Archive repos older than:")
                    .style(Style::default().fg(Color::White))
                    .centered(),
                Line::from(""),
                value_display.centered(),
                Line::from(""),
                Line::from(format!("Created before: {}", age.cutoff_display()))
                    .style(Style::default().fg(Color::Yellow))
                    .centered(),
                Line::from(""),
                Line::from("↑/↓: Adjust | ←/→: Unit | Enter: Confirm | q: Quit")
                    .style(Style::default().fg(Color::DarkGray))
                    .centered(),
            ];

            let widget = Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Cyan))
                    .title(" Repo Archiver "),
            );

            f.render_widget(widget, picker_area);
        })?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(None),
                KeyCode::Up | KeyCode::Char('k') => picker.increment(),
                KeyCode::Down | KeyCode::Char('j') => picker.decrement(),
                KeyCode::Left
                | KeyCode::Right
                | KeyCode::Char('h' | 'l')
                | KeyCode::Tab => {
                    picker.toggle_unit();
                }
                KeyCode::Enter => return Ok(Some(picker.to_age())),
                _ => {}
            }
        }
    }
}

fn run_app<B: Backend>(terminal: &mut Terminal<B>, app: &mut App) -> Result<()> {
    let (tx, rx) = mpsc::channel::<ArchiveResult>();

    loop {
        // Update spinner
        app.tick_spinner();

        // Check for archive results
        while let Ok(result) = rx.try_recv() {
            match result {
                ArchiveResult::Started(idx) => {
                    app.statuses[idx] = RepoStatus::Archiving;
                }
                ArchiveResult::Done(idx) => {
                    app.statuses[idx] = RepoStatus::Done;
                }
                ArchiveResult::Failed(idx, err) => {
                    app.statuses[idx] = RepoStatus::Failed(err);
                }
            }
            if app.is_all_done() {
                app.mode = Mode::Done;
            }
        }

        terminal.draw(|f| ui(f, app))?;

        // Poll for events with timeout to keep spinner animating
        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                match app.mode {
                    Mode::Selecting => match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                        KeyCode::Down | KeyCode::Char('j') => app.next(),
                        KeyCode::Up | KeyCode::Char('k') => app.previous(),
                        KeyCode::Char(' ') | KeyCode::Tab => app.toggle_selection(),
                        KeyCode::Enter => {
                            if app.selected_count() > 0 {
                                app.mode = Mode::ConfirmModal;
                            }
                        }
                        _ => {}
                    },
                    Mode::ConfirmModal => match key.code {
                        KeyCode::Left | KeyCode::Char('h') => {
                            app.modal_button = 0;
                        }
                        KeyCode::Right | KeyCode::Char('l') => {
                            app.modal_button = 1;
                        }
                        KeyCode::Tab => {
                            app.modal_button = 1 - app.modal_button;
                        }
                        KeyCode::Enter => {
                            if app.modal_button == 1 {
                                app.mark_selected_as_pending();
                                app.mode = Mode::Archiving;
                                start_archiving(app, tx.clone());
                            } else {
                                app.mode = Mode::Selecting;
                            }
                        }
                        KeyCode::Char('y') => {
                            app.mark_selected_as_pending();
                            app.mode = Mode::Archiving;
                            start_archiving(app, tx.clone());
                        }
                        KeyCode::Char('n') | KeyCode::Esc => {
                            app.mode = Mode::Selecting;
                        }
                        _ => {}
                    },
                    Mode::Archiving => match key.code {
                        KeyCode::Char('q') => return Ok(()),
                        KeyCode::Down | KeyCode::Char('j') => app.next(),
                        KeyCode::Up | KeyCode::Char('k') => app.previous(),
                        _ => {}
                    },
                    Mode::Done => match key.code {
                        KeyCode::Char('q') | KeyCode::Esc | KeyCode::Enter => return Ok(()),
                        _ => {}
                    },
                }
            }
        }
    }
}

fn start_archiving(app: &App, tx: mpsc::Sender<ArchiveResult>) {
    let repos_to_archive: Vec<(usize, String)> = app
        .repos
        .iter()
        .enumerate()
        .filter(|(i, _)| app.selected[*i])
        .map(|(i, r)| (i, r.name.clone()))
        .collect();

    let dry_run = app.dry_run;

    thread::spawn(move || {
        for (idx, name) in repos_to_archive {
            let _ = tx.send(ArchiveResult::Started(idx));

            if dry_run {
                // Simulate some work in dry run
                thread::sleep(Duration::from_millis(300));
                let _ = tx.send(ArchiveResult::Done(idx));
            } else {
                let result = Command::new("gh")
                    .args(["repo", "archive", &name, "--yes"])
                    .output();

                match result {
                    Ok(output) if output.status.success() => {
                        let _ = tx.send(ArchiveResult::Done(idx));
                    }
                    Ok(output) => {
                        let err = String::from_utf8_lossy(&output.stderr).to_string();
                        let _ = tx.send(ArchiveResult::Failed(idx, err));
                    }
                    Err(e) => {
                        let _ = tx.send(ArchiveResult::Failed(idx, e.to_string()));
                    }
                }
            }

            // Small delay between requests to be nice to GitHub API
            thread::sleep(Duration::from_millis(100));
        }
    });
}

fn ui(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Title
            Constraint::Min(10),   // Table
            Constraint::Length(3), // Help/Status
        ])
        .split(f.area());

    // Title
    let title = match app.mode {
        Mode::Selecting | Mode::ConfirmModal => {
            format!(
                " Repo Archiver {} ({} selected) ",
                if app.dry_run { "[DRY RUN]" } else { "" },
                app.selected_count()
            )
        }
        Mode::Archiving => {
            let done = app
                .statuses
                .iter()
                .filter(|s| matches!(s, RepoStatus::Done | RepoStatus::Failed(_)))
                .count();
            let total = app.selected_count();
            format!(
                " Archiving {} ({}/{}) ",
                if app.dry_run { "[DRY RUN]" } else { "" },
                done,
                total
            )
        }
        Mode::Done => " Done! ".to_string(),
    };
    let title_block = Paragraph::new(title)
        .style(Style::default().fg(Color::Cyan).bold())
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(title_block, chunks[0]);

    // Table
    let header_cells = ["Status", "Name", "Created", "Last Push", "Description"]
        .iter()
        .map(|h| Cell::from(*h).style(Style::default().fg(Color::Yellow).bold()));
    let header = Row::new(header_cells).height(1).bottom_margin(1);

    let rows = app.repos.iter().enumerate().map(|(i, repo)| {
        let status_cell = match &app.statuses[i] {
            RepoStatus::Idle => {
                if app.selected[i] {
                    Cell::from("✓").style(Style::default().fg(Color::Green))
                } else {
                    Cell::from(" ")
                }
            }
            RepoStatus::Pending => {
                Cell::from("⏳").style(Style::default().fg(Color::Yellow))
            }
            RepoStatus::Archiving => {
                Cell::from(app.spinner()).style(Style::default().fg(Color::Cyan))
            }
            RepoStatus::Done => Cell::from("✓").style(Style::default().fg(Color::Green)),
            RepoStatus::Failed(_) => Cell::from("✗").style(Style::default().fg(Color::Red)),
        };

        let created = &repo.created_at[..10];
        let pushed = &repo.pushed_at[..10];
        let desc = repo
            .description
            .as_deref()
            .unwrap_or("-")
            .chars()
            .take(50)
            .collect::<String>();

        let style = match &app.statuses[i] {
            RepoStatus::Done => Style::default().fg(Color::Green),
            RepoStatus::Failed(_) => Style::default().fg(Color::Red),
            RepoStatus::Archiving => Style::default().fg(Color::Cyan),
            _ if app.selected[i] => Style::default().fg(Color::White),
            _ => Style::default().fg(Color::DarkGray),
        };

        Row::new(vec![
            status_cell,
            Cell::from(repo.name.clone()),
            Cell::from(created.to_string()),
            Cell::from(pushed.to_string()),
            Cell::from(desc),
        ])
        .style(style)
        .height(1)
    });

    let table = Table::new(
        rows,
        [
            Constraint::Length(6),  // Status
            Constraint::Length(30), // Name
            Constraint::Length(12), // Created
            Constraint::Length(12), // Last Push
            Constraint::Min(20),    // Description
        ],
    )
    .header(header)
    .block(Block::default().borders(Borders::ALL).title(" Repos "))
    .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED))
    .highlight_symbol("▶ ");

    f.render_stateful_widget(table, chunks[1], &mut app.state);

    // Help bar
    let help_text = match app.mode {
        Mode::Selecting => {
            "↑/↓ or j/k: Navigate | Space/Tab: Toggle | Enter: Confirm | q: Quit"
        }
        Mode::ConfirmModal => "←/→ or Tab: Switch | Enter: Select | Esc: Cancel",
        Mode::Archiving => "↑/↓ or j/k: Scroll | q: Quit",
        Mode::Done => "All done! Press q or Enter to exit.",
    };

    let help = Paragraph::new(help_text)
        .style(Style::default().fg(Color::Gray))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(help, chunks[2]);

    // Confirmation modal
    if app.mode == Mode::ConfirmModal {
        render_modal(f, app);
    }
}

fn render_modal(f: &mut Frame, app: &App) {
    let area = f.area();

    // Center the modal
    let modal_width = 50;
    let modal_height = 7;
    let modal_area = Rect {
        x: area.width.saturating_sub(modal_width) / 2,
        y: area.height.saturating_sub(modal_height) / 2,
        width: modal_width.min(area.width),
        height: modal_height.min(area.height),
    };

    // Clear the area behind the modal
    f.render_widget(Clear, modal_area);

    let count = app.selected_count();

    // Build button spans with highlighting
    let cancel_style = if app.modal_button == 0 {
        Style::default().fg(Color::Black).bg(Color::White)
    } else {
        Style::default().fg(Color::Gray)
    };
    let continue_style = if app.modal_button == 1 {
        Style::default().fg(Color::Black).bg(Color::White)
    } else {
        Style::default().fg(Color::Gray)
    };

    let buttons = Line::from(vec![
        Span::raw("  "),
        Span::styled(" Cancel ", cancel_style),
        Span::raw("    "),
        Span::styled(" Continue ", continue_style),
        Span::raw("  "),
    ]);

    let text = vec![
        Line::from(""),
        Line::from(format!(
            "Archive {} repo{}?",
            count,
            if count == 1 { "" } else { "s" }
        ))
        .centered(),
        Line::from(""),
        Line::from(if app.dry_run {
            "(Dry run - no changes will be made)"
        } else {
            "This action cannot be undone."
        })
        .style(Style::default().fg(if app.dry_run {
            Color::Yellow
        } else {
            Color::Red
        }))
        .centered(),
        Line::from(""),
        buttons.centered(),
    ];

    let modal = Paragraph::new(text).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(" Confirm "),
    );

    f.render_widget(modal, modal_area);
}
