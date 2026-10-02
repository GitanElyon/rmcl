// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use std::path::Path;
use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, ScrollbarState},
};
use tui_widget_list::{ListBuilder, ListState as TuiListState, ListView};

use crate::config::theme::{BORDER_STYLE, THEME};
use crate::instance::launch::parser::LogLevel;
use crate::instance::logs::files::{LogFileEntry, read_log_file, scan_log_files};

type PendingLogs = Arc<Mutex<Option<(String, Vec<LogFileEntry>)>>>;

pub struct LogsState {
    pub entries: Vec<LogFileEntry>,
    pub list_state: TuiListState,
    pub loaded_for: Option<String>,
    pub loading: bool,
    pub viewer_focused: bool,
    pub viewer_lines: Vec<String>,
    pub viewer_scroll: usize,
    pub viewer_max_scroll: usize,
    pub viewer_page: usize,
    pub filter_open: bool,
    pub filter_selected: usize,
    pub hidden_levels: Vec<LogLevel>,
    pub selection: Option<(usize, usize)>,
    pub selecting: bool,
    pub viewer_area: Rect,
    pub scrollbar_state: ScrollbarState,
    pub viewer_scrollbar_state: ScrollbarState,
    pub search: super::search::SearchState,
    pub viewer_search: super::search::SearchState,
    selected_path: Option<std::path::PathBuf>,
    pending: PendingLogs,
    last_rescan: std::time::Instant,
    instances_dir_cache: Option<std::path::PathBuf>,
    was_live: bool,
}

impl Default for LogsState {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            list_state: TuiListState::default(),
            loaded_for: None,
            loading: false,
            viewer_focused: false,
            viewer_lines: Vec::new(),
            viewer_scroll: 0,
            viewer_max_scroll: 0,
            viewer_page: 0,
            filter_open: false,
            filter_selected: 0,
            hidden_levels: Vec::new(),
            selection: None,
            selecting: false,
            viewer_area: Rect::default(),
            scrollbar_state: ScrollbarState::default(),
            viewer_scrollbar_state: ScrollbarState::default(),
            search: super::search::SearchState::default(),
            viewer_search: super::search::SearchState::default(),
            selected_path: None,
            pending: Arc::new(Mutex::new(None)),
            last_rescan: std::time::Instant::now(),
            instances_dir_cache: None,
            was_live: false,
        }
    }
}

impl LogsState {
    pub fn start_load(&mut self, instances_dir: &Path, instance_name: &str) {
        self.pending = Arc::new(Mutex::new(None));
        self.loading = true;
        self.loaded_for = Some(instance_name.to_string());
        self.instances_dir_cache = Some(instances_dir.to_path_buf());
        self.entries.clear();
        self.list_state = TuiListState::default();
        self.viewer_lines.clear();
        self.viewer_scroll = 0;
        self.viewer_focused = false;
        self.selected_path = None;
        self.was_live = false;
        self.last_rescan = std::time::Instant::now();

        let dir = instances_dir.to_path_buf();
        let tag = instance_name.to_string();
        let pending = self.pending.clone();

        tokio::spawn(async move {
            let scan_dir = dir.clone();
            let scan_name = tag.clone();
            let entries =
                tokio::task::spawn_blocking(move || scan_log_files(&scan_dir, &scan_name))
                    .await
                    .unwrap_or_default();

            if let Ok(mut slot) = pending.lock() {
                *slot = Some((tag, entries));
                crate::feedback::request_redraw();
            }
        });
    }

    pub fn drain_pending(&mut self) {
        let live_now = self.has_live();
        let live_changed = live_now != self.was_live;
        let selected_live =
            self.was_live && self.selected_path.is_none() && self.list_state.selected.is_some();

        let taken = match self.pending.lock() {
            Ok(mut slot) => slot.take(),
            _ => None,
        };

        let updated = if let Some((instance_name, entries)) = taken
            && self.loaded_for.as_deref() == Some(&instance_name)
        {
            self.entries = entries;
            self.loading = false;
            true
        } else {
            false
        };
        if updated || live_changed {
            let previous = self.list_state.selected.unwrap_or(0);
            let indices = self.display_indices();
            self.list_state.selected = indices
                .iter()
                .position(|index| match index {
                    None => selected_live,
                    Some(index) => self.selected_path.as_ref() == Some(&self.entries[*index].path),
                })
                .or_else(|| (!indices.is_empty()).then(|| previous.min(indices.len() - 1)));
            self.update_scrollbar();
            self.load_selected_content();
        }
        self.was_live = live_now;
    }

    pub fn try_rescan(&mut self) {
        if self.last_rescan.elapsed() < std::time::Duration::from_secs(2) {
            return;
        }
        self.last_rescan = std::time::Instant::now();

        let (Some(dir), Some(name)) = (&self.instances_dir_cache, &self.loaded_for) else {
            return;
        };
        if !matches!(
            crate::instance::runtime::get(name),
            Some(
                crate::instance::runtime::RunState::Authenticating
                    | crate::instance::runtime::RunState::Starting
                    | crate::instance::runtime::RunState::Running
            )
        ) {
            return;
        }

        let dir = dir.clone();
        let tag = name.clone();
        self.pending = Arc::new(Mutex::new(None));
        let pending = self.pending.clone();

        tokio::spawn(async move {
            let scan_dir = dir.clone();
            let scan_name = tag.clone();
            let entries =
                tokio::task::spawn_blocking(move || scan_log_files(&scan_dir, &scan_name))
                    .await
                    .unwrap_or_default();

            if let Ok(mut slot) = pending.lock() {
                *slot = Some((tag, entries));
                crate::feedback::request_redraw();
            }
        });
    }

    // when an instance is active or has just crashed, a synthetic "Live" entry
    // is injected at index 0 so parsed live log styling is retained.
    fn has_live(&self) -> bool {
        let name = self.loaded_for.as_deref().unwrap_or("");
        matches!(
            crate::instance::runtime::get(name),
            Some(crate::instance::runtime::RunState::Running)
                | Some(crate::instance::runtime::RunState::Starting)
                | Some(crate::instance::runtime::RunState::Crashed(_))
        )
    }

    fn display_count(&self) -> usize {
        self.display_indices().len()
    }

    fn live_display_name(&self) -> String {
        self.entries
            .first()
            .map(|entry| entry.name.trim_end_matches(".log").to_owned())
            .unwrap_or_else(|| "Live".to_owned())
    }

    fn display_indices(&self) -> Vec<Option<usize>> {
        let mut indices = Vec::new();
        if self.has_live() && self.search.matches(&self.live_display_name()) {
            indices.push(None);
        }
        indices.extend(
            self.entries
                .iter()
                .enumerate()
                .filter(|(_, entry)| self.search.matches(entry.name.trim_end_matches(".log")))
                .map(|(index, _)| Some(index)),
        );
        indices
    }

    fn is_live_selected(&self) -> bool {
        self.list_state
            .selected
            .and_then(|selected| self.display_indices().get(selected).copied())
            == Some(None)
    }

    fn file_index_for_selected(&self) -> Option<usize> {
        let selected = self.list_state.selected?;
        self.display_indices().get(selected).copied().flatten()
    }

    fn load_selected_content(&mut self) {
        if self.is_live_selected() {
            self.selected_path = None;
            self.viewer_lines.clear();
            self.viewer_scroll = 0;
            return;
        }

        let path = self
            .file_index_for_selected()
            .and_then(|i| self.entries.get(i))
            .map(|e| e.path.clone());

        if path == self.selected_path {
            return;
        }
        self.selected_path = path.clone();
        self.viewer_scroll = 0;

        if let Some(path) = path {
            self.viewer_lines = read_log_file(&path);
        } else {
            self.viewer_lines.clear();
        }
    }

    fn update_scrollbar(&mut self) {
        let count = self.display_count();
        let max = count.saturating_sub(1);
        if count == 0 {
            self.list_state.selected = None;
        } else if self
            .list_state
            .selected
            .is_none_or(|selected| selected >= count)
        {
            self.list_state.selected = Some(max);
        }
        let pos = self.list_state.selected.unwrap_or(0);
        self.scrollbar_state = ScrollbarState::new(max).position(pos);
    }

    fn update_viewer_scrollbar(&mut self, visible_height: usize, line_count: usize) {
        self.viewer_max_scroll = line_count.saturating_sub(visible_height);
        if self.viewer_scroll > self.viewer_max_scroll {
            self.viewer_scroll = self.viewer_max_scroll;
        }
        self.viewer_scrollbar_state =
            ScrollbarState::new(self.viewer_max_scroll).position(self.viewer_scroll);
    }

    pub fn pending_delete(
        &self,
    ) -> Option<crate::tui::widgets::content::list::PendingContentDelete> {
        let index = self.file_index_for_selected()?;
        let entry = self.entries.get(index)?;
        Some(crate::tui::widgets::content::list::PendingContentDelete {
            name: entry.name.clone(),
            path: entry.path.clone(),
        })
    }

    pub fn remove_path(&mut self, path: &Path) {
        self.entries.retain(|entry| entry.path != path);
        let display_count = self.display_count();
        if display_count == 0 {
            self.list_state.selected = None;
            self.viewer_focused = false;
            self.selected_path = None;
            self.viewer_lines.clear();
            self.viewer_scroll = 0;
        } else if let Some(sel) = self.list_state.selected {
            self.list_state.selected = Some(sel.min(display_count.saturating_sub(1)));
            self.load_selected_content();
        }
        self.update_scrollbar();
    }
}

pub fn handle_key(key_event: &KeyEvent, state: &mut LogsState) -> bool {
    let shift = key_event.modifiers.contains(KeyModifiers::SHIFT);

    if state.filter_open {
        match key_event.code {
            KeyCode::Esc | KeyCode::Char('f') => {
                state.filter_open = false;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                state.filter_selected =
                    (state.filter_selected + 1).min(LOG_LEVEL_FILTER_ORDER.len() - 1);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                state.filter_selected = state.filter_selected.saturating_sub(1);
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                toggle_hidden_level(&mut state.hidden_levels, state.filter_selected);
                state.selection = None;
                state.viewer_scroll = 0;
            }
            KeyCode::Char('r') => {
                state.hidden_levels.clear();
            }
            _ => {}
        }
        return true;
    }

    if state.viewer_focused {
        if state.viewer_search.active {
            match key_event.code {
                KeyCode::Enter => {
                    state.viewer_search.confirm();
                    state.selection = None;
                    state.viewer_scroll = 0;
                }
                KeyCode::Esc => {
                    state.viewer_search.deactivate();
                    state.viewer_scroll = 0;
                }
                KeyCode::Backspace => {
                    state.viewer_search.backspace(key_event.modifiers);
                    state.viewer_scroll = 0;
                }
                KeyCode::Char(c) => {
                    state.viewer_search.push(c);
                    state.viewer_scroll = 0;
                }
                _ => {}
            }
            return true;
        }

        if key_event.code == KeyCode::Char('/') {
            state.viewer_search.activate();
            state.viewer_scroll = 0;
            return true;
        }

        if key_event.code == KeyCode::Char('f') {
            state.filter_open = true;
            state.filter_selected = 0;
            return true;
        }

        match key_event.code {
            KeyCode::Char('j') | KeyCode::Down => {
                if state.viewer_scroll < state.viewer_max_scroll {
                    state.viewer_scroll += 1;
                    state.viewer_scrollbar_state =
                        ScrollbarState::new(state.viewer_max_scroll).position(state.viewer_scroll);
                }
                true
            }
            KeyCode::Char('k') | KeyCode::Up => {
                state.viewer_scroll = state.viewer_scroll.saturating_sub(1);
                state.viewer_scrollbar_state =
                    ScrollbarState::new(state.viewer_max_scroll).position(state.viewer_scroll);
                true
            }
            KeyCode::Char('G') | KeyCode::End => {
                state.viewer_scroll = state.viewer_max_scroll;
                state.viewer_scrollbar_state =
                    ScrollbarState::new(state.viewer_max_scroll).position(state.viewer_scroll);
                true
            }
            KeyCode::Char('g') | KeyCode::Home => {
                state.viewer_scroll = 0;
                state.viewer_scrollbar_state =
                    ScrollbarState::new(state.viewer_max_scroll).position(state.viewer_scroll);
                true
            }
            KeyCode::PageDown => {
                state.viewer_scroll =
                    (state.viewer_scroll + state.viewer_page.max(1)).min(state.viewer_max_scroll);
                state.viewer_scrollbar_state =
                    ScrollbarState::new(state.viewer_max_scroll).position(state.viewer_scroll);
                true
            }
            KeyCode::PageUp => {
                state.viewer_scroll = state.viewer_scroll.saturating_sub(state.viewer_page.max(1));
                state.viewer_scrollbar_state =
                    ScrollbarState::new(state.viewer_max_scroll).position(state.viewer_scroll);
                true
            }
            KeyCode::Esc => {
                if state.selection.take().is_some() {
                    return true;
                }
                state.viewer_focused = false;
                true
            }
            KeyCode::Char('y') => {
                state.selecting = false;
                yank_viewer_selection(state);
                true
            }
            KeyCode::Char('H') | KeyCode::Left if shift => {
                state.viewer_focused = false;
                true
            }
            _ => false,
        }
    } else {
        if state.search.active {
            match key_event.code {
                KeyCode::Enter => {
                    state.search.confirm();
                    state.selection = None;
                    state.list_state.selected = Some(0);
                    state.load_selected_content();
                    state.update_scrollbar();
                }
                KeyCode::Esc => {
                    state.search.deactivate();
                    state.list_state.selected = Some(0);
                    state.load_selected_content();
                    state.update_scrollbar();
                }
                KeyCode::Backspace => {
                    state.search.backspace(key_event.modifiers);
                    state.list_state.selected = Some(0);
                    state.load_selected_content();
                    state.update_scrollbar();
                }
                KeyCode::Char(c) => {
                    state.search.push(c);
                    state.list_state.selected = Some(0);
                    state.load_selected_content();
                    state.update_scrollbar();
                }
                _ => {}
            }
            return true;
        }

        if key_event.code == KeyCode::Char('/') {
            state.search.activate();
            state.list_state.selected = Some(0);
            state.update_scrollbar();
            return true;
        }

        if key_event.code == KeyCode::Char('f') {
            state.filter_open = true;
            state.filter_selected = 0;
            return true;
        }

        let display_count = state.display_count();
        match key_event.code {
            KeyCode::Char('j') | KeyCode::Down => {
                if display_count == 0 {
                    return true;
                }
                let current = state.list_state.selected.unwrap_or(0);
                state.list_state.selected = Some((current + 1).min(display_count - 1));
                state.load_selected_content();
                state.update_scrollbar();
                true
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let current = state.list_state.selected.unwrap_or(0);
                state.list_state.selected = Some(current.saturating_sub(1));
                state.load_selected_content();
                state.update_scrollbar();
                true
            }
            KeyCode::Enter if shift => {
                if let Some(dir) = state
                    .file_index_for_selected()
                    .and_then(|index| state.entries.get(index))
                    .and_then(|entry| entry.path.parent())
                    && let Err(error) = open::that_detached(dir)
                {
                    tracing::error!("Failed to open directory: {error}");
                    crate::feedback::errors::push_message(
                        tracing::Level::ERROR,
                        format!("Could not open {}: {error}", dir.display()),
                    );
                }
                true
            }
            KeyCode::Enter => {
                state.viewer_focused = true;
                true
            }
            KeyCode::Char('L') | KeyCode::Right if shift => {
                state.viewer_focused = true;
                true
            }
            KeyCode::Esc if state.selection.is_some() => {
                state.selection = None;
                true
            }
            KeyCode::Char('y') => {
                state.selecting = false;
                yank_viewer_selection(state);
                true
            }
            _ => false,
        }
    }
}

pub fn render(frame: &mut Frame, area: Rect, state: &mut LogsState, is_focused: bool) {
    let theme = THEME.as_ref();
    if state.loading {
        frame.render_widget(
            Paragraph::new("Loading logs...").style(Style::default().fg(theme.text_dim())),
            area,
        );
        return;
    }

    let display_count = state.display_count();

    if display_count == 0 {
        frame.render_widget(
            Paragraph::new("No logs yet.").style(Style::default().fg(theme.text_dim())),
            area,
        );
        return;
    }

    if state.list_state.selected.is_none() && display_count > 0 {
        state.list_state.selected = Some(0);
        state.load_selected_content();
    }

    let [list_area, viewer_area] =
        Layout::horizontal([Constraint::Length(30), Constraint::Min(0)]).areas(area);

    render_list(frame, list_area, state, is_focused);
    render_viewer(frame, viewer_area, state);
}

fn render_list(frame: &mut Frame, area: Rect, state: &mut LogsState, is_focused: bool) {
    let theme = THEME.as_ref();
    let list_focused = is_focused && !state.viewer_focused;
    let border_color = if list_focused {
        theme.accent()
    } else {
        theme.border()
    };

    let block = Block::default()
        .borders(Borders::RIGHT)
        .border_type(BORDER_STYLE.to_border_type())
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let display_count = state.display_count();

    let entries_snapshot: Vec<(String, bool)> = state
        .display_indices()
        .into_iter()
        .map(|index| match index {
            Some(index) => (
                state.entries[index]
                    .name
                    .trim_end_matches(".log")
                    .to_owned(),
                false,
            ),
            None => (state.live_display_name(), true),
        })
        .collect();
    let search = &state.search;
    let success = theme.success();
    let accent = theme.accent();
    let text = theme.text();
    let background = theme.background();
    let stripe = theme.stripe();

    let builder = ListBuilder::new(move |context| {
        let (name, is_live) = &entries_snapshot[context.index];
        let show_selected = list_focused && context.is_selected;

        let style = if *is_live && show_selected {
            Style::default().fg(success).add_modifier(Modifier::BOLD)
        } else if *is_live {
            Style::default().fg(success)
        } else if show_selected {
            Style::default().fg(accent).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(text)
        };

        let bg = if context.index % 2 == 0 {
            background
        } else {
            stripe
        };

        let selector = if show_selected {
            Span::styled("\u{258c} ", Style::default().fg(accent))
        } else {
            Span::raw("  ")
        };
        let mut spans = vec![selector];
        spans.extend(search.highlight_spans(name, style));
        let item = ratatui::text::Text::from(Line::from(spans)).style(Style::default().bg(bg));
        (item, 1)
    });

    let list = ListView::new(builder, display_count);
    frame.render_stateful_widget(list, inner, &mut state.list_state);

    let scrollbar_area = Rect {
        x: inner.x + inner.width.saturating_sub(0),
        y: inner.y + 1,
        width: 1,
        height: inner.height.saturating_sub(2),
    };
    frame.render_stateful_widget(
        super::scrollbar(theme.accent()),
        scrollbar_area,
        &mut state.scrollbar_state,
    );
}

fn render_viewer(frame: &mut Frame, area: Rect, state: &mut LogsState) {
    let theme = THEME.as_ref();
    state.viewer_area = area;
    let is_live = state.is_live_selected();

    let lines = filtered_viewer_lines(state);

    let visible_height = area.height as usize;
    state.viewer_page = visible_height;
    // auto-scroll: if the user was already at the bottom, keep following
    // new lines as they come in (like `tail -f` behavior)
    let was_at_bottom = state.viewer_scroll >= state.viewer_max_scroll;
    state.update_viewer_scrollbar(visible_height, lines.len());

    if is_live && was_at_bottom && !state.viewer_search.active {
        state.viewer_scroll = state.viewer_max_scroll;
        state.viewer_scrollbar_state =
            ScrollbarState::new(state.viewer_max_scroll).position(state.viewer_scroll);
    }

    if lines.is_empty() {
        if state.filter_open {
            render_level_filter(frame, area, state.filter_selected, &state.hidden_levels);
        }
        return;
    }

    let search = &state.viewer_search;
    let selected = state.selection.map(ordered_selection);
    let styled_lines: Vec<Line> = lines
        .iter()
        .enumerate()
        .skip(state.viewer_scroll)
        .take(visible_height)
        .map(|(index, line)| {
            let mut style = line
                .level
                .map(log_level_style)
                .unwrap_or_else(|| line_level_style(&line.text));
            if selected.is_some_and(|(from, to)| index >= from && index <= to) {
                style = style.add_modifier(Modifier::REVERSED);
            }
            search.highlight_line(&line.text, style)
        })
        .collect();

    frame.render_widget(Paragraph::new(styled_lines), area);

    if state.filter_open {
        render_level_filter(frame, area, state.filter_selected, &state.hidden_levels);
    }

    let scrollbar_area = Rect {
        x: area.x + area.width.saturating_sub(0),
        y: area.y + 1,
        width: 1,
        height: area.height.saturating_sub(2),
    };
    frame.render_stateful_widget(
        super::scrollbar(theme.accent()),
        scrollbar_area,
        &mut state.viewer_scrollbar_state,
    );
}

pub(crate) const LOG_LEVEL_FILTER_ORDER: [LogLevel; 5] = [
    LogLevel::Error,
    LogLevel::Warn,
    LogLevel::Info,
    LogLevel::Debug,
    LogLevel::Trace,
];

pub(crate) fn level_label(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Error => "Error",
        LogLevel::Warn => "Warn",
        LogLevel::Info => "Info",
        LogLevel::Debug => "Debug",
        LogLevel::Trace => "Trace",
    }
}

pub(crate) fn level_of_line(line: &str) -> Option<LogLevel> {
    let upper = line.to_uppercase();
    if upper.contains("ERROR") || upper.contains("FATAL") || upper.contains("[STDERR]") {
        Some(LogLevel::Error)
    } else if upper.contains("WARN") {
        Some(LogLevel::Warn)
    } else if upper.contains("DEBUG") {
        Some(LogLevel::Debug)
    } else if upper.contains("TRACE") {
        Some(LogLevel::Trace)
    } else if upper.contains("INFO") {
        Some(LogLevel::Info)
    } else {
        None
    }
}

/// Whether a line is hidden by the level filter. Lines without a
/// recognizable level are always shown.
pub(crate) fn level_hidden(hidden: &[LogLevel], text: &str) -> bool {
    level_of_line(text).is_some_and(|level| hidden.contains(&level))
}

pub(crate) fn toggle_hidden_level(hidden: &mut Vec<LogLevel>, selected: usize) {
    if let Some(level) = LOG_LEVEL_FILTER_ORDER.get(selected) {
        if hidden.contains(level) {
            hidden.retain(|kept| kept != level);
        } else {
            hidden.push(*level);
        }
    }
}

fn ordered_selection(selection: (usize, usize)) -> (usize, usize) {
    (selection.0.min(selection.1), selection.0.max(selection.1))
}

fn filtered_viewer_lines(state: &LogsState) -> Vec<ViewerLine> {
    let all_lines: Vec<ViewerLine> = if state.is_live_selected() {
        let name = state.loaded_for.as_deref().unwrap_or("");
        crate::instance::logs::live::get_entries(name)
            .into_iter()
            .map(|line| ViewerLine {
                text: line.text,
                level: Some(line.level),
            })
            .collect()
    } else {
        state
            .viewer_lines
            .iter()
            .cloned()
            .map(|text| ViewerLine { text, level: None })
            .collect()
    };
    all_lines
        .into_iter()
        .filter(|line| state.viewer_search.matches(&line.text))
        .filter(|line| match line.level {
            Some(level) => !state.hidden_levels.contains(&level),
            None => !level_hidden(&state.hidden_levels, &line.text),
        })
        .collect()
}

pub(crate) fn filtered_viewer_line_count(state: &LogsState) -> usize {
    filtered_viewer_lines(state).len()
}

pub(crate) fn yank_viewer_selection(state: &mut LogsState) {
    let Some(selection) = state.selection.map(ordered_selection) else {
        crate::feedback::errors::push_message(
            tracing::Level::INFO,
            "Drag across log lines to select them, then press y to copy",
        );
        return;
    };
    let lines = filtered_viewer_lines(state);
    if lines.is_empty() {
        return;
    }
    let end = selection.1.min(lines.len() - 1);
    if selection.0 > end {
        return;
    }
    let text = lines[selection.0..=end]
        .iter()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    match copy_to_clipboard(&text) {
        Some((count, false)) => crate::feedback::errors::push_message(
            tracing::Level::INFO,
            format!("Copied {count} line(s) to the clipboard"),
        ),
        Some((count, true)) => crate::feedback::errors::push_message(
            tracing::Level::WARN,
            format!("Copied the first {count} lines; the selection was truncated"),
        ),
        None => {}
    }
}

/// Copies text through the terminal (OSC 52) so it works over SSH too.
/// Returns the copied line count and whether the payload was truncated.
pub(crate) fn copy_to_clipboard(text: &str) -> Option<(usize, bool)> {
    use base64::Engine;
    const MAX_LINES: usize = 2000;
    let lines: Vec<&str> = text.lines().take(MAX_LINES + 1).collect();
    if lines.is_empty() {
        return None;
    }
    let truncated = lines.len() > MAX_LINES;
    let count = lines.len().min(MAX_LINES);
    let encoded = base64::engine::general_purpose::STANDARD.encode(lines[..count].join("\n"));
    let sequence = format!("\x1b]52;c;{encoded}\x07");
    use std::io::Write;
    std::io::stdout()
        .write_all(sequence.as_bytes())
        .and_then(|_| std::io::stdout().flush())
        .ok()?;
    Some((count, truncated))
}

pub(crate) fn render_level_filter(
    frame: &mut Frame,
    area: Rect,
    selected: usize,
    hidden: &[LogLevel],
) {
    use ratatui::layout::Constraint;
    use ratatui::widgets::Widget;
    let theme = THEME.as_ref();
    let popup = area.centered(Constraint::Length(26), Constraint::Length(9));
    let rows = LOG_LEVEL_FILTER_ORDER
        .iter()
        .enumerate()
        .map(|(index, level)| {
            let marker = if hidden.contains(level) { "[ ]" } else { "[x]" };
            let prefix = if index == selected { "▶ " } else { "  " };
            Line::from(vec![
                Span::styled(
                    prefix,
                    Style::default().fg(if index == selected {
                        theme.accent()
                    } else {
                        theme.text()
                    }),
                ),
                Span::styled(
                    format!("{marker} {}", level_label(*level)),
                    Style::default().fg(theme.text()),
                ),
            ])
        })
        .collect::<Vec<_>>();
    let frame_widget = super::popups::base::PopupFrame {
        title: Line::from(" Log levels ").style(
            Style::default()
                .fg(theme.text())
                .add_modifier(Modifier::BOLD),
        ),
        border_color: theme.accent(),
        bg: Some(theme.surface()),
        keybinds: Some(super::popups::keybind_line(&[
            ("Enter", " toggle"),
            ("r", " reset"),
            ("Esc", " close"),
        ])),
        search_line: None,
        content: Box::new(move |area, buffer| {
            Paragraph::new(rows.clone()).render(area, buffer);
        }),
    };
    frame.render_widget(frame_widget, popup);
}

struct ViewerLine {
    text: String,
    level: Option<LogLevel>,
}

fn log_level_style(level: LogLevel) -> Style {
    let theme = THEME.as_ref();
    match level {
        LogLevel::Error => Style::default().fg(theme.error()),
        LogLevel::Warn => Style::default().fg(theme.warning()),
        LogLevel::Debug | LogLevel::Trace => Style::default().fg(theme.text_dim()),
        LogLevel::Info => Style::default().fg(theme.text()),
    }
}

pub(crate) fn line_level_style(line: &str) -> Style {
    let theme = THEME.as_ref();
    let upper = line.to_uppercase();
    if upper.contains("ERROR") || upper.contains("FATAL") || upper.contains("[STDERR]") {
        Style::default().fg(theme.error())
    } else if upper.contains("WARN") {
        Style::default().fg(theme.warning())
    } else if upper.contains("DEBUG") || upper.contains("TRACE") {
        Style::default().fg(theme.text_dim())
    } else {
        Style::default().fg(theme.text())
    }
}

#[cfg(test)]
#[path = "../tests/widgets/logs_viewer.rs"]
mod tests;
