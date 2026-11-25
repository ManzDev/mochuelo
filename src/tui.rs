use crate::render;
use crate::server::Server;
use crate::sessions::{self, Session};
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
use ratatui::text::{Line, Span};
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
    if sessions.is_empty() {
        anyhow::bail!("no OpenCode sessions found for this project");
    }

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
    }

    draw_footer(frame, chunks[2], app);
}

fn draw_header(frame: &mut Frame, area: ratatui::layout::Rect, screen: &Screen) {
    let subtitle = match screen {
        Screen::List => "Pick a session to inspect it in your browser",
        Screen::Loading { .. } => "Preparing session…",
        Screen::Serving { .. } => "Session is being served locally",
        Screen::Error { .. } => "Something went wrong",
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

fn draw_list(frame: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let rows = app.sessions().iter().map(|session| {
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
