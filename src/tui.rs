use crate::pricing::Pricing;
use crate::render;
use crate::server::Server;
use crate::sessions::{self, Session};
use crate::stats::{self, ActivityBucket, GlobalStats, Period};
use crate::{load_export, open_url};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Borders, Cell, Paragraph, Row, Table, TableState, Wrap};
use ratatui::{Frame, Terminal};
use std::io::Stdout;
use std::time::Duration;

const BG: Color = Color::Rgb(10, 12, 17);
const SURFACE: Color = Color::Rgb(16, 19, 27);
const BORDER: Color = Color::Rgb(36, 43, 58);
const TEXT: Color = Color::Rgb(232, 236, 245);
const MUTED: Color = Color::Rgb(141, 151, 171);
const FAINT: Color = Color::Rgb(98, 109, 130);
const ACCENT: Color = Color::Rgb(138, 164, 255);
const ACCENT2: Color = Color::Rgb(90, 209, 192);
const ERR: Color = Color::Rgb(248, 113, 113);
const HIGHLIGHT_BG: Color = Color::Rgb(32, 42, 66);
const SUBTLE: Color = Color::Rgb(75, 85, 104);
const YELLOW: Color = Color::Rgb(250, 204, 21);

enum Screen {
    List,
    Loading { title: String },
    Serving {
        title: String,
        id: String,
        directory: String,
        url: String,
        #[allow(dead_code)]
        server: Server,
    },
    Error {
        message: String,
    },
    Stats(StatsState),
}

struct App {
    local_sessions: Vec<Session>,
    all_sessions: Option<Vec<Session>>,
    show_all: bool,
    selected: usize,
    screen: Screen,
    running: bool,
    status: Option<String>,
}

struct StatsState {
    period: Period,
    offset: i64,
    label: String,
    data: GlobalStats,
    activity: Vec<ActivityBucket>,
    error: Option<String>,
    pricing: Pricing,
}

impl StatsState {
    fn new() -> Self {
        let mut state = Self {
            period: Period::Global,
            offset: 0,
            label: String::new(),
            data: GlobalStats::default(),
            activity: Vec::new(),
            error: None,
            pricing: Pricing::load_all(),
        };
        state.reload();
        state
    }

    fn reload(&mut self) {
        match stats::load(self.period, self.offset, &self.pricing) {
            Ok((label, data, activity)) => {
                self.label = label;
                self.data = data;
                self.activity = activity;
                self.error = None;
            }
            Err(err) => {
                self.label.clear();
                self.data = GlobalStats::default();
                self.activity.clear();
                self.error = Some(format!("{err:#}"));
            }
        }
    }

    fn set_period(&mut self, period: Period) {
        self.period = period;
        self.offset = 0;
        self.reload();
    }

    fn page_size(&self) -> i64 {
        match self.period {
            Period::Global => 1,
            Period::Monthly => 6,
            Period::Weekly => 5,
            Period::Daily => 8,
        }
    }

    fn page(&self) -> i64 {
        self.offset / self.page_size()
    }

    fn older_page(&mut self) {
        if self.period == Period::Global {
            return;
        }
        self.offset = (self.page() + 1) * self.page_size();
        self.reload();
    }

    fn newer_page(&mut self) {
        if self.period == Period::Global {
            return;
        }
        let page = self.page();
        if page > 0 {
            self.offset = (page - 1) * self.page_size();
            self.reload();
        }
    }

    fn step(&mut self, newer: bool) {
        if self.period == Period::Global {
            return;
        }
        if newer {
            if self.offset > 0 {
                self.offset -= 1;
                self.reload();
            }
        } else {
            self.offset = self.offset.saturating_add(1);
            self.reload();
        }
    }
}

impl App {
    fn sessions(&self) -> &[Session] {
        if self.show_all {
            if let Some(all) = &self.all_sessions {
                return all;
            }
        }
        &self.local_sessions
    }

    fn scope_label(&self) -> &'static str {
        if self.show_all {
            "all projects"
        } else {
            "this project"
        }
    }

    /// Switch between project sessions and every session in OpenCode's database.
    fn toggle_scope(&mut self) {
        let current_id = self
            .sessions()
            .get(self.selected)
            .map(|session| session.id.clone());

        if !self.show_all {
            if self.all_sessions.is_none() {
                match sessions::list_all() {
                    Ok(all) => self.all_sessions = Some(all),
                    Err(err) => {
                        self.status = Some(format!("{err:#}"));
                        return;
                    }
                }
            }
            self.show_all = true;
        } else {
            self.show_all = false;
        }

        self.status = None;
        self.selected = current_id
            .and_then(|id| self.sessions().iter().position(|s| s.id == id))
            .unwrap_or(0);
    }
}

/// Run the interactive session picker.
pub fn run(port: u16, open: bool) -> Result<()> {
    let sessions = sessions::list()?;

    let mut term = Term::new()?;
    let mut app = App {
        local_sessions: sessions,
        all_sessions: None,
        show_all: false,
        selected: 0,
        screen: Screen::List,
        running: true,
        status: None,
    };

    while app.running {
        term.terminal.draw(|frame| draw(frame, &app))?;

        if !event::poll(Duration::from_millis(200))? {
            continue;
        }

        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                handle_key(&mut app, key.code, key.modifiers, port, open, &mut term)?;
            }
            Event::Resize(_, _) => {}
            _ => {}
        }
    }

    Ok(())
}

fn handle_key(
    app: &mut App,
    code: KeyCode,
    modifiers: KeyModifiers,
    port: u16,
    open: bool,
    term: &mut Term,
) -> Result<()> {
    if modifiers.contains(KeyModifiers::CONTROL) && matches!(code, KeyCode::Char('c')) {
        app.running = false;
        return Ok(());
    }

    if matches!(app.screen, Screen::Stats(_)) {
        let mut back = false;
        let mut quit = false;

        if let Screen::Stats(state) = &mut app.screen {
            match code {
                KeyCode::Esc => back = true,
                KeyCode::Char('q') => quit = true,
                KeyCode::Tab => {
                    let next = state.period.next();
                    state.set_period(next);
                }
                KeyCode::BackTab => {
                    let previous = state.period.previous();
                    state.set_period(previous);
                }
                KeyCode::Char('g') => state.set_period(Period::Global),
                KeyCode::Char('m') => state.set_period(Period::Monthly),
                KeyCode::Char('w') => state.set_period(Period::Weekly),
                KeyCode::Char('d') => state.set_period(Period::Daily),
                KeyCode::Left | KeyCode::Char('h') => state.step(false),
                KeyCode::Right | KeyCode::Char('l') => state.step(true),
                KeyCode::Char('[') | KeyCode::Char('-') => state.older_page(),
                KeyCode::Char(']') | KeyCode::Char('+') => state.newer_page(),
                KeyCode::Char('r') => state.reload(),
                _ => {}
            }
        }

        if quit {
            app.running = false;
        } else if back {
            app.screen = Screen::List;
        }
        return Ok(());
    }

    match &app.screen {
        Screen::List => match code {
            KeyCode::Up | KeyCode::Char('k') => {
                app.selected = app.selected.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if app.selected + 1 < app.sessions().len() {
                    app.selected += 1;
                }
            }
            KeyCode::Home | KeyCode::Char('g') => app.selected = 0,
            KeyCode::End | KeyCode::Char('G') => {
                app.selected = app.sessions().len().saturating_sub(1);
            }
            KeyCode::Char('a') => app.toggle_scope(),
            KeyCode::Char('s') => app.screen = Screen::Stats(StatsState::new()),
            KeyCode::Enter => start_selected(app, port, open, term)?,
            KeyCode::Esc | KeyCode::Char('q') => app.running = false,
            _ => {}
        },
        Screen::Serving { .. } => match code {
            KeyCode::Esc | KeyCode::Char('b') => app.screen = Screen::List,
            KeyCode::Char('o') => {
                if let Screen::Serving { url, .. } = &app.screen {
                    let url = url.clone();
                    app.status = open_url(&url)
                        .err()
                        .map(|err| format!("could not open browser: {err}"));
                }
            }
            KeyCode::Char('q') => app.running = false,
            _ => {}
        },
        Screen::Error { .. } => match code {
            KeyCode::Char('q') => app.running = false,
            _ => app.screen = Screen::List,
        },
        Screen::Loading { .. } => {}
        Screen::Stats(_) => {}
    }

    Ok(())
}

fn start_selected(app: &mut App, port: u16, open: bool, term: &mut Term) -> Result<()> {
    let Some(session) = app.sessions().get(app.selected).cloned() else {
        return Ok(());
    };

    app.screen = Screen::Loading {
        title: session.title().to_string(),
    };
    term.terminal.draw(|frame| draw(frame, app))?;

    let mut open_error = None;
    let started = (|| -> Result<Server> {
        let export = load_export(&session.id)?;
        let html = render::render(&export);
        let server = Server::start(html, port)?;

        if open {
            server.wait_ready()?;
            open_error = open_url(server.url()).err();
        }

        Ok(server)
    })();

    match started {
        Ok(server) => {
            app.status = open_error.map(|err| format!("could not open browser: {err}"));
            app.screen = Screen::Serving {
                title: session.title().to_string(),
                id: session.id.clone(),
                directory: session
                    .directory
                    .clone()
                    .unwrap_or_else(|| "—".to_string()),
                url: server.url().to_string(),
                server,
            };
        }
        Err(err) => {
            app.screen = Screen::Error {
                message: format!("{err:#}"),
            };
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    frame.render_widget(Block::default().style(Style::default().bg(BG)), area);

    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(5),
        Constraint::Length(2),
    ])
    .split(area);

    draw_header(frame, chunks[0], &app.screen);

    match &app.screen {
        Screen::List => draw_list(frame, chunks[1], app),
        Screen::Loading { title } => draw_loading(frame, chunks[1], title),
        Screen::Serving {
            title,
            id,
            directory,
            url,
            ..
        } => draw_serving(frame, chunks[1], title, id, directory, url),
        Screen::Error { message } => draw_error(frame, chunks[1], message),
        Screen::Stats(state) => draw_stats(frame, chunks[1], state),
    }

    draw_footer(frame, chunks[2], app);
}

fn draw_header(frame: &mut Frame, area: ratatui::layout::Rect, screen: &Screen) {
    let subtitle = match screen {
        Screen::List => "Pick a session to inspect it in your browser",
        Screen::Loading { .. } => "Preparing session…",
        Screen::Serving { .. } => "Session is being served locally",
        Screen::Error { .. } => "Something went wrong",
        Screen::Stats(_) => "Usage statistics",
    };

    let lines = vec![
        Line::from(vec![
            Span::styled(
                "◈ mochuelo",
                Style::default()
                    .fg(ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  ·  ", Style::default().fg(FAINT)),
            Span::styled("OpenCode sessions", Style::default().fg(MUTED)),
        ]),
        Line::from(Span::styled(subtitle, Style::default().fg(FAINT))),
    ];

    let header = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(BORDER)),
    );
    frame.render_widget(header, area);
}

fn draw_footer(frame: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let hint = match &app.screen {
        Screen::List => vec![
            key("↑/↓"),
            plain(" or "),
            key("j/k"),
            plain(" move   "),
            key("Enter"),
            plain(" view   "),
            key("a"),
            plain(if app.show_all {
                " this project   "
            } else {
                " all projects   "
            }),
            key("s"),
            plain(" stats   "),
            key("q"),
            plain(" quit"),
        ],
        Screen::Loading { .. } => vec![plain("Exporting session…")],
        Screen::Serving { .. } => vec![
            key("Esc"),
            plain(" back   "),
            key("o"),
            plain(" reopen browser   "),
            key("q"),
            plain(" quit"),
        ],
        Screen::Error { .. } => vec![
            key("any key"),
            plain(" back to list   "),
            key("q"),
            plain(" quit"),
        ],
        Screen::Stats(_) => vec![
            key("Tab"),
            plain(" mode   "),
            key("←/→"),
            plain(" period   "),
            key("-/+"),
            plain(" page   "),
            key("g/m/w/d"),
            plain(" jump   "),
            key("Esc"),
            plain(" back   "),
            key("q"),
            plain(" quit"),
        ],
    };

    let mut lines = vec![Line::from(hint)];
    if let Some(status) = &app.status {
        lines.push(Line::from(Span::styled(
            status.to_string(),
            Style::default().fg(ERR),
        )));
    }

    let footer = Paragraph::new(lines)
        .style(Style::default().bg(BG))
        .wrap(Wrap { trim: true });
    frame.render_widget(footer, area);
}

fn draw_list(frame: &mut Frame, area: ratatui::layout::Rect, app: &App) {    let rows = app.sessions().iter().map(|session| {
        Row::new(vec![
            Cell::from(session.id.clone()).style(Style::default().fg(FAINT)),
            Cell::from(session.title().to_string()).style(Style::default().fg(TEXT)),
            Cell::from(session.when()).style(Style::default().fg(MUTED)),
        ])
    });

    let table = Table::new(
        rows,
        [
            Constraint::Length(36),
            Constraint::Min(16),
            Constraint::Length(17),
        ],
    )
    .header(
        Row::new(["ID", "TITLE", "CREATED"]).style(
            Style::default()
                .fg(FAINT)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .column_spacing(2)
    .block(
        Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(BORDER))
            .title(Span::styled(
                format!(
                    " sessions ({}) · {} ",
                    app.sessions().len(),
                    app.scope_label()
                ),
                Style::default().fg(MUTED),
            )),
    )
    .row_highlight_style(
        Style::default()
            .bg(HIGHLIGHT_BG)
            .fg(TEXT)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol("▌ ");

    let mut state = TableState::default();
    state.select(Some(app.selected));
    frame.render_stateful_widget(table, area, &mut state);
}

fn draw_stats(frame: &mut Frame, area: ratatui::layout::Rect, state: &StatsState) {
    let chunks = Layout::vertical([Constraint::Length(5), Constraint::Min(6)]).split(area);

    let mut tabs = Vec::new();
    for (index, period) in Period::ALL.iter().enumerate() {
        if index > 0 {
            tabs.push(Span::styled("   ", Style::default().fg(FAINT)));
        }
        let selected = *period == state.period;
        let style = if selected {
            Style::default()
                .bg(HIGHLIGHT_BG)
                .fg(TEXT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(MUTED)
        };
        tabs.push(Span::styled(format!(" {} ", period.label()), style));
    }

    let mut submenu = Vec::new();
    if state.period != Period::Global {
        let page = state.page();
        submenu.push(pager("[-]", true));
        submenu.push(Span::styled("  ", Style::default().fg(FAINT)));

        let size = state.page_size();
        let page_start = page * size;
        let mut first = true;
        for index in (0..size).rev() {
            let offset = page_start + index;
            if !first {
                submenu.push(Span::styled("  ", Style::default().fg(FAINT)));
            }
            first = false;
            if let Some(range) = state.period.range(offset) {
                let selected = offset == state.offset;
                let style = if selected {
                    Style::default()
                        .bg(HIGHLIGHT_BG)
                        .fg(TEXT)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(MUTED)
                };
                submenu.push(Span::styled(range.short_label, style));
            }
        }

        submenu.push(Span::styled("  ", Style::default().fg(FAINT)));
        submenu.push(pager("[+]", page > 0));
    }

    let header = Paragraph::new(vec![
        Line::from(tabs),
        Line::from(submenu),
        Line::from(Span::styled(
            state.label.clone(),
            Style::default().fg(ACCENT2),
        )),
    ]);
    frame.render_widget(header, chunks[0]);

    if let Some(error) = &state.error {
        let paragraph = Paragraph::new(Span::styled(error.clone(), Style::default().fg(ERR)))
            .wrap(Wrap { trim: true })
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(ERR))
                    .title(Span::styled(" usage ", Style::default().fg(ERR))),
            );
        frame.render_widget(paragraph, chunks[1]);
        return;
    }

    let body = Layout::vertical([Constraint::Min(5), Constraint::Length(12)]).split(chunks[1]);

    let stats = &state.data;
    let multiplier = match stats.cache_multiplier() {
        Some(value) => format!("x{value:.1}"),
        None => "—".to_string(),
    };
    let cached_pct = if stats.input_total == 0 {
        0.0
    } else {
        stats.cached_input as f64 * 100.0 / stats.input_total as f64
    };
    let uncached_input = stats.input_total.saturating_sub(stats.cached_input);
    let uncached_pct = if stats.input_total == 0 {
        0.0
    } else {
        uncached_input as f64 * 100.0 / stats.input_total as f64
    };

    let rows = vec![
        stat_row(
            "total (input + output)",
            render::fmt_compact(stats.total_tokens()),
        ),
        stat_indent_row(
            "├ input tokens",
            render::fmt_compact(stats.input_total),
            0,
        ),
        stat_sub_row(
            "│ ├",
            "cached input",
            format!(
                "{} ({cached_pct:.1}%)",
                render::fmt_compact(stats.cached_input)
            ),
        ),
        stat_sub_row(
            "│ └",
            "uncached input",
            format!(
                "{} ({uncached_pct:.1}%)",
                render::fmt_compact(uncached_input)
            ),
        ),
        stat_indent_row(
            "└ output tokens",
            render::fmt_compact(stats.output_total),
            0,
        ),
        stat_row("messages", render::fmt_int(stats.messages)),
        stat_row("cache multiplier", multiplier),
        stat_row(
            "total cost",
            format!(
                "{} ({} cost/1M)",
                render::fmt_cost(stats.cost),
                render::fmt_cost(stats.cost_per_million())
            ),
        ),
        stat_row(
            "session duration",
            render::fmt_duration(stats.duration_ms),
        ),
        stat_row("sessions", render::fmt_int(stats.sessions)),
    ];

    let table = Table::new(rows, [Constraint::Length(24), Constraint::Min(20)])
        .column_spacing(2)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(BORDER))
                .title(Span::styled(" usage ", Style::default().fg(MUTED))),
        );
    draw_activity(frame, body[0], state);
    frame.render_widget(table, body[1]);
}

fn pager(label: &str, enabled: bool) -> Span<'static> {
    Span::styled(
        format!(" {label} "),
        if enabled {
            Style::default().fg(ACCENT)
        } else {
            Style::default().fg(BORDER)
        },
    )
}

fn stat_row(label: &str, value: String) -> Row<'static> {
    stat_indent_row(label, value, 0)
}

fn stat_indent_row(label: &str, value: String, indent: usize) -> Row<'static> {
    Row::new(vec![
        Cell::from(format!("{}{}", " ".repeat(indent), label))
            .style(Style::default().fg(FAINT)),
        Cell::from(value).style(Style::default().fg(TEXT)),
    ])
}

fn stat_sub_row(connector: &str, label: &str, value: String) -> Row<'static> {
    let label = Text::from(Line::from(vec![
        Span::styled(format!("{connector} "), Style::default().fg(FAINT)),
        Span::styled(label.to_string(), Style::default().fg(SUBTLE)),
    ]));
    Row::new(vec![
        Cell::from(label),
        Cell::from(value).style(Style::default().fg(SUBTLE)),
    ])
}

fn draw_activity(frame: &mut Frame, area: ratatui::layout::Rect, state: &StatsState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(BORDER))
        .title(Span::styled(" activity ", Style::default().fg(MUTED)));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.period == Period::Global || state.activity.is_empty() {
        return;
    }

    let max_tokens = state
        .activity
        .iter()
        .map(|bucket| bucket.tokens)
        .max()
        .unwrap_or(0);
    let columns = (inner.width as usize / 4).max(1);

    let mut lines = Vec::new();
    let row_count = state.activity.len().div_ceil(columns);
    for (row, chunk) in state.activity.chunks(columns).enumerate() {
        let mut spans = Vec::new();
        for bucket in chunk {
            let color = ACTIVITY_COLORS[activity_level(bucket.tokens, max_tokens)];
            if bucket.offset == state.offset {
                spans.push(Span::styled("[", Style::default().fg(YELLOW)));
                spans.push(Span::styled("██", Style::default().fg(color)));
                spans.push(Span::styled("]", Style::default().fg(YELLOW)));
            } else {
                spans.push(Span::raw(" "));
                spans.push(Span::styled("██", Style::default().fg(color)));
                spans.push(Span::raw(" "));
            }
        }
        lines.push(Line::from(spans));
        if row + 1 < row_count {
            lines.push(Line::from(""));
        }
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}

fn activity_level(tokens: u64, max: u64) -> usize {
    if tokens == 0 || max == 0 {
        0
    } else {
        ((tokens as f64 / max as f64) * 4.0).ceil() as usize
    }
    .clamp(0, 4)
}

const ACTIVITY_COLORS: [Color; 5] = [
    Color::Rgb(34, 30, 48),
    Color::Rgb(57, 45, 91),
    Color::Rgb(91, 68, 147),
    Color::Rgb(132, 101, 205),
    Color::Rgb(196, 173, 255),
];

fn draw_loading(frame: &mut Frame, area: ratatui::layout::Rect, title: &str) {
    let chunks = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(3),
        Constraint::Fill(1),
    ])
    .split(area);

    let paragraph = Paragraph::new(Line::from(vec![
        Span::styled("Exporting ", Style::default().fg(MUTED)),
        Span::styled(
            title.to_string(),
            Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" …", Style::default().fg(MUTED)),
    ]))
    .alignment(Alignment::Center)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(BORDER))
            .title(Span::styled(" loading ", Style::default().fg(FAINT))),
    );

    frame.render_widget(paragraph, chunks[1]);
}

fn draw_serving(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    title: &str,
    id: &str,
    directory: &str,
    url: &str,
) {
    let chunks = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(12),
        Constraint::Fill(1),
    ])
    .split(area);

    let lines = vec![
        Line::from(Span::styled(
            title.to_string(),
            Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("URL   ", Style::default().fg(FAINT)),
            Span::styled(
                url.to_string(),
                Style::default()
                    .fg(ACCENT2)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("ID    ", Style::default().fg(FAINT)),
            Span::styled(id.to_string(), Style::default().fg(MUTED)),
        ]),
        Line::from(vec![
            Span::styled("DIR   ", Style::default().fg(FAINT)),
            Span::styled(directory.to_string(), Style::default().fg(MUTED)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Serving locally. Open the URL above if your browser did not open.",
            Style::default().fg(MUTED),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Press Esc to pick another session.",
            Style::default().fg(FAINT),
        )),
    ];

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: true }).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT))
            .title(Span::styled(" serving ", Style::default().fg(ACCENT)))
            .style(Style::default().bg(SURFACE)),
    );

    frame.render_widget(paragraph, chunks[1]);
}

fn draw_error(frame: &mut Frame, area: ratatui::layout::Rect, message: &str) {
    let chunks = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(8),
        Constraint::Fill(1),
    ])
    .split(area);

    let lines = vec![
        Line::from(Span::styled(
            "Could not open this session",
            Style::default().fg(ERR).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(message.to_string(), Style::default().fg(MUTED))),
    ];

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: true }).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ERR))
            .title(Span::styled(" error ", Style::default().fg(ERR))),
    );

    frame.render_widget(paragraph, chunks[1]);
}

fn key(text: &str) -> Span<'static> {
    Span::styled(
        format!(" {text} "),
        Style::default()
            .fg(TEXT)
            .bg(Color::Rgb(26, 32, 44))
            .add_modifier(Modifier::BOLD),
    )
}

fn plain(text: &str) -> Span<'static> {
    Span::styled(text.to_string(), Style::default().fg(FAINT))
}

// ---------------------------------------------------------------------------
// Terminal lifecycle
// ---------------------------------------------------------------------------

struct Term {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl Term {
    fn new() -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        execute!(stdout, EnterAlternateScreen, crossterm::cursor::Hide)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;
        terminal.clear()?;
        Ok(Self { terminal })
    }
}

impl Drop for Term {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            self.terminal.backend_mut(),
            LeaveAlternateScreen,
            crossterm::cursor::Show
        );
        let _ = self.terminal.show_cursor();
    }
}
