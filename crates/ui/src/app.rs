use std::sync::Mutex;

use adw::prelude::*;
use clap::Parser;
use gtk::glib;
use relm4::adw;
use relm4::prelude::*;
use tokio::sync::broadcast;

use v2ray_rs_core::cli::{CliArgs, PathOverrides};
use v2ray_rs_core::config::ConfigWriter;
use v2ray_rs_core::instance::{check_compatibility, InstanceLock, InstanceStamp, reset_instance, CompatibilityResult};
use v2ray_rs_core::models::{
    AppSettings, ConnectionMetadata, LastSuccessMetadata, ManualNode, RoutingRuleSet, Subscription,
    SubscriptionSource,
};
use v2ray_rs_core::persistence::AppPaths;
use v2ray_rs_core::profile::{AppProfile, StdEnv};
use v2ray_rs_core::resolve::{ConnectionPlanner, LatencySnapshot};
use v2ray_rs_core::runtime_snapshot::RuntimeConfigSnapshot;
use v2ray_rs_process::{PidFile, ProcessEvent, ProcessState};
use v2ray_rs_tray::{TrayAction, TrayHandle};

static TRAY_HANDLE: Mutex<Option<TrayHandle>> = Mutex::new(None);
static TRAY_EVENT_TX: Mutex<Option<broadcast::Sender<ProcessEvent>>> = Mutex::new(None);

const DEFAULT_WINDOW_WIDTH: i32 = 900;
const DEFAULT_WINDOW_HEIGHT: i32 = 650;
const EVENT_CHANNEL_CAPACITY: usize = 16;

pub struct AppInit {
    pub paths: AppPaths,
    pub tray_action_rx: Option<tokio::sync::mpsc::UnboundedReceiver<TrayAction>>,
}

use crate::connection::{ConnectionHandle, ConnectionRequest};
use crate::geodata_service::{GeodataRefreshConfig, GeodataRefreshService};
use crate::logs::{LogsMsg, LogsPage};
use crate::nodes::{NodesOutput, NodesPage};
use crate::subscriptions::{SubscriptionsMsg, SubscriptionsOutput, SubscriptionsPage};
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
    runtime_snapshot: Option<RuntimeConfigSnapshot>,
    restart_required: bool,
    current_view: usize,
    geodata_service: GeodataRefreshService,
    pending_exit: bool,
    settings_debounce: Option<glib::SourceId>,
}

#[derive(Debug)]
pub enum AppMsg {
    OnboardingComplete(AppSettings, Option<(String, SubscriptionSource)>),
    SettingsChanged(AppSettings),
    ToggleConnection,
    Connect,
    Disconnect,
    CloseRequested,
    TrayShowWindow,
    TrayQuit,
    ActiveNodesChanged(bool),
    ProcessStateConnection(ProcessState, Option<ConnectionMetadata>),
    ProcessLogLine(String),
    OpenPreferences,
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
}

impl App {
    fn show_toast(&self, msg: &str) {
        self.toast_overlay.add_toast(adw::Toast::new(msg));
    }

    fn apply_state(&mut self, state: &ProcessState) {
        let from = self.process_state.clone();
        match state {
            ProcessState::Stopped => {
                self.connected = false;
                self.button_sensitive = true;
            }
            ProcessState::Starting => {
                self.connected = false;
                self.button_sensitive = false;
            }
            ProcessState::Running => {
                self.connected = true;
                self.button_sensitive = true;
            }
            ProcessState::Stopping => {
                self.connected = true;
                self.button_sensitive = false;
            }
            ProcessState::Error(msg) => {
                self.connected = false;
                self.button_sensitive = true;
                self.show_toast(&format!("Error: {msg}"));
            }
        }
        self.process_state = state.clone();

        let locked = matches!(state, ProcessState::Running | ProcessState::Starting);
        self.subscriptions_page
            .emit(SubscriptionsMsg::SetLocked(locked));

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

    fn clear_restart_flow(&mut self) {
        self.runtime_snapshot = None;
        self.restart_required = false;
        self.reconnect_pending = false;
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
        );
        let candidate = planner
            .runtime_candidate(subscriptions, manual_nodes)
            .map(|candidate| candidate.node);

        let Some(node) = candidate else {
            log::debug!("No enabled nodes, skipping config regeneration");
            return;
        };

        let rules = self.store.load_routing_rules().unwrap_or_default();
        let enabled_rules: Vec<_> = rules.enabled_rules().cloned().collect();

        let writer = ConfigWriter::new(&self.settings, &self.paths);
        match writer.write_config(std::slice::from_ref(&node), &enabled_rules, &self.settings) {
            Ok(path) => log::info!("Regenerated config at {:?}", path),
            Err(e) => {
                log::error!("Failed to regenerate config: {}", e);
                self.show_toast(&format!("Failed to regenerate config: {e}"));
            }
        }
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
                        TrayAction::Connect => s.emit(AppMsg::Connect),
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
        if settings_load_error.is_none()
            && let Err(err) = cleanup_orphaned_backend(&paths)
        {
            log::warn!("failed to clean orphaned backend process: {err}");
        }

        let show_wizard = settings_load_error.is_none() && !settings.onboarding_complete;

        let subscriptions_page = SubscriptionsPage::builder()
            .launch((store.clone(), settings.clone()))
            .forward(sender.input_sender(), |msg| match msg {
                SubscriptionsOutput::ActiveNodesChanged(has) => AppMsg::ActiveNodesChanged(has),
                SubscriptionsOutput::SubscriptionsChanged => AppMsg::SubscriptionsChanged,
                SubscriptionsOutput::Notice(message) => AppMsg::ShowToast(message),
            });

        let nodes_page = NodesPage::builder()
            .launch((store.clone(), settings.clone()))
            .forward(sender.input_sender(), |msg| match msg {
                NodesOutput::ActiveNodesChanged(has) => AppMsg::ActiveNodesChanged(has),
                NodesOutput::NodesChanged => AppMsg::ManualNodesChanged,
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
        let geodata_service =
            GeodataRefreshService::spawn(GeodataRefreshConfig::from_settings(&paths, &settings));

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
            runtime_snapshot: None,
            restart_required: false,
            current_view: 0,
            geodata_service,
            pending_exit: false,
            settings_debounce: None,
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
                s.emit(AppMsg::OpenPreferences);
            });
        }
        root.add_action(&prefs_action);

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
                    ));
                self.subscriptions_page
                    .emit(SubscriptionsMsg::SyncSettings {
                        auto_update_enabled: self.settings.auto_update_subscriptions,
                        auto_update_interval_secs: self.settings.subscription_update_interval_secs,
                    });

                if let Some((name, source)) = subscription {
                    self.subscriptions_page
                        .emit(SubscriptionsMsg::AddSubscription(name, source));
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
                let previous_settings = self.settings.clone();
                let strategy_changed =
                    previous_settings.auto_resolve_strategy != settings.auto_resolve_strategy;
                if let Err(err) = self.persist_settings(settings) {
                    log::error!("save settings: {err}");
                    self.show_toast(&format!("Failed to save settings: {err}"));
                    return;
                }
                crate::i18n::switch_language(self.settings.language);
                update_tray_notification_setting(self.settings.notifications_enabled);
                let was_connected = self.process_handle.is_some();
                self.geodata_service
                    .update(GeodataRefreshConfig::from_settings(
                        &self.paths,
                        &self.settings,
                    ));
                self.subscriptions_page
                    .emit(SubscriptionsMsg::SyncSettings {
                        auto_update_enabled: self.settings.auto_update_subscriptions,
                        auto_update_interval_secs: self.settings.subscription_update_interval_secs,
                    });
                self.restart_required = self.check_restart_required();
                if was_connected && strategy_changed {
                    self.reconnect_pending = true;
                    sender.input(AppMsg::Disconnect);
                }
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
            AppMsg::ToggleConnection => {
                if self.connected {
                    sender.input(AppMsg::Disconnect);
                } else {
                    sender.input(AppMsg::Connect);
                }
            }
            AppMsg::Connect => {
                if self.process_handle.is_some() {
                    return;
                }

                let binary_path = match &self.settings.backend.binary_path {
                    Some(p) => p.clone(),
                    None => {
                        self.show_toast("No backend binary configured — check Preferences");
                        return;
                    }
                };

                if let Err(err) = cleanup_orphaned_backend(&self.paths) {
                    log::warn!("failed to clean orphaned backend process: {err}");
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
                let snapshot = match self.store.load_latency_snapshot() {
                    Ok(snapshot) => snapshot,
                    Err(err) => {
                        self.show_toast(&format!("Failed to load latency data: {err}"));
                        return;
                    }
                };
                let planner = ConnectionPlanner::new(
                    self.settings.auto_resolve_strategy,
                    self.settings.last_success.clone(),
                    snapshot,
                );
                let candidates = planner.plan(&subscriptions, &manual_nodes);

                if candidates.is_empty() {
                    self.show_toast(
                        "No enabled proxy nodes — add a subscription or manual node first",
                    );
                    return;
                }

                let rules = match self.store.load_routing_rules() {
                    Ok(rules) => rules,
                    Err(err) => {
                        self.show_toast(&format!("Failed to load routing rules: {err}"));
                        return;
                    }
                };
                let enabled_rules: Vec<_> = rules.enabled_rules().cloned().collect();

                self.runtime_snapshot = Some(RuntimeConfigSnapshot {
                    backend_type: self.settings.backend.backend_type,
                    binary_path: self.settings.backend.binary_path.clone(),
                    socks_port: self.settings.socks_port,
                    http_port: self.settings.http_port,
                    dns: self.settings.dns.clone(),
                    routing: rules,
                    manual_nodes: manual_nodes.clone(),
                    subscriptions: subscriptions.clone(),
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
                        pid_path,
                        geodata_dir,
                        settings,
                        enabled_rules,
                    },
                    sender.input_sender().clone(),
                );
                self.process_handle = Some(handle);
            }
            AppMsg::Disconnect => {
                self.clear_restart_flow();
                if let Some(handle) = self.process_handle.take() {
                    self.apply_state(&ProcessState::Stopping);
                    handle.stop();
                } else {
                    self.show_toast("Not connected");
                }
            }
            AppMsg::ProcessStateConnection(state, connection) => {
                let stopped = matches!(state, ProcessState::Stopped | ProcessState::Error(_));
                if stopped {
                    self.process_handle = None;
                    self.logs_page.emit(LogsMsg::SetRunning(false));
                    self.clear_restart_flow();
                }
                if connection.is_some() {
                    self.connection_status = connection;
                    if let Some(meta) = &self.connection_status {
                        let node_ref = meta.node_ref;
                        let connected_at = meta.connected_since;
                        let mut settings = self.settings.clone();
                        settings.last_success = Some(LastSuccessMetadata {
                            node_ref,
                            connected_at,
                        });
                        if let Err(err) = self.persist_settings(settings) {
                            log::error!("save settings: {err}");
                        }
                    }
                } else if matches!(state, ProcessState::Stopped | ProcessState::Error(_)) {
                    self.connection_status = None;
                }
                self.apply_state(&state);
                if stopped && self.pending_exit {
                    self.pending_exit = false;
                    self.window.destroy();
                    return;
                }
                if stopped && !self.reconnect_pending {
                    self.regenerate_config_disconnected();
                }
                if matches!(state, ProcessState::Stopped) && self.reconnect_pending {
                    self.reconnect_pending = false;
                    sender.input(AppMsg::Connect);
                }
            }
            AppMsg::ProcessLogLine(line) => {
                self.logs_page.emit(LogsMsg::AppendLine(line));
            }
            AppMsg::CloseRequested => {
                if self.settings.minimize_to_tray && tray_available() {
                    self.window.set_visible(false);
                } else if let Some(handle) = self.process_handle.take() {
                    self.pending_exit = true;
                    handle.stop();
                } else {
                    self.window.destroy();
                }
            }
            AppMsg::TrayShowWindow => {
                self.window.set_visible(true);
                self.window.present();
            }
            AppMsg::TrayQuit => {
                if let Some(handle) = self.process_handle.take() {
                    self.pending_exit = true;
                    handle.stop();
                } else {
                    self.window.destroy();
                }
            }
            AppMsg::OpenPreferences => {
                if let Some(dialog) = &self.preferences_dialog {
                    dialog.present(Some(&self.window));
                    return;
                }

                let settings = self.settings.clone();
                let window = self.window.clone();
                let s = sender.input_sender().clone();
                let s1 = s.clone();
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
                );
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
            AppMsg::ResetBrokenSettings => match std::fs::remove_file(self.paths.settings_path()) {
                Ok(()) => {
                    self.settings = AppSettings::default();
                    self.settings_load_error = None;
                    self.show_wizard = true;
                    self.geodata_service
                        .update(GeodataRefreshConfig::from_settings(
                            &self.paths,
                            &self.settings,
                        ));
                    self.subscriptions_page
                        .emit(SubscriptionsMsg::SyncSettings {
                            auto_update_enabled: self.settings.auto_update_subscriptions,
                            auto_update_interval_secs: self
                                .settings
                                .subscription_update_interval_secs,
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
    use v2ray_rs_core::models::{ProxyNode, TransportSettings, VlessConfig};

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
    let cli_args = CliArgs::try_parse().map_err(|e: clap::error::Error| {
        if e.use_stderr() {
            let _ = e.print();
            std::process::exit(e.exit_code());
        }
        e.to_string()
    })?;

    let profile = AppProfile::resolve(cli_args.profile.as_deref(), &StdEnv)
        .map_err(|e| format!("invalid profile: {e}"))?;

    let overrides = PathOverrides::resolve(&cli_args, &StdEnv);

    let paths = AppPaths::with_overrides(profile.clone(), &overrides)
        .map_err(|err| format!("failed to determine XDG directories: {err}"))?;

    paths.ensure_dirs()
        .map_err(|err| format!("failed to create directories: {err}"))?;

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
            log::warn!("Instance stamp needs forward migration (schema version {} < {}), continuing", stamp.schema_version, v2ray_rs_core::instance::CURRENT_SCHEMA_VERSION);
        }
        CompatibilityResult::IncompatibleProfile => {
            return Err(format!("Instance profile '{}' is incompatible with current profile '{}'. Reset instance with --reset-instance to continue.", stamp.profile, profile.qualifier()));
        }
        CompatibilityResult::IncompatibleAppId => {
            return Err(format!("Instance app_id '{}' is incompatible with current app_id '{}'. Reset instance with --reset-instance to continue.", stamp.app_id, profile.app_id()));
        }
        CompatibilityResult::TooNew => {
            return Err(format!("Instance schema version {} is newer than current {}. Downgrade the application or reset instance with --reset-instance to continue.", stamp.schema_version, v2ray_rs_core::instance::CURRENT_SCHEMA_VERSION));
        }
    }

    stamp.update_started(&paths)
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

    let should_install_icons = profile == AppProfile::Production || overrides.install_icons.unwrap_or(false);
    let tray_action_rx = if should_install_icons {
        let (tray_tx, tray_rx) = tokio::sync::mpsc::unbounded_channel::<TrayAction>();
        let notifier = v2ray_rs_tray::Notifier::new(settings.notifications_enabled);
        let data_dir = paths.data_dir().to_path_buf();
        match rt.block_on(async {
            v2ray_rs_tray::TrayService::spawn_with_data_dir(event_rx, notifier, move |action| {
                let _ = tray_tx.send(action);
            }, &data_dir)
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
