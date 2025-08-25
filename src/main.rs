mod new_session_info;
mod resurrectable_sessions;
mod session_list;
mod ui;
use std::collections::BTreeMap;
use uuid::Uuid;
use zellij_tile::prelude::*;

use new_session_info::NewSessionInfo;
use ui::{
    components::{
        render_controls_line, render_error, render_new_session_block, render_prompt,
        render_renaming_session_screen, render_screen_toggle, Colors,
    },
    welcome_screen::{render_banner, render_welcome_boundaries},
    SessionUiInfo,
};

use resurrectable_sessions::ResurrectableSessions;
use session_list::SessionList;

#[derive(Clone, Debug, Copy, Default)]
enum ActiveScreen {
    New,
    #[default]
    Attach,
    Resurrect,
}

#[derive(Default)]
struct State {
    session_name: Option<String>,
    sessions: SessionList,
    resurrectable_sessions: ResurrectableSessions,
    search_term: String,
    search_cursor: usize, // Cursor position in search term
    new_session_info: NewSessionInfo,
    renaming_session_name: Option<String>,
    error: Option<String>,
    active_screen: ActiveScreen,
    colors: Colors,
    is_welcome_screen: bool,
    show_kill_all_sessions_warning: bool,
    request_ids: Vec<String>,
    is_web_client: bool,
}

register_plugin!(State);

impl ZellijPlugin for State {
    fn load(&mut self, configuration: BTreeMap<String, String>) {
        self.is_welcome_screen = configuration
            .get("welcome_screen")
            .map(|v| v == "true")
            .unwrap_or(false);
        if self.is_welcome_screen {
            self.active_screen = ActiveScreen::New;
        }
        request_permission(&[
            PermissionType::ReadApplicationState,
            PermissionType::ChangeApplicationState,
        ]);
        subscribe(&[
            EventType::ModeUpdate,
            EventType::SessionUpdate,
            EventType::Key,
            EventType::RunCommandResult,
        ]);
    }

    fn pipe(&mut self, pipe_message: PipeMessage) -> bool {
        if pipe_message.name == "filepicker_result" {
            if let (Some(payload), Some(request_id)) = (pipe_message.payload, pipe_message.args.get("request_id")) {
                match self.request_ids.iter().position(|p| p == request_id) {
                    Some(request_id_position) => {
                        self.request_ids.remove(request_id_position);
                        let new_session_folder = std::path::PathBuf::from(payload);
                        self.new_session_info.new_session_folder = Some(new_session_folder);
                    },
                    None => {
                        eprintln!("request id not found");
                    },
                }
            }
            true
        } else {
            false
        }
    }
    fn update(&mut self, event: Event) -> bool {
        let mut should_render = false;
        match event {
            Event::ModeUpdate(mode_info) => {
                self.colors = Colors::new(mode_info.style.colors);
                self.is_web_client = mode_info.is_web_client.unwrap_or(false);
                should_render = true;
            },
            Event::Key(key) => {
                should_render = self.handle_key(key);
            },
            Event::PermissionRequestResult(_result) => {
                should_render = true;
            },
            Event::SessionUpdate(session_infos, resurrectable_session_list) => {
                for session_info in &session_infos {
                    if session_info.is_current_session {
                        self.new_session_info
                            .update_layout_list(session_info.available_layouts.clone());
                    }
                }
                self.resurrectable_sessions
                    .update(resurrectable_session_list);
                self.update_session_infos(session_infos);
                should_render = true;
            },
            _ => (),
        };
        should_render
    }

    fn render(&mut self, rows: usize, cols: usize) {
        let (x, y, width, height) = self.main_menu_size(rows, cols);

        let background = self.colors.palette.text_unselected.background;

        if self.is_welcome_screen {
            render_banner(x, 0, rows.saturating_sub(height), width);
        }
        render_screen_toggle(
            self.active_screen,
            x,
            y,
            width.saturating_sub(2),
            &background,
        );

        match self.active_screen {
            ActiveScreen::New => {
                render_new_session_block(
                    &self.new_session_info,
                    self.colors,
                    height.saturating_sub(2),
                    width,
                    x,
                    y + 2,
                );
            },
            ActiveScreen::Attach => {
                if let Some(new_session_name) = &self.renaming_session_name {
                    render_renaming_session_screen(&new_session_name, height, width, x, y + 2);
                } else if self.show_kill_all_sessions_warning {
                    self.render_kill_all_sessions_warning(height, width, x, y);
                } else {
                    render_prompt(&self.search_term, self.search_cursor, self.sessions.is_expanded(), self.colors, x, y + 2);
                    let room_for_list = height.saturating_sub(6); // search line and controls;
                    self.sessions.update_rows(room_for_list);
                    let list =
                        self.sessions
                            .render(room_for_list, width.saturating_sub(7), self.colors); // 7 for various ui
                    for (i, line) in list.iter().enumerate() {
                        print!("\u{1b}[{};{}H{}", y + i + 5, x, line.render());
                    }
                }
            },
            ActiveScreen::Resurrect => {
                self.resurrectable_sessions.render(height, width, x, y);
            },
        }
        if let Some(error) = &self.error {
            render_error(&error, height, width, x, y);
        } else {
            render_controls_line(self.active_screen, width, self.colors, x + 1, rows);
        }
        if self.is_welcome_screen {
            render_welcome_boundaries(rows, cols); // explicitly done in the end to override some
                                                   // stuff, see comment in function
        }
    }
}

impl State {
    fn reset_selected_index(&mut self) {
        self.sessions.reset_selected_index();
    }
    fn handle_key(&mut self, key: KeyWithModifier) -> bool {
        if self.error.is_some() {
            self.error = None;
            return true;
        }
        match self.active_screen {
            ActiveScreen::New => self.handle_new_session_key(key),
            ActiveScreen::Attach => self.handle_attach_to_session(key),
            ActiveScreen::Resurrect => self.handle_resurrect_session_key(key),
        }
    }
    fn handle_new_session_key(&mut self, key: KeyWithModifier) -> bool {
        let mut should_render = false;
        
        // Universal quit keys - escape and ctrl+c always quit
        match key.bare_key {
            BareKey::Esc if key.has_no_modifiers() && !self.is_welcome_screen => {
                hide_self();
                return false;
            },
            BareKey::Char('c') if key.has_modifiers(&[KeyModifier::Ctrl]) && !self.is_welcome_screen => {
                hide_self();
                return false;
            },
            _ => {}
        }
        
        match key.bare_key {
            BareKey::Down if key.has_no_modifiers() => {
                self.new_session_info.handle_key(key);
                should_render = true;
            },
            BareKey::Up if key.has_no_modifiers() => {
                self.new_session_info.handle_key(key);
                should_render = true;
            },
            BareKey::Char('n') if key.has_modifiers(&[KeyModifier::Ctrl]) => {
                // Simulate down arrow for ctrl+n
                let down_key = KeyWithModifier::new(BareKey::Down);
                self.new_session_info.handle_key(down_key);
                should_render = true;
            },
            BareKey::Char('p') if key.has_modifiers(&[KeyModifier::Ctrl]) => {
                // Simulate up arrow for ctrl+p
                let up_key = KeyWithModifier::new(BareKey::Up);
                self.new_session_info.handle_key(up_key);
                should_render = true;
            },
            BareKey::Char('j') if key.has_modifiers(&[KeyModifier::Ctrl]) => {
                // Simulate down arrow for ctrl+j (vim style)
                let down_key = KeyWithModifier::new(BareKey::Down);
                self.new_session_info.handle_key(down_key);
                should_render = true;
            },
            BareKey::Char('k') if key.has_modifiers(&[KeyModifier::Ctrl]) => {
                // Simulate up arrow for ctrl+k (vim style)
                let up_key = KeyWithModifier::new(BareKey::Up);
                self.new_session_info.handle_key(up_key);
                should_render = true;
            },
            BareKey::Enter if key.has_no_modifiers() => {
                self.handle_selection();
                should_render = true;
            },
            BareKey::Char(character) if key.has_no_modifiers() => {
                if character == '\n' {
                    self.handle_selection();
                } else {
                    self.new_session_info.handle_key(key);
                }
                should_render = true;
            },
            BareKey::Backspace if key.has_no_modifiers() => {
                self.new_session_info.handle_key(key);
                should_render = true;
            },
            BareKey::Tab if key.has_no_modifiers() => {
                self.toggle_active_screen();
                should_render = true;
            },
            BareKey::Tab if key.has_modifiers(&[KeyModifier::Shift]) => {
                self.toggle_active_screen_reverse();
                should_render = true;
            },
            BareKey::Char('/') if key.has_modifiers(&[KeyModifier::Ctrl]) => {
                let request_id = Uuid::new_v4();
                let mut config = BTreeMap::new();
                let mut args = BTreeMap::new();
                self.request_ids.push(request_id.to_string());
                // we insert this into the config so that a new plugin will be opened (the plugin's
                // uniqueness is determined by its name/url as well as its config)
                config.insert("request_id".to_owned(), request_id.to_string());
                // we also insert this into the args so that the plugin will have an easier access to
                // it
                args.insert("request_id".to_owned(), request_id.to_string());
                pipe_message_to_plugin(
                    MessageToPlugin::new("filepicker")
                        .with_plugin_url("filepicker")
                        .with_plugin_config(config)
                        .new_plugin_instance_should_have_pane_title(
                            "Select folder for the new session...",
                        )
                        .new_plugin_instance_should_be_focused()
                        .with_args(args),
                );
                should_render = true;
            },
            BareKey::Char('/') if key.has_modifiers(&[KeyModifier::Ctrl]) => {
                let request_id = Uuid::new_v4();
                let mut config = BTreeMap::new();
                let mut args = BTreeMap::new();
                self.request_ids.push(request_id.to_string());
                // we insert this into the config so that a new plugin will be opened (the plugin's
                // uniqueness is determined by its name/url as well as its config)
                config.insert("request_id".to_owned(), request_id.to_string());
                // we also insert this into the args so that the plugin will have an easier access to
                // it
                args.insert("request_id".to_owned(), request_id.to_string());
                pipe_message_to_plugin(
                    MessageToPlugin::new("filepicker")
                        .with_plugin_url("filepicker")
                        .with_plugin_config(config)
                        .new_plugin_instance_should_have_pane_title(
                            "Select folder for the new session...",
                        )
                        .new_plugin_instance_should_be_focused()
                        .with_args(args),
                );
                should_render = true;
            },
            BareKey::Char('c') if key.has_modifiers(&[KeyModifier::Ctrl]) => {
                self.new_session_info.new_session_folder = None;
                should_render = true;
            },
            BareKey::Esc if key.has_no_modifiers() => {
                self.new_session_info.handle_key(key);
                should_render = true;
            },
            _ => {},
        }
        should_render
    }
    fn handle_attach_to_session(&mut self, key: KeyWithModifier) -> bool {
        let mut should_render = false;
        
        // Universal quit keys - escape and ctrl+c always quit
        match key.bare_key {
            BareKey::Esc if key.has_no_modifiers() && !self.is_welcome_screen => {
                hide_self();
                return false;
            },
            BareKey::Char('c') if key.has_modifiers(&[KeyModifier::Ctrl]) && !self.is_welcome_screen => {
                hide_self();
                return false;
            },
            _ => {}
        }
        
        if self.show_kill_all_sessions_warning {
            match key.bare_key {
                BareKey::Char('y') if key.has_no_modifiers() => {
                    let all_other_sessions = self.sessions.all_other_sessions();
                    kill_sessions(&all_other_sessions);
                    self.reset_selected_index();
                    self.search_term.clear();
                    self.search_cursor = 0;
                    self.sessions
                        .update_search_term(&self.search_term, &self.colors);
                    self.show_kill_all_sessions_warning = false;
                    should_render = true;
                },
                BareKey::Char('n') | BareKey::Esc if key.has_no_modifiers() => {
                    self.show_kill_all_sessions_warning = false;
                    should_render = true;
                },
                BareKey::Char('c') if key.has_modifiers(&[KeyModifier::Ctrl]) => {
                    self.show_kill_all_sessions_warning = false;
                    should_render = true;
                },
                _ => {},
            }
        } else {
            match key.bare_key {
                BareKey::Right if key.has_no_modifiers() => {
                    self.sessions.result_expand();
                    should_render = true;
                },
                BareKey::Left if key.has_no_modifiers() => {
                    self.sessions.result_shrink();
                    should_render = true;
                },
                BareKey::Char('.') if key.has_modifiers(&[KeyModifier::Ctrl]) => {
                    self.sessions.result_expand();
                    should_render = true;
                },
                BareKey::Char(',') if key.has_modifiers(&[KeyModifier::Ctrl]) => {
                    self.sessions.result_shrink();
                    should_render = true;
                },
                BareKey::Down if key.has_no_modifiers() => {
                    self.sessions.move_selection_down();
                    should_render = true;
                },
                BareKey::Up if key.has_no_modifiers() => {
                    self.sessions.move_selection_up();
                    should_render = true;
                },
                BareKey::Char('n') if key.has_modifiers(&[KeyModifier::Ctrl]) => {
                    self.sessions.move_selection_down();
                    should_render = true;
                },
                BareKey::Char('p') if key.has_modifiers(&[KeyModifier::Ctrl]) => {
                    self.sessions.move_selection_up();
                    should_render = true;
                },
                BareKey::Char('j') if key.has_modifiers(&[KeyModifier::Ctrl]) => {
                    self.sessions.move_selection_down();
                    should_render = true;
                },
                BareKey::Char('k') if key.has_modifiers(&[KeyModifier::Ctrl]) => {
                    self.sessions.move_selection_up();
                    should_render = true;
                },
                BareKey::Char('h') if key.has_modifiers(&[KeyModifier::Ctrl]) => {
                    self.sessions.result_shrink();
                    should_render = true;
                },
                BareKey::Char('l') if key.has_modifiers(&[KeyModifier::Ctrl]) => {
                    self.sessions.result_expand();
                    should_render = true;
                },
                BareKey::Char('t') if key.has_modifiers(&[KeyModifier::Ctrl]) => {
                    // Toggle session expansion 
                    self.sessions.toggle_expansion();
                    // Need to update search results since they depend on expansion state
                    self.sessions.update_search_term(&self.search_term, &self.colors);
                    should_render = true;
                },
                BareKey::Enter if key.has_no_modifiers() => {
                    self.handle_selection();
                    should_render = true;
                },
                BareKey::Char(character) if key.has_no_modifiers() => {
                    if character == '\n' {
                        self.handle_selection();
                    } else if let Some(new_session_name) = self.renaming_session_name.as_mut() {
                        new_session_name.push(character);
                    } else {
                        // Insert character at cursor position
                        self.search_term.insert(self.search_cursor, character);
                        self.search_cursor += 1;
                        self.sessions
                            .update_search_term(&self.search_term, &self.colors);
                    }
                    should_render = true;
                },
                BareKey::Backspace if key.has_no_modifiers() => {
                    if let Some(new_session_name) = self.renaming_session_name.as_mut() {
                        if new_session_name.is_empty() {
                            self.renaming_session_name = None;
                        } else {
                            new_session_name.pop();
                        }
                    } else if self.search_cursor > 0 {
                        // Delete character before cursor
                        self.search_cursor -= 1;
                        self.search_term.remove(self.search_cursor);
                        self.sessions
                            .update_search_term(&self.search_term, &self.colors);
                    }
                    should_render = true;
                },
                BareKey::Char('r') if key.has_modifiers(&[KeyModifier::Ctrl]) => {
                    self.renaming_session_name = Some(String::new());
                    should_render = true;
                },
                BareKey::Delete if key.has_no_modifiers() => {
                    if let Some(selected_session_name) = self.sessions.get_selected_session_name() {
                        kill_sessions(&[selected_session_name]);
                        self.reset_selected_index();
                        self.search_term.clear();
                        self.search_cursor = 0;
                        self.sessions
                            .update_search_term(&self.search_term, &self.colors);
                    } else {
                        self.show_error("Must select session before killing it.");
                    }
                    should_render = true;
                },
                BareKey::Char('d') if key.has_modifiers(&[KeyModifier::Ctrl]) => {
                    let all_other_sessions = self.sessions.all_other_sessions();
                    if all_other_sessions.is_empty() {
                        self.show_error("No other sessions to kill. Quit to kill the current one.");
                    } else {
                        self.show_kill_all_sessions_warning = true;
                    }
                    should_render = true;
                },
                BareKey::Char('x') if key.has_modifiers(&[KeyModifier::Ctrl]) => {
                    disconnect_other_clients()
                },
                // Readline bindings for search field
                BareKey::Char('f') if key.has_modifiers(&[KeyModifier::Ctrl]) => {
                    // Move cursor forward (right)
                    if self.renaming_session_name.is_none() && self.search_cursor < self.search_term.len() {
                        self.search_cursor += 1;
                        should_render = true;
                    }
                },
                BareKey::Char('b') if key.has_modifiers(&[KeyModifier::Ctrl]) => {
                    // Move cursor backward (left)
                    if self.renaming_session_name.is_none() && self.search_cursor > 0 {
                        self.search_cursor -= 1;
                        should_render = true;
                    }
                },
                BareKey::Char('a') if key.has_modifiers(&[KeyModifier::Ctrl]) => {
                    // Move to beginning of line
                    if self.renaming_session_name.is_none() {
                        self.search_cursor = 0;
                        should_render = true;
                    }
                },
                BareKey::Char('e') if key.has_modifiers(&[KeyModifier::Ctrl]) => {
                    // Check if we're in session expansion toggle mode or readline end-of-line
                    if self.renaming_session_name.is_none() {
                        // If search field is focused, move to end of line (readline behavior)
                        self.search_cursor = self.search_term.len();
                        should_render = true;
                    }
                },
                BareKey::Char('k') if key.has_modifiers(&[KeyModifier::Ctrl]) => {
                    // Check if we're using vim navigation or readline kill-to-end
                    if self.renaming_session_name.is_none() && !self.search_term.is_empty() {
                        // Kill from cursor to end of line (readline behavior)
                        self.search_term.truncate(self.search_cursor);
                        self.sessions
                            .update_search_term(&self.search_term, &self.colors);
                        should_render = true;
                    } else {
                        // Vim-style up navigation
                        self.sessions.move_selection_up();
                        should_render = true;
                    }
                },
                BareKey::Char('u') if key.has_modifiers(&[KeyModifier::Ctrl]) => {
                    // Kill entire line (readline)
                    if self.renaming_session_name.is_none() && !self.search_term.is_empty() {
                        self.search_term.clear();
                        self.search_cursor = 0;
                        self.sessions
                            .update_search_term(&self.search_term, &self.colors);
                        self.reset_selected_index();
                        should_render = true;
                    }
                },
                BareKey::Char('w') if key.has_modifiers(&[KeyModifier::Ctrl]) => {
                    // Delete word backward (readline)
                    if self.renaming_session_name.is_none() && self.search_cursor > 0 {
                        let mut new_cursor = self.search_cursor;
                        let chars: Vec<char> = self.search_term.chars().collect();
                        
                        // Skip whitespace backwards
                        while new_cursor > 0 && chars[new_cursor - 1].is_whitespace() {
                            new_cursor -= 1;
                        }
                        
                        // Delete word backwards
                        while new_cursor > 0 && !chars[new_cursor - 1].is_whitespace() {
                            new_cursor -= 1;
                        }
                        
                        // Remove the characters
                        self.search_term.drain(new_cursor..self.search_cursor);
                        self.search_cursor = new_cursor;
                        self.sessions
                            .update_search_term(&self.search_term, &self.colors);
                        should_render = true;
                    }
                },
                BareKey::Char('c') if key.has_modifiers(&[KeyModifier::Ctrl]) => {
                    if !self.search_term.is_empty() {
                        self.search_term.clear();
                        self.search_cursor = 0;
                        self.sessions
                            .update_search_term(&self.search_term, &self.colors);
                        self.reset_selected_index();
                    } else if !self.is_welcome_screen {
                        self.reset_selected_index();
                        hide_self();
                    }
                    should_render = true;
                },
                BareKey::Char('d') if key.has_modifiers(&[KeyModifier::Alt]) => {
                    // Delete word forward (readline)
                    if self.renaming_session_name.is_none() && self.search_cursor < self.search_term.len() {
                        let mut new_cursor = self.search_cursor;
                        let chars: Vec<char> = self.search_term.chars().collect();
                        
                        // Skip whitespace forward
                        while new_cursor < chars.len() && chars[new_cursor].is_whitespace() {
                            new_cursor += 1;
                        }
                        
                        // Delete word forward
                        while new_cursor < chars.len() && !chars[new_cursor].is_whitespace() {
                            new_cursor += 1;
                        }
                        
                        // Remove the characters
                        self.search_term.drain(self.search_cursor..new_cursor);
                        self.sessions
                            .update_search_term(&self.search_term, &self.colors);
                        should_render = true;
                    }
                },
                BareKey::Char('x') if key.has_modifiers(&[KeyModifier::Alt]) => {
                    // Delete character forward (readline)
                    if self.renaming_session_name.is_none() && self.search_cursor < self.search_term.len() {
                        self.search_term.remove(self.search_cursor);
                        self.sessions
                            .update_search_term(&self.search_term, &self.colors);
                        should_render = true;
                    }
                },
                BareKey::Char('x') if key.has_modifiers(&[KeyModifier::Alt, KeyModifier::Shift]) => {
                    // Cut entire line (readline)
                    if self.renaming_session_name.is_none() && !self.search_term.is_empty() {
                        self.search_term.clear();
                        self.search_cursor = 0;
                        self.sessions
                            .update_search_term(&self.search_term, &self.colors);
                        self.reset_selected_index();
                        should_render = true;
                    }
                },
                BareKey::Tab if key.has_no_modifiers() => {
                    self.toggle_active_screen();
                    should_render = true;
                },
                BareKey::Tab if key.has_modifiers(&[KeyModifier::Shift]) => {
                    self.toggle_active_screen_reverse();
                    should_render = true;
                },
                BareKey::Esc if key.has_no_modifiers() => {
                    if self.renaming_session_name.is_some() {
                        self.renaming_session_name = None;
                        should_render = true;
                    } else if !self.is_welcome_screen {
                        hide_self();
                    }
                },
                _ => {},
            }
        }
        should_render
    }
    fn handle_resurrect_session_key(&mut self, key: KeyWithModifier) -> bool {
        let mut should_render = false;
        
        // Universal quit keys - escape and ctrl+c always quit
        match key.bare_key {
            BareKey::Esc if key.has_no_modifiers() && !self.is_welcome_screen => {
                hide_self();
                return false;
            },
            BareKey::Char('c') if key.has_modifiers(&[KeyModifier::Ctrl]) && !self.is_welcome_screen => {
                hide_self();
                return false;
            },
            _ => {}
        }
        
        match key.bare_key {
            BareKey::Down if key.has_no_modifiers() => {
                self.resurrectable_sessions.move_selection_down();
                should_render = true;
            },
            BareKey::Up if key.has_no_modifiers() => {
                self.resurrectable_sessions.move_selection_up();
                should_render = true;
            },
            BareKey::Char('n') if key.has_modifiers(&[KeyModifier::Ctrl]) => {
                self.resurrectable_sessions.move_selection_down();
                should_render = true;
            },
            BareKey::Char('p') if key.has_modifiers(&[KeyModifier::Ctrl]) => {
                self.resurrectable_sessions.move_selection_up();
                should_render = true;
            },
            BareKey::Char('j') if key.has_modifiers(&[KeyModifier::Ctrl]) => {
                self.resurrectable_sessions.move_selection_down();
                should_render = true;
            },
            BareKey::Char('k') if key.has_modifiers(&[KeyModifier::Ctrl]) => {
                self.resurrectable_sessions.move_selection_up();
                should_render = true;
            },
            BareKey::Enter if key.has_no_modifiers() => {
                self.handle_selection();
                should_render = true;
            },
            BareKey::Char(character) if key.has_no_modifiers() => {
                if character == '\n' {
                    self.handle_selection();
                } else {
                    self.resurrectable_sessions.handle_character(character);
                }
                should_render = true;
            },
            BareKey::Backspace if key.has_no_modifiers() => {
                self.resurrectable_sessions.handle_backspace();
                should_render = true;
            },
            BareKey::Tab if key.has_no_modifiers() => {
                self.toggle_active_screen();
                should_render = true;
            },
            BareKey::Tab if key.has_modifiers(&[KeyModifier::Shift]) => {
                self.toggle_active_screen_reverse();
                should_render = true;
            },
            BareKey::Delete if key.has_no_modifiers() => {
                self.resurrectable_sessions.delete_selected_session();
                should_render = true;
            },
            BareKey::Char('d') if key.has_modifiers(&[KeyModifier::Ctrl]) => {
                self.resurrectable_sessions
                    .show_delete_all_sessions_warning();
                should_render = true;
            },
            BareKey::Esc if key.has_no_modifiers() => {
                if !self.is_welcome_screen {
                    hide_self();
                }
            },
            _ => {},
        }
        should_render
    }
    fn handle_selection(&mut self) {
        match self.active_screen {
            ActiveScreen::New => {
                if self.new_session_info.name().len() >= 108 {
                    // this is due to socket path limitations
                    // TODO: get this from Zellij (for reference: this is part of the interprocess
                    // package, we should get if from there if possible because it's configurable
                    // through the package)
                    self.show_error("Session name must be shorter than 108 bytes");
                    return;
                } else if self.new_session_info.name().contains('/') {
                    self.show_error("Session name cannot contain '/'");
                    return;
                } else if self
                    .sessions
                    .has_forbidden_session(self.new_session_info.name())
                {
                    self.show_error("This session exists and web clients cannot attach to it.");
                    return;
                }
                self.new_session_info.handle_selection(&self.session_name);
            },
            ActiveScreen::Attach => {
                if let Some(renaming_session_name) = self.renaming_session_name.take() {
                    if renaming_session_name.is_empty() {
                        self.show_error("New name must not be empty.");
                        return; // so that we don't hide self
                    } else if &self.session_name == &Some(renaming_session_name.clone()) {
                        // noop - we're already called that!
                        return; // so that we don't hide self
                    } else if self.sessions.has_session(&renaming_session_name) {
                        self.show_error("A session by this name already exists.");
                        return; // so that we don't hide self
                    } else if self
                        .resurrectable_sessions
                        .has_session(&renaming_session_name)
                    {
                        self.show_error("A resurrectable session by this name already exists.");
                        return; // s that we don't hide self
                    } else {
                        if renaming_session_name.contains('/') {
                            self.show_error("Session names cannot contain '/'");
                            return;
                        }
                        self.update_current_session_name_in_ui(&renaming_session_name);
                        rename_session(&renaming_session_name);
                        return; // s that we don't hide self
                    }
                }
                if let Some(selected_session_name) = self.sessions.get_selected_session_name() {
                    let selected_tab = self.sessions.get_selected_tab_position();
                    let selected_pane = self.sessions.get_selected_pane_id();
                    let is_current_session = self.sessions.selected_is_current_session();
                    if is_current_session {
                        if let Some((pane_id, is_plugin)) = selected_pane {
                            if is_plugin {
                                focus_plugin_pane(pane_id, true);
                            } else {
                                focus_terminal_pane(pane_id, true);
                            }
                        } else if let Some(tab_position) = selected_tab {
                            go_to_tab(tab_position as u32);
                        } else {
                            self.show_error("Already attached...");
                        }
                    } else {
                        switch_session_with_focus(
                            &selected_session_name,
                            selected_tab,
                            selected_pane,
                        );
                    }
                }
                self.reset_selected_index();
                self.search_term.clear();
                self.search_cursor = 0;
                self.sessions
                    .update_search_term(&self.search_term, &self.colors);
                if !self.is_welcome_screen {
                    // we usually don't want to hide_self() if we're the welcome screen because
                    // unless the user did something odd like opening an extra pane/tab in the
                    // welcome screen, this will result in the current session closing, as this is
                    // the last selectable pane...
                    hide_self();
                }
            },
            ActiveScreen::Resurrect => {
                if let Some(session_name_to_resurrect) =
                    self.resurrectable_sessions.get_selected_session_name()
                {
                    switch_session(Some(&session_name_to_resurrect));
                }
            },
        }
    }
    fn toggle_active_screen(&mut self) {
        self.active_screen = match self.active_screen {
            ActiveScreen::New => ActiveScreen::Attach,
            ActiveScreen::Attach => ActiveScreen::Resurrect,
            ActiveScreen::Resurrect => ActiveScreen::New,
        };
    }
    fn toggle_active_screen_reverse(&mut self) {
        self.active_screen = match self.active_screen {
            ActiveScreen::New => ActiveScreen::Resurrect,
            ActiveScreen::Attach => ActiveScreen::New,
            ActiveScreen::Resurrect => ActiveScreen::Attach,
        };
    }
    fn show_error(&mut self, error_text: &str) {
        self.error = Some(error_text.to_owned());
    }
    fn update_current_session_name_in_ui(&mut self, new_name: &str) {
        if let Some(old_session_name) = &self.session_name {
            self.sessions
                .update_session_name(&old_session_name, new_name);
        }
        self.session_name = Some(new_name.to_owned());
    }
    fn update_session_infos(&mut self, session_infos: Vec<SessionInfo>) {
        let session_ui_infos: Vec<SessionUiInfo> = session_infos
            .iter()
            .filter_map(|s| {
                if self.is_web_client && !s.web_clients_allowed {
                    None
                } else if self.is_welcome_screen && s.is_current_session {
                    // do not display current session if we're the welcome screen
                    // because:
                    // 1. attaching to the welcome screen from the welcome screen is not a thing
                    // 2. it can cause issues on the web (since we're disconnecting and
                    //    reconnecting to a session we just closed by disconnecting...)
                    None
                } else {
                    Some(SessionUiInfo::from_session_info(s))
                }
            })
            .collect();
        let forbidden_sessions: Vec<SessionUiInfo> = session_infos
            .iter()
            .filter_map(|s| {
                if self.is_web_client && !s.web_clients_allowed {
                    Some(SessionUiInfo::from_session_info(s))
                } else {
                    None
                }
            })
            .collect();
        let current_session_name = session_infos.iter().find_map(|s| {
            if s.is_current_session {
                Some(s.name.clone())
            } else {
                None
            }
        });
        if let Some(current_session_name) = current_session_name {
            self.session_name = Some(current_session_name);
        }
        self.sessions
            .set_sessions(session_ui_infos, forbidden_sessions);
    }
    fn main_menu_size(&self, rows: usize, cols: usize) -> (usize, usize, usize, usize) {
        // x, y, width, height
        let width = if self.is_welcome_screen {
            std::cmp::min(cols, 101)
        } else {
            cols
        };
        let x = if self.is_welcome_screen {
            (cols.saturating_sub(width) as f64 / 2.0).floor() as usize + 2
        } else {
            0
        };
        let y = if self.is_welcome_screen {
            (rows.saturating_sub(15) as f64 / 2.0).floor() as usize
        } else {
            0
        };
        let height = rows.saturating_sub(y);
        (x, y, width, height)
    }
    fn render_kill_all_sessions_warning(&self, rows: usize, columns: usize, x: usize, y: usize) {
        if rows == 0 || columns == 0 {
            return;
        }
        let session_count = self.sessions.all_other_sessions().len();
        let session_count_len = session_count.to_string().chars().count();
        let warning_description_text = format!("This will kill {session_count} active sessions");
        let confirmation_text = "Are you sure? (y/n)";
        let warning_y_location = y + (rows / 2).saturating_sub(1);
        let confirmation_y_location = y + (rows / 2) + 1;
        let warning_x_location =
            x + columns.saturating_sub(warning_description_text.chars().count()) / 2;
        let confirmation_x_location =
            x + columns.saturating_sub(confirmation_text.chars().count()) / 2;
        print_text_with_coordinates(
            Text::new(warning_description_text).color_range(0, 15..16 + session_count_len),
            warning_x_location,
            warning_y_location,
            None,
            None,
        );
        print_text_with_coordinates(
            Text::new(confirmation_text).color_indices(2, vec![15, 17]),
            confirmation_x_location,
            confirmation_y_location,
            None,
            None,
        );
    }
}
