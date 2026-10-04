// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};

use super::*;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn offline_account_requires_a_microsoft_account_and_can_be_dismissed() {
    let temp = tempfile::tempdir().unwrap();
    let mut state = AccountState {
        store: AccountStore::empty_for_test(temp.path().join("accounts.json")),
        list_state: Default::default(),
        add_mode: AddMode::None,
    };

    assert!(handle_key(&key(KeyCode::Char('a')), &mut state));
    assert!(matches!(state.add_mode, AddMode::ChooseType));

    assert!(handle_key(&key(KeyCode::Char('o')), &mut state));
    assert!(matches!(state.add_mode, AddMode::OfflineBlocked));

    assert!(handle_key(&key(KeyCode::Esc), &mut state));
    assert!(matches!(state.add_mode, AddMode::None));
}

#[test]
fn a_cancelled_login_cannot_publish_the_next_attempts_device_code() {
    let temp = tempfile::tempdir().unwrap();
    let first = auth::MicrosoftAuth::default();
    let stale_display = first.device_code.clone();
    let mut state = AccountState {
        store: AccountStore::empty_for_test(temp.path().join("accounts.json")),
        list_state: Default::default(),
        add_mode: AddMode::DeviceCodeWaiting {
            info: DeviceCodeInfo {
                user_code: String::new(),
                verification_uri: String::new(),
            },
            pending: first,
        },
    };
    handle_key(&key(KeyCode::Esc), &mut state);
    let second = auth::MicrosoftAuth::default();
    *second.device_code.lock().unwrap() = Some(DeviceCodeInfo {
        user_code: "current".to_owned(),
        verification_uri: "current-uri".to_owned(),
    });
    state.add_mode = AddMode::DeviceCodeWaiting {
        info: DeviceCodeInfo {
            user_code: String::new(),
            verification_uri: String::new(),
        },
        pending: second,
    };
    *stale_display.lock().unwrap() = Some(DeviceCodeInfo {
        user_code: "stale".to_owned(),
        verification_uri: "stale-uri".to_owned(),
    });
    drain_device_code(&mut state);
    let AddMode::DeviceCodeWaiting { info, .. } = &state.add_mode else {
        panic!("login must still be pending")
    };
    assert_eq!(info.user_code, "current");
}

#[test]
fn account_type_popup_uses_the_theme_surface() {
    let temp = tempfile::tempdir().unwrap();
    let mut state = AccountState {
        store: AccountStore::empty_for_test(temp.path().join("accounts.json")),
        list_state: Default::default(),
        add_mode: AddMode::ChooseType,
    };
    let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();

    terminal
        .draw(|frame| render(frame, frame.area(), FocusedArea::Account, &mut state))
        .unwrap();

    assert_eq!(
        terminal.backend().buffer().cell((11, 3)).unwrap().bg,
        THEME.as_ref().surface()
    );
}
