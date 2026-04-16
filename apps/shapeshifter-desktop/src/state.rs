use anyhow::{Context, Result, bail};
use codex_auth::{BrowserAuthOptions, CodexAuthService, DeviceCodePrompt};
use codex_limits::CodexLimitsClient;
use domain::{AccountProfile, AuthFile, HostTarget, LimitsSnapshotSet, ManagedHost, OAuthSession};
use host_ops::{HostOperator, RemoteHostSnapshot};
use profile_store::ProfileStore;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::Duration;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeKind {
    Success,
    Error,
}

#[derive(Debug, Clone)]
pub struct OperationNotice {
    pub kind: NoticeKind,
    pub message: String,
    pub created_at: std::time::Instant,
}

impl OperationNotice {
    fn new(kind: NoticeKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            created_at: std::time::Instant::now(),
        }
    }
}

pub struct AppState {
    pub current_auth: Option<AuthFile>,
    pub current_session: Option<OAuthSession>,
    pub profiles: Vec<AccountProfile>,
    pub hosts: Vec<ManagedHost>,
    pub limits_by_profile: HashMap<String, LimitsSnapshotSet>,
    pub selected_host_profiles: Vec<AccountProfile>,
    pub selected_host_active_auth: Option<AuthFile>,
    pub export_profile_label: Option<String>,
    pub export_text: String,
    pub import_modal_open: bool,
    pub import_text: String,
    pub pending_delete_profile: Option<String>,
    pub device_prompt: Option<DeviceCodePrompt>,
    pub selected_host_index: usize,
    pub busy_operation: Option<BusyOperation>,
    pub notice: Option<OperationNotice>,
    worker_tx: Sender<WorkerMessage>,
    worker_rx: Receiver<WorkerMessage>,
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

impl AppState {
    pub fn load() -> Result<Self> {
        let (worker_tx, worker_rx) = mpsc::channel();
        let mut state = Self {
            current_auth: None,
            current_session: None,
            profiles: Vec::new(),
            hosts: Vec::new(),
            limits_by_profile: HashMap::new(),
            selected_host_profiles: Vec::new(),
            selected_host_active_auth: None,
            export_profile_label: None,
            export_text: String::new(),
            import_modal_open: false,
            import_text: String::new(),
            pending_delete_profile: None,
            device_prompt: None,
            selected_host_index: 0,
            busy_operation: None,
            notice: None,
            worker_tx,
            worker_rx,
        };
        state.spawn_refresh_limits(true);
        Ok(state)
    }

    pub fn poll_background(&mut self) {
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
            if notice.created_at.elapsed() >= Duration::from_secs(3) {
                self.notice = None;
            }
        }
    }

    pub fn refresh_disk_state(&mut self) {
        self.spawn_reload(false, "Reloading accounts…");
    }

    pub fn refresh_all_limits(&mut self) {
        self.spawn_refresh_limits(false);
    }

    pub fn activate_profile_locally(&mut self, profile_id: &str) {
        let profile_id = profile_id.to_string();
        self.run_background(BusyOperation::ActivateLocal, move || {
            let store = ProfileStore::new();
            let operator = HostOperator::new();
            let profile = store
                .list_profiles()?
                .into_iter()
                .find(|profile| profile.id == profile_id)
                .context("profile not found")?;
            let local_host = store
                .load_hosts()?
                .into_iter()
                .find(|host| matches!(host.target, HostTarget::Local { .. }))
                .context("missing local host")?;
            operator.write_auth(&local_host, &profile.auth_file)?;
            let snapshot = load_snapshot(false)?;
            Ok(WorkerMessage::ActivatedLocal {
                snapshot,
                profile_id: profile.id,
            })
        });
    }

    pub fn activate_profile_remotely(&mut self, profile_id: &str) {
        let profile_id = profile_id.to_string();
        let selected_host_index = self.selected_host_index;
        self.run_background(BusyOperation::ActivateRemote, move || {
            let store = ProfileStore::new();
            let operator = HostOperator::new();
            let profile = store
                .list_profiles()?
                .into_iter()
                .find(|profile| profile.id == profile_id)
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
                    format!(
                        "Activated `{}` on remote host `{}`.",
                        profile.label, remote_alias
                    ),
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
                HostTarget::Remote(remote) => remote,
                HostTarget::Local { .. } => bail!("selected host is local"),
            };
            operator.sync_managed_data_dir(remote, store.managed_data_dir())?;
            let snapshot = load_snapshot(false)?;
            Ok(WorkerMessage::Snapshot {
                snapshot,
                notice: OperationNotice::new(
                    NoticeKind::Success,
                    format!("Synced managed accounts to remote host `{remote_alias}`."),
                ),
            })
        });
    }

    pub fn activate_profile_for_selected_host(&mut self, profile_id: &str) {
        match self
            .hosts
            .get(self.selected_host_index)
            .map(|host| &host.target)
        {
            Some(HostTarget::Local { .. }) | None => self.activate_profile_locally(profile_id),
            Some(HostTarget::Remote(_)) => self.activate_profile_remotely(profile_id),
        }
    }

    pub fn set_selected_host_index(&mut self, host_index: usize) {
        if self.selected_host_index == host_index {
            return;
        }
        self.selected_host_index = host_index;
        self.notice = None;
        self.refresh_selected_host_view(false);
    }

    pub fn prompt_delete_profile(&mut self, profile_id: impl Into<String>) {
        self.pending_delete_profile = Some(profile_id.into());
    }

    pub fn open_export_profile(&mut self, profile_id: &str) {
        let Some(profile) = self
            .profiles
            .iter()
            .find(|profile| profile.id == profile_id)
        else {
            self.notice = Some(OperationNotice::new(
                NoticeKind::Error,
                "profile not found",
            ));
            return;
        };
        match serde_json::to_string_pretty(&profile.auth_file) {
            Ok(text) => {
                self.export_profile_label = Some(profile.label.clone());
                self.export_text = text;
            }
            Err(err) => {
                self.notice = Some(OperationNotice::new(
                    NoticeKind::Error,
                    err.to_string(),
                ));
            }
        }
    }

    pub fn close_export_modal(&mut self) {
        self.export_profile_label = None;
        self.export_text.clear();
    }

    pub fn open_import_modal(&mut self) {
        self.import_modal_open = true;
    }

    pub fn close_import_modal(&mut self) {
        self.import_modal_open = false;
        self.import_text.clear();
    }

    pub fn import_profile_from_text(&mut self) {
        let payload = self.import_text.trim().to_string();
        if payload.is_empty() {
            self.notice = Some(OperationNotice::new(
                NoticeKind::Error,
                "import payload is empty",
            ));
            return;
        }
        self.import_modal_open = false;
        self.import_text.clear();
        self.run_background(BusyOperation::ReloadAccounts, move || {
            let auth_file: AuthFile =
                serde_json::from_str(&payload).context("invalid account JSON")?;
            let auth_service = CodexAuthService::new();
            let session = auth_file.to_session(
                auth_service.default_issuer(),
                auth_service.default_client_id(),
            );
            let profile_name = default_profile_name(&session, &auth_file);
            let store = ProfileStore::new();
            store.save_profile(&profile_name, &auth_file)?;
            let snapshot = load_snapshot(true)?;
            Ok(WorkerMessage::Snapshot {
                snapshot,
                notice: OperationNotice::new(
                    NoticeKind::Success,
                    format!("Imported account `{profile_name}`."),
                ),
            })
        });
    }

    pub fn cancel_delete_profile(&mut self) {
        self.pending_delete_profile = None;
    }

    pub fn confirm_delete_profile(&mut self) {
        let Some(profile_id) = self.pending_delete_profile.clone() else {
            self.notice = Some(OperationNotice::new(
                NoticeKind::Error,
                "no profile selected for deletion",
            ));
            return;
        };
        self.pending_delete_profile = None;
        self.run_background(BusyOperation::DeleteProfile, move || {
            let store = ProfileStore::new();
            let profile = store
                .list_profiles()?
                .into_iter()
                .find(|profile| profile.id == profile_id)
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

    pub fn login_browser(&mut self) {
        self.run_background(BusyOperation::BrowserLogin, move || {
            let auth_service = CodexAuthService::new();
            let store = ProfileStore::new();
            let operator = HostOperator::new();
            let options = BrowserAuthOptions::default();
            let session = auth_service
                .login_with_browser(&options)
                .map_err(anyhow::Error::msg)?;
            let auth_file = auth_service.auth_file_from_session(session.clone());
            let profile_name = default_profile_name(&session, &auth_file);
            let local_host = store
                .load_hosts()?
                .into_iter()
                .find(|host| matches!(host.target, HostTarget::Local { .. }))
                .context("missing local host")?;
            operator.write_auth(&local_host, &auth_file)?;
            store.save_profile(&profile_name, &auth_file)?;
            let snapshot = load_snapshot(true)?;
            Ok(WorkerMessage::Snapshot {
                snapshot,
                notice: OperationNotice::new(
                    NoticeKind::Success,
                    format!("Browser login completed and added profile `{profile_name}`."),
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
                        "Device login ready. Visit {} and enter code {}.",
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
                "no pending device login challenge",
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
                .find(|host| matches!(host.target, HostTarget::Local { .. }))
                .context("missing local host")?;
            operator.write_auth(&local_host, &auth_file)?;
            store.save_profile(&profile_name, &auth_file)?;
            let snapshot = load_snapshot(true)?;
            Ok(WorkerMessage::Snapshot {
                snapshot,
                notice: OperationNotice::new(
                    NoticeKind::Success,
                    format!("Device login completed and added profile `{profile_name}`."),
                ),
            })
        });
    }

    pub fn clear_notice(&mut self) {
        self.notice = None;
    }

    pub fn is_busy(&self, operation: BusyOperation) -> bool {
        self.busy_operation == Some(operation)
    }

    pub fn selected_remote_host_label(&self) -> String {
        self.hosts
            .get(self.selected_host_index)
            .map(|host| host_display_label(host))
            .unwrap_or_else(|| "none".into())
    }

    pub fn selected_host_active_profile_label(&self) -> String {
        self.selected_host_profiles
            .iter()
            .find(|profile| self.is_profile_active_on_selected_host(profile))
            .map(|profile| profile.label.clone())
            .unwrap_or_else(|| "none".into())
    }

    pub fn profile_limits(&self, profile_id: &str) -> Option<&LimitsSnapshotSet> {
        self.limits_by_profile.get(profile_id)
    }

    pub fn is_profile_active_on_selected_host(&self, profile: &AccountProfile) -> bool {
        let Some(current) = self.selected_host_active_auth.as_ref() else {
            return false;
        };
        current.tokens.access_token == profile.auth_file.tokens.access_token
    }

    pub fn host_choices(&self) -> Vec<(usize, String)> {
        let mut seen = HashSet::new();
        self.hosts
            .iter()
            .enumerate()
            .filter(|(_, host)| seen.insert(host.id.clone()))
            .map(|(i, host)| (i, host_display_label(host)))
            .collect()
    }

    pub fn selected_host_is_remote(&self) -> bool {
        matches!(
            self.hosts
                .get(self.selected_host_index)
                .map(|host| &host.target),
            Some(HostTarget::Remote(_))
        )
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
                                format!("Loaded accounts from host `{}`.", host.label),
                            ))
                        } else {
                            None
                        },
                    })
                });
            }
        }
    }

    fn spawn_refresh_limits(&mut self, initial: bool) {
        let _ = initial;
        self.run_background(BusyOperation::RefreshLimits, move || {
            let snapshot = load_snapshot(true)?;
            Ok(WorkerMessage::Snapshot {
                snapshot,
                notice: OperationNotice::new(
                    NoticeKind::Success,
                    "Refreshed limits.",
                ),
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
                .find(|profile| profile.id == profile_id)
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

    fn spawn_reload(&mut self, fetch_limits: bool, _status: &str) {
        self.run_background(BusyOperation::ReloadAccounts, move || {
            let snapshot = load_snapshot(fetch_limits)?;
            Ok(WorkerMessage::Snapshot {
                snapshot,
                notice: OperationNotice::new(
                    NoticeKind::Success,
                    "Reloaded accounts.",
                ),
            })
        });
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
                Ok(message) => message,
                Err(err) => WorkerMessage::Error {
                    notice: OperationNotice::new(NoticeKind::Error, err.to_string()),
                },
            };
            let _ = sender.send(message);
        });
    }

    fn apply_snapshot(&mut self, snapshot: AppSnapshot) {
        let previous_host_id = self
            .hosts
            .get(self.selected_host_index)
            .map(|host| host.id.clone());
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
            .and_then(|id| self.hosts.iter().position(|host| host.id == id))
            .unwrap_or(0);
        self.prune_limit_cache();
        if matches!(
            self.hosts
                .get(self.selected_host_index)
                .map(|host| &host.target),
            Some(HostTarget::Local { .. }) | None
        ) {
            self.selected_host_profiles = self.profiles.clone();
            self.selected_host_active_auth = self.current_auth.clone();
        }
    }

    fn prune_limit_cache(&mut self) {
        let valid_ids = self
            .profiles
            .iter()
            .map(|profile| profile.id.clone())
            .collect::<HashSet<_>>();
        self.limits_by_profile
            .retain(|profile_id, _| valid_ids.contains(profile_id));
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
    let profiles = profile_store.list_profiles().unwrap_or_default();
    let hosts = profile_store.load_hosts().unwrap_or_default();

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
        HostTarget::Local { .. } => "local".into(),
        HostTarget::Remote(remote) => remote.ssh_alias.clone(),
    }
}
