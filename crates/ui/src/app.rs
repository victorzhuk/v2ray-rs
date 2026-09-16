use std::sync::Mutex;

use adw::prelude::*;
use clap::Parser;
use gtk::glib;
use relm4::adw;
use relm4::prelude::*;
use tokio::sync::broadcast;

use crate::cli::CliArgs;
use v2ray_rs_core::cli::PathOverrides;
use v2ray_rs_core::config::ConfigWriter;
use v2ray_rs_core::geodata::GeodataManager;
use v2ray_rs_core::instance::{
    CompatibilityResult, InstanceLock, InstanceStamp, check_compatibility, reset_instance,
};
use v2ray_rs_core::models::{
    AppSettings, BackendType, ConnectionMetadata, ConnectionNodeRef, DnsConfig, DnsRuleMatch,
    LastSuccessMetadata, ManualNode, RoutingRule, RoutingRuleSet, RuleMatch, Subscription,
    SubscriptionSource, TunConfig, resolve_effective_config,
};
use v2ray_rs_core::persistence::{AppPaths, TunSession};
use v2ray_rs_core::profile::{AppProfile, StdEnv};
use v2ray_rs_core::resolve::{
    ConnectionCandidate, ConnectionPlanner, LatencySnapshot, resolve_candidate,
};
use v2ray_rs_core::rotating_log::{DEFAULT_MAX_BYTES, RotatingFileWriter};
use v2ray_rs_core::runtime_snapshot::RuntimeConfigSnapshot;
use v2ray_rs_process::{PidFile, ProcessEvent, ProcessState};
use v2ray_rs_tray::{TrayAction, TrayHandle};

static TRAY_HANDLE: Mutex<Option<TrayHandle>> = Mutex::new(None);
static TRAY_EVENT_TX: Mutex<Option<broadcast::Sender<ProcessEvent>>> = Mutex::new(None);

const DEFAULT_WINDOW_WIDTH: i32 = 900;
const DEFAULT_WINDOW_HEIGHT: i32 = 650;
const EVENT_CHANNEL_CAPACITY: usize = 16;
const MAX_AUTO_RECONNECTS: u32 = 3;
const AUTO_RECONNECT_DELAY: std::time::Duration = std::time::Duration::from_secs(5);

pub struct AppInit {
    pub paths: AppPaths,
    pub tray_action_rx: Option<tokio::sync::mpsc::UnboundedReceiver<TrayAction>>,
}

use crate::config_preview::{ConfigPreviewDialog, ConfigPreviewInput};
use crate::connection::{ConnectionHandle, ConnectionRequest};
use crate::geodata_service::{GeodataRefreshConfig, GeodataRefreshService};
use crate::logs::{LogsMsg, LogsPage};
use crate::nodes::{NodesMsg, NodesOutput, NodesPage};
use crate::subscriptions::{
    SubscriptionSourceInput, SubscriptionsMsg, SubscriptionsOutput, SubscriptionsPage,
};
use crate::wizard::OnboardingWizard;
use crate::workspace::WorkspaceStore;

pub struct App {
    settings: AppSettings,
    paths: AppPaths,
    store: WorkspaceStore,
    subscriptions_page: Controller<SubscriptionsPage>,
    nodes_page: Controller<NodesPage>,
    logs_page: Controller<LogsPage>,
    show_wizard: bool,
    settings_load_error: Option<String>,
    wizard: Controller<OnboardingWizard>,
    window: adw::ApplicationWindow,
    process_handle: Option<ConnectionHandle>,
    tun_lifecycle: crate::connection::TunLifecycle,
    connection_generation: u64,
    grant_generation: Option<u64>,
    process_state: ProcessState,
    reconnect_pending: bool,
    connected: bool,
    button_sensitive: bool,
    has_active_nodes: bool,
    connection_status: Option<ConnectionMetadata>,
    status_label: gtk::Label,
    status_details: gtk::Label,
    toast_overlay: adw::ToastOverlay,
    preferences_dialog: Option<adw::PreferencesDialog>,
    config_preview: Option<Controller<ConfigPreviewDialog>>,
    runtime_snapshot: Option<RuntimeConfigSnapshot>,
    restart_required: bool,
    current_view: usize,
    geodata_service: GeodataRefreshService,
    pending_exit: bool,
    pending_direct_target: Option<ConnectionNodeRef>,
    session_target: Option<SessionTarget>,
    settings_debounce: Option<glib::SourceId>,
    auto_reconnect_attempts: u32,
    reconnect_generation: u32,
    tun_release_in_flight: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectOrigin {
    /// Started by the user: cancels a pending auto-reconnect.
    User,
    /// Fired by the reconnect timer: keeps the attempt budget intact.
    AutoReconnect,
    /// Applies-and-restarts the anchored session: keeps the chosen node and
    /// falls back to the configured strategy when it no longer resolves.
    Restart,
}

#[derive(Debug)]
pub enum AppMsg {
    OnboardingComplete(AppSettings, Option<(String, SubscriptionSource)>),
    SettingsChanged(AppSettings),
    ToggleConnection,
    Connect(ConnectOrigin),
    ConnectToNode(ConnectionNodeRef, ConnectOrigin),
    Disconnect,
    CloseRequested,
    TrayShowWindow,
    TrayQuit,
    ActiveNodesChanged(bool),
    ProcessStateConnection(u64, ProcessState, Option<ConnectionMetadata>),
    ProcessLogLine(u64, String),
    OpenPreferences(Option<&'static str>),
    ViewGeneratedConfig,
    PreferencesClosed,
    ResetBrokenSettings,
    QuitAfterSettingsError,
    RoutingChanged(RoutingRuleSet),
    ManualNodesChanged,
    SubscriptionsChanged,
    ApplyAndRestart,
    SwitchView(usize),
    ShowToast(String),
    FlushSettings(AppSettings),
    AutoReconnect(u32),
    TunReleased,
    TunGrantRequired(u64),
    DownloadGeodata,
}

impl App {
    fn show_toast(&self, msg: &str) {
        self.toast_overlay.add_toast(adw::Toast::new(msg));
    }

    fn apply_state(&mut self, state: &ProcessState) {
        let from = self.process_state.clone();
        (self.connected, self.button_sensitive) = connect_toggle(state);
        if let ProcessState::Error(msg) = state {
            // An armed TUN grant is consumed here; the caller replaces the
            // plain toast with the actionable one while the sender is alive.
            if error_toast_action(self.connection_generation, self.grant_generation.take())
                .is_none()
            {
                self.show_toast(&format!("Error: {msg}"));
            }
        }
        self.process_state = state.clone();

        let node_ref = if matches!(state, ProcessState::Running) {
            self.connection_status.as_ref().map(|meta| meta.node_ref)
        } else {
            None
        };
        let (sub_active, manual_active) = match node_ref {
            Some(ConnectionNodeRef::Subscription {
                subscription_id,
                node_id,
            }) => (Some((subscription_id, node_id)), None),
            Some(ConnectionNodeRef::Manual { node_id }) => (None, Some(node_id)),
            None => (None, None),
        };
        self.subscriptions_page
            .emit(SubscriptionsMsg::SetActiveNode(sub_active));
        self.nodes_page.emit(NodesMsg::SetActiveNode(manual_active));

        let tun_active = tun_active_for(state, self.runtime_snapshot.as_ref());
        self.subscriptions_page
            .emit(SubscriptionsMsg::SetTunActive(tun_active));

        if let Ok(guard) = TRAY_EVENT_TX.lock()
            && let Some(tx) = guard.as_ref()
        {
            let _ = tx.send(ProcessEvent::StateChanged {
                from,
                to: state.clone(),
                connection: self.connection_status.clone(),
            });
        }
        self.update_status_labels();
    }

    fn update_status_labels(&self) {
        let (primary, details) = match (&self.process_state, &self.connection_status) {
            (ProcessState::Running, Some(meta)) => {
                let latency = meta
                    .latency_ms
                    .map(|ms| format!("{ms} ms"))
                    .unwrap_or_else(|| "n/a".into());
                let details = format!(
                    "{} · {} · {} · {} · {} · since {}",
                    meta.source,
                    meta.node_name,
                    latency,
                    meta.backend,
                    meta.strategy,
                    meta.connected_since.format("%Y-%m-%d %H:%M")
                );
                ("Connected".to_string(), details)
            }
            (ProcessState::Starting, _) => ("Connecting…".to_string(), "Resolving nodes".into()),
            (ProcessState::Stopping, _) => {
                ("Disconnecting…".to_string(), "Stopping backend".into())
            }
            (ProcessState::Error(msg), _) => ("Error".to_string(), msg.clone()),
            _ => (
                "Disconnected".to_string(),
                "No active connection".to_string(),
            ),
        };
        self.status_label.set_text(&primary);
        self.status_details.set_text(&details);
    }

    fn restart_banner_visible(&self) -> bool {
        restart_banner_visible_for_state(self.restart_required, &self.process_state)
    }

    // Deliberately leaves `reconnect_pending` alone: restart flows set it right
    // before dispatching Disconnect, and both Disconnect and the stopped-state
    // handler run this cleanup before the flag is consumed.
    fn clear_restart_flow(&mut self) {
        self.runtime_snapshot = None;
        self.restart_required = false;
    }

    /// Cancels any pending auto-reconnect and resets the attempt budget. Bumping
    /// the generation invalidates timers that were already scheduled.
    fn cancel_auto_reconnect(&mut self) {
        self.auto_reconnect_attempts = 0;
        self.reconnect_generation = self.reconnect_generation.wrapping_add(1);
    }

    /// Schedules a bounded retry after the backend gives up (terminal Error),
    /// so a flaky upstream reconnects on its own and can pick a fresh candidate.
    fn schedule_auto_reconnect(&mut self, sender: &ComponentSender<Self>) -> bool {
        if !auto_reconnect_allowed(self.pending_exit, self.auto_reconnect_attempts) {
            return false;
        }
        self.auto_reconnect_attempts += 1;
        let generation = self.reconnect_generation;
        let s = sender.clone();
        glib::timeout_add_local_once(AUTO_RECONNECT_DELAY, move || {
            s.input(AppMsg::AutoReconnect(generation));
        });
        true
    }

    fn tun_marker_present(&self) -> bool {
        self.paths.tun_session_path().exists()
    }

    /// Releases TUN routes left installed by a session that ended without a
    /// clean stop. Takes the lifecycle lock like the startup pass so it cannot
    /// flush routes from under a connection that is starting.
    fn release_tun_session(&mut self, sender: &ComponentSender<Self>) {
        if self.tun_release_in_flight {
            return;
        }
        self.tun_release_in_flight = true;
        let paths = self.paths.clone();
        let lifecycle = self.tun_lifecycle.clone();
        let s = sender.input_sender().clone();
        tokio::spawn(async move {
            let _lifecycle = lifecycle.lock().await;
            if let Err(failure) =
                recover_tun_session(&paths, &v2ray_rs_process::helper_path()).await
            {
                s.emit(AppMsg::ShowToast(failure.toast()));
            }
            s.emit(AppMsg::TunReleased);
        });
    }

    fn quit(&mut self, sender: &ComponentSender<Self>) {
        self.cancel_auto_reconnect();
        if self.tun_release_in_flight {
            self.pending_exit = true;
            return;
        }
        let plan = quit_plan(
            self.process_handle.is_some(),
            &self.process_state,
            self.tun_marker_present(),
        );
        match plan {
            QuitPlan::Stop => {
                self.pending_exit = true;
                if let Some(handle) = self.process_handle.take() {
                    handle.stop();
                }
            }
            QuitPlan::AwaitStopped => self.pending_exit = true,
            QuitPlan::Release => {
                self.pending_exit = true;
                self.release_tun_session(sender);
            }
            QuitPlan::Exit => self.window.destroy(),
        }
    }

    fn persist_settings(&mut self, settings: AppSettings) -> Result<(), String> {
        self.store
            .save_settings(&settings)
            .map_err(|err| err.to_string())?;
        self.settings = settings;
        Ok(())
    }

    fn load_latency_snapshot_or_default(&self) -> LatencySnapshot {
        match self.store.load_latency_snapshot() {
            Ok(snapshot) => snapshot,
            Err(err) => {
                log::warn!("load latency snapshot: {err}");
                LatencySnapshot::default()
            }
        }
    }

    fn handle_manual_node_changed(&mut self) {
        if runtime_process_active(&self.process_state) {
            self.restart_required = self.check_restart_required();
        } else {
            self.regenerate_config_disconnected();
        }
    }

    fn handle_subscription_changed(&mut self) {
        let subscriptions = match self.store.load_subscriptions() {
            Ok(s) => s,
            Err(err) => {
                log::warn!("load subscriptions: {err}");
                self.has_active_nodes = false;
                return;
            }
        };
        let manual_nodes = self.store.load_manual_nodes_or_default();

        self.has_active_nodes = active_nodes_available(&subscriptions, &manual_nodes);

        self.geodata_service
            .update(GeodataRefreshConfig::from_settings(
                &self.paths,
                &self.settings,
                &self.store.load_routing_rules().unwrap_or_default(),
                &subscriptions,
            ));

        if runtime_process_active(&self.process_state) {
            self.restart_required = self.check_restart_required_with(&subscriptions, &manual_nodes);
        } else {
            self.regenerate_config_with(&subscriptions, &manual_nodes);
        }
    }

    fn check_restart_required(&self) -> bool {
        let subscriptions = match self.store.load_subscriptions() {
            Ok(s) => s,
            Err(_) => return false,
        };
        let manual_nodes = self.store.load_manual_nodes_or_default();
        self.check_restart_required_with(&subscriptions, &manual_nodes)
    }

    fn check_restart_required_with(
        &self,
        subscriptions: &[Subscription],
        manual_nodes: &[ManualNode],
    ) -> bool {
        if !runtime_process_active(&self.process_state) {
            return false;
        }
        let Some(snapshot) = &self.runtime_snapshot else {
            return false;
        };

        let current_rules = match self.store.load_routing_rules() {
            Ok(rules) => rules,
            Err(_) => return false,
        };

        snapshot.diverges_from(&self.settings, &current_rules, manual_nodes, subscriptions)
    }

    fn refresh_has_active_nodes(&mut self) {
        let subscriptions = match self.store.load_subscriptions() {
            Ok(subscriptions) => subscriptions,
            Err(err) => {
                log::warn!("load subscriptions for active state: {err}");
                self.has_active_nodes = false;
                return;
            }
        };
        let manual_nodes = match self.store.load_manual_nodes() {
            Ok(manual_nodes) => manual_nodes,
            Err(err) => {
                log::warn!("load manual nodes for active state: {err}");
                self.has_active_nodes = subscriptions.iter().any(Subscription::has_enabled_nodes);
                return;
            }
        };
        self.has_active_nodes = active_nodes_available(&subscriptions, &manual_nodes);
    }

    fn regenerate_config_disconnected(&mut self) {
        let subscriptions = match self.store.load_subscriptions() {
            Ok(s) => s,
            Err(err) => {
                self.show_toast(&format!("Failed to load subscriptions: {err}"));
                return;
            }
        };
        let manual_nodes = self.store.load_manual_nodes_or_default();
        self.regenerate_config_with(&subscriptions, &manual_nodes);
    }

    fn regenerate_config_with(
        &mut self,
        subscriptions: &[Subscription],
        manual_nodes: &[ManualNode],
    ) {
        let planner = ConnectionPlanner::new(
            self.settings.auto_resolve_strategy,
            self.settings.last_success.clone(),
            self.load_latency_snapshot_or_default(),
        )
        .with_real_delay_for_lowest_latency(self.settings.real_delay.use_for_lowest_latency);
        let Some(candidate) = planner.runtime_candidate(subscriptions, manual_nodes) else {
            log::debug!("No enabled nodes, skipping config regeneration");
            return;
        };

        let rules = self.store.load_routing_rules().unwrap_or_default();
        let enabled_rules: Vec<_> = rules.enabled_rules().cloned().collect();
        let (mut effective_rules, effective_settings) = resolve_effective_config(
            &candidate.node_ref,
            subscriptions,
            &enabled_rules,
            &self.settings,
        );
        let mut nodes = vec![candidate.node.clone()];
        nodes.extend(v2ray_rs_core::resolve::resolve_via_nodes(
            &mut effective_rules,
            subscriptions,
            manual_nodes,
        ));

        let writer = ConfigWriter::new(&self.settings, &self.paths);
        match writer.write_config(&nodes, &effective_rules, &effective_settings) {
            Ok(path) => log::info!("Regenerated config at {:?}", path),
            Err(e) => {
                log::error!("Failed to regenerate config: {}", e);
                self.show_toast(&format!("Failed to regenerate config: {e}"));
            }
        }
    }

    fn load_pruned_latency_snapshot(
        &mut self,
        subscriptions: &[Subscription],
        manual_nodes: &[ManualNode],
    ) -> Result<(LatencySnapshot, bool), String> {
        let mut snapshot = self
            .store
            .load_latency_snapshot()
            .map_err(|err| err.to_string())?;
        let known = v2ray_rs_core::resolve::all_node_refs(subscriptions, manual_nodes);
        let before = snapshot.len();
        snapshot.retain_known(&known);
        let pruned = snapshot.len() != before;
        Ok((snapshot, pruned))
    }

    fn start_connection(
        &mut self,
        candidates: Vec<ConnectionCandidate>,
        subscriptions: Vec<Subscription>,
        manual_nodes: Vec<ManualNode>,
        sender: &ComponentSender<Self>,
    ) -> Result<(), String> {
        let binary_path = match &self.settings.backend.binary_path {
            Some(p) => p.clone(),
            None => {
                self.show_toast("No backend binary configured — check Preferences");
                return Err("no backend binary configured".into());
            }
        };

        let host_has_ipv6 = v2ray_rs_process::host_has_ipv6();
        if tun_ipv6_unavailable(
            &self.settings.tun,
            self.settings.backend.backend_type,
            host_has_ipv6,
        ) {
            self.show_toast(TUN_IPV6_DISABLED);
            return Err(TUN_IPV6_DISABLED.into());
        }

        if let Some(warning) = crate::connection::v2ray_tun_warning(&self.settings) {
            self.show_toast(&warning);
        }

        let rules = match self.store.load_routing_rules() {
            Ok(rules) => rules,
            Err(err) => {
                self.show_toast(&format!("Failed to load routing rules: {err}"));
                return Err(err.to_string());
            }
        };
        let enabled_rules: Vec<_> = rules.enabled_rules().cloned().collect();

        let candidate_subscriptions: Vec<Subscription> = subscriptions
            .iter()
            .filter(|s| {
                candidates.iter().any(|c| match c.node_ref {
                    ConnectionNodeRef::Subscription {
                        subscription_id, ..
                    } => subscription_id == s.id,
                    ConnectionNodeRef::Manual { .. } => false,
                })
            })
            .cloned()
            .collect();
        let geodata = GeodataManager::new(&self.paths);
        if missing_geodata(
            self.settings.backend.backend_type,
            &enabled_rules,
            &candidate_subscriptions,
            &self.settings.dns,
            geodata.geoip_path().exists(),
            geodata.geosite_path().exists(),
        ) {
            let toast = adw::Toast::builder()
                .title("GeoIP/GeoSite rules need geodata that has not been downloaded")
                .button_label("Download geodata")
                .build();
            let s = sender.input_sender().clone();
            toast.connect_button_clicked(move |_| s.emit(AppMsg::DownloadGeodata));
            self.toast_overlay.add_toast(toast);
            return Err("geodata not downloaded".into());
        }

        let connection_subscriptions = subscriptions.clone();
        let connection_manual_nodes = manual_nodes.clone();
        self.connection_generation = self.connection_generation.wrapping_add(1);
        self.grant_generation = None;
        let generation = self.connection_generation;

        self.runtime_snapshot = Some(RuntimeConfigSnapshot {
            backend_type: self.settings.backend.backend_type,
            binary_path: self.settings.backend.binary_path.clone(),
            socks_port: self.settings.socks_port,
            http_port: self.settings.http_port,
            listen_address: self.settings.listen_address.clone(),
            dns: self.settings.dns.clone(),
            routing: rules,
            manual_nodes,
            subscriptions,
            auto_resolve_strategy: self.settings.auto_resolve_strategy,
            use_real_delay_for_lowest_latency: self.settings.real_delay.use_for_lowest_latency,
            tun: self.settings.tun.clone(),
            idle_timeout_secs: self.settings.idle_timeout_secs,
            ws_heartbeat_secs: self.settings.ws_heartbeat_secs,
            timestamp: chrono::Utc::now().timestamp(),
        });

        let writer = ConfigWriter::new(&self.settings, &self.paths);
        let pid_path = self.paths.pid_file_path();
        let geodata_dir = self.paths.geodata_dir();

        self.apply_state(&ProcessState::Starting);
        self.logs_page.emit(LogsMsg::SetRunning(true));
        self.logs_page.emit(LogsMsg::Clear);

        let settings = self.settings.clone();
        let handle = crate::connection::spawn(
            ConnectionRequest {
                binary_path,
                candidates,
                writer,
                paths: self.paths.clone(),
                pid_path,
                geodata_dir,
                settings,
                enabled_rules,
                subscriptions: connection_subscriptions,
                manual_nodes: connection_manual_nodes,
                lifecycle: self.tun_lifecycle.clone(),
                generation,
                host_has_ipv6,
            },
            sender.input_sender().clone(),
        );
        self.process_handle = Some(handle);
        Ok(())
    }
}

#[relm4::component(pub)]
impl SimpleComponent for App {
    type Init = AppInit;
    type Input = AppMsg;
    type Output = ();

    view! {
        adw::ApplicationWindow {
            set_default_width: DEFAULT_WINDOW_WIDTH,
            set_default_height: DEFAULT_WINDOW_HEIGHT,
            set_title: Some("V2Ray Manager"),
            set_icon_name: Some(&model.paths.profile().app_id()),

            connect_close_request[sender] => move |_| {
                sender.input(AppMsg::CloseRequested);
                gtk::glib::Propagation::Stop
            },

            if model.settings_load_error.is_some() {
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_vexpand: true,
                    set_valign: gtk::Align::Center,

                    adw::StatusPage {
                        set_icon_name: Some("dialog-warning-symbolic"),
                        set_title: "Settings File Needs Repair",
                        set_description: model.settings_load_error.as_deref(),
                        set_vexpand: true,
                    },

                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_halign: gtk::Align::Center,
                        set_spacing: 12,
                        set_margin_all: 24,

                        gtk::Button {
                            set_label: "Quit",
                            add_css_class: "pill",
                            connect_clicked => AppMsg::QuitAfterSettingsError,
                        },

                        gtk::Button {
                            set_label: "Reset Settings",
                            add_css_class: "pill",
                            add_css_class: "destructive-action",
                            connect_clicked => AppMsg::ResetBrokenSettings,
                        },
                    },
                }
            } else if model.show_wizard {
                model.wizard.widget().clone() {}
            } else {
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,

                    adw::HeaderBar {
                        #[wrap(Some)]
                        set_title_widget = &adw::WindowTitle {
                            set_title: "V2Ray Manager",
                        },

                        pack_end = &gtk::MenuButton {
                            set_icon_name: "open-menu-symbolic",
                            set_tooltip_text: Some("Main Menu"),
                            #[wrap(Some)]
                            set_popover = &gtk::PopoverMenu::from_model(Some(&{
                                let menu = gtk::gio::Menu::new();
                                menu.append(Some("View Generated Config"), Some("win.view-generated-config"));
                                menu.append(Some("Preferences"), Some("win.preferences"));
                                menu
                            })) {},
                        },
                    },

                    #[local_ref]
                    toast_overlay -> adw::ToastOverlay {
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_vexpand: true,

                            gtk::Box {
                                set_orientation: gtk::Orientation::Horizontal,
                                set_spacing: 12,
                                set_margin_top: 6,
                                set_margin_start: 6,
                                set_margin_end: 6,
                                #[watch]
                                set_visible: model.restart_banner_visible(),

                                adw::Banner {
                                    set_hexpand: true,
                                    set_title: "Configuration changed",
                                    set_button_label: Some("Apply & Restart"),
                                    #[watch]
                                    set_revealed: model.restart_banner_visible(),

                                    connect_button_clicked[sender] => move |_| {
                                        sender.input(AppMsg::ApplyAndRestart);
                                    },
                                },

                            },

                            gtk::Paned {
                            set_orientation: gtk::Orientation::Vertical,
                            set_vexpand: true,
                            set_position: 380,
                            set_shrink_start_child: false,
                            set_shrink_end_child: false,

                            #[wrap(Some)]
                            set_start_child = &gtk::Box {
                                set_orientation: gtk::Orientation::Vertical,
                                set_vexpand: true,

                                gtk::Box {
                                    set_orientation: gtk::Orientation::Horizontal,
                                    set_halign: gtk::Align::Center,
                                    set_margin_top: 6,
                                    set_margin_bottom: 6,

                                    gtk::ToggleButton {
                                        set_label: "Subscriptions",
                                        #[watch]
                                        set_active: model.current_view == 0,
                                        connect_clicked => AppMsg::SwitchView(0),
                                    },

                                    gtk::ToggleButton {
                                        set_label: "Nodes",
                                        #[watch]
                                        set_active: model.current_view == 1,
                                        connect_clicked => AppMsg::SwitchView(1),
                                    },
                                },

                                #[name = "pane_stack"]
                                gtk::Stack {
                                    set_hexpand: true,
                                    set_vexpand: true,

                                    #[watch]
                                    set_visible_child_name: if model.current_view == 0 {
                                        "subscriptions"
                                    } else {
                                        "nodes"
                                    },
                                },
                            },

                            #[wrap(Some)]
                            set_end_child = model.logs_page.widget(),
                        },
                    },
                },

                gtk::ActionBar {
                        set_hexpand: true,

                        pack_start = &gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_spacing: 2,
                            set_margin_top: 6,
                            set_margin_bottom: 6,
                            set_margin_start: 12,

                            #[local_ref]
                            status_label -> gtk::Label {
                                set_xalign: 0.0,
                                add_css_class: "title-4",
                            },

                            #[local_ref]
                            status_details -> gtk::Label {
                                set_xalign: 0.0,
                                add_css_class: "caption",
                            },
                        },

                        pack_end = &gtk::Button {
                            set_margin_top: 6,
                            set_margin_bottom: 6,
                            set_margin_end: 12,
                            #[wrap(Some)]
                            set_child = &adw::ButtonContent {
                                #[watch]
                                set_icon_name: if model.connected {
                                    "network-wired-disconnected-symbolic"
                                } else {
                                    "network-wired-symbolic"
                                },
                                #[watch]
                                set_label: if model.connected { "Disconnect" } else { "Connect" },
                            },
                            #[watch]
                            set_sensitive: model.button_sensitive && (model.connected || model.has_active_nodes),
                            #[watch]
                            set_tooltip_text: Some(if !model.connected && !model.has_active_nodes {
                                "No enabled proxy nodes"
                            } else if model.connected {
                                "Disconnect from proxy"
                            } else {
                                "Connect to proxy"
                            }),
                            #[watch]
                            set_css_classes: &["pill", if model.connected { "destructive-action" } else { "suggested-action" }],
                            connect_clicked => AppMsg::ToggleConnection,
                        },
                    },
                }
            }
        }
    }

    fn init(
        init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let AppInit {
            paths,
            tray_action_rx,
        } = init;
        if let Some(mut rx) = tray_action_rx {
            let s = sender.input_sender().clone();
            glib::spawn_future_local(async move {
                while let Some(action) = rx.recv().await {
                    match action {
                        TrayAction::ShowWindow => s.emit(AppMsg::TrayShowWindow),
                        TrayAction::Quit => s.emit(AppMsg::TrayQuit),
                        TrayAction::Connect => s.emit(AppMsg::Connect(ConnectOrigin::User)),
                        TrayAction::Disconnect => s.emit(AppMsg::Disconnect),
                    }
                }
            });
        }

        let store = WorkspaceStore::new(paths.clone());
        let (settings, settings_load_error) = match store.load_settings() {
            Ok(settings) => (settings, None),
            Err(err) => {
                log::error!("load settings: {err}");
                (AppSettings::default(), Some(err.to_string()))
            }
        };
        // Off the GTK thread: orphan reaping can wait ~1.5s on a stubborn
        // process and TUN recovery up to HELPER_TIMEOUT. Recovery flushes the
        // tunnel routing table wholesale, so it takes the lifecycle lock: an
        // early Connect must queue behind it rather than have its fresh rules
        // flushed out from under it.
        let tun_lifecycle: crate::connection::TunLifecycle = Default::default();
        {
            let bg_paths = paths.clone();
            let skip_orphans = settings_load_error.is_some();
            let lifecycle = tun_lifecycle.clone();
            let s = sender.input_sender().clone();
            tokio::spawn(async move {
                let _lifecycle = lifecycle.lock().await;
                let orphan_paths = bg_paths.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    if !skip_orphans && let Err(err) = cleanup_orphaned_backend(&orphan_paths) {
                        log::warn!("failed to clean orphaned backend process: {err}");
                    }
                })
                .await;
                if let Err(failure) =
                    recover_tun_session(&bg_paths, &v2ray_rs_process::helper_path()).await
                {
                    s.emit(AppMsg::ShowToast(failure.toast()));
                }
            });
        }

        let show_wizard = settings_load_error.is_none() && !settings.onboarding_complete;

        let subscriptions_page = SubscriptionsPage::builder()
            .launch((store.clone(), settings.clone()))
            .forward(sender.input_sender(), |msg| match msg {
                SubscriptionsOutput::ActiveNodesChanged(has) => AppMsg::ActiveNodesChanged(has),
                SubscriptionsOutput::SubscriptionsChanged => AppMsg::SubscriptionsChanged,
                SubscriptionsOutput::ConnectNode(sub_id, node_id) => AppMsg::ConnectToNode(
                    ConnectionNodeRef::Subscription {
                        subscription_id: sub_id,
                        node_id,
                    },
                    ConnectOrigin::User,
                ),
                SubscriptionsOutput::Notice(message) => AppMsg::ShowToast(message),
            });

        let nodes_page = NodesPage::builder()
            .launch((store.clone(), settings.clone()))
            .forward(sender.input_sender(), |msg| match msg {
                NodesOutput::ActiveNodesChanged(has) => AppMsg::ActiveNodesChanged(has),
                NodesOutput::NodesChanged => AppMsg::ManualNodesChanged,
                NodesOutput::ConnectNode(node_id) => AppMsg::ConnectToNode(
                    ConnectionNodeRef::Manual { node_id },
                    ConnectOrigin::User,
                ),
                NodesOutput::Notice(message) => AppMsg::ShowToast(message),
            });

        let logs_page = LogsPage::builder().launch(()).detach();

        let wizard = OnboardingWizard::builder()
            .launch(())
            .forward(sender.input_sender(), |msg| match msg {
                crate::wizard::WizardOutput::Complete {
                    settings,
                    subscription,
                } => AppMsg::OnboardingComplete(settings, subscription),
            });

        let toast_overlay = adw::ToastOverlay::new();
        let status_label = gtk::Label::new(None);
        let status_details = gtk::Label::new(None);

        let subscriptions = store.load_subscriptions().unwrap_or_else(|err| {
            log::warn!("load subscriptions for init: {err}");
            Vec::new()
        });
        let manual_nodes = store.load_manual_nodes().unwrap_or_else(|err| {
            log::warn!("load manual nodes for init: {err}");
            Vec::new()
        });
        let has_active_nodes = active_nodes_available(&subscriptions, &manual_nodes);
        let (geodata_toasts, mut geodata_toast_rx) =
            tokio::sync::mpsc::unbounded_channel::<String>();
        let geodata_service = GeodataRefreshService::spawn(
            GeodataRefreshConfig::from_settings(
                &paths,
                &settings,
                &store.load_routing_rules().unwrap_or_default(),
                &subscriptions,
            ),
            geodata_toasts,
        );
        let geodata_toast_forwarder = sender.clone();
        tokio::spawn(async move {
            while let Some(toast) = geodata_toast_rx.recv().await {
                geodata_toast_forwarder.input(AppMsg::ShowToast(toast));
            }
        });

        let model = App {
            settings,
            paths,
            store,
            subscriptions_page,
            nodes_page,
            logs_page,
            show_wizard,
            settings_load_error,
            wizard,
            window: root.clone(),
            process_handle: None,
            tun_lifecycle,
            connection_generation: 0,
            grant_generation: None,
            process_state: ProcessState::Stopped,
            reconnect_pending: false,
            connected: false,
            button_sensitive: true,
            has_active_nodes,
            connection_status: None,
            status_label: status_label.clone(),
            status_details: status_details.clone(),
            toast_overlay: toast_overlay.clone(),
            preferences_dialog: None,
            config_preview: None,
            runtime_snapshot: None,
            restart_required: false,
            current_view: 0,
            geodata_service,
            pending_exit: false,
            pending_direct_target: None,
            session_target: None,
            settings_debounce: None,
            auto_reconnect_attempts: 0,
            reconnect_generation: 0,
            tun_release_in_flight: false,
        };

        let toast_overlay = &model.toast_overlay;
        let status_label = &model.status_label;
        let status_details = &model.status_details;
        let widgets = view_output!();
        widgets
            .pane_stack
            .add_named(model.subscriptions_page.widget(), Some("subscriptions"));
        widgets
            .pane_stack
            .add_named(model.nodes_page.widget(), Some("nodes"));
        model.update_status_labels();

        let prefs_action = gtk::gio::SimpleAction::new("preferences", None);
        {
            let s = sender.input_sender().clone();
            prefs_action.connect_activate(move |_, _| {
                s.emit(AppMsg::OpenPreferences(None));
            });
        }
        root.add_action(&prefs_action);

        let view_config_action = gtk::gio::SimpleAction::new("view-generated-config", None);
        {
            let s = sender.input_sender().clone();
            view_config_action.connect_activate(move |_, _| {
                s.emit(AppMsg::ViewGeneratedConfig);
            });
        }
        root.add_action(&view_config_action);

        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>) {
        match msg {
            AppMsg::OnboardingComplete(settings, subscription) => {
                if let Err(err) = self.persist_settings(settings) {
                    log::error!("save settings: {err}");
                    self.show_toast(&format!("Failed to save settings: {err}"));
                    return;
                }
                self.show_wizard = false;
                self.settings_load_error = None;
                self.geodata_service
                    .update(GeodataRefreshConfig::from_settings(
                        &self.paths,
                        &self.settings,
                        &self.store.load_routing_rules().unwrap_or_default(),
                        &self.store.load_subscriptions().unwrap_or_default(),
                    ));
                self.subscriptions_page
                    .emit(SubscriptionsMsg::SyncSettings {
                        auto_update_enabled: self.settings.auto_update_subscriptions,
                        auto_update_interval_secs: self.settings.subscription_update_interval_secs,
                        backend_type: self.settings.backend.backend_type,
                        binary_path: self.settings.backend.binary_path.clone(),
                        real_delay_settings: self.settings.real_delay.clone(),
                    });

                if let Some((name, source)) = subscription {
                    let source_input = match source {
                        SubscriptionSource::Url { url } => SubscriptionSourceInput::Url(url),
                        SubscriptionSource::File { path } => SubscriptionSourceInput::File(path),
                    };
                    self.subscriptions_page
                        .emit(SubscriptionsMsg::AddSubscription(name, source_input));
                }
            }
            AppMsg::SettingsChanged(settings) => {
                if let Some(id) = self.settings_debounce.take() {
                    id.remove();
                }
                let s = sender.clone();
                self.settings_debounce = Some(glib::timeout_add_local_once(
                    std::time::Duration::from_millis(300),
                    move || s.input(AppMsg::FlushSettings(settings)),
                ));
            }
            AppMsg::FlushSettings(settings) => {
                self.settings_debounce = None;
                let settings = keep_last_success(settings, &self.settings);
                if let Err(err) = self.persist_settings(settings) {
                    log::error!("save settings: {err}");
                    self.show_toast(&format!("Failed to save settings: {err}"));
                    return;
                }
                crate::i18n::switch_language(self.settings.language);
                update_tray_notification_setting(self.settings.notifications_enabled);
                self.geodata_service
                    .update(GeodataRefreshConfig::from_settings(
                        &self.paths,
                        &self.settings,
                        &self.store.load_routing_rules().unwrap_or_default(),
                        &self.store.load_subscriptions().unwrap_or_default(),
                    ));
                self.subscriptions_page
                    .emit(SubscriptionsMsg::SyncSettings {
                        auto_update_enabled: self.settings.auto_update_subscriptions,
                        auto_update_interval_secs: self.settings.subscription_update_interval_secs,
                        backend_type: self.settings.backend.backend_type,
                        binary_path: self.settings.backend.binary_path.clone(),
                        real_delay_settings: self.settings.real_delay.clone(),
                    });
                self.restart_required = self.check_restart_required();
                if self.process_handle.is_none() {
                    self.regenerate_config_disconnected();
                }
            }
            AppMsg::ActiveNodesChanged(_has) => {
                self.refresh_has_active_nodes();
            }
            AppMsg::ShowToast(message) => {
                self.show_toast(&message);
            }
            AppMsg::DownloadGeodata => {
                let paths = self.paths.clone();
                let backend = self.settings.backend.backend_type;
                let s = sender.input_sender().clone();
                tokio::spawn(async move {
                    let message = match tokio::task::spawn_blocking(move || {
                        crate::geodata_service::update_geodata(&paths, backend)
                    })
                    .await
                    {
                        Ok(Ok(())) => "Geodata updated successfully".to_string(),
                        Ok(Err(err)) => err,
                        Err(err) => format!("Geodata download task failed: {err}"),
                    };
                    s.emit(AppMsg::ShowToast(message));
                });
            }
            AppMsg::ToggleConnection => {
                if self.connected {
                    sender.input(AppMsg::Disconnect);
                } else {
                    sender.input(AppMsg::Connect(ConnectOrigin::User));
                }
            }
            AppMsg::Connect(origin) => {
                if self.process_handle.is_some() || self.tun_release_in_flight {
                    return;
                }

                let subscriptions = match self.store.load_subscriptions() {
                    Ok(subscriptions) => subscriptions,
                    Err(err) => {
                        self.show_toast(&format!("Failed to load subscriptions: {err}"));
                        return;
                    }
                };
                let manual_nodes = match self.store.load_manual_nodes() {
                    Ok(manual_nodes) => manual_nodes,
                    Err(err) => {
                        self.show_toast(&format!("Failed to load manual nodes: {err}"));
                        return;
                    }
                };
                let (snapshot, pruned) =
                    match self.load_pruned_latency_snapshot(&subscriptions, &manual_nodes) {
                        Ok(result) => result,
                        Err(err) => {
                            self.show_toast(&format!("Failed to load latency data: {err}"));
                            return;
                        }
                    };
                if pruned && let Err(err) = self.store.save_latency_snapshot(&snapshot) {
                    log::warn!("prune latency snapshot: {err}");
                }

                let planner = ConnectionPlanner::new(
                    self.settings.auto_resolve_strategy,
                    self.settings.last_success.clone(),
                    snapshot,
                )
                .with_real_delay_for_lowest_latency(
                    self.settings.real_delay.use_for_lowest_latency,
                );
                let candidates = planner.plan(&subscriptions, &manual_nodes);

                if candidates.is_empty() {
                    self.show_toast(
                        "No enabled proxy nodes — add a subscription or manual node first",
                    );
                    return;
                }

                if cancels_auto_reconnect(origin) {
                    self.cancel_auto_reconnect();
                }
                let _ = self.start_connection(candidates, subscriptions, manual_nodes, &sender);
                self.session_target = None;
            }
            AppMsg::ConnectToNode(target, origin) => {
                if self.tun_release_in_flight {
                    return;
                }
                // Resolve the requested node before touching any connection state.
                // An invalid target must toast and exit without canceling reconnects,
                // setting a pending target, or tearing down an existing session.
                let subscriptions = match self.store.load_subscriptions() {
                    Ok(subscriptions) => subscriptions,
                    Err(err) => {
                        self.show_toast(&format!("Failed to load subscriptions: {err}"));
                        return;
                    }
                };
                let manual_nodes = match self.store.load_manual_nodes() {
                    Ok(manual_nodes) => manual_nodes,
                    Err(err) => {
                        self.show_toast(&format!("Failed to load manual nodes: {err}"));
                        return;
                    }
                };
                let (snapshot, _) =
                    match self.load_pruned_latency_snapshot(&subscriptions, &manual_nodes) {
                        Ok(result) => result,
                        Err(err) => {
                            self.show_toast(&format!("Failed to load latency data: {err}"));
                            return;
                        }
                    };

                let Some(candidate) =
                    resolve_candidate(target, &subscriptions, &manual_nodes, &snapshot)
                else {
                    if falls_back_to_planner(origin) {
                        self.show_toast(
                            "Chosen node is unavailable, reconnecting with the configured strategy",
                        );
                        self.session_target = None;
                        sender.input(AppMsg::Connect(origin));
                    } else {
                        self.show_toast("Node not available or disabled");
                    }
                    return;
                };

                if self.process_handle.is_some() {
                    if cancels_auto_reconnect(origin) {
                        self.cancel_auto_reconnect();
                    }
                    self.reconnect_pending = false;
                    self.pending_direct_target = Some(target);
                    sender.input(AppMsg::Disconnect);
                    return;
                }

                if cancels_auto_reconnect(origin) {
                    self.cancel_auto_reconnect();
                }
                self.reconnect_pending = false;

                self.session_target = self
                    .start_connection(vec![candidate], subscriptions, manual_nodes, &sender)
                    .ok()
                    .map(|_| direct_session(target, origin));
            }
            AppMsg::Disconnect => {
                let reconnect_pending = self.reconnect_pending;
                self.session_target =
                    session_target_after_stop(self.session_target, reconnect_pending);
                self.clear_restart_flow();
                self.cancel_auto_reconnect();
                match disconnect_plan(self.process_handle.is_some(), self.tun_marker_present()) {
                    DisconnectPlan::Stop => {
                        if let Some(handle) = self.process_handle.take() {
                            self.apply_state(&ProcessState::Stopping);
                            handle.stop();
                        }
                    }
                    DisconnectPlan::Release => {
                        self.reconnect_pending = false;
                        self.apply_state(&ProcessState::Stopped);
                        self.release_tun_session(&sender);
                    }
                    DisconnectPlan::Nothing => {
                        self.reconnect_pending = false;
                        self.show_toast("Not connected");
                    }
                }
            }
            AppMsg::ProcessStateConnection(generation, state, connection) => {
                // A superseded connection keeps reporting until its teardown
                // finishes; acting on its terminal state would clear the handle
                // of the connection that replaced it.
                if !is_current_generation(generation, self.connection_generation) {
                    return;
                }
                let was_stopping = matches!(self.process_state, ProcessState::Stopping);
                let stopped = matches!(state, ProcessState::Stopped | ProcessState::Error(_));
                if stopped {
                    self.process_handle = None;
                    self.logs_page.emit(LogsMsg::SetRunning(false));
                    self.clear_restart_flow();
                    // Clear the TUN recovery marker only on a clean stop. On
                    // Error the routes stay installed as a kill switch while a
                    // retry follows; the release pass clears it on give-up.
                    if matches!(state, ProcessState::Stopped) {
                        let _ = v2ray_rs_core::persistence::clear_tun_session(&self.paths);
                    }
                }
                if connection.is_some() {
                    self.connection_status = connection;
                    if let Some(meta) = &self.connection_status {
                        let settings = last_success_settings(&self.settings, meta);
                        if let Err(err) = self.persist_settings(settings) {
                            log::error!("save settings: {err}");
                        }
                    }
                } else if matches!(state, ProcessState::Stopped | ProcessState::Error(_)) {
                    self.connection_status = None;
                }
                let grant_action = error_toast_action(generation, self.grant_generation);
                self.apply_state(&state);
                if let (Some(ToastAction::GrantTun), ProcessState::Error(msg)) =
                    (grant_action, &state)
                {
                    let toast = adw::Toast::builder()
                        .title(format!("Error: {msg}"))
                        .button_label("Grant TUN privileges")
                        .build();
                    let s = sender.input_sender().clone();
                    toast.connect_button_clicked(move |_| {
                        s.emit(AppMsg::OpenPreferences(Some(
                            crate::preferences::TUN_PAGE_NAME,
                        )));
                    });
                    self.toast_overlay.add_toast(toast);
                }
                if stopped && self.pending_exit {
                    if matches!(state, ProcessState::Error(_)) && self.tun_marker_present() {
                        self.release_tun_session(&sender);
                        return;
                    }
                    self.pending_exit = false;
                    self.window.destroy();
                    return;
                }
                if let Some(target) =
                    consume_terminal_direct_state(&state, &mut self.pending_direct_target)
                {
                    self.reconnect_pending = false;
                    self.cancel_auto_reconnect();
                    sender.input(AppMsg::ConnectToNode(target, ConnectOrigin::User));
                    return;
                }
                if stopped && !self.reconnect_pending {
                    self.regenerate_config_disconnected();
                }
                if reconnect_after_stop(&state, self.reconnect_pending) {
                    self.reconnect_pending = false;
                    sender.input(reconnect_msg(self.session_target, ConnectOrigin::Restart));
                } else {
                    match &state {
                        ProcessState::Running => {
                            self.cancel_auto_reconnect();
                            self.session_target = session_target_after_running(self.session_target);
                        }
                        ProcessState::Error(_) => {
                            let left =
                                MAX_AUTO_RECONNECTS.saturating_sub(self.auto_reconnect_attempts);
                            let retry = retry_after_error(self.session_target, was_stopping, left)
                                && self.schedule_auto_reconnect(&sender);
                            if !retry {
                                self.session_target = None;
                                if self.tun_marker_present() {
                                    self.release_tun_session(&sender);
                                }
                            }
                        }
                        ProcessState::Stopped => {
                            self.session_target = session_target_after_stop(
                                self.session_target,
                                self.reconnect_pending,
                            );
                        }
                        _ => {}
                    }
                }
            }
            AppMsg::AutoReconnect(generation) => {
                if auto_reconnect_fires(
                    generation,
                    self.reconnect_generation,
                    self.process_handle.is_some(),
                ) {
                    sender.input(reconnect_msg(
                        self.session_target,
                        ConnectOrigin::AutoReconnect,
                    ));
                }
            }
            AppMsg::TunReleased => {
                self.tun_release_in_flight = false;
                if self.pending_exit {
                    self.pending_exit = false;
                    self.window.destroy();
                }
            }
            AppMsg::TunGrantRequired(generation) => {
                // A superseded connection's grant report arms nothing.
                if is_current_generation(generation, self.connection_generation) {
                    self.grant_generation = Some(generation);
                }
            }
            AppMsg::ProcessLogLine(generation, line) => {
                // A superseded connection keeps streaming until its teardown
                // finishes; drop what it logged meanwhile.
                if !is_current_generation(generation, self.connection_generation) {
                    return;
                }
                self.logs_page.emit(LogsMsg::AppendLine(line));
            }
            AppMsg::CloseRequested => {
                if self.settings.minimize_to_tray && tray_available() {
                    self.window.set_visible(false);
                } else {
                    self.quit(&sender);
                }
            }
            AppMsg::TrayShowWindow => {
                self.window.set_visible(true);
                self.window.present();
            }
            AppMsg::TrayQuit => {
                self.quit(&sender);
            }
            AppMsg::OpenPreferences(page) => {
                if let Some(dialog) = &self.preferences_dialog {
                    dialog.present(Some(&self.window));
                    if let Some(name) = page {
                        dialog.set_visible_page_name(name);
                    }
                    return;
                }

                let settings = self.settings.clone();
                let window = self.window.clone();
                let s = sender.input_sender().clone();
                let s1 = s.clone();
                let toast_overlay = self.toast_overlay.clone();
                let dialog = crate::preferences::show_preferences(
                    &window,
                    &self.store,
                    &settings,
                    move |new_settings| {
                        s.emit(AppMsg::SettingsChanged(new_settings));
                    },
                    move |rules| {
                        s1.emit(AppMsg::RoutingChanged(rules));
                    },
                    move |msg| {
                        toast_overlay.add_toast(adw::Toast::new(msg));
                    },
                );
                if let Some(name) = page {
                    dialog.set_visible_page_name(name);
                }
                {
                    let s = sender.input_sender().clone();
                    dialog.connect_closed(move |_| {
                        s.emit(AppMsg::PreferencesClosed);
                    });
                }
                self.preferences_dialog = Some(dialog);
            }
            AppMsg::PreferencesClosed => {
                self.preferences_dialog = None;
            }
            AppMsg::ViewGeneratedConfig => {
                let backend = self.settings.backend.backend_type;
                let writer = ConfigWriter::new(&self.settings, &self.paths);
                let path = writer.output_path(backend);
                if let Some(dialog) = &self.config_preview {
                    dialog.emit(ConfigPreviewInput::SetPath(path));
                    dialog.widget().present(Some(&self.window));
                    return;
                }

                let window = self.window.clone();
                let dialog = ConfigPreviewDialog::builder().launch(path).detach();
                dialog.widget().present(Some(&window));
                self.config_preview = Some(dialog);
            }
            AppMsg::ResetBrokenSettings => match std::fs::remove_file(self.paths.settings_path()) {
                Ok(()) => {
                    self.settings = AppSettings::default();
                    self.settings_load_error = None;
                    self.show_wizard = true;
                    self.geodata_service
                        .update(GeodataRefreshConfig::from_settings(
                            &self.paths,
                            &self.settings,
                            &self.store.load_routing_rules().unwrap_or_default(),
                            &self.store.load_subscriptions().unwrap_or_default(),
                        ));
                    self.subscriptions_page
                        .emit(SubscriptionsMsg::SyncSettings {
                            auto_update_enabled: self.settings.auto_update_subscriptions,
                            auto_update_interval_secs: self
                                .settings
                                .subscription_update_interval_secs,
                            backend_type: self.settings.backend.backend_type,
                            binary_path: self.settings.backend.binary_path.clone(),
                            real_delay_settings: self.settings.real_delay.clone(),
                        });
                }
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    self.settings = AppSettings::default();
                    self.settings_load_error = None;
                    self.show_wizard = true;
                }
                Err(err) => {
                    self.show_toast(&format!("Failed to reset settings: {err}"));
                }
            },
            AppMsg::QuitAfterSettingsError => {
                self.window.destroy();
            }
            AppMsg::RoutingChanged(rules) => {
                if let Err(err) = self.store.save_routing_rules(&rules) {
                    log::error!("save routing rules: {err}");
                    self.show_toast(&format!("Failed to save routing rules: {err}"));
                    return;
                }
                log::info!("Routing rules changed");
                self.geodata_service
                    .update(GeodataRefreshConfig::from_settings(
                        &self.paths,
                        &self.settings,
                        &rules,
                        &self.store.load_subscriptions().unwrap_or_default(),
                    ));
                self.restart_required = self.check_restart_required();
                if self.process_handle.is_none() {
                    self.regenerate_config_disconnected();
                }
            }
            AppMsg::ManualNodesChanged => {
                self.refresh_has_active_nodes();
                self.handle_manual_node_changed();
            }
            AppMsg::SubscriptionsChanged => {
                self.handle_subscription_changed();
            }
            AppMsg::ApplyAndRestart => {
                self.restart_required = false;
                self.reconnect_pending = true;
                sender.input(AppMsg::Disconnect);
            }
            AppMsg::SwitchView(view_index) => {
                self.current_view = view_index;
            }
        }
    }
}

fn runtime_process_active(state: &ProcessState) -> bool {
    matches!(state, ProcessState::Starting | ProcessState::Running)
}

fn restart_banner_visible_for_state(restart_required: bool, state: &ProcessState) -> bool {
    restart_required && runtime_process_active(state)
}

fn reconnect_after_stop(state: &ProcessState, reconnect_pending: bool) -> bool {
    reconnect_pending && matches!(state, ProcessState::Stopped | ProcessState::Error(_))
}

#[derive(Debug, PartialEq, Eq)]
enum QuitPlan {
    Stop,
    AwaitStopped,
    Release,
    Exit,
}

/// A Disconnect already took the handle while the app is `Stopping`; the
/// window must outlive that stop so its teardown and marker clear finish.
fn quit_plan(has_handle: bool, state: &ProcessState, marker_present: bool) -> QuitPlan {
    if has_handle {
        QuitPlan::Stop
    } else if matches!(state, ProcessState::Stopping) {
        QuitPlan::AwaitStopped
    } else if marker_present {
        QuitPlan::Release
    } else {
        QuitPlan::Exit
    }
}

/// Returns `(shows_disconnect, sensitive)`. `Starting` offers Disconnect so a
/// slow connect can be cancelled; `Stopping` has nothing left to cancel.
fn connect_toggle(state: &ProcessState) -> (bool, bool) {
    match state {
        ProcessState::Stopped | ProcessState::Error(_) => (false, true),
        ProcessState::Starting | ProcessState::Running => (true, true),
        ProcessState::Stopping => (true, false),
    }
}

fn auto_reconnect_allowed(pending_exit: bool, attempts: u32) -> bool {
    !pending_exit && attempts < MAX_AUTO_RECONNECTS
}

/// Only user- and restart-initiated connects invalidate a pending
/// auto-reconnect; the timer's own connect must leave the attempt budget
/// counting toward MAX_AUTO_RECONNECTS.
fn cancels_auto_reconnect(origin: ConnectOrigin) -> bool {
    origin != ConnectOrigin::AutoReconnect
}

/// The message a terminal state reconnects with: an anchored session goes
/// back to its own node; a plan without a target replans.
fn reconnect_msg(target: Option<SessionTarget>, origin: ConnectOrigin) -> AppMsg {
    match target {
        Some(session) => AppMsg::ConnectToNode(session.node, origin),
        None => AppMsg::Connect(origin),
    }
}

/// A lost node surfaces to the user only when the user picked it; restarts
/// and auto-reconnects fall back to the configured strategy instead.
fn falls_back_to_planner(origin: ConnectOrigin) -> bool {
    origin != ConnectOrigin::User
}

/// The scheduled timer fires only for the generation it was armed with and
/// only while no connection is up.
fn auto_reconnect_fires(message: u32, current: u32, has_handle: bool) -> bool {
    message == current && !has_handle
}

/// Log and state messages carry the generation of the connection that
/// produced them; only the app's current one is live.
fn is_current_generation(message: u64, current: u64) -> bool {
    message == current
}

/// The extra affordance an Error toast can carry.
#[derive(Debug, PartialEq, Eq)]
enum ToastAction {
    GrantTun,
}

/// The Error toast grows a "Grant TUN privileges" button only when the
/// failed connection is the one whose TUN capability grant was armed.
fn error_toast_action(generation: u64, grant_generation: Option<u64>) -> Option<ToastAction> {
    (grant_generation == Some(generation)).then_some(ToastAction::GrantTun)
}

const TUN_IPV6_DISABLED: &str = "TUN IPv6 address is set but the kernel has IPv6 disabled (ipv6.disable=1); clear the IPv6 address in TUN settings";

fn tun_ipv6_unavailable(tun: &TunConfig, backend: BackendType, host_has_ipv6: bool) -> bool {
    tun.enabled
        && matches!(backend, BackendType::SingBox | BackendType::Xray)
        && tun.address_v6.is_some()
        && !host_has_ipv6
}

/// v2ray and xray read `geoip:`/`geosite:` references from local .dat files and
/// refuse to start without them; sing-box rule-sets are fetched per tag elsewhere.
fn missing_geodata(
    backend: BackendType,
    rules: &[RoutingRule],
    subscriptions: &[Subscription],
    dns: &DnsConfig,
    geoip_exists: bool,
    geosite_exists: bool,
) -> bool {
    if backend == BackendType::SingBox || (geoip_exists && geosite_exists) {
        return false;
    }
    let geo_rule = |r: &RoutingRule| {
        r.enabled
            && matches!(
                r.match_condition,
                RuleMatch::GeoIp { .. } | RuleMatch::GeoSite { .. }
            )
    };
    let geo_dns = |d: &DnsConfig| {
        d.enabled
            && d.use_custom_rules
            && d.rules
                .iter()
                .any(|r| matches!(r.match_condition, DnsRuleMatch::GeoSite { .. }))
    };
    rules.iter().any(geo_rule)
        || geo_dns(dns)
        || subscriptions
            .iter()
            .filter(|s| s.use_imported_profile)
            .filter_map(|s| s.imported_profile.as_ref())
            .any(|p| p.rules.iter().any(geo_rule) || p.dns.as_ref().is_some_and(geo_dns))
}

/// TUN follows the session that is running: the launched snapshot decides,
/// not the current settings, which may already have been edited mid-session.
fn tun_active_for(state: &ProcessState, snapshot: Option<&RuntimeConfigSnapshot>) -> bool {
    matches!(state, ProcessState::Running)
        && snapshot.is_some_and(|s| {
            s.tun.enabled && s.backend_type != v2ray_rs_core::models::BackendType::V2ray
        })
}

#[derive(Debug, PartialEq, Eq)]
enum DisconnectPlan {
    Stop,
    Release,
    Nothing,
}

fn disconnect_plan(has_handle: bool, marker_present: bool) -> DisconnectPlan {
    if has_handle {
        DisconnectPlan::Stop
    } else if marker_present {
        DisconnectPlan::Release
    } else {
        DisconnectPlan::Nothing
    }
}

/// An Error that arrives while the user is disconnecting, or after the
/// automatic reconnect budget is spent, gets no retry, so the kill-switch
/// routes must be released.
fn release_on_error(app_state_stopping: bool, reconnects_left: u32) -> bool {
    app_state_stopping || reconnects_left == 0
}

/// The node the current session attempt is anchored to, and whether it ever
/// reached `Running`. An established target survives backend errors and is
/// retried within the auto-reconnect budget; an unestablished one surfaces
/// the failure immediately. A session the user ended drops it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SessionTarget {
    node: ConnectionNodeRef,
    established: bool,
}

/// A direct connect starts unestablished; only the reconnect timer re-enters
/// a session that had already been established.
fn direct_session(node: ConnectionNodeRef, origin: ConnectOrigin) -> SessionTarget {
    SessionTarget {
        node,
        established: origin == ConnectOrigin::AutoReconnect,
    }
}

/// `Running` anchors the session: its target becomes established.
fn session_target_after_running(target: Option<SessionTarget>) -> Option<SessionTarget> {
    target.map(|mut session| {
        session.established = true;
        session
    })
}

/// A stop keeps the target only while a restart will follow; a session the
/// user ended drops it.
fn session_target_after_stop(
    target: Option<SessionTarget>,
    reconnect_pending: bool,
) -> Option<SessionTarget> {
    if reconnect_pending { target } else { None }
}

/// An unestablished direct target must surface its failure instead of
/// retrying; established and planned sessions follow the normal release
/// decision.
fn retry_after_error(
    target: Option<SessionTarget>,
    app_state_stopping: bool,
    reconnects_left: u32,
) -> bool {
    match target {
        Some(session) if !session.established => false,
        _ => !release_on_error(app_state_stopping, reconnects_left),
    }
}

/// Pure helper for terminal-state direct-connect bookkeeping: a terminal
/// `Stopped` or `Error` replays a pending direct target exactly once, so the
/// revalidation path decides the next state.
fn consume_terminal_direct_state(
    state: &ProcessState,
    pending_direct_target: &mut Option<ConnectionNodeRef>,
) -> Option<ConnectionNodeRef> {
    match state {
        ProcessState::Stopped | ProcessState::Error(_) => pending_direct_target.take(),
        _ => None,
    }
}

/// A settings flush from the preferences dialog carries a copy taken when the
/// dialog opened; a connection may have refreshed `last_success` since, so the
/// stale copy must not overwrite the app's current record.
fn keep_last_success(mut incoming: AppSettings, current: &AppSettings) -> AppSettings {
    incoming.last_success = current.last_success.clone();
    incoming
}

/// A session that reached `Running` records which node last served traffic,
/// seeding the `LastSuccessful` auto-resolve strategy.
fn last_success_settings(current: &AppSettings, meta: &ConnectionMetadata) -> AppSettings {
    let mut settings = current.clone();
    settings.last_success = Some(LastSuccessMetadata {
        node_ref: meta.node_ref,
        connected_at: meta.connected_since,
    });
    settings
}

fn active_nodes_available(subscriptions: &[Subscription], manual_nodes: &[ManualNode]) -> bool {
    subscriptions
        .iter()
        .any(|subscription| subscription.has_enabled_nodes())
        || manual_nodes.iter().any(|node| node.enabled)
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;
    use v2ray_rs_core::models::{
        AutoResolveStrategy, BackendType, DnsConfig, ProxyNode, SubscriptionNode,
        TransportSettings, TunConfig, VlessConfig,
    };

    #[test]
    fn restart_banner_only_visible_while_runtime_is_active() {
        assert!(restart_banner_visible_for_state(
            true,
            &ProcessState::Starting
        ));
        assert!(restart_banner_visible_for_state(
            true,
            &ProcessState::Running
        ));
        assert!(!restart_banner_visible_for_state(
            true,
            &ProcessState::Stopping
        ));
        assert!(!restart_banner_visible_for_state(
            true,
            &ProcessState::Stopped
        ));
        assert!(!restart_banner_visible_for_state(
            false,
            &ProcessState::Running
        ));
    }

    #[test]
    fn toggle_is_actionable_disconnect_while_starting() {
        assert_eq!(connect_toggle(&ProcessState::Starting), (true, true));
        assert_eq!(connect_toggle(&ProcessState::Running), (true, true));
        assert_eq!(connect_toggle(&ProcessState::Stopped), (false, true));
        assert_eq!(
            connect_toggle(&ProcessState::Error("boom".into())),
            (false, true)
        );
    }

    #[test]
    fn toggle_disabled_while_stopping() {
        let (_, sensitive) = connect_toggle(&ProcessState::Stopping);
        assert!(!sensitive);
    }

    fn snapshot(backend: BackendType, tun_enabled: bool) -> RuntimeConfigSnapshot {
        RuntimeConfigSnapshot {
            backend_type: backend,
            binary_path: None,
            socks_port: 1080,
            http_port: 1081,
            listen_address: "127.0.0.1".into(),
            dns: DnsConfig::default(),
            routing: RoutingRuleSet::default(),
            manual_nodes: Vec::new(),
            subscriptions: Vec::new(),
            auto_resolve_strategy: AutoResolveStrategy::default(),
            use_real_delay_for_lowest_latency: false,
            tun: TunConfig {
                enabled: tun_enabled,
                ..TunConfig::default()
            },
            idle_timeout_secs: 300,
            ws_heartbeat_secs: 30,
            timestamp: 0,
        }
    }

    #[test]
    fn tun_active_follows_launched_snapshot() {
        assert!(!tun_active_for(
            &ProcessState::Running,
            Some(&snapshot(BackendType::Xray, false))
        ));
        assert!(tun_active_for(
            &ProcessState::Running,
            Some(&snapshot(BackendType::Xray, true))
        ));
        assert!(!tun_active_for(
            &ProcessState::Running,
            Some(&snapshot(BackendType::V2ray, true))
        ));
    }

    #[test]
    fn tun_active_false_outside_running() {
        let snap = snapshot(BackendType::Xray, true);
        for state in [
            ProcessState::Starting,
            ProcessState::Stopping,
            ProcessState::Stopped,
            ProcessState::Error("boom".into()),
        ] {
            assert!(!tun_active_for(&state, Some(&snap)));
        }
        assert!(!tun_active_for(&ProcessState::Running, None));
    }

    #[test]
    fn reconnect_pending_consumed_on_stop_or_error() {
        assert!(reconnect_after_stop(&ProcessState::Stopped, true));
        assert!(reconnect_after_stop(
            &ProcessState::Error("boom".into()),
            true
        ));
        assert!(!reconnect_after_stop(&ProcessState::Stopped, false));
        assert!(!reconnect_after_stop(&ProcessState::Running, true));
        assert!(!reconnect_after_stop(&ProcessState::Stopping, true));
    }

    fn session_target_node() -> ConnectionNodeRef {
        ConnectionNodeRef::Manual {
            node_id: uuid::Uuid::nil(),
        }
    }

    #[test]
    fn direct_session_establishment_follows_origin() {
        assert!(!direct_session(session_target_node(), ConnectOrigin::User).established);
        assert!(direct_session(session_target_node(), ConnectOrigin::AutoReconnect).established);
        let session = direct_session(session_target_node(), ConnectOrigin::User);
        assert_eq!(session.node, session_target_node());
    }

    #[test]
    fn running_establishes_the_session_target() {
        let node = session_target_node();
        let target = direct_session(node, ConnectOrigin::User);

        assert_eq!(
            session_target_after_running(Some(target)),
            Some(SessionTarget {
                node,
                established: true
            })
        );
        assert_eq!(session_target_after_running(None), None);
    }

    #[test]
    fn user_disconnect_clears_target_and_restart_keeps_it() {
        let target = direct_session(session_target_node(), ConnectOrigin::User);

        assert_eq!(session_target_after_stop(Some(target), true), Some(target));
        assert_eq!(session_target_after_stop(Some(target), false), None);
        assert_eq!(session_target_after_stop(None, true), None);
    }

    #[test]
    fn error_retry_suppressed_for_unestablished_direct_target() {
        let unestablished = direct_session(session_target_node(), ConnectOrigin::User);
        let established = SessionTarget {
            node: session_target_node(),
            established: true,
        };

        assert!(!retry_after_error(
            Some(unestablished),
            false,
            MAX_AUTO_RECONNECTS
        ));
        assert!(retry_after_error(
            Some(established),
            false,
            MAX_AUTO_RECONNECTS
        ));
        assert!(!retry_after_error(
            Some(established),
            true,
            MAX_AUTO_RECONNECTS
        ));
        assert!(!retry_after_error(Some(established), false, 0));
        assert!(retry_after_error(None, false, MAX_AUTO_RECONNECTS));
        assert!(!retry_after_error(None, true, 0));
    }

    #[test]
    fn error_without_pending_target_replays_nothing() {
        let mut pending = None;

        let replay =
            consume_terminal_direct_state(&ProcessState::Error("boom".into()), &mut pending);

        assert!(replay.is_none());
        assert!(pending.is_none());
    }

    #[test]
    fn error_replays_pending_direct_target_before_any_other_reconnect() {
        let target = session_target_node();
        let mut pending = Some(target);

        let replay =
            consume_terminal_direct_state(&ProcessState::Error("boom".into()), &mut pending);

        assert_eq!(replay, Some(target));
        assert!(pending.is_none());
    }

    #[test]
    fn stopped_without_pending_target_replays_nothing() {
        let mut pending = None;

        let replay = consume_terminal_direct_state(&ProcessState::Stopped, &mut pending);

        assert!(replay.is_none());
        assert!(pending.is_none());
    }

    #[test]
    fn stopped_replays_pending_direct_target_before_any_other_reconnect() {
        let target = session_target_node();
        let mut pending = Some(target);

        let replay = consume_terminal_direct_state(&ProcessState::Stopped, &mut pending);

        assert_eq!(replay, Some(target));
        assert!(pending.is_none());
    }

    #[test]
    fn resolve_candidate_disabled_subscription_with_enabled_node_is_unavailable() {
        let mut subscription = Subscription::new_from_url("DisabledSub", "https://example.com");
        subscription.enabled = false;
        let node_id = uuid::Uuid::new_v4();
        subscription.nodes = vec![SubscriptionNode::with_id(
            node_id,
            ProxyNode::Vless(VlessConfig {
                address: "a.example.com".into(),
                port: 443,
                uuid: "test-uuid".into(),
                encryption: None,
                flow: None,
                transport: TransportSettings::Tcp,
                tls: None,
                remark: Some("A".into()),
            }),
            true,
        )];
        let node_ref = ConnectionNodeRef::Subscription {
            subscription_id: subscription.id,
            node_id,
        };

        assert!(
            resolve_candidate(node_ref, &[subscription], &[], &LatencySnapshot::default())
                .is_none(),
            "a disabled subscription must make even an enabled node unavailable for direct connect"
        );
    }

    #[test]
    fn active_nodes_include_manual_nodes_and_subscriptions() {
        let mut subscription = Subscription::new_from_url("Test", "https://example.com");
        subscription.enabled = true;
        subscription.nodes.clear();

        let manual_nodes = vec![ManualNode::with_id(
            uuid::Uuid::nil(),
            ProxyNode::Vless(VlessConfig {
                address: "manual.example.com".into(),
                port: 443,
                uuid: "manual-uuid".into(),
                encryption: None,
                flow: None,
                transport: TransportSettings::Tcp,
                tls: None,
                remark: Some("Manual".into()),
            }),
            true,
        )];

        assert!(active_nodes_available(&[subscription], &manual_nodes));
    }

    #[test]
    fn quit_while_stopping_waits_for_stopped() {
        assert_eq!(
            quit_plan(false, &ProcessState::Stopping, false),
            QuitPlan::AwaitStopped
        );
        assert_eq!(
            quit_plan(false, &ProcessState::Stopping, true),
            QuitPlan::AwaitStopped
        );
    }

    #[test]
    fn quit_with_handle_stops_first() {
        assert_eq!(
            quit_plan(true, &ProcessState::Running, true),
            QuitPlan::Stop
        );
        assert_eq!(
            quit_plan(true, &ProcessState::Starting, false),
            QuitPlan::Stop
        );
    }

    #[test]
    fn quit_with_leftover_marker_releases_first() {
        assert_eq!(
            quit_plan(false, &ProcessState::Error("boom".into()), true),
            QuitPlan::Release
        );
        assert_eq!(
            quit_plan(false, &ProcessState::Stopped, true),
            QuitPlan::Release
        );
    }

    #[test]
    fn quit_idle_exits() {
        assert_eq!(
            quit_plan(false, &ProcessState::Stopped, false),
            QuitPlan::Exit
        );
        assert_eq!(
            quit_plan(false, &ProcessState::Error("boom".into()), false),
            QuitPlan::Exit
        );
    }

    #[test]
    fn auto_reconnect_exhausted_after_three_attempts() {
        let mut attempts = 0;
        while auto_reconnect_allowed(false, attempts) {
            attempts += 1;
        }
        assert_eq!(attempts, 3);
        assert!(!auto_reconnect_allowed(false, MAX_AUTO_RECONNECTS));
    }

    #[test]
    fn auto_reconnect_suppressed_on_exit() {
        assert!(!auto_reconnect_allowed(true, 0));
    }

    #[test]
    fn user_connect_cancels_pending_auto_reconnect() {
        assert!(cancels_auto_reconnect(ConnectOrigin::User));
    }

    #[test]
    fn auto_reconnect_connect_keeps_budget() {
        assert!(!cancels_auto_reconnect(ConnectOrigin::AutoReconnect));
    }

    #[test]
    fn restart_origin_cancels_like_user() {
        assert!(cancels_auto_reconnect(ConnectOrigin::Restart));
        assert!(!direct_session(session_target_node(), ConnectOrigin::Restart).established);
    }

    #[test]
    fn reconnect_msg_targets_the_chosen_node_when_set() {
        let target = direct_session(session_target_node(), ConnectOrigin::User);

        assert!(matches!(
            reconnect_msg(Some(target), ConnectOrigin::Restart),
            AppMsg::ConnectToNode(node, ConnectOrigin::Restart) if node == session_target_node()
        ));
        assert!(matches!(
            reconnect_msg(Some(target), ConnectOrigin::AutoReconnect),
            AppMsg::ConnectToNode(node, ConnectOrigin::AutoReconnect)
                if node == session_target_node()
        ));
    }

    #[test]
    fn reconnect_msg_without_target_replans() {
        assert!(matches!(
            reconnect_msg(None, ConnectOrigin::Restart),
            AppMsg::Connect(ConnectOrigin::Restart)
        ));
        assert!(matches!(
            reconnect_msg(None, ConnectOrigin::AutoReconnect),
            AppMsg::Connect(ConnectOrigin::AutoReconnect)
        ));
    }

    #[test]
    fn planner_fallback_skips_user_clicks() {
        assert!(!falls_back_to_planner(ConnectOrigin::User));
        assert!(falls_back_to_planner(ConnectOrigin::Restart));
        assert!(falls_back_to_planner(ConnectOrigin::AutoReconnect));
    }

    #[test]
    fn auto_reconnect_fires_only_for_current_generation_without_handle() {
        let generation = 7_u32;
        assert!(!auto_reconnect_fires(
            generation,
            generation.wrapping_add(1),
            false
        ));
        assert!(auto_reconnect_fires(generation, generation, false));
        assert!(!auto_reconnect_fires(generation, generation, true));
    }

    #[test]
    fn interleaved_generations_keep_only_current() {
        let lines = [(6, "old"), (7, "new"), (6, "late")];
        let kept: Vec<&str> = lines
            .into_iter()
            .filter(|(generation, _)| is_current_generation(*generation, 7))
            .map(|(_, line)| line)
            .collect();
        assert_eq!(kept, ["new"]);
    }

    #[test]
    fn error_toast_action_arms_only_for_matching_generation() {
        let generation = 7_u64;
        assert_eq!(
            error_toast_action(generation, Some(generation)),
            Some(ToastAction::GrantTun)
        );
        assert_eq!(error_toast_action(generation, Some(generation + 1)), None);
        assert_eq!(error_toast_action(generation, None), None);
    }

    fn routing_rule(match_condition: RuleMatch, enabled: bool) -> RoutingRule {
        RoutingRule {
            id: uuid::Uuid::new_v4(),
            match_condition,
            action: v2ray_rs_core::models::RuleAction::Proxy,
            enabled,
            group: None,
            via_node: None,
        }
    }

    fn geosite_rule(enabled: bool) -> RoutingRule {
        routing_rule(
            RuleMatch::GeoSite {
                category: "google".into(),
            },
            enabled,
        )
    }

    fn geosite_dns(enabled: bool, use_custom_rules: bool) -> DnsConfig {
        DnsConfig {
            enabled,
            use_custom_rules,
            rules: vec![v2ray_rs_core::models::DnsRule {
                match_condition: DnsRuleMatch::GeoSite {
                    category: "netflix".into(),
                },
                server_tag: "remote".into(),
            }],
            ..DnsConfig::default()
        }
    }

    fn profile_sub(active: bool, rules: Vec<RoutingRule>, dns: Option<DnsConfig>) -> Subscription {
        let mut sub = Subscription::new_from_url("Provider", "https://example.com/sub");
        sub.use_imported_profile = active;
        sub.imported_profile = Some(v2ray_rs_core::models::ImportedProfile {
            rules,
            dns,
            skipped: Vec::new(),
            imported_at: chrono::Utc::now(),
        });
        sub
    }

    #[test]
    fn missing_geodata_xray_geosite_rule_without_file() {
        let rules = [geosite_rule(true)];
        let dns = DnsConfig::default();
        assert!(missing_geodata(
            BackendType::Xray,
            &rules,
            &[],
            &dns,
            false,
            false
        ));
        let geoip = [routing_rule(
            RuleMatch::GeoIp {
                country_code: "ru".into(),
            },
            true,
        )];
        assert!(missing_geodata(
            BackendType::V2ray,
            &geoip,
            &[],
            &dns,
            false,
            true
        ));
    }

    #[test]
    fn tun_ipv6_disabled_error_names_kernel_state_and_setting() {
        assert!(TUN_IPV6_DISABLED.contains("kernel has IPv6 disabled"));
        assert!(TUN_IPV6_DISABLED.contains("clear the IPv6 address in TUN settings"));
    }

    #[test]
    fn tun_ipv6_unavailable_table() {
        use BackendType::{SingBox, V2ray, Xray};
        let v6 = Some("fd00::1/126".to_string());
        let cases = [
            (true, SingBox, v6.clone(), false, true),
            (true, SingBox, v6.clone(), true, false),
            (true, SingBox, None, false, false),
            (true, SingBox, None, true, false),
            (true, Xray, v6.clone(), false, true),
            (true, Xray, v6.clone(), true, false),
            (true, Xray, None, false, false),
            (true, Xray, None, true, false),
            (true, V2ray, v6.clone(), false, false),
            (true, V2ray, v6.clone(), true, false),
            (true, V2ray, None, false, false),
            (true, V2ray, None, true, false),
            (false, SingBox, v6.clone(), false, false),
            (false, Xray, v6.clone(), false, false),
        ];
        for (enabled, backend, address_v6, host_has_ipv6, want) in cases {
            let tun = TunConfig {
                enabled,
                address_v6: address_v6.clone(),
                ..TunConfig::default()
            };
            assert_eq!(
                tun_ipv6_unavailable(&tun, backend, host_has_ipv6),
                want,
                "enabled={enabled} backend={backend:?} v6={address_v6:?} host_v6={host_has_ipv6}"
            );
        }
    }

    #[test]
    fn missing_geodata_singbox_is_never_gated() {
        let rules = [geosite_rule(true)];
        let subs = [profile_sub(true, vec![geosite_rule(true)], None)];
        let dns = geosite_dns(true, true);
        assert!(!missing_geodata(
            BackendType::SingBox,
            &rules,
            &subs,
            &dns,
            false,
            false
        ));
    }

    #[test]
    fn missing_geodata_without_geo_rules() {
        let rules = [
            routing_rule(
                RuleMatch::Domain {
                    pattern: "example.com".into(),
                },
                true,
            ),
            routing_rule(
                RuleMatch::IpCidr {
                    cidr: "10.0.0.0/8".parse().unwrap(),
                },
                true,
            ),
        ];
        let dns = DnsConfig::default();
        assert!(!missing_geodata(
            BackendType::Xray,
            &rules,
            &[],
            &dns,
            false,
            false
        ));
    }

    #[test]
    fn missing_geodata_imported_profile_geo_rule() {
        let subs = [profile_sub(true, vec![geosite_rule(true)], None)];
        let dns = DnsConfig::default();
        assert!(missing_geodata(
            BackendType::Xray,
            &[],
            &subs,
            &dns,
            false,
            false
        ));
    }

    #[test]
    fn missing_geodata_ignores_disabled_and_inactive_rules() {
        let dns = DnsConfig::default();
        let disabled = [geosite_rule(false)];
        assert!(!missing_geodata(
            BackendType::Xray,
            &disabled,
            &[],
            &dns,
            false,
            false
        ));
        let inactive = [profile_sub(false, vec![geosite_rule(true)], None)];
        assert!(!missing_geodata(
            BackendType::Xray,
            &[],
            &inactive,
            &dns,
            false,
            false
        ));
        let disabled_profile = [profile_sub(true, vec![geosite_rule(false)], None)];
        assert!(!missing_geodata(
            BackendType::Xray,
            &[],
            &disabled_profile,
            &dns,
            false,
            false
        ));
    }

    #[test]
    fn missing_geodata_custom_dns_geosite_rule() {
        let xray = BackendType::Xray;
        assert!(missing_geodata(
            xray,
            &[],
            &[],
            &geosite_dns(true, true),
            false,
            false
        ));
        assert!(!missing_geodata(
            xray,
            &[],
            &[],
            &geosite_dns(true, false),
            false,
            false
        ));
        assert!(!missing_geodata(
            xray,
            &[],
            &[],
            &geosite_dns(false, true),
            false,
            false
        ));
        let subs = [profile_sub(true, Vec::new(), Some(geosite_dns(true, true)))];
        assert!(missing_geodata(
            xray,
            &[],
            &subs,
            &DnsConfig::default(),
            false,
            false
        ));
    }

    #[test]
    fn missing_geodata_both_files_present() {
        let rules = [geosite_rule(true)];
        let dns = geosite_dns(true, true);
        assert!(!missing_geodata(
            BackendType::Xray,
            &rules,
            &[],
            &dns,
            true,
            true
        ));
    }

    #[test]
    fn missing_geodata_one_file_missing() {
        let rules = [geosite_rule(true)];
        let dns = DnsConfig::default();
        assert!(missing_geodata(
            BackendType::Xray,
            &rules,
            &[],
            &dns,
            true,
            false
        ));
        assert!(missing_geodata(
            BackendType::Xray,
            &rules,
            &[],
            &dns,
            false,
            true
        ));
    }

    #[test]
    fn disconnect_without_handle_releases_marker() {
        assert_eq!(disconnect_plan(false, true), DisconnectPlan::Release);
        assert_eq!(disconnect_plan(false, false), DisconnectPlan::Nothing);
        assert_eq!(disconnect_plan(true, true), DisconnectPlan::Stop);
        assert_eq!(disconnect_plan(true, false), DisconnectPlan::Stop);
    }

    #[test]
    fn error_while_stopping_releases_without_retry() {
        assert!(release_on_error(true, MAX_AUTO_RECONNECTS));
        assert!(release_on_error(true, 0));
    }

    #[test]
    fn error_with_reconnects_left_keeps_killswitch() {
        assert!(!release_on_error(false, 1));
        assert!(!release_on_error(false, MAX_AUTO_RECONNECTS));
        assert!(release_on_error(false, 0));
    }

    fn paths_with_marker(tmp: &tempfile::TempDir) -> AppPaths {
        let paths = AppPaths::for_profile_in(AppProfile::Test, tmp.path());
        v2ray_rs_core::persistence::save_tun_session(
            &paths,
            &v2ray_rs_core::persistence::TunSession {
                backend: v2ray_rs_core::models::BackendType::Xray,
                iface: "tun9".into(),
            },
        )
        .unwrap();
        paths
    }

    fn stub_helper(tmp: &tempfile::TempDir, body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let helper = tmp.path().join("netctl");
        std::fs::write(&helper, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();
        helper
    }

    #[tokio::test]
    async fn recover_runs_helper_and_clears_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths_with_marker(&tmp);
        let args = tmp.path().join("args");
        let helper = stub_helper(&tmp, &format!("echo \"$@\" > {}", args.display()));

        assert!(recover_tun_session(&paths, &helper).await.is_ok());

        let recorded = std::fs::read_to_string(&args).unwrap();
        assert_eq!(recorded.trim_end(), "recover --xray --iface tun9");
        assert!(v2ray_rs_core::persistence::load_tun_session(&paths).is_none());
    }

    #[tokio::test]
    async fn recover_clears_marker_when_helper_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths_with_marker(&tmp);
        let helper = stub_helper(&tmp, "exit 1");

        let failure = recover_tun_session(&paths, &helper).await.unwrap_err();

        assert!(!failure.timed_out);
        assert!(v2ray_rs_core::persistence::load_tun_session(&paths).is_none());
    }

    #[tokio::test]
    async fn recover_logs_helper_output_and_exit_status() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths_with_marker(&tmp);
        let helper = stub_helper(&tmp, "echo route-busy; exit 1");

        let failure = recover_tun_session(&paths, &helper).await.unwrap_err();

        assert!(!failure.timed_out);
        let log = std::fs::read_to_string(paths.logs_dir().join("backend.log")).unwrap();
        assert!(log.contains(" helper route-busy"), "{log}");
        assert!(
            log.contains(" helper recover exited with exit status: 1"),
            "{log}"
        );
        assert!(v2ray_rs_core::persistence::load_tun_session(&paths).is_none());
    }

    #[tokio::test]
    async fn recover_logs_ok_outcome() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths_with_marker(&tmp);
        let helper = stub_helper(&tmp, "exit 0");

        assert!(recover_tun_session(&paths, &helper).await.is_ok());

        let log = std::fs::read_to_string(paths.logs_dir().join("backend.log")).unwrap();
        assert!(log.contains(" helper recover ok"), "{log}");
    }

    #[test]
    fn recovery_hint_names_singbox_flag_and_iface() {
        let session = TunSession {
            backend: BackendType::SingBox,
            iface: "tun0".into(),
        };
        assert_eq!(
            recovery_hint(&session),
            "v2ray-rs-netctl recover --singbox --iface tun0"
        );
    }

    #[test]
    fn recovery_hint_names_xray_flag_and_iface() {
        let session = TunSession {
            backend: BackendType::Xray,
            iface: "tun9".into(),
        };
        assert_eq!(
            recovery_hint(&session),
            "v2ray-rs-netctl recover --xray --iface tun9"
        );
    }

    #[test]
    fn recovery_toast_distinguishes_timeout() {
        let session = TunSession {
            backend: BackendType::Xray,
            iface: "tun9".into(),
        };
        let failed = RecoveryFailure {
            session: session.clone(),
            timed_out: false,
        };
        let timed_out = RecoveryFailure {
            session,
            timed_out: true,
        };
        assert_eq!(
            failed.toast(),
            "TUN route recovery failed: run v2ray-rs-netctl recover --xray --iface tun9"
        );
        assert_eq!(
            timed_out.toast(),
            "TUN route recovery timed out: run v2ray-rs-netctl recover --xray --iface tun9"
        );
    }

    #[tokio::test]
    async fn recover_without_marker_does_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_profile_in(AppProfile::Test, tmp.path());
        let args = tmp.path().join("args");
        let helper = stub_helper(&tmp, &format!("echo \"$@\" > {}", args.display()));

        assert!(recover_tun_session(&paths, &helper).await.is_ok());

        assert!(!args.exists());
        assert!(!paths.logs_dir().join("backend.log").exists());
    }
    fn last_success_at(at: chrono::DateTime<chrono::Utc>) -> LastSuccessMetadata {
        LastSuccessMetadata {
            node_ref: session_target_node(),
            connected_at: at,
        }
    }

    #[test]
    fn flush_keeps_current_last_success() {
        let stale_at = chrono::Utc::now() - chrono::Duration::hours(1);
        let fresh_at = chrono::Utc::now();
        let current = AppSettings {
            last_success: Some(last_success_at(fresh_at)),
            ..AppSettings::default()
        };
        let incoming = AppSettings {
            last_success: Some(last_success_at(stale_at)),
            ..AppSettings::default()
        };

        let kept = keep_last_success(incoming, &current);
        assert_eq!(kept.last_success, current.last_success);
    }

    #[test]
    fn flush_does_not_invent_last_success() {
        let incoming = AppSettings {
            last_success: Some(last_success_at(chrono::Utc::now())),
            ..AppSettings::default()
        };

        assert_eq!(
            keep_last_success(incoming, &AppSettings::default()).last_success,
            None
        );
    }

    #[test]
    fn flush_keeps_other_fields() {
        let incoming = AppSettings {
            socks_port: 2080,
            ..AppSettings::default()
        };

        let kept = keep_last_success(incoming, &AppSettings::default());
        assert_eq!(kept.socks_port, 2080);
    }

    #[test]
    fn direct_session_success_records_last_success() {
        let node = session_target_node();
        let now = chrono::Utc::now();
        let connection = ConnectionMetadata {
            node_ref: node,
            source: "manual".into(),
            source_id: String::new(),
            node_name: "node".into(),
            node_address: "127.0.0.1".into(),
            node_port: 1080,
            backend: BackendType::Xray,
            strategy: AutoResolveStrategy::default(),
            latency_ms: None,
            connected_since: now,
        };

        let recorded = last_success_settings(&AppSettings::default(), &connection);

        assert_eq!(
            recorded.last_success,
            Some(LastSuccessMetadata {
                node_ref: node,
                connected_at: now,
            })
        );
    }
}

fn tray_available() -> bool {
    TRAY_HANDLE
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().map(|_| ()))
        .is_some()
}

fn update_tray_notification_setting(enabled: bool) {
    if let Ok(mut guard) = TRAY_HANDLE.lock()
        && let Some(handle) = guard.as_mut()
    {
        handle.set_notifications_enabled(enabled);
    }
}

fn cleanup_orphaned_backend(paths: &AppPaths) -> std::io::Result<bool> {
    let pid_file = PidFile::new(paths.pid_file_path());
    pid_file.check_and_kill_orphaned()
}

struct RecoveryFailure {
    session: TunSession,
    timed_out: bool,
}

impl RecoveryFailure {
    fn toast(&self) -> String {
        let hint = recovery_hint(&self.session);
        if self.timed_out {
            format!("TUN route recovery timed out: run {hint}")
        } else {
            format!("TUN route recovery failed: run {hint}")
        }
    }
}

fn recovery_hint(session: &TunSession) -> String {
    format!(
        "v2ray-rs-netctl recover {} --iface {}",
        recover_flag(session.backend),
        session.iface
    )
}

fn recover_flag(backend: BackendType) -> &'static str {
    match backend {
        BackendType::SingBox => "--singbox",
        _ => "--xray",
    }
}

/// If a TUN session marker is present, run the route helper's recovery pass
/// and clear the marker. Runs at startup after an unclean shutdown and
/// whenever a TUN session ends without a clean stop. The marker is cleared
/// even when the helper fails or hangs, so a broken helper cannot wedge
/// every later launch.
async fn recover_tun_session(
    paths: &AppPaths,
    helper: &std::path::Path,
) -> Result<(), RecoveryFailure> {
    let Some(session) = v2ray_rs_core::persistence::load_tun_session(paths) else {
        return Ok(());
    };
    let args = [
        "recover".to_string(),
        recover_flag(session.backend).to_string(),
        "--iface".to_string(),
        session.iface.clone(),
    ];
    let run = v2ray_rs_process::run_helper(helper, &args, v2ray_rs_process::HELPER_TIMEOUT).await;
    let outcome = match &run.result {
        Ok(()) => {
            log::info!("recovered leftover TUN state on {}", session.iface);
            "recover ok".to_string()
        }
        Err(err) => {
            log::warn!("tun {err}");
            err.clone()
        }
    };
    match RotatingFileWriter::open(paths.logs_dir().join("backend.log"), DEFAULT_MAX_BYTES) {
        Ok(log) => {
            for line in &run.output {
                log.append_line("helper", line);
            }
            log.append_line("helper", &outcome);
        }
        Err(err) => log::warn!("open backend log: {err}"),
    }
    let _ = v2ray_rs_core::persistence::clear_tun_session(paths);
    match run.result {
        Ok(()) => Ok(()),
        Err(_) => Err(RecoveryFailure {
            session,
            timed_out: run.timed_out,
        }),
    }
}

fn install_icon_for_compositor(profile: &AppProfile) {
    let data_dir = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share"))
        });

    let Some(data_dir) = data_dir else { return };
    let app_id = profile.app_id();
    let res_prefix = format!("/{}/icons/hicolor", app_id.replace('.', "/"));

    let installed = install_resource_icon(
        &data_dir,
        &res_prefix,
        "scalable/apps",
        &format!("{app_id}.svg"),
    ) | install_resource_icon(
        &data_dir,
        &res_prefix,
        "symbolic/apps",
        &format!("{app_id}-symbolic.svg"),
    );

    if installed {
        let theme_dir = data_dir.join("icons/hicolor");
        let _ = std::process::Command::new("gtk-update-icon-cache")
            .arg("-f")
            .arg("-t")
            .arg(&theme_dir)
            .spawn();
    }
}

fn install_resource_icon(
    data_dir: &std::path::Path,
    res_prefix: &str,
    subdir: &str,
    filename: &str,
) -> bool {
    let icon_dir = data_dir.join(format!("icons/hicolor/{subdir}"));
    let icon_path = icon_dir.join(filename);
    if icon_path.exists() {
        return false;
    }

    let Ok(svg) = gtk::gio::resources_lookup_data(
        &format!("{res_prefix}/{subdir}/{filename}"),
        gtk::gio::ResourceLookupFlags::NONE,
    ) else {
        return false;
    };

    if let Err(err) = std::fs::create_dir_all(&icon_dir) {
        log::debug!("create icon dir {icon_dir:?}: {err}");
        return false;
    }
    if let Err(err) = std::fs::write(&icon_path, &svg) {
        log::debug!("write icon {icon_path:?}: {err}");
        return false;
    }
    true
}

pub fn run() {
    if let Err(err) = try_run() {
        eprintln!("v2ray-rs startup failed: {err}");
        std::process::exit(1);
    }
}

fn try_run() -> Result<(), String> {
    // clap sends --help/--version to stdout with exit code 0 and real errors to
    // stderr; `print` and `exit_code` already know which. Routing the stdout
    // cases through the startup-failure path printed help text to stderr and
    // exited 1.
    let cli_args = CliArgs::try_parse().unwrap_or_else(|e: clap::error::Error| {
        let _ = e.print();
        std::process::exit(e.exit_code());
    });

    let profile = AppProfile::resolve(cli_args.profile.as_deref(), &StdEnv)
        .map_err(|e| format!("invalid profile: {e}"))?;

    let overrides = PathOverrides::resolve(&cli_args.paths(), &StdEnv);

    let paths = AppPaths::with_overrides(profile.clone(), &overrides)
        .map_err(|err| format!("failed to determine XDG directories: {err}"))?;

    paths
        .ensure_dirs()
        .map_err(|err| format!("failed to create directories: {err}"))?;

    crate::logging::init_logging(&paths);
    log::info!(
        "v2ray-rs {} starting, profile '{}'",
        env!("CARGO_PKG_VERSION"),
        profile.qualifier()
    );

    if cli_args.reset_instance {
        reset_instance(&paths, &profile, cli_args.i_understand)
            .map_err(|e| format!("failed to reset instance: {e}"))?;
        println!("Instance reset successfully.");
        return Ok(());
    }

    let _lock = InstanceLock::acquire(&paths).map_err(|e| {
        if let v2ray_rs_core::instance::InstanceError::LockHeld { pid, profile: p } = e {
            eprintln!("Another instance is already running (PID {pid}), profile '{p}' is locked.");
            std::process::exit(75);
        }
        format!("failed to acquire instance lock: {e}")
    })?;

    let mut stamp = InstanceStamp::load_or_create(&paths)
        .map_err(|e| format!("failed to load instance stamp: {e}"))?;

    let compatibility = check_compatibility(&stamp, &profile);
    match compatibility {
        CompatibilityResult::Match => {
            log::info!("Instance stamp is compatible");
        }
        CompatibilityResult::NeedsForwardMigration => {
            log::warn!(
                "Instance stamp needs forward migration (schema version {} < {}), continuing",
                stamp.schema_version,
                v2ray_rs_core::instance::CURRENT_SCHEMA_VERSION
            );
        }
        CompatibilityResult::IncompatibleProfile => {
            return Err(format!(
                "Instance profile '{}' is incompatible with current profile '{}'. Reset instance with --reset-instance to continue.",
                stamp.profile,
                profile.qualifier()
            ));
        }
        CompatibilityResult::IncompatibleAppId => {
            return Err(format!(
                "Instance app_id '{}' is incompatible with current app_id '{}'. Reset instance with --reset-instance to continue.",
                stamp.app_id,
                profile.app_id()
            ));
        }
        CompatibilityResult::TooNew => {
            return Err(format!(
                "Instance schema version {} is newer than current {}. Downgrade the application or reset instance with --reset-instance to continue.",
                stamp.schema_version,
                v2ray_rs_core::instance::CURRENT_SCHEMA_VERSION
            ));
        }
    }

    stamp
        .update_started(&paths)
        .map_err(|e| format!("failed to update instance stamp: {e}"))?;

    if let Err(err) = rustls::crypto::ring::default_provider().install_default() {
        log::debug!("rustls crypto provider already installed or unavailable: {err:?}");
    }

    let app_id = profile.app_id();

    let settings = match v2ray_rs_core::persistence::load_settings(&paths) {
        Ok(settings) => settings,
        Err(err) => {
            log::warn!("load settings during startup: {err}");
            AppSettings::default()
        }
    };
    crate::i18n::init(settings.language);

    let rt = tokio::runtime::Runtime::new()
        .map_err(|err| format!("failed to create tokio runtime: {err}"))?;
    let _rt_guard = rt.enter();

    let (event_tx, event_rx) = broadcast::channel::<ProcessEvent>(EVENT_CHANNEL_CAPACITY);
    if let Ok(mut guard) = TRAY_EVENT_TX.lock() {
        *guard = Some(event_tx);
    }

    let should_install_icons =
        profile == AppProfile::Production || overrides.install_icons.unwrap_or(false);
    let tray_action_rx = if should_install_icons {
        let (tray_tx, tray_rx) = tokio::sync::mpsc::unbounded_channel::<TrayAction>();
        let notifier = v2ray_rs_tray::Notifier::new(settings.notifications_enabled);
        let data_dir = paths.data_dir().to_path_buf();
        match rt.block_on(async {
            v2ray_rs_tray::TrayService::spawn_with_data_dir(
                event_rx,
                notifier,
                move |action| {
                    let _ = tray_tx.send(action);
                },
                &data_dir,
            )
            .await
        }) {
            Ok(handle) => {
                if let Ok(mut guard) = TRAY_HANDLE.lock() {
                    *guard = Some(handle);
                }
                Some(tray_rx)
            }
            Err(err) => {
                log::warn!("failed to start tray service: {err}");
                None
            }
        }
    } else {
        None
    };

    let resource_bytes =
        glib::Bytes::from_static(include_bytes!(concat!(env!("OUT_DIR"), "/icons.gresource")));
    let resource = gtk::gio::Resource::from_data(&resource_bytes)
        .map_err(|err| format!("failed to load icon resource: {err}"))?;
    gtk::gio::resources_register(&resource);

    let app = adw::Application::builder().application_id(&app_id).build();

    // Register our CLI options with GTK so it doesn't complain about "unknown options"
    app.add_main_option(
        "profile",
        glib::Char(0),
        glib::OptionFlags::NONE,
        glib::OptionArg::String,
        "Runtime profile (production, development, test, custom:name)",
        Some("PROFILE"),
    );
    app.add_main_option(
        "config-dir",
        glib::Char(0),
        glib::OptionFlags::NONE,
        glib::OptionArg::String,
        "Override config directory",
        Some("PATH"),
    );
    app.add_main_option(
        "data-dir",
        glib::Char(0),
        glib::OptionFlags::NONE,
        glib::OptionArg::String,
        "Override data directory",
        Some("PATH"),
    );
    app.add_main_option(
        "cache-dir",
        glib::Char(0),
        glib::OptionFlags::NONE,
        glib::OptionArg::String,
        "Override cache directory",
        Some("PATH"),
    );
    app.add_main_option(
        "runtime-dir",
        glib::Char(0),
        glib::OptionFlags::NONE,
        glib::OptionArg::String,
        "Override runtime directory",
        Some("PATH"),
    );
    app.add_main_option(
        "state-dir",
        glib::Char(0),
        glib::OptionFlags::NONE,
        glib::OptionArg::String,
        "Override state directory",
        Some("PATH"),
    );
    app.add_main_option(
        "reset-instance",
        glib::Char(0),
        glib::OptionFlags::NONE,
        glib::OptionArg::None,
        "Reset instance data for current profile",
        None,
    );
    app.add_main_option(
        "install-icons",
        glib::Char(0),
        glib::OptionFlags::NONE,
        glib::OptionArg::None,
        "Install icons even for non-production profiles",
        None,
    );
    app.add_main_option(
        "i-understand",
        glib::Char(0),
        glib::OptionFlags::NONE,
        glib::OptionArg::None,
        "Confirm destructive operations",
        None,
    );

    let app_id_clone = app_id.clone();
    let profile_clone = profile.clone();
    app.connect_startup(move |_| {
        if let Some(display) = gtk::gdk::Display::default() {
            let theme = gtk::IconTheme::for_display(&display);
            theme.add_resource_path("/com/github/v2ray-rs/icons");
        }
        if should_install_icons {
            install_icon_for_compositor(&profile_clone);
        }
        gtk::Window::set_default_icon_name(&app_id_clone);
    });

    app.connect_activate(|app| {
        if let Some(window) = app.active_window() {
            window.set_visible(true);
            window.present();
        }
    });

    let relm_app = RelmApp::from_app(app);
    relm_app.run::<App>(AppInit {
        paths,
        tray_action_rx,
    });

    if let Ok(mut guard) = TRAY_HANDLE.lock()
        && let Some(handle) = guard.take()
    {
        rt.block_on(handle.shutdown());
    }
    if let Ok(mut guard) = TRAY_EVENT_TX.lock() {
        guard.take();
    }

    Ok(())
}
