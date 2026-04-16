use anyhow::{Context, Result, bail};
use codex_auth::{CodexAuthService, DeviceCodePrompt};
use codex_limits::CodexLimitsClient;
use domain::{AccountProfile, AuthFile, HostTarget, LimitsSnapshotSet, ManagedHost, OAuthSession};
use host_ops::{HostOperator, RemoteHostSnapshot};
use profile_store::ProfileStore;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusyOperation {
    RefreshLimits,
    ReloadAccounts,
    ActivateLocal,
    ActivateRemote,
    SyncHost,
    InspectHost,
    DeleteProfile,
    BrowserLogin,
    DeviceLoginStart,
    DeviceLoginFinish,
}

impl BusyOperation {
    pub fn label(self) -> &'static str {
        match self {
            Self::RefreshLimits => "Refreshing limits",
            Self::ReloadAccounts => "Reloading accounts",
            Self::ActivateLocal => "Activating locally",
            Self::ActivateRemote => "Activating remotely",
            Self::SyncHost => "Syncing host",
            Self::InspectHost => "Loading host",
            Self::DeleteProfile => "Deleting profile",
            Self::BrowserLogin => "Browser login",
            Self::DeviceLoginStart => "Starting device login",
            Self::DeviceLoginFinish => "Finishing device login",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeKind {
    Success,
    Error,
}

#[derive(Debug, Clone)]
pub struct OperationNotice {
    pub kind: NoticeKind,
    pub message: String,
    pub created_at: Instant,
}

impl OperationNotice {
    fn new(kind: NoticeKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            created_at: Instant::now(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modal {
    DeleteConfirm,
    Import,
    Help,
    UpdateConfirm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusArea {
    HostSelector,
    ProfileList,
    ActionBar,
}

pub struct App {
    pub current_auth: Option<AuthFile>,
    pub current_session: Option<OAuthSession>,
    pub profiles: Vec<AccountProfile>,
    pub hosts: Vec<ManagedHost>,
    pub limits_by_profile: HashMap<String, LimitsSnapshotSet>,
    pub selected_host_profiles: Vec<AccountProfile>,
    pub selected_host_active_auth: Option<AuthFile>,

    pub selected_host_index: usize,
    pub selected_profile_index: usize,
    pub busy_operation: Option<BusyOperation>,
    pub notice: Option<OperationNotice>,
    pub device_prompt: Option<DeviceCodePrompt>,

    pub modal: Option<Modal>,
    pub pending_delete_profile: Option<String>,
    pub import_text: String,

    pub focus: FocusArea,
    pub action_bar_index: usize,
    pub should_quit: bool,
    pub host_selector_open: bool,
    pub tick: u64,

    pub search_active: bool,
    pub search_query: String,
    pub filtered_indices: Vec<usize>,
    pub marked_profiles: HashSet<String>,
    pub available_update: Option<crate::updater::ReleaseInfo>,
    pub update_in_progress: bool,

    worker_tx: Sender<WorkerMessage>,
    worker_rx: Receiver<WorkerMessage>,
    update_rx: Option<Receiver<UpdateMessage>>,
}

pub enum UpdateMessage {
    CheckResult(Option<crate::updater::ReleaseInfo>),
    DownloadComplete(Result<()>),
}

#[derive(Clone)]
struct AppSnapshot {
    current_auth: Option<AuthFile>,
    current_session: Option<OAuthSession>,
    profiles: Vec<AccountProfile>,
    hosts: Vec<ManagedHost>,
    limits_by_profile: HashMap<String, LimitsSnapshotSet>,
}

enum WorkerMessage {
    Snapshot {
        snapshot: AppSnapshot,
        notice: OperationNotice,
    },
    ActivatedLocal {
        snapshot: AppSnapshot,
        profile_id: String,
    },
    ProfileLimits {
        profile_id: String,
        limits: LimitsSnapshotSet,
        notice: OperationNotice,
    },
    HostInspection {
        host_index: usize,
        snapshot: RemoteHostSnapshot,
        notice: Option<OperationNotice>,
    },
    DevicePrompt {
        prompt: DeviceCodePrompt,
        notice: OperationNotice,
    },
    Error {
        notice: OperationNotice,
    },
}

impl App {
    pub fn new() -> Result<Self> {
        let (worker_tx, worker_rx) = mpsc::channel();
        let mut app = Self {
            current_auth: None,
            current_session: None,
            profiles: Vec::new(),
            hosts: Vec::new(),
            limits_by_profile: HashMap::new(),
            selected_host_profiles: Vec::new(),
            selected_host_active_auth: None,
            selected_host_index: 0,
            selected_profile_index: 0,
            busy_operation: None,
            notice: None,
            device_prompt: None,
            modal: None,
            pending_delete_profile: None,
            import_text: String::new(),
            focus: FocusArea::ProfileList,
            action_bar_index: 0,
            should_quit: false,
            host_selector_open: false,
            tick: 0,
            search_active: false,
            search_query: String::new(),
            filtered_indices: Vec::new(),
            marked_profiles: HashSet::new(),
            available_update: None,
            update_in_progress: false,
            worker_tx,
            worker_rx,
            update_rx: None,
        };
        // Load accounts synchronously (fast) so the UI renders immediately
        if let Ok(snapshot) = load_snapshot(false) {
            app.apply_snapshot(snapshot);
        }
        // Check for updates in the background
        app.spawn_update_check();
        // Then kick off limits fetch in the background
        app.spawn_refresh_limits(false);
        Ok(app)
    }

    pub fn poll_background(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        loop {
            match self.worker_rx.try_recv() {
                Ok(message) => match message {
                    WorkerMessage::Snapshot { snapshot, notice } => {
                        self.apply_snapshot(snapshot);
                        self.busy_operation = None;
                        self.notice = Some(notice);
                        if self.selected_host_is_remote() {
                            self.refresh_selected_host_view(false);
                        }
                    }
                    WorkerMessage::ActivatedLocal {
                        snapshot,
                        profile_id,
                    } => {
                        self.apply_snapshot(snapshot);
                        self.busy_operation = None;
                        self.notice = None;
                        self.spawn_refresh_profile_limits(
                            profile_id,
                            "Activated account and refreshed limits.".into(),
                        );
                    }
                    WorkerMessage::ProfileLimits {
                        profile_id,
                        limits,
                        notice,
                    } => {
                        self.busy_operation = None;
                        self.limits_by_profile.insert(profile_id, limits);
                        self.notice = Some(notice);
                    }
                    WorkerMessage::HostInspection {
                        host_index,
                        snapshot,
                        notice,
                    } => {
                        if self.selected_host_index == host_index {
                            self.selected_host_profiles = snapshot.profiles;
                            self.selected_host_active_auth = Some(snapshot.active_auth);
                        }
                        self.busy_operation = None;
                        if let Some(notice) = notice {
                            self.notice = Some(notice);
                        }
                    }
                    WorkerMessage::DevicePrompt { prompt, notice } => {
                        self.device_prompt = Some(prompt);
                        self.busy_operation = None;
                        self.notice = Some(notice);
                    }
                    WorkerMessage::Error { notice } => {
                        self.busy_operation = None;
                        self.notice = Some(notice);
                    }
                },
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.busy_operation = None;
                    self.notice = Some(OperationNotice::new(
                        NoticeKind::Error,
                        "background worker disconnected",
                    ));
                    break;
                }
            }
        }

        if let Some(notice) = &self.notice {
            if notice.created_at.elapsed() >= Duration::from_secs(5) {
                self.notice = None;
            }
        }

        // Poll update channel
        if let Some(rx) = &self.update_rx {
            match rx.try_recv() {
                Ok(UpdateMessage::CheckResult(Some(release))) => {
                    self.available_update = Some(release);
                    self.update_rx = None;
                }
                Ok(UpdateMessage::CheckResult(None)) => {
                    self.update_rx = None;
                }
                Ok(UpdateMessage::DownloadComplete(Ok(()))) => {
                    self.update_in_progress = false;
                    self.update_rx = None;
                    self.notice = Some(OperationNotice::new(
                        NoticeKind::Success,
                        "Update downloaded. Restart to use the new version.",
                    ));
                }
                Ok(UpdateMessage::DownloadComplete(Err(e))) => {
                    self.update_in_progress = false;
                    self.update_rx = None;
                    self.notice = Some(OperationNotice::new(
                        NoticeKind::Error,
                        format!("Update failed: {e}"),
                    ));
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.update_in_progress = false;
                    self.update_rx = None;
                }
            }
        }
    }

    pub fn visible_profiles(&self) -> Vec<(usize, &AccountProfile)> {
        if self.search_query.is_empty() {
            self.selected_host_profiles
                .iter()
                .enumerate()
                .collect()
        } else {
            self.filtered_indices
                .iter()
                .filter_map(|&i| self.selected_host_profiles.get(i).map(|p| (i, p)))
                .collect()
        }
    }

    pub fn selected_profile(&self) -> Option<&AccountProfile> {
        self.selected_host_profiles.get(self.selected_profile_index)
    }

    pub fn is_profile_active(&self, profile: &AccountProfile) -> bool {
        let Some(current) = self.selected_host_active_auth.as_ref() else {
            return false;
        };
        current.tokens.access_token == profile.auth_file.tokens.access_token
    }

    pub fn selected_host_is_remote(&self) -> bool {
        matches!(
            self.hosts
                .get(self.selected_host_index)
                .map(|h| &h.target),
            Some(HostTarget::Remote(_))
        )
    }

    pub fn selected_host_label(&self) -> String {
        self.hosts
            .get(self.selected_host_index)
            .map(|h| host_display_label(h))
            .unwrap_or_else(|| "none".into())
    }

    pub fn active_profile_label(&self) -> String {
        self.selected_host_profiles
            .iter()
            .find(|p| self.is_profile_active(p))
            .map(|p| p.label.clone())
            .unwrap_or_else(|| "none".into())
    }

    pub fn profile_limits(&self, profile_id: &str) -> Option<&LimitsSnapshotSet> {
        self.limits_by_profile.get(profile_id)
    }

    // --- Actions ---

    pub fn next_profile(&mut self) {
        let visible = self.visible_profiles();
        if visible.is_empty() {
            return;
        }
        // Find current position in visible list
        let cur_pos = visible
            .iter()
            .position(|(i, _)| *i == self.selected_profile_index)
            .unwrap_or(0);
        let next_pos = (cur_pos + 1) % visible.len();
        self.selected_profile_index = visible[next_pos].0;
    }

    pub fn prev_profile(&mut self) {
        let visible = self.visible_profiles();
        if visible.is_empty() {
            return;
        }
        let cur_pos = visible
            .iter()
            .position(|(i, _)| *i == self.selected_profile_index)
            .unwrap_or(0);
        let prev_pos = if cur_pos == 0 {
            visible.len() - 1
        } else {
            cur_pos - 1
        };
        self.selected_profile_index = visible[prev_pos].0;
    }

    pub fn start_search(&mut self) {
        self.search_active = true;
        self.search_query.clear();
        self.update_filter();
    }

    pub fn search_push(&mut self, c: char) {
        self.search_query.push(c);
        self.update_filter();
    }

    pub fn search_pop(&mut self) {
        self.search_query.pop();
        self.update_filter();
    }

    pub fn finish_search(&mut self) {
        self.search_active = false;
        // Keep the filter active
    }

    pub fn cancel_search(&mut self) {
        self.search_active = false;
        self.search_query.clear();
        self.filtered_indices.clear();
    }

    fn update_filter(&mut self) {
        let query = self.search_query.to_lowercase();
        if query.is_empty() {
            self.filtered_indices.clear();
        } else {
            self.filtered_indices = self
                .selected_host_profiles
                .iter()
                .enumerate()
                .filter(|(_, p)| p.label.to_lowercase().contains(&query))
                .map(|(i, _)| i)
                .collect();
        }
        // Adjust selection to first visible match
        let visible = self.visible_profiles();
        if !visible.is_empty()
            && !visible
                .iter()
                .any(|(i, _)| *i == self.selected_profile_index)
        {
            self.selected_profile_index = visible[0].0;
        }
    }

    pub fn next_host(&mut self) {
        if !self.hosts.is_empty() {
            let new_index = (self.selected_host_index + 1) % self.hosts.len();
            self.set_selected_host_index(new_index);
        }
    }

    pub fn prev_host(&mut self) {
        if !self.hosts.is_empty() {
            let new_index = self
                .selected_host_index
                .checked_sub(1)
                .unwrap_or(self.hosts.len() - 1);
            self.set_selected_host_index(new_index);
        }
    }

    pub fn set_selected_host_index(&mut self, host_index: usize) {
        if self.selected_host_index == host_index {
            return;
        }
        self.selected_host_index = host_index;
        self.selected_profile_index = 0;
        self.notice = None;
        self.refresh_selected_host_view(false);
    }

    pub fn activate_selected_profile(&mut self) {
        let Some(profile) = self.selected_profile() else {
            return;
        };
        let profile_id = profile.id.clone();
        match self
            .hosts
            .get(self.selected_host_index)
            .map(|h| &h.target)
        {
            Some(HostTarget::Local { .. }) | None => self.activate_profile_locally(&profile_id),
            Some(HostTarget::Remote(_)) => self.activate_profile_remotely(&profile_id),
        }
    }

    pub fn delete_selected_profile(&mut self) {
        if let Some(profile) = self.selected_profile() {
            self.pending_delete_profile = Some(profile.id.clone());
            self.modal = Some(Modal::DeleteConfirm);
        }
    }

    pub fn toggle_mark(&mut self) {
        if let Some(profile) = self.selected_profile() {
            let id = profile.id.clone();
            if !self.marked_profiles.remove(&id) {
                self.marked_profiles.insert(id);
            }
        }
    }

    pub fn mark_all_visible(&mut self) {
        let visible: Vec<String> = self.visible_profiles().iter().map(|(_, p)| p.id.clone()).collect();
        let all_marked = visible.iter().all(|id| self.marked_profiles.contains(id));
        if all_marked {
            for id in &visible {
                self.marked_profiles.remove(id);
            }
        } else {
            self.marked_profiles.extend(visible);
        }
    }

    pub fn clear_marks(&mut self) {
        self.marked_profiles.clear();
    }

    fn export_targets(&self) -> Vec<&AccountProfile> {
        if self.marked_profiles.is_empty() {
            self.selected_profile().into_iter().collect()
        } else {
            self.selected_host_profiles
                .iter()
                .filter(|p| self.marked_profiles.contains(&p.id))
                .collect()
        }
    }

    pub fn export_selected_profile(&mut self) {
        let targets = self.export_targets();
        if targets.is_empty() {
            return;
        }
        let (label, json) = if targets.len() == 1 {
            (
                targets[0].label.clone(),
                serde_json::to_string_pretty(&targets[0].auth_file),
            )
        } else {
            let auth_files: Vec<_> = targets.iter().map(|p| &p.auth_file).collect();
            (
                format!("{} accounts", targets.len()),
                serde_json::to_string_pretty(&auth_files),
            )
        };
        match json {
            Ok(text) => match cli_clipboard::set_contents(text) {
                Ok(()) => {
                    self.notice = Some(OperationNotice::new(
                        NoticeKind::Success,
                        format!("Copied `{label}` to clipboard."),
                    ));
                }
                Err(err) => {
                    self.notice = Some(OperationNotice::new(
                        NoticeKind::Error,
                        format!("Clipboard error: {err}"),
                    ));
                }
            },
            Err(err) => {
                self.notice = Some(OperationNotice::new(NoticeKind::Error, err.to_string()));
            }
        }
    }

    pub fn open_import_modal(&mut self) {
        self.import_text.clear();
        self.modal = Some(Modal::Import);
    }

    pub fn close_modal(&mut self) {
        self.modal = None;
        self.pending_delete_profile = None;
    }

    pub fn open_update_modal(&mut self) {
        if self.available_update.is_some() {
            self.modal = Some(Modal::UpdateConfirm);
        }
    }

    pub fn confirm_update(&mut self) {
        self.modal = None;
        let Some(release) = self.available_update.clone() else {
            return;
        };
        self.update_in_progress = true;
        let (tx, rx) = mpsc::channel();
        self.update_rx = Some(rx);
        thread::spawn(move || {
            let result = crate::updater::download_and_replace(&release);
            let _ = tx.send(UpdateMessage::DownloadComplete(result));
        });
    }

    fn spawn_update_check(&mut self) {
        let (tx, rx) = mpsc::channel();
        self.update_rx = Some(rx);
        thread::spawn(move || {
            let result = crate::updater::check_for_update().ok().flatten();
            let _ = tx.send(UpdateMessage::CheckResult(result));
        });
    }

    pub fn confirm_delete(&mut self) {
        let Some(profile_id) = self.pending_delete_profile.clone() else {
            return;
        };
        self.modal = None;
        self.pending_delete_profile = None;
        self.run_background(BusyOperation::DeleteProfile, move || {
            let store = ProfileStore::new();
            let profile = store
                .list_profiles()?
                .into_iter()
                .find(|p| p.id == profile_id)
                .context("profile not found")?;
            store.delete_profile(&profile.label)?;
            let snapshot = load_snapshot(false)?;
            Ok(WorkerMessage::Snapshot {
                snapshot,
                notice: OperationNotice::new(
                    NoticeKind::Success,
                    format!("Deleted profile `{}`.", profile.label),
                ),
            })
        });
    }

    pub fn import_from_text(&mut self) {
        let payload = self.import_text.trim().to_string();
        if payload.is_empty() {
            self.notice = Some(OperationNotice::new(
                NoticeKind::Error,
                "import payload is empty",
            ));
            return;
        }
        self.modal = None;
        self.import_text.clear();
        self.run_background(BusyOperation::ReloadAccounts, move || {
            let auth_service = CodexAuthService::new();
            let store = ProfileStore::new();

            // Try parsing as array first, then single object
            let auth_files: Vec<AuthFile> = if payload.trim_start().starts_with('[') {
                serde_json::from_str(&payload).context("invalid account JSON array")?
            } else {
                let single: AuthFile =
                    serde_json::from_str(&payload).context("invalid account JSON")?;
                vec![single]
            };

            let mut imported = Vec::new();
            for auth_file in &auth_files {
                let session = auth_file.to_session(
                    auth_service.default_issuer(),
                    auth_service.default_client_id(),
                );
                let profile_name = default_profile_name(&session, auth_file);
                store.save_profile(&profile_name, auth_file)?;
                imported.push(profile_name);
            }

            let snapshot = load_snapshot(true)?;
            let msg = if imported.len() == 1 {
                format!("Imported account `{}`.", imported[0])
            } else {
                format!("Imported {} accounts.", imported.len())
            };
            Ok(WorkerMessage::Snapshot {
                snapshot,
                notice: OperationNotice::new(NoticeKind::Success, msg),
            })
        });
    }

    pub fn refresh_all_limits(&mut self) {
        self.spawn_refresh_limits(false);
    }

    pub fn reload_accounts(&mut self) {
        self.spawn_reload(false);
    }

    pub fn login_browser(&mut self) {
        self.run_background(BusyOperation::BrowserLogin, move || {
            let auth_service = CodexAuthService::new();
            let store = ProfileStore::new();
            let operator = HostOperator::new();
            let options = codex_auth::BrowserAuthOptions::default();
            let session = auth_service
                .login_with_browser(&options)
                .map_err(anyhow::Error::msg)?;
            let auth_file = auth_service.auth_file_from_session(session.clone());
            let profile_name = default_profile_name(&session, &auth_file);
            let local_host = store
                .load_hosts()?
                .into_iter()
                .find(|h| matches!(h.target, HostTarget::Local { .. }))
                .context("missing local host")?;
            operator.write_auth(&local_host, &auth_file)?;
            store.save_profile(&profile_name, &auth_file)?;
            let snapshot = load_snapshot(true)?;
            Ok(WorkerMessage::Snapshot {
                snapshot,
                notice: OperationNotice::new(
                    NoticeKind::Success,
                    format!("Browser login completed: `{profile_name}`."),
                ),
            })
        });
    }

    pub fn start_device_login(&mut self) {
        self.run_background(BusyOperation::DeviceLoginStart, move || {
            let auth_service = CodexAuthService::new();
            let prompt = auth_service
                .request_device_code(
                    auth_service.default_issuer(),
                    auth_service.default_client_id(),
                )
                .map_err(anyhow::Error::msg)?;
            Ok(WorkerMessage::DevicePrompt {
                notice: OperationNotice::new(
                    NoticeKind::Success,
                    format!(
                        "Visit {} and enter code {}",
                        prompt.verification_url, prompt.user_code
                    ),
                ),
                prompt,
            })
        });
    }

    pub fn finish_device_login(&mut self) {
        let Some(prompt) = self.device_prompt.clone() else {
            self.notice = Some(OperationNotice::new(
                NoticeKind::Error,
                "no pending device login",
            ));
            return;
        };
        self.run_background(BusyOperation::DeviceLoginFinish, move || {
            let auth_service = CodexAuthService::new();
            let store = ProfileStore::new();
            let operator = HostOperator::new();
            let session = auth_service
                .complete_device_login(
                    auth_service.default_issuer(),
                    auth_service.default_client_id(),
                    &prompt,
                    Duration::from_secs(15 * 60),
                )
                .map_err(anyhow::Error::msg)?;
            let auth_file = auth_service.auth_file_from_session(session.clone());
            let profile_name = default_profile_name(&session, &auth_file);
            let local_host = store
                .load_hosts()?
                .into_iter()
                .find(|h| matches!(h.target, HostTarget::Local { .. }))
                .context("missing local host")?;
            operator.write_auth(&local_host, &auth_file)?;
            store.save_profile(&profile_name, &auth_file)?;
            let snapshot = load_snapshot(true)?;
            Ok(WorkerMessage::Snapshot {
                snapshot,
                notice: OperationNotice::new(
                    NoticeKind::Success,
                    format!("Device login completed: `{profile_name}`."),
                ),
            })
        });
    }

    pub fn sync_selected_remote(&mut self) {
        let selected_host_index = self.selected_host_index;
        self.run_background(BusyOperation::SyncHost, move || {
            let store = ProfileStore::new();
            let operator = HostOperator::new();
            let hosts = store.load_hosts()?;
            let remote_host = hosts
                .get(selected_host_index)
                .cloned()
                .context("selected remote host missing")?;
            let remote_alias = remote_host.label.clone();
            let remote = match &remote_host.target {
                HostTarget::Remote(r) => r,
                HostTarget::Local { .. } => bail!("selected host is local"),
            };
            operator.sync_managed_data_dir(remote, store.managed_data_dir())?;
            let snapshot = load_snapshot(false)?;
            Ok(WorkerMessage::Snapshot {
                snapshot,
                notice: OperationNotice::new(
                    NoticeKind::Success,
                    format!("Synced to `{remote_alias}`."),
                ),
            })
        });
    }

    // --- Private ---

    fn activate_profile_locally(&mut self, profile_id: &str) {
        let profile_id = profile_id.to_string();
        self.run_background(BusyOperation::ActivateLocal, move || {
            let store = ProfileStore::new();
            let operator = HostOperator::new();
            let profile = store
                .list_profiles()?
                .into_iter()
                .find(|p| p.id == profile_id)
                .context("profile not found")?;
            let local_host = store
                .load_hosts()?
                .into_iter()
                .find(|h| matches!(h.target, HostTarget::Local { .. }))
                .context("missing local host")?;
            operator.write_auth(&local_host, &profile.auth_file)?;
            let snapshot = load_snapshot(false)?;
            Ok(WorkerMessage::ActivatedLocal {
                snapshot,
                profile_id: profile.id,
            })
        });
    }

    fn activate_profile_remotely(&mut self, profile_id: &str) {
        let profile_id = profile_id.to_string();
        let selected_host_index = self.selected_host_index;
        self.run_background(BusyOperation::ActivateRemote, move || {
            let store = ProfileStore::new();
            let operator = HostOperator::new();
            let profile = store
                .list_profiles()?
                .into_iter()
                .find(|p| p.id == profile_id)
                .context("profile not found")?;
            let hosts = store.load_hosts()?;
            let remote_host = hosts
                .get(selected_host_index)
                .cloned()
                .context("selected remote host missing")?;
            let remote_alias = remote_host.label.clone();
            operator.write_auth(&remote_host, &profile.auth_file)?;
            let snapshot = load_snapshot(false)?;
            Ok(WorkerMessage::Snapshot {
                snapshot,
                notice: OperationNotice::new(
                    NoticeKind::Success,
                    format!("Activated `{}` on `{}`.", profile.label, remote_alias),
                ),
            })
        });
    }

    fn spawn_refresh_limits(&mut self, _initial: bool) {
        self.run_background(BusyOperation::RefreshLimits, move || {
            let snapshot = load_snapshot(true)?;
            Ok(WorkerMessage::Snapshot {
                snapshot,
                notice: OperationNotice::new(NoticeKind::Success, "Refreshed limits."),
            })
        });
    }

    fn spawn_refresh_profile_limits(&mut self, profile_id: String, success_message: String) {
        self.run_background(BusyOperation::RefreshLimits, move || {
            let auth_service = CodexAuthService::new();
            let limits_client = CodexLimitsClient::new("https://chatgpt.com");
            let profile_store = ProfileStore::new();
            let profile = profile_store
                .list_profiles()?
                .into_iter()
                .find(|p| p.id == profile_id)
                .context("profile not found for limit refresh")?;
            let session = profile.auth_file.to_session(
                auth_service.default_issuer(),
                auth_service.default_client_id(),
            );
            let limits = limits_client.fetch(&session)?;
            Ok(WorkerMessage::ProfileLimits {
                profile_id: profile.id,
                limits,
                notice: OperationNotice::new(NoticeKind::Success, success_message),
            })
        });
    }

    fn spawn_reload(&mut self, fetch_limits: bool) {
        self.run_background(BusyOperation::ReloadAccounts, move || {
            let snapshot = load_snapshot(fetch_limits)?;
            Ok(WorkerMessage::Snapshot {
                snapshot,
                notice: OperationNotice::new(NoticeKind::Success, "Reloaded accounts."),
            })
        });
    }

    pub fn refresh_selected_host_view(&mut self, show_notice: bool) {
        let Some(host) = self.hosts.get(self.selected_host_index).cloned() else {
            return;
        };
        match host.target {
            HostTarget::Local { .. } => {
                self.selected_host_profiles = self.profiles.clone();
                self.selected_host_active_auth = self.current_auth.clone();
                if show_notice {
                    self.notice = Some(OperationNotice::new(
                        NoticeKind::Success,
                        "Loaded local host accounts.",
                    ));
                }
            }
            HostTarget::Remote(remote) => {
                let host_index = self.selected_host_index;
                self.run_background(BusyOperation::InspectHost, move || {
                    let snapshot = HostOperator::new().inspect_remote_host(&remote)?;
                    Ok(WorkerMessage::HostInspection {
                        host_index,
                        snapshot,
                        notice: if show_notice {
                            Some(OperationNotice::new(
                                NoticeKind::Success,
                                format!("Loaded accounts from `{}`.", host.label),
                            ))
                        } else {
                            None
                        },
                    })
                });
            }
        }
    }

    fn run_background(
        &mut self,
        operation: BusyOperation,
        job: impl FnOnce() -> Result<WorkerMessage> + Send + 'static,
    ) {
        self.busy_operation = Some(operation);
        self.notice = None;
        let sender = self.worker_tx.clone();
        thread::spawn(move || {
            let message = match job() {
                Ok(msg) => msg,
                Err(err) => WorkerMessage::Error {
                    notice: OperationNotice::new(NoticeKind::Error, err.to_string()),
                },
            };
            let _ = sender.send(message);
        });
    }

    fn apply_snapshot(&mut self, snapshot: AppSnapshot) {
        let previous_host_id = self.hosts.get(self.selected_host_index).map(|h| h.id.clone());
        let previous_limits = self.limits_by_profile.clone();
        self.current_auth = snapshot.current_auth;
        self.current_session = snapshot.current_session;
        self.profiles = snapshot.profiles;
        self.hosts = snapshot.hosts;
        self.limits_by_profile = if snapshot.limits_by_profile.is_empty() {
            previous_limits
        } else {
            snapshot.limits_by_profile
        };
        self.selected_host_index = previous_host_id
            .and_then(|id| self.hosts.iter().position(|h| h.id == id))
            .unwrap_or(0);
        self.prune_limit_cache();
        if matches!(
            self.hosts
                .get(self.selected_host_index)
                .map(|h| &h.target),
            Some(HostTarget::Local { .. }) | None
        ) {
            self.selected_host_profiles = self.profiles.clone();
            self.selected_host_active_auth = self.current_auth.clone();
        }
        if self.selected_profile_index >= self.selected_host_profiles.len() {
            self.selected_profile_index = self
                .selected_host_profiles
                .len()
                .saturating_sub(1);
        }
    }

    fn prune_limit_cache(&mut self) {
        let valid_ids: HashSet<_> = self.profiles.iter().map(|p| p.id.clone()).collect();
        self.limits_by_profile
            .retain(|id, _| valid_ids.contains(id));
    }
}

fn load_snapshot(fetch_limits: bool) -> Result<AppSnapshot> {
    let auth_service = CodexAuthService::new();
    let limits_client = CodexLimitsClient::new("https://chatgpt.com");
    let profile_store = ProfileStore::new();

    let current_auth = profile_store.load_current_auth().ok();
    let current_session = current_auth.as_ref().map(|auth| {
        auth.to_session(
            auth_service.default_issuer(),
            auth_service.default_client_id(),
        )
    });
    let mut profiles = profile_store.list_profiles().unwrap_or_default();
    let hosts = profile_store.load_hosts().unwrap_or_default();

    // Auto-import: if auth.json has an account not yet saved as a profile, add it
    if let Some(auth) = &current_auth {
        let already_saved = profiles
            .iter()
            .any(|p| p.auth_file.tokens.access_token == auth.tokens.access_token);
        if !already_saved && !auth.tokens.access_token.is_empty() {
            let session = auth.to_session(
                auth_service.default_issuer(),
                auth_service.default_client_id(),
            );
            let name = default_profile_name(&session, auth);
            if profile_store.save_profile(&name, auth).is_ok() {
                // Re-read profiles so the new one appears
                profiles = profile_store.list_profiles().unwrap_or_default();
            }
        }
    }

    let limits_by_profile = if fetch_limits {
        let mut limits = HashMap::new();
        for profile in &profiles {
            let session = profile.auth_file.to_session(
                auth_service.default_issuer(),
                auth_service.default_client_id(),
            );
            if let Ok(snapshot) = limits_client.fetch(&session) {
                limits.insert(profile.id.clone(), snapshot);
            }
        }
        limits
    } else {
        HashMap::new()
    };

    Ok(AppSnapshot {
        current_auth,
        current_session,
        profiles,
        hosts,
        limits_by_profile,
    })
}

fn default_profile_name(session: &OAuthSession, auth_file: &AuthFile) -> String {
    session
        .email
        .clone()
        .or_else(|| auth_file.tokens.account_id.clone())
        .unwrap_or_else(|| "new-account".into())
}

fn host_display_label(host: &ManagedHost) -> String {
    match &host.target {
        HostTarget::Local { .. } => {
            let name = hostname().unwrap_or_else(|| host.label.clone());
            format!("{name} *L")
        }
        HostTarget::Remote(remote) => format!("{} *R", remote.ssh_alias),
    }
}

fn hostname() -> Option<String> {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}
