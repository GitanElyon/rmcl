// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use serde::{Deserialize, Serialize};

use crate::instance::{ContentKind, ContentManifest, InstanceConfig, ModLoader, ProviderProject};
use crate::net::modrinth::VersionInfo;

use super::dependencies::{DependencyPlan, InstallRoot};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvailableUpdate {
    pub installed: ProviderProject,
    #[serde(default)]
    pub current: Option<VersionInfo>,
    pub target: VersionInfo,
    pub kind: ContentKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateSnapshot {
    pub game_version: String,
    pub loader: ModLoader,
    #[serde(default)]
    pub inventory: Vec<ProviderProject>,
    #[serde(default)]
    pub checked_at: i64,
    pub updates: Vec<AvailableUpdate>,
    pub failures: Vec<UpdateCheckFailure>,
}

/// How long a snapshot is trusted before the next reconciliation rechecks it.
/// Anything shorter re-scans every mod against the provider APIs each time the
/// instance is selected.
const RECHECK_AFTER_SECONDS: i64 = 30 * 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateCheckFailure {
    pub installed: ProviderProject,
    pub kind: ContentKind,
    pub reason: String,
}

pub struct PendingUpdateSnapshot {
    pub instance_name: String,
    pub instance_created: chrono::DateTime<chrono::Utc>,
    pub snapshot: UpdateSnapshot,
}

#[derive(Debug, Clone)]
pub struct UpdateRequest {
    pub title: String,
    pub installed_path: std::path::PathBuf,
    pub target_world: Option<std::path::PathBuf>,
    pub update: AvailableUpdate,
}

#[derive(Debug, Clone)]
pub struct PlannedRootUpdate {
    pub title: String,
    pub installed_path: std::path::PathBuf,
    pub current_version: String,
    pub target: VersionInfo,
}

#[derive(Debug, Clone)]
pub struct UpdateConflict {
    pub title: String,
    pub installed_path: std::path::PathBuf,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct BulkUpdatePlan {
    pub dependency_plan: DependencyPlan,
    pub roots: Vec<PlannedRootUpdate>,
    pub conflicts: Vec<UpdateConflict>,
}

pub static PENDING_UPDATE_SNAPSHOTS: LazyLock<Arc<Mutex<Vec<PendingUpdateSnapshot>>>> =
    LazyLock::new(|| Arc::new(Mutex::new(Vec::new())));

struct RunningCheck {
    instance: InstanceConfig,
    inventory: Vec<ProviderProject>,
    task: tokio::task::JoinHandle<()>,
}

impl RunningCheck {
    fn matches(&self, instance: &InstanceConfig, manifest: &ContentManifest) -> bool {
        self.instance.created == instance.created
            && self.instance.game_version == instance.game_version
            && self.instance.loader == instance.loader
            && self.inventory == resolved_inventory(manifest)
    }
}

static RUNNING_CHECKS: LazyLock<Mutex<HashMap<std::path::PathBuf, RunningCheck>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

impl UpdateSnapshot {
    pub fn load(path: &std::path::Path) -> Option<Self> {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    }

    pub fn applies_to(&self, instance: &InstanceConfig) -> bool {
        self.game_version == instance.game_version && self.loader == instance.loader
    }

    pub fn matches_manifest(&self, manifest: &ContentManifest) -> bool {
        self.inventory == resolved_inventory(manifest)
    }

    /// Whether the snapshot is worth re-scanning. Note that a stale snapshot is
    /// still displayed: every entry is keyed by its exact installed version, so
    /// content that changed since the scan simply stops matching instead of
    /// invalidating the labels of everything else.
    pub fn is_stale(&self, manifest: &ContentManifest) -> bool {
        !self.matches_manifest(manifest)
            || chrono::Utc::now()
                .timestamp()
                .saturating_sub(self.checked_at)
                >= RECHECK_AFTER_SECONDS
    }

    pub fn update_for(&self, installed: &ProviderProject) -> Option<&AvailableUpdate> {
        self.updates.iter().find(|update| {
            update.installed.provider == installed.provider
                && update.installed.project_id == installed.project_id
                && update.installed.version_id == installed.version_id
        })
    }
}

pub fn spawn(
    instance: InstanceConfig,
    manifest: ContentManifest,
    path: std::path::PathBuf,
    priority: Vec<ProviderProject>,
    previous: Option<UpdateSnapshot>,
) {
    spawn_with_registry(
        instance,
        manifest,
        path,
        priority,
        previous,
        Arc::new(super::provider::ProviderRegistry::configured(
            crate::net::HttpClient::new(),
        )),
    );
}

pub(crate) fn is_running(
    instance: &InstanceConfig,
    manifest: &ContentManifest,
    path: &std::path::Path,
) -> bool {
    RUNNING_CHECKS.lock().is_ok_and(|checks| {
        checks
            .get(path)
            .is_some_and(|check| !check.task.is_finished() && check.matches(instance, manifest))
    })
}

pub(crate) fn cancel(path: Option<&std::path::Path>) {
    if let Ok(mut checks) = RUNNING_CHECKS.lock() {
        checks.retain(|check_path, check| {
            if path.is_none_or(|path| path == check_path) {
                check.task.abort();
                if let Ok(mut pending) = PENDING_UPDATE_SNAPSHOTS.lock() {
                    pending.retain(|pending| {
                        pending.instance_name != check.instance.name
                            || pending.instance_created != check.instance.created
                    });
                }
                false
            } else {
                true
            }
        });
    }
}

pub(crate) fn spawn_with_registry(
    instance: InstanceConfig,
    manifest: ContentManifest,
    path: std::path::PathBuf,
    priority: Vec<ProviderProject>,
    previous: Option<UpdateSnapshot>,
    registry: Arc<super::provider::ProviderRegistry>,
) -> bool {
    let Ok(mut checks) = RUNNING_CHECKS.lock() else {
        return false;
    };
    if let Some(check) = checks.get(&path) {
        if !check.task.is_finished() && check.matches(&instance, &manifest) {
            return false;
        }
        check.task.abort();
    }
    let check_path = path.clone();
    let check_instance = instance.clone();
    let inventory = resolved_inventory(&manifest);
    let task = tokio::spawn(async move {
        let previous = previous
            .or_else(|| UpdateSnapshot::load(&path))
            .filter(|previous| previous.applies_to(&instance));
        let snapshot = scan_with_registry(
            &instance,
            &manifest,
            &priority,
            registry,
            previous.as_ref(),
            |snapshot| publish_current_snapshot(&instance, snapshot.clone(), &path, false),
        )
        .await;
        publish_current_snapshot(&instance, snapshot, &path, true);
    });
    checks.insert(
        check_path,
        RunningCheck {
            instance: check_instance,
            inventory,
            task,
        },
    );
    true
}

fn publish_current_snapshot(
    instance: &InstanceConfig,
    snapshot: UpdateSnapshot,
    path: &std::path::Path,
    complete: bool,
) {
    let Ok(checks) = RUNNING_CHECKS.lock() else {
        return;
    };
    if checks
        .get(path)
        .is_none_or(|check| check.task.id() != tokio::task::id())
    {
        return;
    }
    // Hold the check registry through persistence and publication so a replaced
    // worker cannot overwrite its successor's snapshot.
    if complete
        && let Ok(bytes) = serde_json::to_vec_pretty(&snapshot)
        && let Err(error) = crate::storage::write_atomic(path, &bytes)
    {
        tracing::debug!("Could not cache content update snapshot: {error}");
    }
    publish_snapshot(instance, snapshot);
}

fn publish_snapshot(instance: &InstanceConfig, snapshot: UpdateSnapshot) {
    if let Ok(mut pending) = PENDING_UPDATE_SNAPSHOTS.lock() {
        pending.retain(|pending| {
            pending.instance_name != instance.name
                || pending.instance_created != instance.created
                || pending.snapshot.game_version != snapshot.game_version
                || pending.snapshot.loader != snapshot.loader
        });
        pending.push(PendingUpdateSnapshot {
            instance_name: instance.name.clone(),
            instance_created: instance.created,
            snapshot,
        });
        crate::feedback::request_redraw();
    }
}

pub async fn scan(instance: &InstanceConfig, manifest: &ContentManifest) -> UpdateSnapshot {
    scan_with_registry(
        instance,
        manifest,
        &[],
        Arc::new(super::provider::ProviderRegistry::configured(
            crate::net::HttpClient::new(),
        )),
        None,
        |_| {},
    )
    .await
}

pub(super) async fn scan_with_registry(
    instance: &InstanceConfig,
    manifest: &ContentManifest,
    priority: &[ProviderProject],
    registry: Arc<super::provider::ProviderRegistry>,
    previous: Option<&UpdateSnapshot>,
    mut publish: impl FnMut(&UpdateSnapshot),
) -> UpdateSnapshot {
    let mut projects = manifest
        .files
        .iter()
        .filter_map(|record| {
            record
                .resolved_project()
                .cloned()
                .map(|project| (project, record.kind))
        })
        .collect::<Vec<_>>();
    projects.sort_by(|left, right| {
        (&left.0.provider, &left.0.project_id, &left.0.version_id).cmp(&(
            &right.0.provider,
            &right.0.project_id,
            &right.0.version_id,
        ))
    });
    projects.dedup_by(|left, right| {
        left.0.provider == right.0.provider
            && left.0.project_id == right.0.project_id
            && left.0.version_id == right.0.version_id
    });
    let inventory = resolved_inventory(manifest);
    let priority = priority
        .iter()
        .enumerate()
        .rev()
        .map(|(index, project)| {
            (
                (&project.provider, &project.project_id, &project.version_id),
                index,
            )
        })
        .collect::<HashMap<_, _>>();
    projects.sort_by_key(|(project, _)| {
        priority
            .get(&(&project.provider, &project.project_id, &project.version_id))
            .copied()
            .unwrap_or(usize::MAX)
    });
    let mut snapshot = UpdateSnapshot {
        game_version: instance.game_version.clone(),
        loader: instance.loader,
        inventory,
        checked_at: 0,
        updates: previous
            .into_iter()
            .flat_map(|previous| previous.updates.iter())
            .filter(|update| {
                projects
                    .iter()
                    .any(|(installed, _)| installed == &update.installed)
            })
            .cloned()
            .collect(),
        failures: Vec::new(),
    };
    if projects.is_empty() {
        snapshot.checked_at = chrono::Utc::now().timestamp();
        return snapshot;
    }
    let total = projects.len() as u64;
    let progress = crate::feedback::progress::ProgressTask::start("Checking content updates");
    progress.set_progress(0, total);
    let mut completed = 0;
    let mut projects = projects.into_iter();
    let mut tasks = tokio::task::JoinSet::new();
    loop {
        while tasks.len() < 8 {
            let Some((installed, kind)) = projects.next() else {
                break;
            };
            let registry = registry.clone();
            let game_version = instance.game_version.clone();
            let loader = instance.loader;
            tasks.spawn(async move {
                let result = async {
                    let provider = registry.get(&installed.provider).ok_or_else(|| {
                        format!("{} content provider is unavailable", installed.provider)
                    })?;
                    let versions = provider
                        .compatible_versions(&installed.project_id, kind, &game_version, loader)
                        .await
                        .map_err(|error| error.to_string())?;
                    let Some(newest) =
                        crate::instance::content::provider::newest_version(&versions)
                    else {
                        return Ok(None);
                    };
                    // Packs can pin versions omitted from the compatible list.
                    let current = match versions
                        .iter()
                        .find(|version| version.id == installed.version_id)
                    {
                        Some(current) => current.clone(),
                        None => provider
                            .version(&installed.version_id)
                            .await
                            .map_err(|error| error.to_string())?,
                    };
                    let target =
                        super::provider::is_newer(newest, &current).then(|| newest.clone());
                    Ok::<_, String>(target.map(|target| (Some(current), target)))
                }
                .await;
                (installed, kind, result)
            });
        }
        let Some(result) = tasks.join_next().await else {
            break;
        };
        if let Ok((installed, _, Ok(_))) = &result {
            snapshot
                .updates
                .retain(|update| &update.installed != installed);
        }
        match result {
            Ok((installed, kind, Ok(Some((current, target))))) => {
                snapshot.updates.push(AvailableUpdate {
                    installed,
                    current,
                    target,
                    kind,
                })
            }
            Ok((_, _, Ok(None))) => {}
            Ok((installed, kind, Err(reason))) => snapshot.failures.push(UpdateCheckFailure {
                installed,
                kind,
                reason,
            }),
            Err(error) => tracing::debug!("Content update task failed: {error}"),
        }
        completed += 1;
        progress.set_sub_action(format!("{completed}/{total} item(s) checked"));
        progress.set_progress(completed, total);
        publish(&snapshot);
    }
    sort_updates(&mut snapshot.updates);
    snapshot.checked_at = chrono::Utc::now().timestamp();
    snapshot
}

fn sort_updates(updates: &mut [AvailableUpdate]) {
    updates.sort_by(|left, right| {
        (&left.installed.provider, &left.installed.project_id)
            .cmp(&(&right.installed.provider, &right.installed.project_id))
    });
}

fn resolved_inventory(manifest: &ContentManifest) -> Vec<ProviderProject> {
    let mut inventory = manifest
        .files
        .iter()
        .filter_map(|record| record.resolved_project().cloned())
        .collect::<Vec<_>>();
    inventory.sort_by(|left, right| {
        (&left.provider, &left.project_id, &left.version_id).cmp(&(
            &right.provider,
            &right.project_id,
            &right.version_id,
        ))
    });
    inventory.dedup();
    inventory
}

pub async fn plan_bulk(
    instance: &InstanceConfig,
    manifest: &ContentManifest,
    minecraft_dir: &std::path::Path,
    requests: Vec<UpdateRequest>,
    conflicts: Vec<UpdateConflict>,
) -> BulkUpdatePlan {
    let registry = crate::instance::content::provider::ProviderRegistry::configured(
        crate::net::HttpClient::new(),
    );
    plan_bulk_with_registry(
        &registry,
        instance,
        manifest,
        minecraft_dir,
        requests,
        conflicts,
    )
    .await
}

pub(super) async fn plan_bulk_with_registry(
    registry: &super::provider::ProviderRegistry,
    instance: &InstanceConfig,
    manifest: &ContentManifest,
    minecraft_dir: &std::path::Path,
    mut requests: Vec<UpdateRequest>,
    mut conflicts: Vec<UpdateConflict>,
) -> BulkUpdatePlan {
    loop {
        let request_count = requests.len();
        let projected_manifest = project_updates(manifest, minecraft_dir, &requests);
        let mut accepted = Vec::new();
        let mut survivors = Vec::new();
        let mut dependency_plan = DependencyPlan {
            items: Vec::new(),
            root_count: 0,
            optional_dependencies: 0,
        };
        for request in requests {
            let root = InstallRoot {
                provider: request.update.installed.provider.clone(),
                project_id: request.update.installed.project_id.clone(),
                title: request.title.clone(),
                version: request.update.target.clone(),
                installed_path: Some(request.installed_path.clone()),
                kind: request.update.kind,
                target_world: request.target_world.clone(),
                force_reinstall: false,
            };
            let mut resolution_manifest = projected_manifest.clone();
            if let Ok(relative_path) = request.installed_path.strip_prefix(minecraft_dir)
                && let Some(current) = manifest.record(relative_path)
                && let Some(projected) = resolution_manifest
                    .files
                    .iter_mut()
                    .find(|record| record.relative_path == relative_path)
            {
                *projected = current.clone();
            }
            let mut plan = match super::dependencies::resolve(
                registry,
                &resolution_manifest,
                minecraft_dir,
                instance,
                root,
            )
            .await
            {
                Ok(plan) => plan,
                Err(error) => {
                    conflicts.push(UpdateConflict {
                        title: request.title,
                        installed_path: request.installed_path,
                        reason: error.to_string(),
                    });
                    continue;
                }
            };
            for item in &mut plan.items {
                item.expected_record = item
                    .installed_path
                    .as_ref()
                    .and_then(|path| path.strip_prefix(minecraft_dir).ok())
                    .and_then(|relative| manifest.record(relative))
                    .cloned();
            }
            let mut proposed = accepted.clone();
            proposed.push(plan.clone());
            match super::dependencies::merge(proposed) {
                Ok(merged) => dependency_plan = merged,
                Err(error) => {
                    conflicts.push(UpdateConflict {
                        title: request.title,
                        installed_path: request.installed_path,
                        reason: error.to_string(),
                    });
                    continue;
                }
            }
            accepted.push(plan);
            survivors.push(request);
        }
        if survivors.len() != request_count {
            requests = survivors;
            continue;
        }
        let roots = survivors
            .into_iter()
            .map(|request| PlannedRootUpdate {
                current_version: request.update.current.as_ref().map_or_else(
                    || request.update.installed.version_id.clone(),
                    |version| version.version_number.clone(),
                ),
                title: request.title,
                installed_path: request.installed_path,
                target: request.update.target,
            })
            .collect();
        return BulkUpdatePlan {
            dependency_plan,
            roots,
            conflicts,
        };
    }
}

fn project_updates(
    manifest: &ContentManifest,
    minecraft_dir: &std::path::Path,
    requests: &[UpdateRequest],
) -> ContentManifest {
    let mut projected = manifest.clone();
    for request in requests {
        let Ok(relative_path) = request.installed_path.strip_prefix(minecraft_dir) else {
            continue;
        };
        let Some(record) = projected
            .files
            .iter_mut()
            .find(|record| record.relative_path == relative_path)
        else {
            continue;
        };
        record.resolution = crate::instance::Resolution::Resolved {
            project: ProviderProject {
                provider: request.update.installed.provider.clone(),
                project_id: request.update.installed.project_id.clone(),
                version_id: request.update.target.id.clone(),
            },
        };
        record.provider_aliases.clear();
    }
    projected
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::modrinth::{VersionFile, VersionType};

    fn version(id: &str) -> VersionInfo {
        VersionInfo {
            id: id.to_owned(),
            project_id: "project".to_owned(),
            name: id.to_owned(),
            version_number: id.to_owned(),
            game_versions: vec!["1.21.1".to_owned()],
            loaders: vec!["fabric".to_owned()],
            version_type: VersionType::Release,
            dependencies: Vec::new(),
            date_published: String::new(),
            files: Vec::<VersionFile>::new(),
        }
    }

    #[test]
    fn cached_updates_match_the_exact_installed_version() {
        let installed = ProviderProject {
            provider: "modrinth".to_owned(),
            project_id: "project".to_owned(),
            version_id: "old".to_owned(),
        };
        let snapshot = UpdateSnapshot {
            game_version: "1.21.1".to_owned(),
            loader: ModLoader::Fabric,
            inventory: vec![installed.clone()],
            checked_at: chrono::Utc::now().timestamp(),
            updates: vec![AvailableUpdate {
                installed: installed.clone(),
                current: Some(version("old")),
                target: version("new"),
                kind: ContentKind::Mod,
            }],
            failures: Vec::new(),
        };

        assert!(snapshot.update_for(&installed).is_some());
        assert!(
            snapshot
                .update_for(&ProviderProject {
                    version_id: "different".to_owned(),
                    ..installed
                })
                .is_none()
        );
    }

    #[test]
    fn a_changed_content_inventory_only_schedules_a_rescan() {
        let project = ProviderProject {
            provider: "modrinth".to_owned(),
            project_id: "project".to_owned(),
            version_id: "old".to_owned(),
        };
        let snapshot = UpdateSnapshot {
            game_version: "1.21.1".to_owned(),
            loader: ModLoader::Fabric,
            inventory: vec![project.clone()],
            checked_at: chrono::Utc::now().timestamp(),
            updates: Vec::new(),
            failures: Vec::new(),
        };
        let mut manifest = ContentManifest::default();
        manifest.files.push(crate::instance::ContentFileRecord {
            relative_path: "mods/example.jar".into(),
            kind: ContentKind::Mod,
            enabled: true,
            fingerprint: crate::instance::FileFingerprint {
                size: 0,
                modified_ns: 0,
                hashes: Default::default(),
            },
            resolution: crate::instance::Resolution::Resolved { project },
            provider_aliases: Vec::new(),
            provider_checks: vec!["modrinth".to_owned()],
            required_dependencies: Vec::new(),
            automatic_dependency: false,
            cleanup_eligible: false,
        });

        assert!(!snapshot.is_stale(&manifest));
        manifest.files[0].resolution = crate::instance::Resolution::Unmatched {
            checked_at: 0,
            providers: Vec::new(),
        };
        assert!(snapshot.is_stale(&manifest));

        let expired = UpdateSnapshot {
            checked_at: chrono::Utc::now().timestamp() - RECHECK_AFTER_SECONDS,
            ..snapshot
        };
        assert!(expired.is_stale(&manifest));
    }

    #[test]
    fn bulk_planning_projects_other_selected_updates() {
        let minecraft = std::path::Path::new("/instance/minecraft");
        let installed = ProviderProject {
            provider: "modrinth".to_owned(),
            project_id: "library".to_owned(),
            version_id: "old".to_owned(),
        };
        let mut manifest = ContentManifest::default();
        manifest.files.push(crate::instance::ContentFileRecord {
            relative_path: "mods/library.jar".into(),
            kind: ContentKind::Mod,
            enabled: true,
            fingerprint: crate::instance::FileFingerprint {
                size: 1,
                modified_ns: 1,
                hashes: Default::default(),
            },
            resolution: crate::instance::Resolution::Resolved {
                project: installed.clone(),
            },
            provider_aliases: Vec::new(),
            provider_checks: vec!["modrinth".to_owned()],
            required_dependencies: Vec::new(),
            automatic_dependency: true,
            cleanup_eligible: true,
        });
        let requests = vec![UpdateRequest {
            title: "Library".to_owned(),
            installed_path: minecraft.join("mods/library.jar"),
            target_world: None,
            update: AvailableUpdate {
                installed,
                current: Some(version("old")),
                target: version("new"),
                kind: ContentKind::Mod,
            },
        }];

        let projected = project_updates(&manifest, minecraft, &requests);

        assert_eq!(
            projected.files[0].resolved_project().unwrap().version_id,
            "new"
        );
        assert_eq!(
            manifest.files[0].resolved_project().unwrap().version_id,
            "old"
        );
    }
}
