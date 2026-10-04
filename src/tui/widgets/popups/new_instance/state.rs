// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use crate::instance::{loader::GameVersion, models::ModLoader};
use crate::tui::widgets::instances;
use crossterm::event::{KeyCode, KeyEvent};
use std::sync::LazyLock;
use std::sync::{Arc, Mutex};
use tui_prompts::{FocusState, State as PromptState, TextState};

use super::super::LoadState;

pub(crate) static WIZARD_STATE: LazyLock<Arc<Mutex<WizardState>>> =
    LazyLock::new(|| Arc::new(Mutex::new(WizardState::default())));
pub(crate) static WIZARD_RESULT: LazyLock<Arc<Mutex<Option<WizardParams>>>> =
    LazyLock::new(|| Arc::new(Mutex::new(None)));

#[derive(Debug, Clone)]
pub struct WizardParams {
    pub name: String,
    pub game_version: String,
    pub loader: ModLoader,
    pub loader_version: Option<String>,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub enum WizardStep {
    #[default]
    Name,
    Version,
    Loader,
    LoaderVersion,
    Confirm,
}

#[derive(Debug, Clone)]
pub struct WizardState {
    request_epoch: u64,
    pub step: WizardStep,
    pub name_state: TextState<'static>,
    pub versions: LoadState<Vec<GameVersion>>,
    pub version_idx: usize,
    pub show_snapshots: bool,
    pub loader_idx: usize,
    pub loader_versions: LoadState<Vec<String>>,
    pub loader_version_idx: usize,
    pub version_search: crate::tui::widgets::search::SearchState,
}

impl Default for WizardState {
    fn default() -> Self {
        Self {
            request_epoch: 0,
            step: WizardStep::Name,
            name_state: TextState::new().with_focus(FocusState::Focused),
            versions: LoadState::Idle,
            version_idx: 0,
            show_snapshots: false,
            loader_idx: 0,
            loader_versions: LoadState::Idle,
            loader_version_idx: 0,
            version_search: crate::tui::widgets::search::SearchState::default(),
        }
    }
}

impl WizardState {
    pub fn reset(&mut self) {
        let next_epoch = self.request_epoch.wrapping_add(1);
        *self = WizardState::default();
        self.request_epoch = next_epoch;
    }

    pub fn selected_version(&self) -> Option<&GameVersion> {
        visible_versions(self).nth(self.version_idx)
    }

    pub fn selected_loader(&self) -> ModLoader {
        super::super::select_list::MOD_LOADERS[self.loader_idx % 5]
    }

    pub fn selected_loader_version(&self) -> Option<String> {
        if let LoadState::Loaded(ref versions) = self.loader_versions {
            versions.get(self.loader_version_idx).cloned()
        } else {
            None
        }
    }
}

pub fn handle_key(key_event: &KeyEvent, instances_state: &mut instances::State) {
    let mut state = match WIZARD_STATE.lock() {
        Ok(state) => state,
        Err(e) => {
            tracing::error!("Wizard state lock poisoned: {}", e);
            instances_state.show_popup = false;
            return;
        }
    };

    match state.step {
        WizardStep::Name => handle_name_key(&mut state, key_event, instances_state),
        WizardStep::Version => handle_version_key(&mut state, key_event, instances_state),
        WizardStep::Loader => handle_loader_key(&mut state, key_event, instances_state),
        WizardStep::LoaderVersion => {
            handle_loader_version_key(&mut state, key_event, instances_state)
        }
        WizardStep::Confirm => handle_confirm_key(&mut state, key_event, instances_state),
    }
}

pub fn take_result() -> Option<WizardParams> {
    match WIZARD_RESULT.lock() {
        Ok(mut r) => r.take(),
        Err(_) => None,
    }
}

pub(in crate::tui) fn text_input_active() -> bool {
    WIZARD_STATE.lock().is_ok_and(|state| {
        state.step == WizardStep::Name
            || (state.step == WizardStep::Version && state.version_search.active)
    })
}

fn handle_name_key(
    state: &mut WizardState,
    key_event: &KeyEvent,
    instances_state: &mut instances::State,
) {
    match key_event.code {
        KeyCode::Esc => {
            close_popup(state, instances_state);
        }
        KeyCode::Enter => {
            if state.name_state.value().trim().is_empty() {
                return;
            }
            state.step = WizardStep::Loader;
        }
        KeyCode::Backspace
            if key_event
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL) =>
        {
            let position = state.name_state.position();
            let position = crate::tui::widgets::search::delete_previous_word(
                state.name_state.value_mut(),
                position,
            );
            *state.name_state.position_mut() = position;
        }
        _ => {
            state.name_state.handle_key_event(*key_event);
        }
    }
}

fn handle_version_key(
    state: &mut WizardState,
    key_event: &KeyEvent,
    instances_state: &mut instances::State,
) {
    if state.version_search.active {
        match key_event.code {
            KeyCode::Esc => {
                state.version_search.deactivate();
                clamp_version_index(state);
                return;
            }
            KeyCode::Backspace => {
                state.version_search.backspace(key_event.modifiers);
                clamp_version_index(state);
                return;
            }
            KeyCode::Char('j') | KeyCode::Down => {}
            KeyCode::Char('k') | KeyCode::Up => {}
            KeyCode::Char(c) => {
                state.version_search.push(c);
                state.version_idx = 0;
                return;
            }
            _ => {}
        }
    }

    let visible_count = visible_versions(state).count();

    match key_event.code {
        KeyCode::Esc => {
            close_popup(state, instances_state);
        }
        KeyCode::Left | KeyCode::Char('h') if !state.version_search.active => {
            state.step = WizardStep::Loader;
        }
        KeyCode::Char('j') | KeyCode::Down if visible_count > 0 => {
            state.version_idx = (state.version_idx + 1).min(visible_count.saturating_sub(1));
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.version_idx = state.version_idx.saturating_sub(1);
        }
        KeyCode::Char('s') if !state.version_search.active => {
            state.show_snapshots = !state.show_snapshots;
            clamp_version_index(state);
        }
        KeyCode::Char('/') if !state.version_search.active => {
            state.version_search.activate();
            state.version_idx = 0;
        }
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') if !state.version_search.active => {
            if state.selected_version().is_none() {
                return;
            }
            state.loader_versions = LoadState::Idle;
            state.loader_version_idx = 0;
            if state.selected_loader() == ModLoader::Vanilla {
                state.step = WizardStep::Confirm;
            } else {
                state.step = WizardStep::LoaderVersion;
                let game_version = state.selected_version().map(|v| v.id.clone());
                let loader = state.selected_loader();
                if let Some(gv) = game_version {
                    ensure_loader_versions_loaded(state, loader, gv);
                }
            }
        }
        KeyCode::Enter if state.version_search.active => {
            state.version_search.active = false;
        }
        _ => {}
    }
}

fn handle_loader_key(
    state: &mut WizardState,
    key_event: &KeyEvent,
    instances_state: &mut instances::State,
) {
    match key_event.code {
        KeyCode::Esc => close_popup(state, instances_state),
        KeyCode::Left | KeyCode::Char('h') => state.step = WizardStep::Name,
        KeyCode::Char('j') | KeyCode::Down => {
            state.loader_idx = (state.loader_idx + 1).min(4);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.loader_idx = state.loader_idx.saturating_sub(1);
        }
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
            state.versions = LoadState::Idle;
            state.version_idx = 0;
            state.version_search.deactivate();
            state.version_search.active = false;
            state.step = WizardStep::Version;
            ensure_versions_loaded(state);
        }
        _ => {}
    }
}

fn handle_loader_version_key(
    state: &mut WizardState,
    key_event: &KeyEvent,
    instances_state: &mut instances::State,
) {
    if state.selected_loader() == ModLoader::Vanilla {
        state.step = WizardStep::Confirm;
        return;
    }

    let version_count = match &state.loader_versions {
        LoadState::Loaded(versions) => versions.len(),
        _ => 0,
    };

    match key_event.code {
        KeyCode::Esc => close_popup(state, instances_state),
        KeyCode::Left | KeyCode::Char('h') => state.step = WizardStep::Version,
        KeyCode::Char('j') | KeyCode::Down if version_count > 0 => {
            state.loader_version_idx =
                (state.loader_version_idx + 1).min(version_count.saturating_sub(1));
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.loader_version_idx = state.loader_version_idx.saturating_sub(1);
        }
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
            if state.selected_loader_version().is_none() {
                return;
            }
            state.step = WizardStep::Confirm;
        }
        _ => {}
    }
}

fn handle_confirm_key(
    state: &mut WizardState,
    key_event: &KeyEvent,
    instances_state: &mut instances::State,
) {
    match key_event.code {
        KeyCode::Esc => close_popup(state, instances_state),
        KeyCode::Left | KeyCode::Char('h') => {
            if state.selected_loader() == ModLoader::Vanilla {
                state.step = WizardStep::Version;
            } else {
                state.step = WizardStep::LoaderVersion;
            }
        }
        KeyCode::Enter => {
            let selected_version = match state.selected_version() {
                Some(version) => version.id.clone(),
                None => return,
            };

            let params = WizardParams {
                name: state.name_state.value().trim().to_string(),
                game_version: selected_version,
                loader: state.selected_loader(),
                loader_version: if state.selected_loader() == ModLoader::Vanilla {
                    None
                } else {
                    state.selected_loader_version()
                },
            };

            match WIZARD_RESULT.lock() {
                Ok(mut result) => {
                    *result = Some(params);
                }
                Err(e) => {
                    tracing::error!("Wizard result lock poisoned: {}", e);
                }
            }

            close_popup(state, instances_state);
        }
        _ => {}
    }
}

fn close_popup(state: &mut WizardState, instances_state: &mut instances::State) {
    state.reset();
    instances_state.show_popup = false;
}

pub(crate) fn visible_versions(state: &WizardState) -> impl Iterator<Item = &GameVersion> {
    let q = state.version_search.query.to_lowercase();
    let versions = match &state.versions {
        LoadState::Loaded(versions) => versions.as_slice(),
        _ => &[],
    };
    versions.iter().filter(move |version| {
        (state.show_snapshots || version.stable)
            && (q.is_empty() || version.id.to_lowercase().contains(&q))
    })
}

pub(crate) fn clamp_version_index(state: &mut WizardState) {
    let count = visible_versions(state).count();
    if count == 0 {
        state.version_idx = 0;
    } else if state.version_idx >= count {
        state.version_idx = count.saturating_sub(1);
    }
}

pub(crate) fn clamp_loader_version_index(state: &mut WizardState) {
    if let LoadState::Loaded(versions) = &state.loader_versions {
        if versions.is_empty() {
            state.loader_version_idx = 0;
        } else if state.loader_version_idx >= versions.len() {
            state.loader_version_idx = versions.len().saturating_sub(1);
        }
    } else {
        state.loader_version_idx = 0;
    }
}

// Spawn only on Idle -> Loading to avoid repeating network requests per frame.
pub(crate) fn ensure_versions_loaded(state: &mut WizardState) {
    if !matches!(state.versions, LoadState::Idle) {
        return;
    }

    state.versions = LoadState::Loading;
    state.request_epoch = state.request_epoch.wrapping_add(1);
    let epoch = state.request_epoch;
    let versions_arc = WIZARD_STATE.clone();
    let loader = state.selected_loader();
    tokio::spawn(async move {
        match super::super::version_lists::game_versions(loader).await {
            Ok(versions) => match versions_arc.lock() {
                Ok(mut s) => {
                    if s.request_epoch != epoch || !matches!(s.versions, LoadState::Loading) {
                        return;
                    }
                    s.versions = LoadState::Loaded(versions);
                    clamp_version_index(&mut s);
                }
                Err(e) => {
                    tracing::error!("Wizard state lock poisoned: {}", e);
                }
            },
            Err(e) => match versions_arc.lock() {
                Ok(mut s) => {
                    if s.request_epoch != epoch || !matches!(s.versions, LoadState::Loading) {
                        return;
                    }
                    s.versions = LoadState::Error(e.to_string());
                }
                Err(lock_error) => {
                    tracing::error!("Wizard state lock poisoned: {}", lock_error);
                }
            },
        }
    });
}

pub(crate) fn ensure_loader_versions_loaded(
    state: &mut WizardState,
    loader: ModLoader,
    game_version: String,
) {
    if !matches!(state.loader_versions, LoadState::Idle) {
        return;
    }

    state.loader_versions = LoadState::Loading;
    state.request_epoch = state.request_epoch.wrapping_add(1);
    let epoch = state.request_epoch;
    let versions_arc = WIZARD_STATE.clone();
    tokio::spawn(async move {
        match super::super::version_lists::loader_versions(loader, &game_version).await {
            Ok(versions) => match versions_arc.lock() {
                Ok(mut s) => {
                    if s.request_epoch != epoch || !matches!(s.loader_versions, LoadState::Loading)
                    {
                        return;
                    }
                    s.loader_versions = LoadState::Loaded(versions);
                    clamp_loader_version_index(&mut s);
                }
                Err(e) => {
                    tracing::error!("Wizard state lock poisoned: {}", e);
                }
            },
            Err(e) => match versions_arc.lock() {
                Ok(mut s) => {
                    if s.request_epoch != epoch || !matches!(s.loader_versions, LoadState::Loading)
                    {
                        return;
                    }
                    s.loader_versions = LoadState::Error(e.to_string());
                }
                Err(lock_error) => {
                    tracing::error!("Wizard state lock poisoned: {}", lock_error);
                }
            },
        }
    });
}

#[cfg(test)]
#[path = "../../../tests/widgets/popups/new_instance/state.rs"]
mod tests;
