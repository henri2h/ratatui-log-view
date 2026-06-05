use std::{io, time::Duration};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Layout},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use tui_hex_view::{HexView, HexViewEvent, HexViewState, LogView, LogViewState};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    Log,
    Hex,
}

struct DemoApp {
    focus: Focus,
    log: LogViewState,
    hex: HexViewState,
    search_input: Option<String>,
    status: String,
    next_log_id: usize,
    next_marker_id: usize,
}

impl DemoApp {
    fn new() -> Self {
        let mut log = LogViewState::from_text(
            "\u{1b}[32mINFO\u{1b}[0m booting viewer demo\n\
             \u{1b}[33mWARN\u{1b}[0m wrap-aware scrolling is enabled\n\
             \u{1b}[31mERROR\u{1b}[0m sample failure signature found\n\
             \u{1b}[34mDEBUG\u{1b}[0m press 'a' to append more logs",
        );
        log.set_line_limit(Some(512));
        log.push_lines((1..=12).map(|i| {
            let (level, color) = match i % 4 {
                0 => ("INFO", 32),
                1 => ("WARN", 33),
                2 => ("ERROR", 31),
                _ => ("DEBUG", 34),
            };
            format!(
                "\u{1b}[{color}m{level}\u{1b}[0m seeded demo line #{i}: this intentionally makes the log panel long enough to scroll and includes a few wrapped messages for visibility"
            )
        }));
        log.set_search_query("error");

        let mut hex = HexViewState::new(
            b"\x7fELF\x02\x01\x01\x00firmware-demo\x00\x10\x20\x30\xffhello world".to_vec(),
        );
        hex.add_marker(0, "header", Color::Cyan);

        Self {
            focus: Focus::Log,
            log,
            hex,
            search_input: None,
            status: "Tab switches panels. '/' starts log search. 'w' toggles log wrap. 'a' appends a log line.".into(),
            next_log_id: 13,
            next_marker_id: 1,
        }
    }

    fn append_log(&mut self) {
        let level = match self.next_log_id % 4 {
            0 => "INFO",
            1 => "WARN",
            2 => "ERROR",
            _ => "DEBUG",
        };
        let color = match level {
            "INFO" => 32,
            "WARN" => 33,
            "ERROR" => 31,
            _ => 34,
        };
        self.log.push_lines(vec![format!(
            "\u{1b}[{color}m{level}\u{1b}[0m appended log line #{} for demo scrolling",
            self.next_log_id
        )]);
        self.status = format!("Appended log line #{}", self.next_log_id);
        self.next_log_id += 1;
    }

    fn handle_search_key(&mut self, key: KeyEvent) {
        let Some(query) = self.search_input.as_mut() else {
            return;
        };

        match key.code {
            KeyCode::Esc => {
                self.search_input = None;
                self.status = "Search cancelled".into();
            }
            KeyCode::Enter => {
                let query = self.search_input.take().unwrap_or_default();
                if query.is_empty() {
                    self.log.clear_search();
                    self.status = "Search cleared".into();
                } else {
                    self.log.set_search_query(query.clone());
                    self.status = format!("Searching for '{query}'");
                }
            }
            KeyCode::Backspace => {
                query.pop();
            }
            KeyCode::Char(c)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                query.push(c);
            }
            _ => {}
        }
    }

    fn handle_hex_event(&mut self, event: HexViewEvent) {
        match event {
            HexViewEvent::ByteEdited { pos, old, new } => {
                self.status = format!("Edited byte 0x{pos:04x}: {old:02x} -> {new:02x}");
            }
            HexViewEvent::MarkerRequested { at } => {
                let label = format!("mark{}", self.next_marker_id);
                self.hex.add_marker(at, label.clone(), Color::Yellow);
                self.next_marker_id += 1;
                self.status = format!("Added marker '{label}' at 0x{at:04x}");
            }
            HexViewEvent::ModeChanged { new_mode } => {
                self.status = format!("Hex view mode: {}", new_mode.label());
            }
            HexViewEvent::EditStarted { at } => {
                self.status = format!("Editing byte 0x{at:04x} — type two hex digits");
            }
            HexViewEvent::EditCancelled => {
                self.status = "Hex edit cancelled".into();
            }
            HexViewEvent::MarkersCleared => {
                self.status = "Cleared markers".into();
            }
            HexViewEvent::BytesReset => {
                self.status = "Reset bytes to baseline".into();
            }
            HexViewEvent::CursorMoved { new_pos } => {
                self.status = format!("Cursor at 0x{new_pos:04x}");
            }
            HexViewEvent::None => {}
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        if key.kind != KeyEventKind::Press {
            return false;
        }

        if self.search_input.is_some() {
            self.handle_search_key(key);
            return false;
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return true,
            KeyCode::Tab => {
                self.focus = match self.focus {
                    Focus::Log => Focus::Hex,
                    Focus::Hex => Focus::Log,
                };
            }
            KeyCode::Char('/') if self.focus == Focus::Log => {
                self.search_input = Some(String::new());
                self.status = "Type a search term and press Enter".into();
            }
            KeyCode::Char('a') => self.append_log(),
            KeyCode::Char('w') if self.focus == Focus::Log => {
                let enabled = self.log.toggle_word_wrap();
                self.status = if enabled {
                    "Enabled log word wrap".into()
                } else {
                    "Disabled log word wrap".into()
                };
            }
            KeyCode::Char('n') if self.focus == Focus::Log && self.log.has_search() => {
                self.log.next_match();
                self.status = "Jumped to next log match".into();
            }
            KeyCode::Char('N') if self.focus == Focus::Log && self.log.has_search() => {
                self.log.prev_match();
                self.status = "Jumped to previous log match".into();
            }
            KeyCode::Up if self.focus == Focus::Log => self.log.scroll_up(),
            KeyCode::Down if self.focus == Focus::Log => self.log.scroll_down(),
            KeyCode::PageUp if self.focus == Focus::Log => self.log.page_up(),
            KeyCode::PageDown if self.focus == Focus::Log => self.log.page_down(),
            KeyCode::Home if self.focus == Focus::Log => self.log.scroll_to_start(),
            KeyCode::End if self.focus == Focus::Log => self.log.scroll_to_end(),
            _ if self.focus == Focus::Hex => {
                let event = self.hex.handle_key(key);
                self.handle_hex_event(event);
            }
            _ => {}
        }

        false
    }
}

fn draw(frame: &mut Frame, app: &mut DemoApp) {
    let [main, footer] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(3)]).areas(frame.area());
    let [log_area, hex_area] =
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).areas(main);

    let log_title = if app.focus == Focus::Log {
        if app.log.word_wrap {
            " Logs [focused, wrap] "
        } else {
            " Logs [focused, nowrap] "
        }
    } else {
        if app.log.word_wrap {
            " Logs [wrap] "
        } else {
            " Logs [nowrap] "
        }
    };
    let hex_title = if app.focus == Focus::Hex {
        " Hex / ASCII [focused] "
    } else {
        " Hex / ASCII "
    };

    frame.render_widget(
        LogView::new(&mut app.log).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(if app.focus == Focus::Log {
                    Style::default().fg(Color::Cyan)
                } else {
                    Style::default()
                })
                .title(log_title),
        ),
        log_area,
    );

    frame.render_widget(
        HexView::new(&mut app.hex).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(if app.focus == Focus::Hex {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                })
                .title(hex_title),
        ),
        hex_area,
    );

    let footer_line = if let Some(query) = &app.search_input {
        Line::from(vec![
            Span::styled("/", Style::default().fg(Color::Yellow).bold()),
            Span::raw(query),
            Span::styled("█", Style::default().fg(Color::Yellow)),
        ])
    } else {
        Line::from(vec![
            Span::styled("Tab", Style::default().fg(Color::Cyan)),
            Span::raw(" switch  "),
            Span::styled("/", Style::default().fg(Color::Yellow)),
            Span::raw(" search logs  "),
            Span::styled("n/N", Style::default().fg(Color::Yellow)),
            Span::raw(" next/prev match  "),
            Span::styled("w", Style::default().fg(Color::Yellow)),
            Span::raw(" wrap  "),
            Span::styled("a", Style::default().fg(Color::Yellow)),
            Span::raw(" append log  "),
            Span::styled("q", Style::default().fg(Color::Yellow)),
            Span::raw(" quit"),
        ])
    };

    let status = Paragraph::new(vec![footer_line, Line::raw(app.status.clone())]).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Demo controls "),
    );
    frame.render_widget(status, footer);
}

fn run(mut terminal: DefaultTerminal) -> io::Result<()> {
    let mut app = DemoApp::new();

    loop {
        terminal.draw(|frame| draw(frame, &mut app))?;

        if event::poll(Duration::from_millis(250))?
            && let Event::Key(key) = event::read()?
            && app.handle_key(key)
        {
            break;
        }
    }

    Ok(())
}

fn main() -> io::Result<()> {
    let terminal = ratatui::init();
    let result = run(terminal);
    ratatui::restore();
    result
}
