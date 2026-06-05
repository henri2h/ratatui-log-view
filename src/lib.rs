//! `tui-hex-view` — lightweight ratatui widgets for viewing logs and binary data.
//!
//! Run `cargo run --example demo` for a small interactive demo showing both widgets.
//!
//! # Hex view quick start
//! ```rust,no_run
//! use tui_hex_view::{HexView, HexViewState, HexViewEvent};
//! use crossterm::event::{read, Event, KeyEvent};
//!
//! let bytes = b"Hello, world!".to_vec();
//! let mut state = HexViewState::new(bytes);
//!
//! // In your render function:
//! // frame.render_widget(HexView::new(&mut state), area);
//!
//! // In your event loop:
//! // if let Event::Key(key) = read().unwrap() {
//! //     match state.handle_key(key) {
//! //         HexViewEvent::ByteEdited { pos, old, new } => { /* react */ }
//! //         HexViewEvent::MarkerRequested { at } => { /* show label dialog */ }
//! //         _ => {}
//! //     }
//! // }
//! ```
//!
//! # Log view quick start
//! ```rust
//! use tui_hex_view::{LogView, LogViewState};
//! use ratatui::{buffer::Buffer, layout::Rect, widgets::Widget};
//!
//! let mut state = LogViewState::from_text("INFO booting\nERROR timeout\nINFO retrying");
//! state.set_search_query("error");
//!
//! let area = Rect::new(0, 0, 40, 3);
//! let mut buf = Buffer::empty(area);
//! LogView::new(&mut state).render(area, &mut buf);
//! ```

mod input;
mod render;

pub use render::{HexView, LogView};

use unicode_width::UnicodeWidthChar;

// ── View mode ────────────────────────────────────────────────────────────────

/// Display mode for the hex viewer.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ViewMode {
    /// Hex bytes on the left, ASCII on the right (xxd-style). Default.
    #[default]
    HexAscii,
    /// Hex bytes only — no ASCII column.
    HexOnly,
    /// Plain ASCII text — non-printable bytes shown as `·`.
    AsciiOnly,
}

impl ViewMode {
    /// Cycle to the next mode: HexAscii → HexOnly → AsciiOnly → HexAscii.
    pub fn next(self) -> Self {
        match self {
            Self::HexAscii => Self::HexOnly,
            Self::HexOnly => Self::AsciiOnly,
            Self::AsciiOnly => Self::HexAscii,
        }
    }

    /// Human-readable label.
    pub fn label(self) -> &'static str {
        match self {
            Self::HexAscii => "hex+ascii",
            Self::HexOnly => "hex",
            Self::AsciiOnly => "ascii",
        }
    }
}

// ── Marker ───────────────────────────────────────────────────────────────────

/// A named, coloured annotation anchored to a byte offset.
#[derive(Clone, Debug)]
pub struct Marker {
    /// Byte offset within the buffer.
    pub offset: usize,
    /// Short display label shown in the separator line.
    pub label: String,
    /// Colour used for the separator line and label text.
    pub color: ratatui::style::Color,
}

// ── Nibble editing state ──────────────────────────────────────────────────────

/// Two-pass nibble input state for in-place hex editing.
#[derive(Clone, Debug)]
pub struct NibbleEdit {
    /// Byte offset being edited.
    pub byte_pos: usize,
    /// `Some(high)` after the first nibble is typed; `None` when waiting for the first nibble.
    pub high_nibble: Option<u8>,
}

// ── HexViewState ─────────────────────────────────────────────────────────────

/// All mutable state owned by the hex viewer.  One instance per widget use-site.
pub struct HexViewState {
    /// The bytes being displayed / edited.
    pub bytes: Vec<u8>,
    /// Snapshot taken at load time; bytes that differ from this are highlighted yellow.
    /// Set to `None` to disable modification highlighting.
    pub original_bytes: Option<Vec<u8>>,
    /// Byte-offset of the cursor (hex and ascii columns track it together).
    pub cursor: usize,
    /// First visible row (in row units, not bytes).
    pub scroll: usize,
    /// Active display mode.
    pub mode: ViewMode,
    /// Active nibble-edit session, if any.
    pub edit: Option<NibbleEdit>,
    /// Ordered list of named markers.
    pub markers: Vec<Marker>,
    /// Number of visible rows from the last render call.  Updated automatically;
    /// used by scroll-sync logic in the host app.
    pub visible_rows: usize,
    /// Number of bytes per row (16 by default; render sets this based on area width).
    pub(crate) cols: usize,
}

impl HexViewState {
    /// Create a new state with the given bytes.
    /// `original_bytes` is set to a clone so modification highlighting works out of the box.
    pub fn new(bytes: Vec<u8>) -> Self {
        let original = bytes.clone();
        Self {
            bytes,
            original_bytes: Some(original),
            cursor: 0,
            scroll: 0,
            mode: ViewMode::default(),
            edit: None,
            markers: Vec::new(),
            visible_rows: 20,
            cols: 16,
        }
    }

    /// Replace the byte buffer and reset cursor / scroll / edit state.
    /// The new bytes become the new `original_bytes` baseline.
    pub fn set_bytes(&mut self, bytes: Vec<u8>) {
        self.original_bytes = Some(bytes.clone());
        self.bytes = bytes;
        self.cursor = 0;
        self.scroll = 0;
        self.edit = None;
        self.markers.clear();
    }

    /// Reset the modification baseline to the current bytes (mark all as unmodified).
    pub fn reset_baseline(&mut self) {
        self.original_bytes = Some(self.bytes.clone());
    }

    /// Add a named marker at `offset`.
    pub fn add_marker(
        &mut self,
        offset: usize,
        label: impl Into<String>,
        color: ratatui::style::Color,
    ) {
        // Remove any existing marker at the same offset first.
        self.markers.retain(|m| m.offset != offset);
        self.markers.push(Marker {
            offset,
            label: label.into(),
            color,
        });
        self.markers.sort_by_key(|m| m.offset);
    }

    /// Remove the marker at `offset` (if any).
    pub fn remove_marker(&mut self, offset: usize) {
        self.markers.retain(|m| m.offset != offset);
    }

    /// Clear all markers.
    pub fn clear_markers(&mut self) {
        self.markers.clear();
    }

    /// Cycle to the next display mode.
    pub fn cycle_mode(&mut self) {
        self.mode = self.mode.next();
    }

    /// Total number of rows for the current buffer.
    pub fn total_rows(&self) -> usize {
        if self.cols == 0 || self.bytes.is_empty() {
            0
        } else {
            self.bytes.len().div_ceil(self.cols)
        }
    }

    /// Ensure scroll stays within the currently available rows.
    pub fn clamp_scroll(&mut self) {
        let max = self.total_rows().saturating_sub(1);
        self.scroll = self.scroll.min(max);
    }

    /// Ensure cursor is within bounds.
    pub fn clamp_cursor(&mut self) {
        let max = self.bytes.len().saturating_sub(1);
        self.cursor = self.cursor.min(max);
    }

    fn marker_lines_between(&self, start_row: usize, end_row_exclusive: usize) -> usize {
        if self.cols == 0 || start_row >= end_row_exclusive {
            return 0;
        }

        self.markers
            .iter()
            .filter(|marker| {
                let row = marker.offset / self.cols;
                row >= start_row && row < end_row_exclusive
            })
            .count()
    }

    /// Scroll so the cursor row is visible.
    pub fn sync_scroll(&mut self) {
        if self.cols == 0 || self.bytes.is_empty() {
            self.scroll = 0;
            return;
        }

        self.clamp_cursor();
        self.clamp_scroll();

        let row = self.cursor / self.cols;
        if row < self.scroll {
            self.scroll = row;
        } else {
            let visible_rows = self.visible_rows.max(1);
            while self.scroll < row {
                let lines_before_cursor =
                    (row - self.scroll) + self.marker_lines_between(self.scroll, row);
                if lines_before_cursor + 1 <= visible_rows {
                    break;
                }
                self.scroll += 1;
            }
        }
    }

    /// Process a key event and return the resulting [`HexViewEvent`].
    pub fn handle_key(&mut self, key: crossterm::event::KeyEvent) -> HexViewEvent {
        use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
        if key.kind != KeyEventKind::Press {
            return HexViewEvent::None;
        }

        // ── While in nibble-edit mode ─────────────────────────────────────────
        if self.edit.is_some() {
            return self.handle_nibble(key.code);
        }

        match key.code {
            // Navigation
            KeyCode::Left | KeyCode::Char('h') => {
                self.cursor = self.cursor.saturating_sub(1);
                self.sync_scroll();
                HexViewEvent::CursorMoved {
                    new_pos: self.cursor,
                }
            }
            KeyCode::Right | KeyCode::Char('l') => {
                let max = self.bytes.len().saturating_sub(1);
                self.cursor = (self.cursor + 1).min(max);
                self.sync_scroll();
                HexViewEvent::CursorMoved {
                    new_pos: self.cursor,
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.cursor = self.cursor.saturating_sub(self.cols);
                self.sync_scroll();
                HexViewEvent::CursorMoved {
                    new_pos: self.cursor,
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let max = self.bytes.len().saturating_sub(1);
                self.cursor = (self.cursor + self.cols).min(max);
                self.sync_scroll();
                HexViewEvent::CursorMoved {
                    new_pos: self.cursor,
                }
            }
            KeyCode::PageUp => {
                let step = self.visible_rows * self.cols;
                self.cursor = self.cursor.saturating_sub(step);
                self.sync_scroll();
                HexViewEvent::CursorMoved {
                    new_pos: self.cursor,
                }
            }
            KeyCode::PageDown => {
                let max = self.bytes.len().saturating_sub(1);
                let step = self.visible_rows * self.cols;
                self.cursor = (self.cursor + step).min(max);
                self.sync_scroll();
                HexViewEvent::CursorMoved {
                    new_pos: self.cursor,
                }
            }
            KeyCode::Home | KeyCode::Char('g') if !key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.cursor = 0;
                self.sync_scroll();
                HexViewEvent::CursorMoved { new_pos: 0 }
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.cursor = self.bytes.len().saturating_sub(1);
                self.sync_scroll();
                HexViewEvent::CursorMoved {
                    new_pos: self.cursor,
                }
            }
            // Enter edit mode on Enter
            KeyCode::Enter => {
                if !self.bytes.is_empty() {
                    self.edit = Some(NibbleEdit {
                        byte_pos: self.cursor,
                        high_nibble: None,
                    });
                    HexViewEvent::EditStarted { at: self.cursor }
                } else {
                    HexViewEvent::None
                }
            }
            // Escape: cancel any edit
            KeyCode::Esc => {
                self.edit = None;
                HexViewEvent::EditCancelled
            }
            // 'm' — request a marker at the cursor
            KeyCode::Char('m') | KeyCode::Char('M') => {
                HexViewEvent::MarkerRequested { at: self.cursor }
            }
            // 'v' — cycle view mode
            KeyCode::Char('v') | KeyCode::Char('V') => {
                self.cycle_mode();
                HexViewEvent::ModeChanged {
                    new_mode: self.mode,
                }
            }
            // 'c' — clear all markers
            KeyCode::Char('c') | KeyCode::Char('C') => {
                self.clear_markers();
                HexViewEvent::MarkersCleared
            }
            // 'r' — reset bytes to baseline
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(orig) = self.original_bytes.clone() {
                    self.bytes = orig;
                    self.edit = None;
                    HexViewEvent::BytesReset
                } else {
                    HexViewEvent::None
                }
            }
            _ => HexViewEvent::None,
        }
    }

    fn handle_nibble(&mut self, code: crossterm::event::KeyCode) -> HexViewEvent {
        use crossterm::event::KeyCode;
        if code == KeyCode::Esc {
            self.edit = None;
            return HexViewEvent::EditCancelled;
        }
        let nibble = match code {
            KeyCode::Char(c) => match c.to_ascii_lowercase() {
                '0'..='9' => c as u8 - b'0',
                'a'..='f' => c as u8 - b'a' + 10,
                _ => {
                    self.edit = None;
                    return HexViewEvent::EditCancelled;
                }
            },
            _ => {
                self.edit = None;
                return HexViewEvent::EditCancelled;
            }
        };

        let edit = self.edit.as_mut().unwrap();
        if let Some(high) = edit.high_nibble {
            // Second nibble — commit.
            let new_byte = (high << 4) | nibble;
            let pos = edit.byte_pos;
            let old_byte = self.bytes[pos];
            self.bytes[pos] = new_byte;

            let next_pos = pos + 1;
            if next_pos < self.bytes.len() {
                self.edit = Some(NibbleEdit {
                    byte_pos: next_pos,
                    high_nibble: None,
                });
                self.cursor = next_pos;
                self.sync_scroll();
            } else {
                self.edit = None;
            }
            HexViewEvent::ByteEdited {
                pos,
                old: old_byte,
                new: new_byte,
            }
        } else {
            // First nibble — store and wait.
            edit.high_nibble = Some(nibble);
            HexViewEvent::None
        }
    }
}

// ── HexViewEvent ──────────────────────────────────────────────────────────────

/// Events emitted by [`HexViewState::handle_key`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HexViewEvent {
    /// A byte was edited in place.
    ByteEdited { pos: usize, old: u8, new: u8 },
    /// The cursor moved to a new position.
    CursorMoved { new_pos: usize },
    /// The display mode was cycled.
    ModeChanged { new_mode: ViewMode },
    /// The user pressed `m` — the host app should show a label prompt dialog,
    /// then call [`HexViewState::add_marker`] with the result.
    MarkerRequested { at: usize },
    /// Nibble-edit mode started at a byte offset.
    EditStarted { at: usize },
    /// Edit was cancelled (Esc) or non-hex key pressed during edit.
    EditCancelled,
    /// All markers were cleared.
    MarkersCleared,
    /// Bytes were reset to the original baseline (Ctrl-R).
    BytesReset,
    /// No notable event.
    None,
}

// ── Log view state ─────────────────────────────────────────────────────────────

/// A single search match inside a log line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogMatch {
    /// Zero-based raw line index containing the match.
    pub line: usize,
    /// Visible column range of the match within the ANSI-stripped line.
    pub range: std::ops::Range<usize>,
}

/// Mutable state for a read-only log viewer with wrapped-row scrolling and search.
pub struct LogViewState {
    /// Raw log lines, which may include ANSI color sequences.
    pub lines: std::collections::VecDeque<String>,
    /// When true, render long lines across multiple terminal rows.
    pub word_wrap: bool,
    /// Scroll offset in wrapped rows from the top.
    pub scroll: usize,
    /// Number of visible rows from the last render call.
    pub visible_rows: usize,
    /// `row_offsets[i]` is the first wrapped row of raw line `i`.
    pub(crate) row_offsets: Vec<usize>,
    /// Maximum valid wrapped-row scroll offset.
    pub(crate) max_scroll: usize,
    /// Active search query, if any.
    pub search_query: Option<String>,
    /// When false, search is ASCII case-insensitive.
    pub case_sensitive: bool,
    /// All visible-text matches for the current query.
    pub(crate) search_matches: Vec<LogMatch>,
    /// Index into `search_matches` for the active match.
    pub(crate) current_match: Option<usize>,
    /// Optional host-driven highlighted line, e.g. for copy mode.
    pub highlighted_line: Option<usize>,
    /// Optional cap on retained log lines.
    pub line_limit: Option<usize>,
    /// When true, the next render jumps to the bottom after recomputing geometry.
    pub auto_scroll: bool,
    cached_wrap_width: usize,
    cached_visible_rows: usize,
    geometry_dirty: bool,
}

impl LogViewState {
    /// Create a log view state from pre-split raw lines.
    pub fn new(lines: Vec<String>) -> Self {
        let mut state = Self {
            lines: lines.into_iter().collect(),
            word_wrap: true,
            scroll: 0,
            visible_rows: 20,
            row_offsets: Vec::new(),
            max_scroll: 0,
            search_query: None,
            case_sensitive: false,
            search_matches: Vec::new(),
            current_match: None,
            highlighted_line: None,
            line_limit: None,
            auto_scroll: false,
            cached_wrap_width: 0,
            cached_visible_rows: 0,
            geometry_dirty: true,
        };
        state.rebuild_search();
        state
    }

    /// Create a log view state from a text blob, preserving ANSI sequences.
    pub fn from_text(text: &str) -> Self {
        Self::new(text.lines().map(ToOwned::to_owned).collect())
    }

    /// Replace the displayed lines, reset scroll, and re-run the active search.
    pub fn set_lines(&mut self, lines: Vec<String>) {
        self.lines = lines.into_iter().collect();
        self.scroll = 0;
        self.highlighted_line = None;
        self.auto_scroll = false;
        self.invalidate_geometry();
        self.rebuild_search();
        self.refresh_geometry_if_known();
        self.clamp_scroll();
    }

    /// Append raw log lines and scroll to the bottom on the next render.
    pub fn push_lines<I>(&mut self, lines: I)
    where
        I: IntoIterator<Item = String>,
    {
        self.lines.extend(lines);
        self.enforce_line_limit();
        self.invalidate_geometry();
        self.rebuild_search();
        self.auto_scroll = true;
    }

    /// Set or clear the retained line limit.
    pub fn set_line_limit(&mut self, limit: Option<usize>) {
        self.line_limit = limit;
        self.enforce_line_limit();
        self.invalidate_geometry();
        self.rebuild_search();
        self.refresh_geometry_if_known();
        self.clamp_scroll();
    }

    /// Enable or disable word wrap for rendered log lines.
    pub fn set_word_wrap(&mut self, word_wrap: bool) {
        if self.word_wrap == word_wrap {
            return;
        }

        self.refresh_geometry_if_known();
        let keep_at_end = self.scroll >= self.max_scroll;
        self.word_wrap = word_wrap;
        self.invalidate_geometry();
        if keep_at_end {
            self.auto_scroll = true;
        } else {
            self.refresh_geometry_if_known();
            self.clamp_scroll();
        }
    }

    /// Toggle word wrap and return the new state.
    pub fn toggle_word_wrap(&mut self) -> bool {
        let word_wrap = !self.word_wrap;
        self.set_word_wrap(word_wrap);
        self.word_wrap
    }

    /// Total number of raw log lines.
    pub fn total_lines(&self) -> usize {
        self.lines.len()
    }

    /// Clamp scroll to the last valid wrapped-row offset.
    pub fn clamp_scroll(&mut self) {
        self.scroll = self.scroll.min(self.max_scroll);
    }

    /// Scroll up by one wrapped row.
    pub fn scroll_up(&mut self) {
        self.scroll = self.scroll.saturating_sub(1);
    }

    /// Scroll down by one wrapped row.
    pub fn scroll_down(&mut self) {
        self.scroll = self.scroll.saturating_add(1).min(self.max_scroll);
    }

    /// Scroll up by one page.
    pub fn page_up(&mut self) {
        self.scroll = self.scroll.saturating_sub(self.visible_rows.max(1));
    }

    /// Scroll down by one page.
    pub fn page_down(&mut self) {
        self.scroll = self
            .scroll
            .saturating_add(self.visible_rows.max(1))
            .min(self.max_scroll);
    }

    /// Jump to the first wrapped row.
    pub fn scroll_to_start(&mut self) {
        self.scroll = 0;
    }

    /// Jump to the last wrapped-row page.
    pub fn scroll_to_end(&mut self) {
        self.scroll = self.max_scroll;
    }

    /// Return the current maximum valid wrapped-row scroll offset.
    pub fn max_scroll(&mut self) -> usize {
        self.refresh_geometry_if_known();
        self.max_scroll
    }

    /// Return the raw line index at or above a wrapped-row scroll offset.
    pub fn line_index_at_scroll(&mut self, scroll: usize) -> usize {
        self.refresh_geometry_if_known();
        if self.lines.is_empty() || self.row_offsets.is_empty() {
            return 0;
        }
        self.row_offsets
            .partition_point(|&row| row <= scroll)
            .saturating_sub(1)
            .min(self.lines.len().saturating_sub(1))
    }

    /// Return the wrapped-row scroll offset for a raw line.
    pub fn scroll_offset_for_line(&mut self, line: usize) -> Option<usize> {
        self.refresh_geometry_if_known();
        self.row_offsets.get(line).copied()
    }

    /// Set or clear a host-driven highlighted line.
    pub fn set_highlighted_line(&mut self, line: Option<usize>) {
        self.highlighted_line = line.filter(|&idx| idx < self.lines.len());
    }

    /// Return the raw highlighted line, if any.
    pub fn highlighted_line_text(&self) -> Option<&str> {
        self.highlighted_line
            .and_then(|idx| self.lines.get(idx))
            .map(String::as_str)
    }

    /// Enable or disable case-sensitive search.
    pub fn set_case_sensitive(&mut self, case_sensitive: bool) {
        if self.case_sensitive != case_sensitive {
            self.case_sensitive = case_sensitive;
            self.rebuild_search();
        }
    }

    /// Set the active search query. Empty queries clear search state; non-empty
    /// queries select the first visible-text match.
    pub fn set_search_query(&mut self, query: impl Into<String>) {
        let query = query.into();
        self.search_query = if query.is_empty() { None } else { Some(query) };
        self.rebuild_search();
    }

    /// Clear the active search query and matches.
    pub fn clear_search(&mut self) {
        self.search_query = None;
        self.search_matches.clear();
        self.current_match = None;
    }

    /// Return whether search is active.
    pub fn has_search(&self) -> bool {
        self.search_query.is_some()
    }

    /// Return the currently selected match, if any.
    pub fn current_match(&self) -> Option<&LogMatch> {
        self.current_match
            .and_then(|idx| self.search_matches.get(idx))
    }

    /// Return the raw line number of the current match, if any.
    pub fn current_match_line(&self) -> Option<usize> {
        self.current_match().map(|m| m.line)
    }

    /// Move to the next search match, wrapping around.
    pub fn next_match(&mut self) -> Option<&LogMatch> {
        if self.search_matches.is_empty() {
            return None;
        }
        let next = match self.current_match {
            Some(idx) => (idx + 1) % self.search_matches.len(),
            None => 0,
        };
        self.current_match = Some(next);
        self.sync_scroll_to_current_match();
        self.current_match()
    }

    /// Move to the previous search match, wrapping around.
    pub fn prev_match(&mut self) -> Option<&LogMatch> {
        if self.search_matches.is_empty() {
            return None;
        }
        let prev = match self.current_match {
            Some(0) | None => self.search_matches.len() - 1,
            Some(idx) => idx - 1,
        };
        self.current_match = Some(prev);
        self.sync_scroll_to_current_match();
        self.current_match()
    }

    /// Scroll so the active match is visible.
    pub fn sync_scroll_to_current_match(&mut self) {
        self.refresh_geometry_if_known();
        let Some(line) = self.current_match_line() else {
            return;
        };
        if self.row_offsets.is_empty() {
            return;
        }
        self.scroll = self.row_offsets.get(line).copied().unwrap_or(line);
        self.clamp_scroll();
    }

    /// Recompute wrapped-row geometry for the current width.
    pub(crate) fn recompute_wrap_geometry(&mut self, wrap_at: usize) {
        self.row_offsets.clear();
        let wrap_at = wrap_at.max(1);
        let mut total_rows = 0usize;

        for raw in &self.lines {
            self.row_offsets.push(total_rows);
            let visible = ansi_visible_len(raw);
            total_rows += if !self.word_wrap || visible == 0 {
                1
            } else {
                visible.div_ceil(wrap_at)
            };
        }

        self.max_scroll = total_rows.saturating_sub(self.visible_rows.max(1));
        self.cached_wrap_width = wrap_at;
        self.cached_visible_rows = self.visible_rows.max(1);
        self.geometry_dirty = false;
    }

    pub(crate) fn ensure_wrap_geometry(&mut self, wrap_at: usize) {
        let wrap_at = wrap_at.max(1);
        let visible_rows = self.visible_rows.max(1);
        if self.geometry_dirty
            || self.cached_wrap_width != wrap_at
            || self.cached_visible_rows != visible_rows
        {
            self.recompute_wrap_geometry(wrap_at);
        }
    }

    fn enforce_line_limit(&mut self) {
        let Some(limit) = self.line_limit else {
            return;
        };
        while self.lines.len() > limit {
            self.lines.pop_front();
        }
    }

    fn rebuild_search(&mut self) {
        self.search_matches.clear();

        let Some(query) = self.search_query.as_deref() else {
            self.current_match = None;
            return;
        };

        let needle = if self.case_sensitive {
            query.to_string()
        } else {
            query.to_ascii_lowercase()
        };

        for (line_idx, line) in self.lines.iter().enumerate() {
            let stripped = strip_ansi_codes(line);
            let haystack = if self.case_sensitive {
                stripped.clone()
            } else {
                stripped.to_ascii_lowercase()
            };

            for (start, matched) in haystack.match_indices(&needle) {
                let column_start = display_width(&stripped[..start]);
                let column_end = display_width(&stripped[..start + matched.len()]);
                self.search_matches.push(LogMatch {
                    line: line_idx,
                    range: column_start..column_end,
                });
            }
        }

        self.current_match = (!self.search_matches.is_empty()).then_some(0);
    }

    fn invalidate_geometry(&mut self) {
        self.geometry_dirty = true;
    }

    fn refresh_geometry_if_known(&mut self) {
        if self.cached_wrap_width != 0 {
            self.ensure_wrap_geometry(self.cached_wrap_width);
        }
    }
}

// ── Pure helpers ─────────────────────────────────────────────────────────────

/// Strip common ANSI escape sequences from a log line.
pub fn strip_ansi_codes(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] != 0x1b {
            if let Some(ch) = text[i..].chars().next() {
                out.push(ch);
                i += ch.len_utf8();
            } else {
                break;
            }
            continue;
        }

        i += 1;
        match bytes.get(i).copied() {
            Some(b'[') => {
                i += 1;
                while i < bytes.len() {
                    let b = bytes[i];
                    i += 1;
                    if (0x40..=0x7e).contains(&b) {
                        break;
                    }
                }
            }
            Some(b']') => {
                i += 1;
                while i < bytes.len() {
                    match bytes[i] {
                        0x07 => {
                            i += 1;
                            break;
                        }
                        0x1b if bytes.get(i + 1) == Some(&b'\\') => {
                            i += 2;
                            break;
                        }
                        _ => i += 1,
                    }
                }
            }
            Some(_) | None => {}
        }
    }

    out
}

fn display_width_char(ch: char) -> usize {
    UnicodeWidthChar::width(ch).unwrap_or(if ch == '\t' { 1 } else { 0 })
}

fn display_width(text: &str) -> usize {
    text.chars().map(display_width_char).sum()
}

/// Count the visible display columns in a line after stripping ANSI escape sequences.
pub fn ansi_visible_len(text: &str) -> usize {
    display_width(&strip_ansi_codes(text))
}

/// Format bytes as a hex+ASCII dump (xxd-style).
///
/// `inner_width` is the usable width inside a widget border.
/// Automatically selects 16 bytes/row (needs ≥76 cols) or 8 bytes/row otherwise.
/// When `ascii` is `false` the `|ASCII|` column is omitted.
pub fn hex_dump(bytes: &[u8], inner_width: usize, ascii: bool) -> String {
    let bpr: usize = if inner_width >= 76 { 16 } else { 8 };
    let hex_w = bpr * 3 - 1;
    bytes
        .chunks(bpr)
        .enumerate()
        .map(|(i, chunk)| {
            let off = i * bpr;
            let hex: String = chunk
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<Vec<_>>()
                .join(" ");
            if ascii {
                let asc: String = chunk
                    .iter()
                    .map(|&b| {
                        if (0x20..0x7f).contains(&b) {
                            b as char
                        } else {
                            '.'
                        }
                    })
                    .collect();
                format!("{off:08x}  {hex:<hex_w$}  |{asc}|")
            } else {
                format!("{off:08x}  {hex}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Render bytes as plain text.
///
/// `\r\n` and bare `\n` become newlines; other non-printable bytes become `·`.
pub fn ascii_view(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
            out.push('\n');
            i += 2;
        } else if b == b'\n' {
            out.push('\n');
            i += 1;
        } else if (0x20..0x7f).contains(&b) || b == b'\t' {
            out.push(b as char);
            i += 1;
        } else {
            out.push('·');
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{buffer::Buffer, layout::Rect, widgets::Widget};

    #[test]
    fn hex_dump_basic() {
        let s = hex_dump(b"ABCDEFGH", 80, true);
        assert!(s.contains("41 42 43 44 45 46 47 48"));
        assert!(s.contains("|ABCDEFGH|"));
    }

    #[test]
    fn ascii_view_crlf() {
        let bytes = b"hello\r\nworld";
        let s = ascii_view(bytes);
        assert_eq!(s, "hello\nworld");
    }

    #[test]
    fn view_mode_cycle() {
        let m = ViewMode::HexAscii;
        assert_eq!(m.next(), ViewMode::HexOnly);
        assert_eq!(m.next().next(), ViewMode::AsciiOnly);
        assert_eq!(m.next().next().next(), ViewMode::HexAscii);
    }

    #[test]
    fn nibble_edit_roundtrip() {
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
        let mut state = HexViewState::new(vec![0x00]);
        let mk = |c: char| KeyEvent {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers::empty(),
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        };
        // Enter edit mode
        let _ = state.handle_key(KeyEvent {
            code: KeyCode::Enter,
            modifiers: KeyModifiers::empty(),
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        });
        assert!(state.edit.is_some());

        // Type '4' then '2' → byte becomes 0x42
        state.handle_key(mk('4'));
        let ev = state.handle_key(mk('2'));
        assert_eq!(
            ev,
            HexViewEvent::ByteEdited {
                pos: 0,
                old: 0x00,
                new: 0x42
            }
        );
        assert_eq!(state.bytes[0], 0x42);
    }

    #[test]
    fn add_remove_marker() {
        let mut state = HexViewState::new(vec![0u8; 32]);
        state.add_marker(8, "test", ratatui::style::Color::Cyan);
        assert_eq!(state.markers.len(), 1);
        state.remove_marker(8);
        assert!(state.markers.is_empty());
    }

    #[test]
    fn set_bytes_clears_stale_markers() {
        let mut state = HexViewState::new(vec![0u8; 32]);
        state.add_marker(8, "header", ratatui::style::Color::Cyan);

        state.set_bytes(vec![1u8; 8]);

        assert!(state.markers.is_empty());
        assert_eq!(state.cursor, 0);
        assert_eq!(state.scroll, 0);
    }

    #[test]
    fn sync_scroll_accounts_for_marker_lines() {
        let mut state = HexViewState::new(vec![0u8; 64]);
        state.visible_rows = 3;
        state.cols = 16;
        state.cursor = 32;
        state.add_marker(0, "m0", ratatui::style::Color::Cyan);
        state.add_marker(16, "m1", ratatui::style::Color::Yellow);

        state.sync_scroll();

        assert_eq!(state.scroll, 1);
    }

    #[test]
    fn render_keeps_cursor_row_visible_when_markers_stack_on_same_row() {
        let area = Rect::new(0, 0, 40, 2);
        let mut buf = Buffer::empty(area);
        let mut state = HexViewState::new(vec![0u8; 32]);
        state.visible_rows = 2;
        state.cols = 16;
        state.cursor = 16;
        state.markers.push(Marker {
            offset: 16,
            label: "m0".into(),
            color: ratatui::style::Color::Cyan,
        });
        state.markers.push(Marker {
            offset: 20,
            label: "m1".into(),
            color: ratatui::style::Color::Yellow,
        });
        state.sync_scroll();

        HexView::new(&mut state).render(area, &mut buf);

        assert_eq!(buf.cell((0, 1)).unwrap().symbol(), "0");
    }

    #[test]
    fn render_clamps_scroll_state() {
        let area = Rect::new(0, 0, 80, 4);
        let mut buf = Buffer::empty(area);
        let mut state = HexViewState::new(vec![0u8; 16]);
        state.scroll = 99;

        HexView::new(&mut state).render(area, &mut buf);

        assert_eq!(state.scroll, 0);
    }

    #[test]
    fn strip_ansi_codes_removes_color_sequences() {
        let line = "\u{1b}[31mERROR\u{1b}[0m timeout";
        assert_eq!(strip_ansi_codes(line), "ERROR timeout");
    }

    #[test]
    fn ansi_visible_len_handles_osc_and_utf8() {
        let line = "\u{1b}]0;title\u{7}hé";
        assert_eq!(ansi_visible_len(line), 2);
    }

    #[test]
    fn log_view_from_text_preserves_ansi_and_splits_lines() {
        let state = LogViewState::from_text("INFO ok\r\n\u{1b}[31mERROR\u{1b}[0m fail");
        assert_eq!(
            state.lines.iter().cloned().collect::<Vec<_>>(),
            vec![
                "INFO ok".to_string(),
                "\u{1b}[31mERROR\u{1b}[0m fail".to_string()
            ]
        );
    }

    #[test]
    fn log_push_lines_respects_limit() {
        let mut state = LogViewState::new(vec!["one".into()]);
        state.set_line_limit(Some(2));
        state.push_lines(vec!["two".into(), "three".into()]);

        assert_eq!(
            state.lines.iter().cloned().collect::<Vec<_>>(),
            vec!["two".to_string(), "three".to_string()]
        );
        assert!(state.auto_scroll);
    }

    #[test]
    fn log_search_tracks_matching_lines_on_ansi_text() {
        let mut state = LogViewState::new(vec![
            "\u{1b}[31mERROR\u{1b}[0m first".into(),
            "ok".into(),
            "error second".into(),
        ]);
        state.set_search_query("error");

        assert_eq!(state.search_matches.len(), 2);
        assert_eq!(
            state.search_matches,
            vec![
                LogMatch {
                    line: 0,
                    range: 0..5,
                },
                LogMatch {
                    line: 2,
                    range: 0..5,
                },
            ]
        );
        assert_eq!(state.current_match_line(), Some(0));
    }

    #[test]
    fn log_search_tracks_multiple_matches_per_line() {
        let mut state = LogViewState::new(vec!["prefix ERROR suffix ERROR".into()]);
        state.set_search_query("ERROR");

        assert_eq!(
            state.search_matches,
            vec![
                LogMatch {
                    line: 0,
                    range: 7..12,
                },
                LogMatch {
                    line: 0,
                    range: 20..25,
                },
            ]
        );
    }

    #[test]
    fn log_search_uses_display_columns_for_wide_text() {
        let mut state = LogViewState::new(vec!["界needle".into()]);
        state.set_search_query("needle");

        assert_eq!(
            state.search_matches,
            vec![LogMatch {
                line: 0,
                range: 2..8,
            }]
        );
    }

    #[test]
    fn log_search_navigation_wraps_without_matches_panicking() {
        let mut state = LogViewState::new(vec!["INFO".into()]);
        assert!(state.next_match().is_none());
        assert!(state.prev_match().is_none());

        state.set_lines(vec![
            "INFO".into(),
            "ERROR one".into(),
            "WARN".into(),
            "ERROR two".into(),
        ]);
        state.visible_rows = 2;
        state.recompute_wrap_geometry(20);
        state.set_search_query("ERROR");

        assert_eq!(state.current_match_line(), Some(1));
        assert_eq!(state.next_match().map(|m| m.line), Some(3));
        assert_eq!(state.current_match_line(), Some(3));
        assert_eq!(state.scroll, 2);
        assert_eq!(state.next_match().map(|m| m.line), Some(1));
        assert_eq!(state.prev_match().map(|m| m.line), Some(3));
    }

    #[test]
    fn log_set_lines_rebuilds_search_state() {
        let mut state = LogViewState::new(vec!["ERROR one".into(), "ERROR two".into()]);
        state.visible_rows = 1;
        state.set_search_query("ERROR");
        assert_eq!(state.search_matches.len(), 2);

        state.set_lines(vec!["INFO only".into()]);

        assert!(state.search_matches.is_empty());
        assert_eq!(state.current_match, None);
        assert_eq!(state.scroll, 0);
        assert_eq!(state.highlighted_line, None);
        assert_eq!(state.search_query.as_deref(), Some("ERROR"));
    }

    #[test]
    fn log_render_clamps_scroll_state() {
        let area = Rect::new(0, 0, 40, 2);
        let mut buf = Buffer::empty(area);
        let mut state = LogViewState::new(vec!["one".into(), "two".into()]);
        state.scroll = 99;

        LogView::new(&mut state).render(area, &mut buf);

        assert_eq!(state.scroll, 0);
    }

    #[test]
    fn log_geometry_helpers_map_between_scroll_and_lines() {
        let area = Rect::new(0, 0, 5, 2);
        let mut buf = Buffer::empty(area);
        let mut state = LogViewState::new(vec!["123456789".into(), "tail".into()]);

        LogView::new(&mut state).render(area, &mut buf);

        assert_eq!(state.scroll_offset_for_line(0), Some(0));
        assert_eq!(state.scroll_offset_for_line(1), Some(2));
        assert_eq!(state.line_index_at_scroll(0), 0);
        assert_eq!(state.line_index_at_scroll(1), 0);
        assert_eq!(state.line_index_at_scroll(2), 1);
        assert_eq!(state.max_scroll(), 1);
    }

    #[test]
    fn log_wrap_toggle_updates_scroll_geometry() {
        let area = Rect::new(0, 0, 5, 1);
        let mut state = LogViewState::new(vec!["123456789".into(), "tail".into()]);

        LogView::new(&mut state).render(area, &mut Buffer::empty(area));
        assert!(state.word_wrap);
        assert_eq!(state.max_scroll, 2);

        state.scroll_to_end();
        state.set_word_wrap(false);

        LogView::new(&mut state).render(area, &mut Buffer::empty(area));
        assert!(!state.word_wrap);
        assert_eq!(state.max_scroll, 1);
        assert_eq!(state.scroll, 1);
    }

    #[test]
    fn log_search_scroll_refreshes_geometry_after_wrap_toggle() {
        let area = Rect::new(0, 0, 5, 1);
        let mut state = LogViewState::new(vec!["123456789".into(), "needle".into()]);

        LogView::new(&mut state).render(area, &mut Buffer::empty(area));
        state.set_word_wrap(false);
        state.set_search_query("needle");
        state.sync_scroll_to_current_match();

        assert_eq!(state.scroll, 1);
    }

    #[test]
    fn log_render_highlights_current_match() {
        let area = Rect::new(0, 0, 40, 1);
        let mut buf = Buffer::empty(area);
        let mut state = LogViewState::new(vec!["INFO \u{1b}[31mERROR\u{1b}[0m timeout".into()]);
        state.set_search_query("ERROR");

        LogView::new(&mut state).render(area, &mut buf);

        let prefix_cell = buf.cell((0, 0)).unwrap();
        assert_ne!(prefix_cell.bg, ratatui::style::Color::Yellow);

        let cell = buf.cell((5, 0)).unwrap();
        assert_eq!(cell.symbol(), "E");
        assert_eq!(cell.bg, ratatui::style::Color::Yellow);
    }

    #[test]
    fn log_render_highlights_actual_search_match_not_prefix() {
        let area = Rect::new(0, 0, 40, 1);
        let mut buf = Buffer::empty(area);
        let mut state = LogViewState::new(vec!["prefix needle suffix".into()]);
        state.set_search_query("needle");

        LogView::new(&mut state).render(area, &mut buf);

        let prefix_cell = buf.cell((0, 0)).unwrap();
        assert_ne!(prefix_cell.bg, ratatui::style::Color::Yellow);

        let match_cell = buf.cell((7, 0)).unwrap();
        assert_eq!(match_cell.symbol(), "n");
        assert_eq!(match_cell.bg, ratatui::style::Color::Yellow);
    }

    #[test]
    fn log_render_host_highlight_takes_precedence() {
        let area = Rect::new(0, 0, 40, 1);
        let mut buf = Buffer::empty(area);
        let mut state = LogViewState::new(vec!["ERROR timeout".into()]);
        state.set_search_query("ERROR");
        state.set_highlighted_line(Some(0));

        LogView::new(&mut state).render(area, &mut buf);

        let cell = buf.cell((0, 0)).unwrap();
        assert_eq!(cell.bg, ratatui::style::Color::Rgb(30, 70, 100));
    }
}
