# tui-hex-view

Lightweight [ratatui](https://github.com/ratatui/ratatui) widgets for viewing ANSI-colored logs and binary data.

## Features

- `HexView` for interactive hex/ascii browsing
- In-place byte editing with event callbacks
- Named byte markers with colored separators
- `LogView` with ANSI rendering, search highlighting, and optional wrapping
- Unicode-aware log search/highlighting and wrap geometry

## Installation

Add the crate to your `Cargo.toml`:

```toml
[dependencies]
tui-hex-view = "0.1.0"
```

## Quick start

### Hex view

```rust,no_run
use crossterm::event::{read, Event};
use tui_hex_view::{HexView, HexViewEvent, HexViewState};

let mut state = HexViewState::new(b"Hello, world!".to_vec());

// In your render function:
// frame.render_widget(HexView::new(&mut state), area);

// In your event loop:
// if let Event::Key(key) = read().unwrap() {
//     match state.handle_key(key) {
//         HexViewEvent::ByteEdited { pos, old, new } => { /* react */ }
//         HexViewEvent::MarkerRequested { at } => { /* show a label prompt */ }
//         _ => {}
//     }
// }
```

### Log view

```rust
use ratatui::{buffer::Buffer, layout::Rect, widgets::Widget};
use tui_hex_view::{LogView, LogViewState};

let mut state = LogViewState::from_text("INFO booting\nERROR timeout\nINFO retrying");
state.set_search_query("error");

let area = Rect::new(0, 0, 40, 3);
let mut buf = Buffer::empty(area);
LogView::new(&mut state).render(area, &mut buf);
```

## Demo

Run the interactive example:

```bash
cargo run --example demo
```

The demo includes:

- log search with `/`
- next/previous match with `n` / `N`
- wrap toggle with `w`
- hex cursor movement, marker creation, and byte editing

## API overview

- `HexViewState` stores editable bytes, cursor position, markers, and mode
- `HexViewEvent` reports edits, mode changes, cursor movement, and marker requests
- `LogViewState` stores raw log lines, scroll state, search matches, and wrap mode
- `HexView` / `LogView` are the ratatui widgets rendered from those states
