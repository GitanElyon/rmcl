// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tui_prompts::{State as PromptState, TextState};

use super::*;
use crate::tests::TEST_LOCK;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn dispatch_repeats_version_search_but_not_snapshot_toggle_or_creation() {
    use crate::tui::{app::FocusedArea, tests::harness::UiHarness};
    use crossterm::event::KeyEventKind;

    let mut ui = UiHarness::new();
    ui.app.focused = FocusedArea::Popup;
    ui.app.instances_state.show_popup = true;
    {
        let mut state = WIZARD_STATE.lock().unwrap();
        state.step = WizardStep::Version;
        state.versions = LoadState::Loaded(vec![GameVersion {
            id: "1.21.1".to_owned(),
            stable: true,
        }]);
        state.version_search.activate();
    }
    assert!(ui.key_event(KeyEvent::new_with_kind(
        KeyCode::Char('s'),
        KeyModifiers::NONE,
        KeyEventKind::Repeat
    )));
    {
        let state = WIZARD_STATE.lock().unwrap();
        assert_eq!(state.version_search.query, "s");
        assert!(!state.show_snapshots);
    }
    assert!(!ui.key_event(KeyEvent::new_with_kind(
        KeyCode::Enter,
        KeyModifiers::NONE,
        KeyEventKind::Repeat
    )));
    {
        let mut state = WIZARD_STATE.lock().unwrap();
        state.step = WizardStep::Confirm;
        state.name_state = TextState::new().with_value("Held Confirm");
        state.version_search.deactivate();
    }
    assert!(!ui.key_event(KeyEvent::new_with_kind(
        KeyCode::Enter,
        KeyModifiers::NONE,
        KeyEventKind::Repeat
    )));
    assert!(take_result().is_none());
    assert!(ui.app.instances_state.show_popup);
    ui.key(KeyCode::Enter);
    assert_eq!(take_result().unwrap().name, "Held Confirm");
    assert!(!ui.key_event(KeyEvent::new_with_kind(
        KeyCode::Enter,
        KeyModifiers::NONE,
        KeyEventKind::Repeat
    )));
    assert_eq!(ui.app.focused, FocusedArea::Instances);
}

#[test]
fn filtered_vanilla_wizard_returns_the_displayed_version() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let mut instances = instances::State {
        show_popup: true,
        ..instances::State::default()
    };

    {
        let mut state = WIZARD_STATE.lock().unwrap();
        *state = WizardState::default();
        state.name_state = TextState::new().with_value("Test Instance");
    }
    *WIZARD_RESULT.lock().unwrap() = None;

    handle_key(&key(KeyCode::Enter), &mut instances);
    assert_eq!(WIZARD_STATE.lock().unwrap().step, WizardStep::Loader);

    {
        let mut state = WIZARD_STATE.lock().unwrap();
        state.step = WizardStep::Version;
        state.versions = LoadState::Loaded(vec![
            GameVersion {
                id: "1.20.1".to_owned(),
                stable: true,
            },
            GameVersion {
                id: "1.21.1".to_owned(),
                stable: true,
            },
        ]);
        state.version_search.query = "1.21.1".to_owned();
    }
    handle_key(&key(KeyCode::Enter), &mut instances);
    assert_eq!(WIZARD_STATE.lock().unwrap().step, WizardStep::Confirm);

    handle_key(&key(KeyCode::Enter), &mut instances);
    let result = take_result().expect("wizard result");
    assert_eq!(result.name, "Test Instance");
    assert_eq!(result.game_version, "1.21.1");
    assert_eq!(result.loader, ModLoader::Vanilla);
    assert_eq!(result.loader_version, None);
    assert!(!instances.show_popup);
}

#[test]
fn version_filter_clamps_a_stale_selection() {
    let mut state = WizardState {
        versions: LoadState::Loaded(vec![
            GameVersion {
                id: "1.21.1".to_owned(),
                stable: true,
            },
            GameVersion {
                id: "25w01a".to_owned(),
                stable: false,
            },
        ]),
        version_idx: 9,
        ..WizardState::default()
    };

    clamp_version_index(&mut state);
    assert_eq!(state.version_idx, 0);
    assert_eq!(visible_versions(&state).count(), 1);

    state.show_snapshots = true;
    assert_eq!(visible_versions(&state).count(), 2);
    state.version_search.query = "missing".to_owned();
    assert!(state.selected_version().is_none());
}

#[test]
fn ctrl_backspace_deletes_the_word_before_the_name_cursor() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let mut instances = instances::State::default();

    {
        let mut state = WIZARD_STATE.lock().unwrap();
        *state = WizardState::default();
        state.name_state = TextState::new().with_value("hello brave world");
        *state.name_state.position_mut() = "hello brave".chars().count();
    }

    handle_key(
        &KeyEvent::new(KeyCode::Backspace, KeyModifiers::CONTROL),
        &mut instances,
    );

    let state = WIZARD_STATE.lock().unwrap();
    assert_eq!(state.name_state.value(), "hello  world");
    assert_eq!(state.name_state.position(), "hello ".chars().count());
}
