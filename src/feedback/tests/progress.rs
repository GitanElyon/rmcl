// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use super::*;
use crate::tests::TEST_LOCK;

#[test]
fn newest_task_is_visible_and_drop_restores_previous_task() {
    let _guard = TEST_LOCK.lock().unwrap();
    clear();
    let first = ProgressTask::start("first");
    first.set_progress(1, 2);
    {
        let second = ProgressTask::start("second");
        second.set_sub_action("working");
        let state = PROGRESS.lock().unwrap();
        assert_eq!(state.current_action.as_deref(), Some("second"));
        assert_eq!(state.sub_action.as_deref(), Some("working"));
    }
    let state = PROGRESS.lock().unwrap();
    assert_eq!(state.current_action.as_deref(), Some("first"));
    assert_eq!(state.progress, Some((1, 2)));
    drop(state);
    first.finish();
    clear();
}

#[test]
fn changing_phase_resets_progress_to_indeterminate() {
    let _guard = TEST_LOCK.lock().unwrap();
    clear();
    let task = ProgressTask::start("inventory");
    task.set_progress(4, 10);
    task.set_action("provider matching");

    let state = PROGRESS.lock().unwrap();
    assert_eq!(state.current_action.as_deref(), Some("provider matching"));
    assert_eq!(state.progress, None);
    drop(state);

    task.finish();
    assert!(!is_active());
    clear();
}

#[test]
fn clearing_legacy_progress_keeps_owned_tasks() {
    let _guard = TEST_LOCK.lock().unwrap();
    clear();
    let task = ProgressTask::start("indexing");
    set_action("legacy download");
    clear();

    let state = PROGRESS.lock().unwrap();
    assert_eq!(state.current_action.as_deref(), Some("indexing"));
    drop(state);

    task.finish();
    clear();
}

#[test]
fn owned_tasks_restore_legacy_progress_and_legacy_updates_do_not_cover_them() {
    let _guard = TEST_LOCK.lock().unwrap();
    clear();
    set_action("legacy");
    set_sub_action("download");
    set_progress(1, 3);
    let task = ProgressTask::start("owned");
    set_progress(2, 3);
    assert_eq!(
        PROGRESS.lock().unwrap().current_action.as_deref(),
        Some("owned")
    );
    task.finish();
    let state = PROGRESS.lock().unwrap();
    assert_eq!(state.current_action.as_deref(), Some("legacy"));
    assert_eq!(state.sub_action.as_deref(), Some("download"));
    assert_eq!(state.progress, Some((2, 3)));
    drop(state);
    clear();
}

#[test]
fn a_new_legacy_action_does_not_reuse_the_previous_phase_progress() {
    let _guard = TEST_LOCK.lock().unwrap();
    clear();
    set_action("Downloading installer");
    set_progress(1, 1);

    set_action("Running installer");

    let state = PROGRESS.lock().unwrap();
    assert_eq!(state.current_action.as_deref(), Some("Running installer"));
    assert_eq!(state.progress, None);
    drop(state);
    clear();
}
