// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use crate::instance::content::provider::{ContentProvider, FingerprintQuery, ProviderRegistry};
use crate::instance::{
    ContentFileRecord, ContentKind, ContentManifest, InstanceConfig, ProviderProject,
};
use crate::net::NetError;
use crate::net::modrinth::{
    DependencyType, ProjectInfo, VersionDependency, VersionInfo, VersionType,
};

use super::local::{
    FileTransaction, StagingDirectory, relative_path, require_absent, validate_record,
};
use super::manifest::ContentLock;

#[derive(Debug, Clone)]
pub struct InstallRoot {
    pub provider: String,
    pub project_id: String,
    pub title: String,
    pub version: VersionInfo,
    pub installed_path: Option<PathBuf>,
    pub kind: ContentKind,
    pub target_world: Option<PathBuf>,
    pub force_reinstall: bool,
}

#[derive(Debug, Clone)]
pub struct PlannedInstall {
    pub provider: String,
    pub project_id: String,
    pub title: String,
    pub version: VersionInfo,
    pub installed_path: Option<PathBuf>,
    pub kind: ContentKind,
    pub destination: PathBuf,
    pub expected_record: Option<ContentFileRecord>,
    pub provider_aliases: Vec<ProviderProject>,
    pub required_dependencies: Vec<ProviderProject>,
    pub automatic_dependency: bool,
    pub cleanup_eligible: bool,
    pub replacement: bool,
}

impl PlannedInstall {
    pub fn needs_download(&self) -> bool {
        self.installed_path.is_none() || self.replacement
    }

    pub fn identity(&self) -> ProviderProject {
        ProviderProject {
            provider: self.provider.clone(),
            project_id: self.project_id.clone(),
            version_id: self.version.id.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DependencyPlan {
    pub items: Vec<PlannedInstall>,
    pub root_count: usize,
    pub optional_dependencies: usize,
}

pub struct InstallResult {
    pub root_path: PathBuf,
    pub replaced: bool,
    pub skipped: bool,
    pub orphaned_dependencies: Vec<PathBuf>,
}

impl DependencyPlan {
    pub fn dependency_installs(&self) -> impl Iterator<Item = &PlannedInstall> {
        self.items
            .iter()
            .skip(self.root_count)
            .filter(|item| item.installed_path.is_none())
    }

    pub fn dependency_replacements(&self) -> impl Iterator<Item = &PlannedInstall> {
        self.items
            .iter()
            .skip(self.root_count)
            .filter(|item| item.replacement)
    }

    pub fn has_dependency_changes(&self) -> bool {
        self.dependency_installs().next().is_some()
            || self.dependency_replacements().next().is_some()
            || self.optional_dependencies > 0
    }
}

pub async fn install(
    registry: &ProviderRegistry,
    manifest_path: &Path,
    minecraft_dir: &Path,
    plan: &DependencyPlan,
) -> Result<InstallResult, NetError> {
    plan.items
        .first()
        .ok_or_else(|| NetError::Parse("Dependency plan is empty".to_owned()))?;
    let staging = StagingDirectory::new(minecraft_dir)?;
    let recovery_path = staging.path().to_owned();
    let staged = stage_downloads(registry, staging.path(), plan).await?;
    let manifest_path = manifest_path.to_owned();
    let minecraft_dir = minecraft_dir.to_owned();
    let plan = plan.clone();
    // The blocking job owns both the staging files and the lifetime lock, even
    // if the caller stops awaiting it after the commit has started.
    tokio::task::spawn_blocking(move || {
        commit_install(&manifest_path, &minecraft_dir, staging, &staged, &plan)
    })
    .await
    .map_err(|error| {
        NetError::TaskFailed(format!(
            "Content commit task failed: {error}; recovery location '{}'",
            recovery_path.display()
        ))
    })?
}

struct StagedFile {
    item: usize,
    source: PathBuf,
    target: PathBuf,
}

async fn stage_downloads(
    registry: &ProviderRegistry,
    staging: &Path,
    plan: &DependencyPlan,
) -> Result<Vec<StagedFile>, NetError> {
    let mut staged = Vec::new();
    let mut targets = HashSet::new();
    for (index, item) in plan.items.iter().enumerate() {
        if !item.needs_download() {
            continue;
        }
        let provider = registry.get(&item.provider).ok_or_else(|| {
            NetError::Parse(format!("{} content provider is unavailable", item.provider))
        })?;
        let item_staging = staging.join(format!("item-{index}"));
        tokio::fs::create_dir(&item_staging).await?;
        let outcome = provider
            .download_version(&item.version, &item_staging, None)
            .await?;
        let source = match outcome {
            crate::net::modrinth::DownloadOutcome::Downloaded(path)
            | crate::net::modrinth::DownloadOutcome::SkippedExisting(path) => path,
        };
        let file_name = source.file_name().ok_or_else(|| {
            NetError::Parse(format!(
                "Downloaded dependency '{}' has no filename",
                item.title
            ))
        })?;
        let mut file_name = file_name.to_os_string();
        if item.installed_path.as_ref().is_some_and(|path| {
            path.extension()
                .is_some_and(|extension| extension == "disabled")
        }) {
            file_name.push(".disabled");
        }
        let target = item.destination.join(file_name);
        if !targets.insert(target.clone()) {
            return Err(NetError::Parse(format!(
                "Multiple selected projects install '{}'",
                target.display()
            )));
        }
        staged.push(StagedFile {
            item: index,
            source,
            target,
        });
    }
    Ok(staged)
}

fn commit_install(
    manifest_path: &Path,
    minecraft_dir: &Path,
    staging: StagingDirectory,
    staged: &[StagedFile],
    plan: &DependencyPlan,
) -> Result<InstallResult, NetError> {
    let lock =
        ContentLock::acquire(manifest_path).map_err(|error| NetError::Parse(error.to_string()))?;
    let mut manifest = lock
        .load()
        .map_err(|error| NetError::Parse(error.to_string()))?;
    let previous = manifest.clone();
    validate_install(plan, &manifest, minecraft_dir, staged, staging.path())?;
    let root = &plan.items[0];
    let root_path = if root.needs_download() {
        staged
            .iter()
            .find(|file| file.item == 0)
            .map(|file| file.target.clone())
    } else {
        root.installed_path.clone()
    }
    .ok_or_else(|| NetError::Parse("Installed root path is missing".to_owned()))?;

    let mut transaction = FileTransaction::new(staging);
    let result = (|| {
        for (index, file) in staged.iter().enumerate() {
            if let Some(old_path) = &plan.items[file.item].installed_path {
                let backup = transaction.staging().join(format!("backup-{index}"));
                transaction.move_path(old_path, &backup)?;
            }
            transaction.move_path(&file.source, &file.target)?;
        }
        let records = build_records(plan, &manifest, minecraft_dir, staged)?;
        for (old_relative, record) in &records {
            if let Some(old_relative) = old_relative
                && old_relative != &record.relative_path
            {
                manifest.remove(old_relative);
            }
            manifest.upsert(record.clone());
        }
        let orphaned_dependencies = manifest
            .orphaned_dependencies()
            .into_iter()
            .map(|relative| minecraft_dir.join(relative))
            .collect();
        lock.save_if_unchanged(&manifest, &previous)
            .map_err(|error| NetError::Parse(error.to_string()))?;
        Ok(InstallResult {
            root_path,
            replaced: root.replacement,
            skipped: !plan.items.iter().any(PlannedInstall::needs_download),
            orphaned_dependencies,
        })
    })();
    transaction.finish(result)
}

fn validate_install(
    plan: &DependencyPlan,
    manifest: &ContentManifest,
    minecraft_dir: &Path,
    staged: &[StagedFile],
    staging: &Path,
) -> Result<(), NetError> {
    reject_final_incompatibilities(&plan.items, manifest, minecraft_dir)?;
    let mut old_paths = HashMap::new();
    for (index, item) in plan.items.iter().enumerate() {
        relative_path(minecraft_dir, &item.destination.join(".rmcl-path-check"))?;
        if let Some(path) = &item.installed_path {
            let relative = relative_path(minecraft_dir, path)?;
            let expected = item.expected_record.as_ref().ok_or_else(|| {
                NetError::Parse(format!(
                    "Installation of '{}' has no expected ownership record",
                    item.title
                ))
            })?;
            if expected.relative_path != relative || manifest.record(&relative) != Some(expected) {
                return Err(NetError::Parse(format!(
                    "Ownership of '{}' changed; refresh before retrying",
                    path.display()
                )));
            }
            validate_record(minecraft_dir, expected)?;
            if old_paths.insert(path.canonicalize()?, index).is_some() {
                return Err(NetError::Parse(format!(
                    "Multiple selected projects replace '{}'",
                    path.display()
                )));
            }
        } else if item.expected_record.is_some()
            || manifest.files.iter().any(|record| {
                record.enabled
                    && record.kind == item.kind
                    && record.matches_project(&item.provider, &item.project_id)
                    && minecraft_dir.join(&record.relative_path).parent()
                        == Some(item.destination.as_path())
            })
        {
            return Err(NetError::Parse(format!(
                "Installed project '{}' changed; refresh before retrying",
                item.title
            )));
        }
    }
    for item in &plan.items {
        std::fs::create_dir_all(&item.destination)?;
    }
    let staging = staging.canonicalize()?;
    let mut targets = HashSet::new();
    for file in staged {
        let target_relative = relative_path(minecraft_dir, &file.target)?;
        if let Some(owner) = manifest.record(&target_relative)
            && Some(owner) != plan.items[file.item].expected_record.as_ref()
        {
            return Err(NetError::Parse(format!(
                "Content manifest already owns '{}'",
                file.target.display()
            )));
        }
        let metadata = std::fs::symlink_metadata(&file.source)?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || !file.source.canonicalize()?.starts_with(&staging)
        {
            return Err(NetError::Parse(format!(
                "Invalid staged content '{}'",
                file.source.display()
            )));
        }
        let physical_target = match std::fs::symlink_metadata(&file.target) {
            Ok(metadata) if !metadata.file_type().is_symlink() => {
                let target = file.target.canonicalize()?;
                if old_paths.get(&target) != Some(&file.item) {
                    require_absent(&file.target)?;
                }
                target
            }
            Ok(_) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    format!("'{}' already exists", file.target.display()),
                )
                .into());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                file.target
                    .parent()
                    .ok_or_else(|| NetError::Parse("Content target has no parent".to_owned()))?
                    .canonicalize()?
                    .join(file.target.file_name().ok_or_else(|| {
                        NetError::Parse("Content target has no filename".to_owned())
                    })?)
            }
            Err(error) => return Err(error.into()),
        };
        if !targets.insert(physical_target) {
            return Err(NetError::Parse(format!(
                "Multiple selected projects install '{}'",
                file.target.display()
            )));
        }
    }
    Ok(())
}

fn reject_final_incompatibilities(
    items: &[PlannedInstall],
    manifest: &ContentManifest,
    minecraft_dir: &Path,
) -> Result<(), NetError> {
    for item in items {
        let world = if item.kind == ContentKind::DataPack {
            item.destination.parent()
        } else {
            items
                .first()
                .filter(|root| root.kind == ContentKind::DataPack)
                .and_then(|root| root.destination.parent())
        };
        for dependency in item
            .version
            .dependencies
            .iter()
            .filter(|dependency| dependency.dependency_type == DependencyType::Incompatible)
        {
            let matches = |project: &ProviderProject| {
                project.provider == item.provider
                    && dependency
                        .project_id
                        .as_ref()
                        .is_none_or(|id| project.project_id == *id)
                    && dependency
                        .version_id
                        .as_ref()
                        .is_none_or(|id| project.version_id == *id)
            };
            let selected = items.iter().any(|selected| {
                (selected.kind != ContentKind::DataPack || selected.destination.parent() == world)
                    && (matches(&selected.identity())
                        || selected.provider_aliases.iter().any(&matches))
            });
            let installed = manifest
                .files
                .iter()
                .filter(|record| record.enabled)
                .any(|record| {
                    let replaced = items.iter().any(|replacement| {
                        replacement.needs_download()
                            && replacement.installed_path.as_deref()
                                == Some(minecraft_dir.join(&record.relative_path).as_path())
                    });
                    !replaced
                        && (record.kind != ContentKind::DataPack
                            || record_in_target(record, record.kind, world))
                        && record
                            .resolved_project()
                            .into_iter()
                            .chain(record.provider_aliases.iter())
                            .any(&matches)
                });
            if selected || installed {
                return Err(NetError::Parse(format!(
                    "'{}' is incompatible with project '{}'",
                    item.title,
                    dependency
                        .project_id
                        .as_deref()
                        .or(dependency.version_id.as_deref())
                        .unwrap_or("unknown")
                )));
            }
        }
    }
    Ok(())
}

fn build_records(
    plan: &DependencyPlan,
    previous: &ContentManifest,
    minecraft_dir: &Path,
    staged: &[StagedFile],
) -> Result<Vec<(Option<PathBuf>, ContentFileRecord)>, NetError> {
    plan.items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let old_relative = item
                .installed_path
                .as_ref()
                .and_then(|path| path.strip_prefix(minecraft_dir).ok())
                .map(Path::to_owned);
            if !item.needs_download() {
                let old_relative = old_relative.ok_or_else(|| {
                    NetError::Parse(format!("Installed path for '{}' is invalid", item.title))
                })?;
                let mut record = previous.record(&old_relative).cloned().ok_or_else(|| {
                    NetError::Parse(format!("Manifest record for '{}' is missing", item.title))
                })?;
                let identity = item.identity();
                if !record.matches_project(&identity.provider, &identity.project_id)
                    && !record.provider_aliases.contains(&identity)
                {
                    record.provider_aliases.push(identity);
                }
                record.required_dependencies = item.required_dependencies.clone();
                record.automatic_dependency = item.automatic_dependency;
                record.cleanup_eligible = item.cleanup_eligible;
                return Ok((Some(old_relative), record));
            }
            let path = staged
                .iter()
                .find(|file| file.item == index)
                .map(|file| file.target.clone())
                .ok_or_else(|| {
                    NetError::Parse(format!("Staged file for '{}' is missing", item.title))
                })?;
            let relative_path = path
                .strip_prefix(minecraft_dir)
                .map_err(|error| NetError::Parse(error.to_string()))?
                .to_owned();
            Ok((
                old_relative,
                ContentFileRecord {
                    relative_path,
                    kind: item.kind,
                    enabled: !path
                        .extension()
                        .is_some_and(|extension| extension == "disabled"),
                    fingerprint: crate::instance::content::manifest::fingerprint(&path)?,
                    resolution: crate::instance::Resolution::Resolved {
                        project: item.identity(),
                    },
                    provider_aliases: item.provider_aliases.clone(),
                    provider_checks: vec![item.provider.clone()],
                    required_dependencies: item.required_dependencies.clone(),
                    automatic_dependency: item.automatic_dependency,
                    cleanup_eligible: item.cleanup_eligible,
                },
            ))
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct ProjectKey {
    provider: String,
    project_id: String,
}

impl ProjectKey {
    fn new(provider: &str, project_id: impl Into<String>) -> Self {
        Self {
            provider: provider.to_owned(),
            project_id: project_id.into(),
        }
    }
}

#[derive(Debug, Clone)]
struct Node {
    title: String,
    kind: ContentKind,
    version: VersionInfo,
    installed: Option<InstalledMatch>,
    automatic_dependency: bool,
    cleanup_eligible: bool,
    dependencies: Vec<(ProjectKey, VersionDependency)>,
    optional_dependencies: usize,
    incompatible: Vec<Incompatible>,
}

#[derive(Debug, Clone)]
struct InstalledMatch {
    relative_path: PathBuf,
    identity: ProviderProject,
    aliases: Vec<ProviderProject>,
    automatic_dependency: bool,
    record: ContentFileRecord,
}

#[derive(Debug, Clone)]
struct Incompatible {
    project_id: String,
    version_id: Option<String>,
}

#[derive(Debug, Clone)]
struct Requirement {
    parent: ProjectKey,
    parent_version_id: String,
    target: ProjectKey,
    dependency: VersionDependency,
}

pub async fn resolve(
    registry: &ProviderRegistry,
    manifest: &ContentManifest,
    minecraft_dir: &Path,
    instance: &InstanceConfig,
    root: InstallRoot,
) -> Result<DependencyPlan, NetError> {
    if root.kind == ContentKind::DataPack && root.target_world.is_none() {
        return Err(NetError::Parse(
            "A target world is required for datapack installation".to_owned(),
        ));
    }
    let provider = registry.get(&root.provider).ok_or_else(|| {
        NetError::Parse(format!("{} content provider is unavailable", root.provider))
    })?;
    let root_version = provider.version(&root.version.id).await?;
    validate_compatible(&root_version, instance, root.kind)?;
    if !root_version.project_id.is_empty() && root_version.project_id != root.project_id {
        return Err(NetError::Parse(format!(
            "Selected version belongs to project '{}', not '{}'",
            root_version.project_id, root.project_id
        )));
    }
    let remote_matches = resolve_installed_files(provider, manifest).await?;
    let root_key = ProjectKey::new(&root.provider, root.project_id.clone());
    let root_installed = installed_root(manifest, minecraft_dir, &root, &remote_matches);
    if root.installed_path.is_some() && root_installed.is_none() {
        return Err(NetError::Parse(
            "Installed root ownership is missing; refresh before retrying".to_owned(),
        ));
    }
    let mut nodes = HashMap::from([(
        root_key.clone(),
        Node {
            title: root.title,
            kind: root.kind,
            version: root_version.clone(),
            automatic_dependency: false,
            cleanup_eligible: false,
            installed: root_installed,
            dependencies: Vec::new(),
            optional_dependencies: 0,
            incompatible: Vec::new(),
        },
    )]);
    let mut queue = VecDeque::new();
    expand_node(provider, &root_key, &root_version, &mut nodes, &mut queue).await?;

    let mut seen = HashSet::new();
    let mut errors = Vec::new();
    let mut check_progress = false;
    loop {
        if check_progress {
            let order = reachable_order(&root_key, &nodes);
            let mut selections = order
                .iter()
                .map(|key| {
                    (
                        key.clone(),
                        nodes.get(key).map(|node| node.version.id.clone()),
                    )
                })
                .collect::<Vec<_>>();
            selections.sort_unstable();
            let pending = queue
                .iter()
                .map(|requirement| {
                    (
                        requirement.parent.clone(),
                        requirement.parent_version_id.clone(),
                        requirement.target.clone(),
                        requirement.dependency.version_id.clone(),
                    )
                })
                .collect::<Vec<_>>();
            if !seen.insert((selections, pending)) {
                return Err(NetError::Parse("Dependency selection did not converge: a dependency cycle or conflicting version requirements keep changing selected versions".to_owned()));
            }
        }
        check_progress = false;

        for _ in 0..queue.len() {
            let Some(requirement) = queue.pop_front() else {
                break;
            };
            if !reachable_order(&root_key, &nodes).contains(&requirement.parent)
                || nodes
                    .get(&requirement.parent)
                    .is_none_or(|parent| parent.version.id != requirement.parent_version_id)
            {
                continue;
            }
            if let Some(existing) = nodes.get(&requirement.target)
                && (requirement.target == root_key
                    || requirement
                        .dependency
                        .version_id
                        .as_ref()
                        .is_none_or(|version| version == &existing.version.id))
            {
                continue;
            }
            let parent_kind = nodes[&requirement.parent].kind;
            let choice = match resolve_requirement(
                provider,
                manifest,
                &remote_matches,
                instance,
                parent_kind,
                root.target_world.as_deref(),
                &requirement.dependency,
            )
            .await
            {
                Ok(choice) => choice,
                Err(error) => {
                    errors.push((requirement.parent, requirement.parent_version_id, error));
                    continue;
                }
            };
            if choice.kind == ContentKind::DataPack && root.target_world.is_none() {
                errors.push((
                    requirement.parent,
                    requirement.parent_version_id,
                    NetError::Parse(format!(
                        "Datapack dependency '{}' requires a target world",
                        choice.title
                    )),
                ));
                continue;
            }
            let key = requirement.target;
            let replace_selection = match nodes.get(&key) {
                None => true,
                Some(existing) => {
                    key != root_key
                        && requirement.dependency.version_id.is_some()
                        && existing.version.id != choice.version.id
                        && active_exact_requirements(&root_key, &nodes)
                            .get(&key)
                            .is_some_and(|requirements| {
                                requirements.iter().all(|requirement| {
                                    requirement.dependency.version_id.as_deref()
                                        == Some(choice.version.id.as_str())
                                })
                            })
                }
            };
            if !replace_selection {
                continue;
            }

            check_progress |= nodes.contains_key(&key);
            nodes.insert(
                key.clone(),
                Node {
                    title: choice.title,
                    kind: choice.kind,
                    version: choice.version.clone(),
                    installed: choice.installed,
                    automatic_dependency: choice.automatic_dependency,
                    cleanup_eligible: choice.cleanup_eligible,
                    dependencies: Vec::new(),
                    optional_dependencies: 0,
                    incompatible: Vec::new(),
                },
            );
            if let Err(error) =
                expand_node(provider, &key, &choice.version, &mut nodes, &mut queue).await
            {
                errors.push((key, choice.version.id, error));
            }
        }
        if !queue.is_empty() {
            continue;
        }

        let order = reachable_order(&root_key, &nodes);
        errors.retain(|(key, version, _)| {
            order.contains(key)
                && nodes
                    .get(key)
                    .is_some_and(|node| node.version.id == *version)
        });
        let (mut retry, conflict) = retry_requirements(&root_key, &nodes);
        retry.retain(|requirement| {
            !errors.iter().any(|(key, version, _)| {
                key == &requirement.parent && version == &requirement.parent_version_id
            })
        });
        if !retry.is_empty() {
            queue.extend(retry);
            check_progress = true;
            continue;
        }
        if let Some((_, _, error)) = errors.into_iter().next() {
            return Err(error);
        }
        if let Some(conflict) = conflict {
            return Err(conflict);
        }
        break;
    }

    let order = reachable_order(&root_key, &nodes);
    reject_dependency_cycles(&order, &nodes)?;
    reject_installed_incompatibilities(
        provider,
        manifest,
        &remote_matches,
        &order,
        &nodes,
        root.target_world.as_deref(),
    )?;
    let optional_dependencies = order
        .iter()
        .filter_map(|key| nodes.get(key))
        .map(|node| node.optional_dependencies)
        .sum();
    let items = order
        .iter()
        .filter_map(|key| {
            nodes.get(key).map(|node| {
                planned_install(
                    key,
                    node,
                    &nodes,
                    minecraft_dir,
                    root.target_world.as_deref(),
                    root.force_reinstall && key == &root_key,
                )
            })
        })
        .collect();
    Ok(DependencyPlan {
        items,
        root_count: 1,
        optional_dependencies,
    })
}

pub fn merge(plans: Vec<DependencyPlan>) -> Result<DependencyPlan, NetError> {
    let optional_dependencies = plans.iter().map(|plan| plan.optional_dependencies).sum();
    let mut merged: Vec<(PlannedInstall, bool)> = Vec::new();
    for plan in plans {
        for (index, item) in plan.items.into_iter().enumerate() {
            let root = index < plan.root_count;
            if let Some((existing, existing_root)) = merged.iter_mut().find(|(existing, _)| {
                existing.provider == item.provider
                    && existing.project_id == item.project_id
                    && existing.destination == item.destination
            }) {
                if existing.version.id != item.version.id {
                    return Err(NetError::Parse(format!(
                        "Conflicting selected versions for '{}': '{}' and '{}'",
                        item.title, existing.version.version_number, item.version.version_number
                    )));
                }
                if existing.installed_path.is_some()
                    && item.installed_path.is_some()
                    && (existing.installed_path != item.installed_path
                        || existing.expected_record != item.expected_record)
                {
                    return Err(NetError::Parse(format!(
                        "Conflicting installed ownership for '{}'",
                        item.title
                    )));
                }
                for dependency in item.required_dependencies {
                    if !existing.required_dependencies.contains(&dependency) {
                        existing.required_dependencies.push(dependency);
                    }
                }
                if existing.replacement || item.replacement {
                    existing.provider_aliases.clear();
                } else {
                    for alias in item.provider_aliases {
                        if !existing.provider_aliases.contains(&alias) {
                            existing.provider_aliases.push(alias);
                        }
                    }
                }
                existing.replacement |= item.replacement;
                if root && !*existing_root {
                    existing.title = item.title;
                    if item.installed_path.is_some() {
                        existing.installed_path = item.installed_path;
                        existing.expected_record = item.expected_record;
                    }
                    existing.automatic_dependency = false;
                    existing.cleanup_eligible = false;
                    *existing_root = true;
                }
                continue;
            }
            merged.push((item, root));
        }
    }
    merged.sort_by_key(|(_, root)| !*root);
    let root_count = merged.iter().take_while(|(_, root)| *root).count();
    let items = merged.into_iter().map(|(item, _)| item).collect::<Vec<_>>();
    reject_final_incompatibilities(&items, &ContentManifest::default(), Path::new(""))?;
    Ok(DependencyPlan {
        items,
        root_count,
        optional_dependencies,
    })
}

struct ResolvedChoice {
    title: String,
    kind: ContentKind,
    version: VersionInfo,
    installed: Option<InstalledMatch>,
    automatic_dependency: bool,
    cleanup_eligible: bool,
}

async fn resolve_requirement(
    provider: &dyn ContentProvider,
    manifest: &ContentManifest,
    remote_matches: &HashMap<PathBuf, ProviderProject>,
    instance: &InstanceConfig,
    parent_kind: ContentKind,
    target_world: Option<&Path>,
    dependency: &VersionDependency,
) -> Result<ResolvedChoice, NetError> {
    let exact_version = match dependency.version_id.as_deref() {
        Some(version_id) => Some(provider.version(version_id).await?),
        None => None,
    };
    let project_id = dependency
        .project_id
        .clone()
        .or_else(|| {
            exact_version
                .as_ref()
                .map(|version| version.project_id.clone())
        })
        .filter(|project_id| !project_id.is_empty())
        .ok_or_else(|| {
            NetError::Parse(format!(
                "Required dependency '{}' has no provider project",
                dependency.file_name.as_deref().unwrap_or("unknown")
            ))
        })?;
    let project = match provider.project(&project_id).await {
        Ok(project) => Some(project),
        Err(error) => {
            tracing::warn!(
                "Could not load metadata for dependency '{project_id}'; keeping it out of automatic cleanup: {error}"
            );
            None
        }
    };
    let kind = dependency_kind(project.as_ref(), exact_version.as_ref(), parent_kind);
    let installed = find_installed(
        manifest,
        remote_matches,
        provider.id(),
        &project_id,
        kind,
        target_world,
    );
    let version = if let Some(version) = exact_version {
        version
    } else {
        let installed_version = match &installed {
            Some(installed) => match provider.version(&installed.identity.version_id).await {
                Ok(version) => Some(version),
                Err(error) => {
                    tracing::warn!(
                        "Could not load installed dependency version '{}'; selecting a compatible replacement: {error}",
                        installed.identity.version_id
                    );
                    None
                }
            },
            None => None,
        };
        match installed_version {
            Some(version) if validate_compatible(&version, instance, kind).is_ok() => version,
            _ => {
                let versions = provider
                    .compatible_versions(&project_id, kind, &instance.game_version, instance.loader)
                    .await?;
                select_preferred_version(versions).ok_or_else(|| {
                    NetError::Parse(format!(
                        "No compatible dependency version found for project '{project_id}'"
                    ))
                })?
            }
        }
    };
    validate_compatible(&version, instance, kind)?;
    if !version.project_id.is_empty() && version.project_id != project_id {
        return Err(NetError::Parse(format!(
            "Dependency version '{}' belongs to project '{}', not '{}'",
            version.version_number, version.project_id, project_id
        )));
    }
    let automatic_dependency = installed
        .as_ref()
        .is_none_or(|installed| installed.automatic_dependency);
    let cleanup_eligible =
        automatic_dependency && project.as_ref().is_some_and(ProjectInfo::is_library_only);
    Ok(ResolvedChoice {
        title: project
            .map(|project| project.title)
            .unwrap_or_else(|| project_id.clone()),
        version,
        kind,
        automatic_dependency,
        cleanup_eligible,
        installed,
    })
}

async fn expand_node(
    provider: &dyn ContentProvider,
    key: &ProjectKey,
    version: &VersionInfo,
    nodes: &mut HashMap<ProjectKey, Node>,
    queue: &mut VecDeque<Requirement>,
) -> Result<(), NetError> {
    let mut required = Vec::new();
    let mut incompatible = Vec::new();
    let mut optional_dependencies = 0;
    for dependency in &version.dependencies {
        match dependency.dependency_type {
            DependencyType::Required => {
                let (project_id, _) = dependency_identity(provider, dependency).await?;
                required.push((
                    ProjectKey::new(provider.id(), project_id),
                    dependency.clone(),
                ));
            }
            DependencyType::Optional => optional_dependencies += 1,
            DependencyType::Incompatible => {
                let (project_id, version_id) = dependency_identity(provider, dependency).await?;
                incompatible.push(Incompatible {
                    project_id,
                    version_id,
                });
            }
            DependencyType::Embedded | DependencyType::Unknown => {}
        }
    }
    if let Some(node) = nodes.get_mut(key) {
        node.optional_dependencies = optional_dependencies;
        node.incompatible = incompatible;
        node.dependencies = required.clone();
    }
    queue.extend(
        required
            .into_iter()
            .map(|(target, dependency)| Requirement {
                parent: key.clone(),
                parent_version_id: version.id.clone(),
                target,
                dependency,
            }),
    );
    Ok(())
}

async fn dependency_identity(
    provider: &dyn ContentProvider,
    dependency: &VersionDependency,
) -> Result<(String, Option<String>), NetError> {
    if let Some(project_id) = dependency.project_id.clone() {
        return Ok((project_id, dependency.version_id.clone()));
    }
    let version_id = dependency.version_id.as_deref().ok_or_else(|| {
        NetError::Parse(format!(
            "{:?} dependency has no provider project",
            dependency.dependency_type
        ))
    })?;
    let version = provider.version(version_id).await?;
    Ok((version.project_id, Some(version_id.to_owned())))
}

fn select_preferred_version(mut versions: Vec<VersionInfo>) -> Option<VersionInfo> {
    versions.sort_by(|left, right| {
        release_rank(left.version_type)
            .cmp(&release_rank(right.version_type))
            .then_with(|| right.date_published.cmp(&left.date_published))
    });
    versions.into_iter().next()
}

fn dependency_kind(
    project: Option<&ProjectInfo>,
    version: Option<&VersionInfo>,
    parent_kind: ContentKind,
) -> ContentKind {
    if version.is_some_and(|version| {
        version
            .loaders
            .iter()
            .any(|loader| loader.eq_ignore_ascii_case("datapack"))
    }) {
        return ContentKind::DataPack;
    }
    if version.is_some_and(|version| {
        version.loaders.iter().any(|loader| {
            matches!(
                loader.to_ascii_lowercase().as_str(),
                "fabric" | "forge" | "neoforge" | "quilt"
            )
        })
    }) {
        return ContentKind::Mod;
    }
    if project.is_some_and(|project| {
        project
            .loaders
            .iter()
            .any(|loader| loader.eq_ignore_ascii_case("datapack"))
    }) {
        return ContentKind::DataPack;
    }
    match project.map(|project| project.project_type.as_str()) {
        Some("resourcepack") => ContentKind::ResourcePack,
        Some("shader") => ContentKind::Shader,
        Some("datapack") => ContentKind::DataPack,
        Some("mod") => ContentKind::Mod,
        _ => parent_kind,
    }
}

fn release_rank(version_type: VersionType) -> u8 {
    match version_type {
        VersionType::Release => 0,
        VersionType::Beta => 1,
        VersionType::Alpha => 2,
        VersionType::Unknown => 3,
    }
}

fn validate_compatible(
    version: &VersionInfo,
    instance: &InstanceConfig,
    kind: ContentKind,
) -> Result<(), NetError> {
    let loader = instance.loader.to_string().to_ascii_lowercase();
    let supports_game = version
        .game_versions
        .iter()
        .any(|game_version| game_version == &instance.game_version);
    let supports_loader = kind != ContentKind::Mod
        || version
            .loaders
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(&loader));
    if !supports_game || !supports_loader {
        return Err(NetError::Parse(format!(
            "Required dependency '{}' does not support Minecraft {} with {}",
            version.version_number, instance.game_version, instance.loader
        )));
    }
    Ok(())
}

async fn resolve_installed_files(
    provider: &dyn ContentProvider,
    manifest: &ContentManifest,
) -> Result<HashMap<PathBuf, ProviderProject>, NetError> {
    let files = manifest
        .files
        .iter()
        .filter(|record| record.enabled)
        .map(|record| FingerprintQuery {
            key: record.relative_path.to_string_lossy().into_owned(),
            kind: record.kind,
            fingerprint: record.fingerprint.clone(),
        })
        .collect::<Vec<_>>();
    if files.is_empty() {
        return Ok(HashMap::new());
    }
    Ok(provider
        .resolve_files(&files)
        .await?
        .into_iter()
        .map(|resolved| (PathBuf::from(resolved.key), resolved.project))
        .collect())
}

fn installed_root(
    manifest: &ContentManifest,
    minecraft_dir: &Path,
    root: &InstallRoot,
    remote_matches: &HashMap<PathBuf, ProviderProject>,
) -> Option<InstalledMatch> {
    let path = root.installed_path.as_ref()?;
    let relative_path = path.strip_prefix(minecraft_dir).ok()?.to_owned();
    let record = manifest.record(&relative_path)?;
    if record.kind != root.kind
        || !record_in_target(record, root.kind, root.target_world.as_deref())
        || (!record.matches_project(&root.provider, &root.project_id)
            && !remote_matches.get(&relative_path).is_some_and(|project| {
                project.provider == root.provider && project.project_id == root.project_id
            }))
    {
        return None;
    }
    Some(installed_match(
        record,
        remote_matches.get(&relative_path),
        &root.provider,
        &root.project_id,
    ))
}

fn find_installed(
    manifest: &ContentManifest,
    remote_matches: &HashMap<PathBuf, ProviderProject>,
    provider: &str,
    project_id: &str,
    kind: ContentKind,
    target_world: Option<&Path>,
) -> Option<InstalledMatch> {
    manifest
        .files
        .iter()
        .filter(|record| record.enabled && record.kind == kind)
        .filter(|record| record_in_target(record, kind, target_world))
        .find_map(|record| {
            let remote = remote_matches.get(&record.relative_path);
            if !record.matches_project(provider, project_id)
                && !remote.is_some_and(|project| {
                    project.provider == provider && project.project_id == project_id
                })
            {
                return None;
            }
            Some(installed_match(record, remote, provider, project_id))
        })
}

fn record_in_target(
    record: &ContentFileRecord,
    kind: ContentKind,
    target_world: Option<&Path>,
) -> bool {
    if kind != ContentKind::DataPack {
        return true;
    }
    let Some(world_name) = target_world.and_then(Path::file_name) else {
        return false;
    };
    record
        .relative_path
        .starts_with(Path::new("saves").join(world_name).join("datapacks"))
}

fn installed_match(
    record: &ContentFileRecord,
    remote: Option<&ProviderProject>,
    provider: &str,
    project_id: &str,
) -> InstalledMatch {
    let identity = record
        .project_for_provider(provider, project_id)
        .or_else(|| {
            remote
                .filter(|project| project.provider == provider && project.project_id == project_id)
        })
        .cloned()
        .unwrap_or_else(|| ProviderProject {
            provider: provider.to_owned(),
            project_id: project_id.to_owned(),
            version_id: String::new(),
        });
    let mut aliases = record.provider_aliases.clone();
    if let Some(project) = record.resolved_project()
        && project != &identity
        && !aliases.contains(project)
    {
        aliases.push(project.clone());
    }
    if record.resolved_project() != Some(&identity) && !aliases.contains(&identity) {
        aliases.push(identity.clone());
    }
    InstalledMatch {
        relative_path: record.relative_path.clone(),
        identity,
        aliases,
        automatic_dependency: record.automatic_dependency,
        record: record.clone(),
    }
}

fn reachable_order(root: &ProjectKey, nodes: &HashMap<ProjectKey, Node>) -> Vec<ProjectKey> {
    let mut order = Vec::new();
    let mut pending = VecDeque::from([root.clone()]);
    let mut seen = HashSet::new();
    while let Some(key) = pending.pop_front() {
        if !seen.insert(key.clone()) {
            continue;
        }
        order.push(key.clone());
        if let Some(node) = nodes.get(&key) {
            pending.extend(node.dependencies.iter().map(|(key, _)| key.clone()));
        }
    }
    order
}

fn active_exact_requirements(
    root: &ProjectKey,
    nodes: &HashMap<ProjectKey, Node>,
) -> HashMap<ProjectKey, Vec<Requirement>> {
    let mut active = HashMap::new();
    for parent in reachable_order(root, nodes) {
        let Some(node) = nodes.get(&parent) else {
            continue;
        };
        for (target, dependency) in &node.dependencies {
            if dependency.version_id.is_some() {
                active
                    .entry(target.clone())
                    .or_insert_with(Vec::new)
                    .push(Requirement {
                        parent: parent.clone(),
                        parent_version_id: node.version.id.clone(),
                        target: target.clone(),
                        dependency: dependency.clone(),
                    });
            }
        }
    }
    active
}

fn retry_requirements(
    root: &ProjectKey,
    nodes: &HashMap<ProjectKey, Node>,
) -> (Vec<Requirement>, Option<NetError>) {
    let mut retry = Vec::new();
    let mut queued = HashSet::new();
    let mut conflict = None;
    let mut exact = active_exact_requirements(root, nodes)
        .into_iter()
        .collect::<Vec<_>>();
    exact.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    for (target, requirements) in exact {
        let mut versions = requirements
            .iter()
            .filter_map(|requirement| requirement.dependency.version_id.as_deref())
            .collect::<Vec<_>>();
        if &target == root {
            versions.push(nodes[root].version.id.as_str());
        }
        versions.sort_unstable();
        versions.dedup();
        if versions.len() > 1 {
            conflict.get_or_insert_with(|| {
                NetError::Parse(format!(
                    "Conflicting required versions for '{}': {}",
                    target.project_id,
                    versions.join(", ")
                ))
            });
        } else if let Some(version) = versions.first()
            && nodes
                .get(&target)
                .is_none_or(|node| node.version.id.as_str() != *version)
        {
            queued.insert(target);
            retry.push(requirements[0].clone());
        }
    }
    for parent in reachable_order(root, nodes) {
        let Some(node) = nodes.get(&parent) else {
            continue;
        };
        for (target, dependency) in &node.dependencies {
            if !nodes.contains_key(target) && queued.insert(target.clone()) {
                retry.push(Requirement {
                    parent: parent.clone(),
                    parent_version_id: node.version.id.clone(),
                    target: target.clone(),
                    dependency: dependency.clone(),
                });
            }
        }
    }
    (retry, conflict)
}

fn reject_dependency_cycles(
    order: &[ProjectKey],
    nodes: &HashMap<ProjectKey, Node>,
) -> Result<(), NetError> {
    let mut incoming = order
        .iter()
        .cloned()
        .map(|key| (key, 0usize))
        .collect::<HashMap<_, _>>();
    for key in order {
        let node = nodes.get(key).ok_or_else(|| {
            NetError::Parse(format!(
                "Resolved dependency '{}' is missing",
                key.project_id
            ))
        })?;
        for (dependency, _) in &node.dependencies {
            *incoming.entry(dependency.clone()).or_default() += 1;
        }
    }
    let mut ready = order
        .iter()
        .filter(|key| incoming.get(*key) == Some(&0))
        .cloned()
        .collect::<VecDeque<_>>();
    let mut visited = 0;
    while let Some(key) = ready.pop_front() {
        visited += 1;
        for (dependency, _) in &nodes[&key].dependencies {
            let count = incoming.entry(dependency.clone()).or_default();
            *count -= 1;
            if *count == 0 {
                ready.push_back(dependency.clone());
            }
        }
    }
    if visited != order.len() {
        return Err(NetError::Parse(
            "Dependency cycle detected in selected versions".to_owned(),
        ));
    }
    Ok(())
}

fn reject_installed_incompatibilities(
    provider: &dyn ContentProvider,
    manifest: &ContentManifest,
    remote_matches: &HashMap<PathBuf, ProviderProject>,
    order: &[ProjectKey],
    nodes: &HashMap<ProjectKey, Node>,
    target_world: Option<&Path>,
) -> Result<(), NetError> {
    for key in order {
        let Some(node) = nodes.get(key) else {
            continue;
        };
        for incompatible in &node.incompatible {
            let installed = find_installed_incompatible(
                manifest,
                remote_matches,
                provider.id(),
                &incompatible.project_id,
                target_world,
            );
            let selected = order.iter().find_map(|selected_key| {
                (selected_key.provider == provider.id()
                    && selected_key.project_id == incompatible.project_id)
                    .then(|| nodes.get(selected_key))
                    .flatten()
            });
            let conflicts = installed.iter().any(|installed| {
                let replaced = order
                    .iter()
                    .filter_map(|key| nodes.get(key))
                    .any(|selected| {
                        selected.installed.as_ref().is_some_and(|previous| {
                            previous.relative_path == installed.relative_path
                                && previous.identity.version_id != selected.version.id
                        })
                    });
                !replaced
                    && incompatible
                        .version_id
                        .as_ref()
                        .is_none_or(|version_id| installed.identity.version_id == *version_id)
            }) || selected.is_some_and(|selected| {
                incompatible
                    .version_id
                    .as_ref()
                    .is_none_or(|version_id| selected.version.id == *version_id)
            });
            if conflicts {
                return Err(NetError::Parse(format!(
                    "'{}' is incompatible with installed project '{}'",
                    node.title, incompatible.project_id
                )));
            }
        }
    }
    Ok(())
}

fn find_installed_incompatible(
    manifest: &ContentManifest,
    remote_matches: &HashMap<PathBuf, ProviderProject>,
    provider: &str,
    project_id: &str,
    target_world: Option<&Path>,
) -> Vec<InstalledMatch> {
    manifest
        .files
        .iter()
        .filter(|record| record.enabled)
        .filter(|record| {
            record.kind != ContentKind::DataPack
                || record_in_target(record, ContentKind::DataPack, target_world)
        })
        .filter_map(|record| {
            let remote = remote_matches.get(&record.relative_path);
            (record.matches_project(provider, project_id)
                || remote.is_some_and(|project| {
                    project.provider == provider && project.project_id == project_id
                }))
            .then(|| installed_match(record, remote, provider, project_id))
        })
        .collect()
}

fn planned_install(
    key: &ProjectKey,
    node: &Node,
    nodes: &HashMap<ProjectKey, Node>,
    minecraft_dir: &Path,
    target_world: Option<&Path>,
    force_reinstall: bool,
) -> PlannedInstall {
    let installed_path = node
        .installed
        .as_ref()
        .map(|installed| minecraft_dir.join(&installed.relative_path));
    let replacement = force_reinstall
        || node.installed.as_ref().is_some_and(|installed| {
            installed.identity.version_id.is_empty()
                || installed.identity.version_id != node.version.id
        });
    PlannedInstall {
        provider: key.provider.clone(),
        project_id: key.project_id.clone(),
        title: node.title.clone(),
        version: node.version.clone(),
        installed_path,
        expected_record: node
            .installed
            .as_ref()
            .map(|installed| installed.record.clone()),
        kind: node.kind,
        destination: match node.kind {
            ContentKind::DataPack => target_world
                .expect("datapack dependency plan requires a world")
                .join("datapacks"),
            kind => minecraft_dir.join(kind.directory()),
        },
        provider_aliases: node.installed.as_ref().map_or_else(Vec::new, |installed| {
            if replacement {
                Vec::new()
            } else {
                installed.aliases.clone()
            }
        }),
        required_dependencies: node
            .dependencies
            .iter()
            .filter_map(|(dependency, _)| {
                nodes
                    .get(dependency)
                    .map(|dependency_node| ProviderProject {
                        provider: dependency.provider.clone(),
                        project_id: dependency.project_id.clone(),
                        version_id: dependency_node.version.id.clone(),
                    })
            })
            .fold(Vec::new(), |mut dependencies, dependency| {
                if !dependencies.contains(&dependency) {
                    dependencies.push(dependency);
                }
                dependencies
            }),
        automatic_dependency: node.automatic_dependency,
        cleanup_eligible: node.cleanup_eligible,
        replacement,
    }
}

#[cfg(test)]
#[path = "../tests/content/dependencies.rs"]
pub(crate) mod tests;
