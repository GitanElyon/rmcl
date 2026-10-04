// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use std::collections::BTreeMap;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Debug, Default, Clone)]
pub struct ProgressState {
    pub current_action: Option<String>,
    pub progress: Option<(u64, u64)>,
    pub sub_action: Option<String>,
    tasks: BTreeMap<u64, TaskState>,
    legacy: Option<TaskState>,
}

#[derive(Debug, Clone)]
struct TaskState {
    action: String,
    sub_action: Option<String>,
    progress: Option<(u64, u64)>,
}

pub static PROGRESS: LazyLock<Arc<Mutex<ProgressState>>> =
    LazyLock::new(|| Arc::new(Mutex::new(ProgressState::default())));
static NEXT_TASK_ID: AtomicU64 = AtomicU64::new(1);

pub struct ProgressTask {
    id: u64,
}

#[derive(Clone)]
pub struct ProgressTaskHandle {
    id: u64,
}

impl ProgressTask {
    pub fn start(action: impl Into<String>) -> Self {
        let id = NEXT_TASK_ID.fetch_add(1, Ordering::Relaxed);
        let action = action.into();
        if let Ok(mut state) = PROGRESS.lock() {
            state.tasks.insert(
                id,
                TaskState {
                    action: action.clone(),
                    sub_action: None,
                    progress: None,
                },
            );
            refresh_visible(&mut state);
        }
        tracing::info!("{}", action);
        super::request_redraw();
        Self { id }
    }

    pub fn handle(&self) -> ProgressTaskHandle {
        ProgressTaskHandle { id: self.id }
    }

    pub fn set_action(&self, text: impl Into<String>) {
        update_task_action(self.id, text.into());
    }

    pub fn set_sub_action(&self, text: impl Into<String>) {
        update_task_sub_action(self.id, text.into());
    }

    pub fn set_progress(&self, current: u64, total: u64) {
        update_task_progress(self.id, current, total);
    }

    pub fn finish(self) {
        drop(self);
    }

    pub fn fail(self, error: impl std::fmt::Display) {
        tracing::debug!("Progress task failed: {error}");
        drop(self);
    }

    fn remove(&self) {
        if let Ok(mut state) = PROGRESS.lock() {
            state.tasks.remove(&self.id);
            refresh_visible(&mut state);
        }
        super::request_redraw();
    }
}

impl ProgressTaskHandle {
    pub fn set_sub_action(&self, text: impl Into<String>) {
        update_task_sub_action(self.id, text.into());
    }

    pub fn set_progress(&self, current: u64, total: u64) {
        update_task_progress(self.id, current, total);
    }
}

fn update_task_action(id: u64, text: String) {
    if let Ok(mut state) = PROGRESS.lock() {
        if let Some(task) = state.tasks.get_mut(&id) {
            task.action = text;
            task.progress = None;
        }
        refresh_visible(&mut state);
    }
    super::request_redraw();
}

fn update_task_sub_action(id: u64, text: String) {
    if let Ok(mut state) = PROGRESS.lock() {
        if let Some(task) = state.tasks.get_mut(&id) {
            task.sub_action = Some(text);
        }
        refresh_visible(&mut state);
    }
    super::request_redraw();
}

fn update_task_progress(id: u64, current: u64, total: u64) {
    if let Ok(mut state) = PROGRESS.lock() {
        if let Some(task) = state.tasks.get_mut(&id) {
            task.progress = Some((current, total));
        }
        refresh_visible(&mut state);
    }
    super::request_redraw();
}

impl Drop for ProgressTask {
    fn drop(&mut self) {
        self.remove();
    }
}

fn refresh_visible(state: &mut ProgressState) {
    if let Some(task) = state
        .tasks
        .last_key_value()
        .map(|(_, task)| task)
        .or(state.legacy.as_ref())
    {
        state.current_action = Some(task.action.clone());
        state.sub_action.clone_from(&task.sub_action);
        state.progress = task.progress;
    } else {
        state.current_action = None;
        state.sub_action = None;
        state.progress = None;
    }
}

pub fn set_action(text: impl Into<String>) {
    let text = text.into();
    match PROGRESS.lock() {
        Ok(mut state) => {
            let sub_action = state
                .legacy
                .as_ref()
                .and_then(|task| task.sub_action.clone());
            state.legacy = Some(TaskState {
                action: text.clone(),
                sub_action,
                progress: None,
            });
            refresh_visible(&mut state);
            super::request_redraw();
        }
        Err(e) => {
            tracing::error!("Progress lock poisoned: {}", e);
        }
    }
    tracing::info!("{}", text);
}

pub fn set_progress(current: u64, total: u64) {
    match PROGRESS.lock() {
        Ok(mut state) => {
            if let Some(task) = state.legacy.as_mut() {
                task.progress = Some((current, total));
            }
            refresh_visible(&mut state);
            super::request_redraw();
        }
        Err(e) => {
            tracing::error!("Progress lock poisoned: {}", e);
        }
    }
}

pub fn set_sub_action(text: impl Into<String>) {
    let text = text.into();
    match PROGRESS.lock() {
        Ok(mut state) => {
            if let Some(task) = state.legacy.as_mut() {
                task.sub_action = Some(text.clone());
            }
            refresh_visible(&mut state);
            super::request_redraw();
        }
        Err(e) => {
            tracing::error!("Progress lock poisoned: {}", e);
        }
    }
    tracing::debug!("  {}", text);
}

pub fn clear() {
    match PROGRESS.lock() {
        Ok(mut state) => {
            state.legacy = None;
            refresh_visible(&mut state);
            super::request_redraw();
        }
        Err(e) => {
            tracing::error!("Progress lock poisoned: {}", e);
        }
    }
}

pub fn is_active() -> bool {
    PROGRESS
        .lock()
        .is_ok_and(|state| state.current_action.is_some())
}

#[cfg(test)]
#[path = "tests/progress.rs"]
mod tests;
