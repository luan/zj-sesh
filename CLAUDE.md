# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

This is **zj-sesh**, a fork of the Zellij session manager plugin. It's a WebAssembly plugin written in Rust that provides an interactive terminal UI for managing Zellij terminal multiplexer sessions.

## Build Commands

```bash
# Build and install the WASM plugin to ~/.config/zellij/plugins/
./build.sh

# Or build manually
cargo build --release

# Check code without building
cargo check

# Format code
cargo fmt

# Lint code (requires clippy)
cargo clippy

# The .cargo/config.toml automatically targets wasm32-wasip1
```

**Note**: The project depends on `zellij-tile` which expects to be found at `../../zellij-tile` relative to this directory. You may need to adjust this path or clone the full Zellij repository for builds to work.

## Architecture Overview

### Core State Management
The plugin follows an event-driven architecture centered around a main `State` struct that implements `ZellijPlugin`. The state manages five distinct screens via a `Screen` enum:

- **Welcome**: Initial screen with ASCII banner for creating new sessions
- **SessionList**: Main screen showing active sessions with search/filtering
- **NewSession**: Form for creating sessions with optional layout selection
- **ResurrectSession**: Interface for restoring dead sessions
- **SessionNotFound**: Error state when a requested session doesn't exist

### Key Modules

**`main.rs`** - Plugin entry point with:
- `State` struct managing all plugin state
- Event handling for keyboard/mouse input
- Screen transition logic
- Integration with Zellij APIs (`switch_session`, `kill_session`, etc.)

**`session_list.rs`** - Session management with:
- `SessionList` maintaining active and forbidden sessions 
- `SelectedIndex` enum handling session vs. forbidden session selection
- Fuzzy search functionality using SkimMatcherV2
- Search result management and navigation

**`new_session_info.rs`** - New session creation with:
- `NewSessionInfo` managing session name and layout selection
- `LayoutList` with fuzzy search for available layouts
- State machine via `EnteringState` enum (name entry vs. layout search)

**`resurrectable_sessions.rs`** - Dead session management:
- `ResurrectableSessions` handling session resurrection
- Duration tracking for how long sessions have been dead
- Search and selection within dead sessions

**`ui/`** - Rendering system with:
- `mod.rs`: Screen rendering functions and `render_assets!` macro for list display
- `components.rs`: `LineToRender` system with color management and text truncation
- `welcome_screen.rs`: ASCII banner and welcome screen layout

### Data Flow Patterns

1. **Event Processing**: Events flow through `State::update()` which dispatches to screen-specific handlers
2. **Rendering**: Each screen has a dedicated render function that uses the `LineToRender` system
3. **Selection Management**: Most screens maintain both absolute selection (in full data) and search-filtered selection
4. **Search Integration**: Fuzzy matching is deeply integrated - searches update in real-time and maintain selection state

### Plugin Integration Points

- **Permissions**: Requests `ReadApplicationState` and `ChangeApplicationState`
- **Events**: Subscribes to `SessionUpdate`, `ModeUpdate`, `Key`, and `Mouse` events
- **Zellij APIs**: Calls `get_sessions()`, `switch_session()`, `kill_session()`, `new_session()`, etc.

### UI Rendering Architecture

The UI uses a custom rendering system built on `zellij_tile`'s coordinate-based text printing:
- `LineToRender` handles individual lines with colors, truncation, and state
- `render_assets!` macro manages paginated list display with selection highlighting
- Color theming via the `Colors` struct with terminal RGB values
- Unicode-aware text truncation and width calculations