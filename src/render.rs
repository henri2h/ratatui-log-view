//! Rendering logic for the crate widgets.

use ansi_to_tui::IntoText as _;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Paragraph, Widget, Wrap},
};

use crate::{HexViewState, LogViewState, ViewMode, ansi_visible_len, ascii_view};

const DEFAULT_COLS: usize = 16;

/// The ratatui widget for interactive hex / ascii viewing.
pub struct HexView<'a> {
    state: &'a mut HexViewState,
    block: Option<Block<'a>>,
    /// Bytes per row — set automatically based on available width in `render`.
    columns: Option<usize>,
}

/// A read-only log widget with ANSI rendering, wrapped-row scrolling, and search highlighting.
pub struct LogView<'a> {
    state: &'a mut LogViewState,
    block: Option<Block<'a>>,
}

impl<'a> HexView<'a> {
    /// Create a view for the given state.
    pub fn new(state: &'a mut HexViewState) -> Self {
        Self {
            state,
            block: None,
            columns: None,
        }
    }

    /// Attach a surrounding block (border + title).
    pub fn block(mut self, block: Block<'a>) -> Self {
        self.block = Some(block);
        self
    }

    /// Override the number of bytes per row (default: adaptive 16 or 8).
    pub fn columns(mut self, cols: usize) -> Self {
        self.columns = Some(cols);
        self
    }
}

impl<'a> LogView<'a> {
    /// Create a view for the given state.
    pub fn new(state: &'a mut LogViewState) -> Self {
        Self { state, block: None }
    }

    /// Attach a surrounding block (border + title).
    pub fn block(mut self, block: Block<'a>) -> Self {
        self.block = Some(block);
        self
    }
}

impl Widget for HexView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let inner = match &self.block {
            Some(b) => {
                let inner = b.inner(area);
                b.clone().render(area, buf);
                inner
            }
            None => area,
        };

        let cols = self.columns.unwrap_or({
            if inner.width as usize >= 76 {
                DEFAULT_COLS
            } else {
                8
            }
        });
        self.state.cols = cols;

        match self.state.mode {
            ViewMode::HexAscii | ViewMode::HexOnly => render_hex(self.state, cols, inner, buf),
            ViewMode::AsciiOnly => render_ascii(self.state, inner, buf),
        }
    }
}

impl Widget for LogView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let inner = match &self.block {
            Some(b) => {
                let inner = b.inner(area);
                b.clone().render(area, buf);
                inner
            }
            None => area,
        };

        render_log(self.state, inner, buf);
    }
}

// ── Hex renderer ─────────────────────────────────────────────────────────────

fn render_hex(state: &mut HexViewState, cols: usize, area: Rect, buf: &mut Buffer) {
    state.clamp_scroll();
    let bytes = &state.bytes;
    let total_rows = if bytes.is_empty() {
        0
    } else {
        bytes.len().div_ceil(cols)
    };
    let visible = area.height as usize;
    state.visible_rows = visible.max(1);

    let scroll = state.scroll.min(total_rows.saturating_sub(1));
    state.scroll = scroll;

    let mut marker_map: std::collections::HashMap<usize, Vec<usize>> =
        std::collections::HashMap::new();
    for (idx, m) in state.markers.iter().enumerate() {
        marker_map.entry(m.offset / cols).or_default().push(idx);
    }

    let show_ascii = state.mode == ViewMode::HexAscii;
    let mut rendered = 0usize;
    let cursor_row = state.cursor / cols;

    for row_idx in scroll..total_rows {
        if rendered >= visible {
            break;
        }

        if let Some(idxs) = marker_map.get(&row_idx) {
            let reserve_data_row = row_idx == cursor_row;
            let available_marker_lines = if reserve_data_row {
                visible.saturating_sub(rendered + 1)
            } else {
                visible.saturating_sub(rendered)
            };
            let marker_start = idxs.len().saturating_sub(available_marker_lines);

            for &mi in idxs.iter().skip(marker_start) {
                if rendered >= visible {
                    break;
                }
                let m = &state.markers[mi];
                let sep = format!("── {} (0x{:04x}) ", m.label, m.offset);
                let line = Line::from(Span::styled(sep, Style::default().fg(m.color)));
                render_line(line, area, rendered, buf);
                rendered += 1;
            }
            if rendered >= visible {
                break;
            }
        }

        let byte_start = row_idx * cols;
        let row_bytes = &bytes[byte_start..bytes.len().min(byte_start + cols)];

        let mut spans: Vec<Span<'static>> = Vec::new();
        spans.push(Span::styled(
            format!("{:08x}  ", byte_start),
            Style::default().fg(Color::DarkGray),
        ));

        for (i, &b) in row_bytes.iter().enumerate() {
            let abs = byte_start + i;
            let is_cursor = abs == state.cursor;
            let is_editing = state
                .edit
                .as_ref()
                .map(|e| e.byte_pos == abs)
                .unwrap_or(false);
            let is_modified = state
                .original_bytes
                .as_ref()
                .and_then(|o| o.get(abs))
                .map(|&orig| orig != b)
                .unwrap_or(false);

            let style = if is_editing {
                Style::default().fg(Color::Black).bg(Color::Red)
            } else if is_cursor {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else if is_modified {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default().fg(Color::White)
            };

            spans.push(Span::styled(format!("{b:02x}"), style));
            if i == 7 && cols > 8 {
                spans.push(Span::raw("  "));
            } else if i + 1 < row_bytes.len() {
                spans.push(Span::raw(" "));
            }
        }

        let hex_len = row_bytes.len() * 3
            + if row_bytes.len() > 8 && cols > 8 {
                1
            } else {
                0
            };
        let pad_target = cols * 3 + if cols > 8 { 1 } else { 0 };
        if hex_len < pad_target {
            spans.push(Span::raw(" ".repeat(pad_target - hex_len)));
        }

        if show_ascii {
            spans.push(Span::raw("  │"));
            for (i, &b) in row_bytes.iter().enumerate() {
                let abs = byte_start + i;
                let is_cursor = abs == state.cursor;
                let ch = if b.is_ascii_graphic() || b == b' ' {
                    b as char
                } else {
                    '·'
                };
                let style = if is_cursor {
                    Style::default().fg(Color::Black).bg(Color::Cyan)
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                spans.push(Span::styled(ch.to_string(), style));
            }
            spans.push(Span::raw("│"));
        }

        render_line(Line::from(spans), area, rendered, buf);
        rendered += 1;
    }
}

// ── ASCII renderer ────────────────────────────────────────────────────────────

fn render_ascii(state: &mut HexViewState, area: Rect, buf: &mut Buffer) {
    let text = ascii_view(&state.bytes);
    let lines: Vec<&str> = text.lines().collect();
    let visible = area.height as usize;
    state.visible_rows = visible.max(1);

    let max_scroll = lines.len().saturating_sub(visible);
    state.scroll = state.scroll.min(max_scroll);

    for (i, line_text) in lines.iter().skip(state.scroll).take(visible).enumerate() {
        let line = Line::from(Span::styled(
            line_text.to_string(),
            Style::default().fg(Color::White),
        ));
        render_line(line, area, i, buf);
    }
}

// ── Log renderer ──────────────────────────────────────────────────────────────

fn render_log(state: &mut LogViewState, area: Rect, buf: &mut Buffer) {
    let visible = area.height as usize;
    let wrap_at = (area.width as usize).max(1);
    state.visible_rows = visible.max(1);
    state.ensure_wrap_geometry(wrap_at);

    if state.auto_scroll {
        state.scroll = state.max_scroll;
        state.auto_scroll = false;
    } else {
        state.clamp_scroll();
    }

    let start = state.scroll;
    let first_idx = state
        .row_offsets
        .partition_point(|&row| row <= start)
        .saturating_sub(1);
    let scroll_within =
        start.saturating_sub(state.row_offsets.get(first_idx).copied().unwrap_or(0));

    let mut covered_rows = 0usize;
    let mut window_text = Text::default();
    for line_idx in first_idx..state.lines.len() {
        let Some(raw) = state.lines.get(line_idx) else {
            break;
        };
        let mut parsed = raw
            .as_bytes()
            .into_text()
            .unwrap_or_else(|_| Text::raw(strip_control_only(raw)));
        patch_log_line_style(&mut parsed, state, line_idx);
        covered_rows += if state.word_wrap {
            wrapped_row_count(raw, wrap_at)
        } else {
            1
        };
        window_text.lines.extend(parsed.lines);
        if covered_rows >= scroll_within + visible {
            break;
        }
    }

    let paragraph =
        Paragraph::new(window_text).scroll((scroll_within.min(u16::MAX as usize) as u16, 0));
    if state.word_wrap {
        paragraph.wrap(Wrap { trim: false }).render(area, buf);
    } else {
        paragraph.render(area, buf);
    }
}

fn patch_log_line_style(text: &mut Text<'_>, state: &LogViewState, line_idx: usize) {
    if state.highlighted_line == Some(line_idx) {
        let style = Style::default().bg(Color::Rgb(30, 70, 100));
        text.lines = std::mem::take(&mut text.lines)
            .into_iter()
            .map(|line| line.patch_style(style))
            .collect();
        return;
    }

    let current_match = state.current_match().filter(|m| m.line == line_idx);
    let matches: Vec<_> = state
        .search_matches
        .iter()
        .filter(|m| m.line == line_idx)
        .collect();

    if matches.is_empty() {
        return;
    }

    text.lines = std::mem::take(&mut text.lines)
        .into_iter()
        .map(|line| highlight_log_matches(line, &matches, current_match))
        .collect();
}

fn highlight_log_matches(
    line: Line<'_>,
    matches: &[&crate::LogMatch],
    current_match: Option<&crate::LogMatch>,
) -> Line<'static> {
    let mut out = Vec::new();
    let mut visible_col = 0usize;

    for span in line.spans {
        let mut segment = String::new();
        let mut segment_style = None;

        for ch in span.content.chars() {
            let char_width = crate::display_width_char(ch);
            let style = if char_width == 0 {
                segment_style.unwrap_or(span.style)
            } else {
                let char_start = visible_col;
                let char_end = char_start + char_width;
                let active_match = matches
                    .iter()
                    .find(|matched| {
                        matched.range.start < char_end && char_start < matched.range.end
                    })
                    .copied();

                if let Some(matched) = active_match {
                    let overlay = if current_match == Some(matched) {
                        Style::default()
                            .bg(Color::Yellow)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().bg(Color::Rgb(70, 55, 0))
                    };
                    span.style.patch(overlay)
                } else {
                    span.style
                }
            };

            if segment_style != Some(style) && !segment.is_empty() {
                out.push(Span::styled(
                    std::mem::take(&mut segment),
                    segment_style.unwrap(),
                ));
            }

            segment_style = Some(style);
            segment.push(ch);
            visible_col += char_width;
        }

        if let Some(style) = segment_style {
            out.push(Span::styled(segment, style));
        }
    }

    Line::from(out)
}

fn wrapped_row_count(line: &str, wrap_at: usize) -> usize {
    let visible = ansi_visible_len(line);
    if visible == 0 {
        1
    } else {
        visible.div_ceil(wrap_at.max(1))
    }
}

fn strip_control_only(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect()
}

// ── Helper: render a Line at a given row within an area ───────────────────────

fn render_line(line: Line<'_>, area: Rect, row: usize, buf: &mut Buffer) {
    let y = area.y + row as u16;
    if y >= area.y + area.height {
        return;
    }
    let max_x = area.x + area.width;
    let mut x = area.x;
    for span in &line.spans {
        let style = span.style;
        for ch in span.content.chars() {
            if x >= max_x {
                break;
            }
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_char(ch);
                cell.set_style(style);
            }
            x += 1;
        }
        if x >= max_x {
            break;
        }
    }
}
