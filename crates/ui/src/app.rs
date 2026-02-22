use std::sync::Mutex;
use std::time::Duration;

use adw::prelude::*;
use gtk::glib;
use relm4::adw;
use relm4::prelude::*;
use tokio::sync::broadcast;

use v2ray_rs_core::config::ConfigWriter;
use v2ray_rs_core::models::{AppSettings, ConnectionMetadata, LastSuccessMetadata};
use v2ray_rs_core::persistence::{self, AppPaths};
use v2ray_rs_core::resolve::ConnectionPlanner;
use v2ray_rs_process::{ProcessEvent, ProcessState};
use v2ray_rs_tray::{TrayAction, TrayHandle};

static TRAY_HANDLE: Mutex<Option<TrayHandle>> = Mutex::new(None);
static TRAY_EVENT_TX: Mutex<Option<broadcast::Sender<ProcessEvent>>> = Mutex::new(None);

const DEFAULT_WINDOW_WIDTH: i32 = 900;
const DEFAULT_WINDOW_HEIGHT: i32 = 650;
const TRAY_POLL_INTERVAL: Duration = Duration::from_millis(200);
const EVENT_CHANNEL_CAPACITY: usize = 16;

use crate::logs::{LogsMsg, LogsPage};
use crate::subscriptions::{SubscriptionsMsg, SubscriptionsOutput, SubscriptionsPage};
use crate::wizard::OnboardingWizard;

pub struct App {
    settings: AppSettings,
    paths: AppPaths,
    subscriptions_page: Controller<SubscriptionsPage>,
    logs_page: Controller<LogsPage>,
    show_wizard: bool,
    wizard: Controller<OnboardingWizard>,
    window: adw::ApplicationWindow,
    process_handle: Option<ProcessHandle>,
    process_state: ProcessState,
    reconnect_pending: bool,
    connected: bool,
    button_sensitive: bool,
    has_active_nodes: bool,
    connection_status: Option<ConnectionMetadata>,
    status_label: gtk::Label,
    status_details: gtk::Label,
    toast_overlay: adw::ToastOverlay,
}

struct ProcessHandle {
    cmd_tx: tokio::sync::mpsc::Sender<ProcessCmd>,
}

enum ProcessCmd {
    Stop,
}

#[derive(Debug)]
pub enum AppMsg {
    OnboardingComplete(AppSettings, Option<(String, String)>),
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
                    meta.subscription_name,
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
}

#[relm4::component(pub)]
impl SimpleComponent for App {
    type Init = AppPaths;
    type Input = AppMsg;
    type Output = ();

    view! {
        adw::ApplicationWindow {
            set_default_width: DEFAULT_WINDOW_WIDTH,
            set_default_height: DEFAULT_WINDOW_HEIGHT,
            set_title: Some("V2Ray Manager"),
            set_icon_name: Some(APP_ID),

            connect_close_request[sender] => move |_| {
                sender.input(AppMsg::CloseRequested);
                gtk::glib::Propagation::Stop
            },

            if model.show_wizard {
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
                        gtk::Paned {
                            set_orientation: gtk::Orientation::Vertical,
                            set_vexpand: true,
                            set_position: 380,
                            set_shrink_start_child: false,
                            set_shrink_end_child: false,

                            #[wrap(Some)]
                            set_start_child = model.subscriptions_page.widget(),

                            #[wrap(Some)]
                            set_end_child = model.logs_page.widget(),
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
        paths: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let settings = v2ray_rs_core::persistence::load_settings(&paths).unwrap_or_default();

        let show_wizard = !paths.settings_path().exists();

        setup_tray_polling(sender.input_sender().clone());

        let subscriptions_page = SubscriptionsPage::builder()
            .launch((paths.clone(), settings.clone()))
            .forward(sender.input_sender(), |msg| match msg {
                SubscriptionsOutput::ActiveNodesChanged(has) => AppMsg::ActiveNodesChanged(has),
            });

        let logs_page = LogsPage::builder().launch(()).detach();

        let wizard = OnboardingWizard::builder().launch(paths.clone()).forward(
            sender.input_sender(),
            |msg| match msg {
                crate::wizard::WizardOutput::Complete {
                    settings,
                    subscription,
                } => AppMsg::OnboardingComplete(settings, subscription),
            },
        );

        let toast_overlay = adw::ToastOverlay::new();
        let status_label = gtk::Label::new(None);
        let status_details = gtk::Label::new(None);

        let subscriptions = persistence::load_subscriptions(&paths).unwrap_or_default();
        let has_active_nodes = subscriptions.iter().any(|s| s.has_enabled_nodes());

        let model = App {
            settings,
            paths,
            subscriptions_page,
            logs_page,
            show_wizard,
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
        };

        let _ = persistence::save_connection_state(&model.paths, &None);

        let toast_overlay = &model.toast_overlay;
        let status_label = &model.status_label;
        let status_details = &model.status_details;
        let widgets = view_output!();
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
                if let Err(e) = v2ray_rs_core::persistence::save_settings(&self.paths, &settings) {
                    log::error!("save settings: {e}");
                }
                self.settings = settings;
                self.show_wizard = false;

                if let Some((name, url)) = subscription {
                    self.subscriptions_page
                        .emit(SubscriptionsMsg::AddSubscription(name, url));
                }
            }
            AppMsg::SettingsChanged(settings) => {
                crate::i18n::switch_language(settings.language);
                if let Err(e) = v2ray_rs_core::persistence::save_settings(&self.paths, &settings) {
                    log::error!("save settings: {e}");
                }
                let was_connected = self.process_handle.is_some();
                let strategy_changed =
                    self.settings.auto_resolve_strategy != settings.auto_resolve_strategy;
                self.settings = settings;
                if was_connected && strategy_changed {
                    self.reconnect_pending = true;
                    sender.input(AppMsg::Disconnect);
                }
            }
            AppMsg::ActiveNodesChanged(has) => {
                self.has_active_nodes = has;
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

                let subscriptions =
                    persistence::load_subscriptions(&self.paths).unwrap_or_default();
                let snapshot = persistence::load_latency_snapshot(&self.paths).unwrap_or_default();
                let planner = ConnectionPlanner::new(
                    self.settings.auto_resolve_strategy,
                    self.settings.last_success.clone(),
                    snapshot,
                    Vec::new(),
                );
                let candidates = planner.plan(&subscriptions);

                if candidates.is_empty() {
                    self.show_toast("No enabled proxy nodes — add a subscription first");
                    return;
                }

                let rules = persistence::load_routing_rules(&self.paths).unwrap_or_default();
                let enabled_rules: Vec<_> = rules.enabled_rules().cloned().collect();

                let writer = ConfigWriter::new(&self.settings, &self.paths);
                let pid_path = self.paths.data_dir().join("backend.pid");
                let geodata_dir = self.paths.geodata_dir();

                self.apply_state(&ProcessState::Starting);
                self.logs_page.emit(LogsMsg::SetRunning(true));
                self.logs_page.emit(LogsMsg::Clear);

                let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<ProcessCmd>(4);
                let input_sender = sender.input_sender().clone();
                let settings = self.settings.clone();

                tokio::spawn(async move {
                    let mut connected_meta: Option<ConnectionMetadata> = None;
                    let mut last_error: Option<String> = None;

                    for candidate in candidates {
                        let config_path = match writer.write_config(
                            &[candidate.node.clone()],
                            &enabled_rules,
                            &settings,
                        ) {
                            Ok(path) => path,
                            Err(e) => {
                                last_error = Some(format!("Config generation failed: {e}"));
                                break;
                            }
                        };

                        let node_name = candidate
                            .node
                            .remark()
                            .unwrap_or(candidate.node.address())
                            .to_string();
                        let meta = ConnectionMetadata {
                            subscription_id: candidate.subscription_id,
                            subscription_name: candidate.subscription_name.clone(),
                            node_index: candidate.node_index,
                            node_name,
                            node_address: candidate.node.address().to_string(),
                            node_port: candidate.node.port(),
                            backend: settings.backend.backend_type,
                            strategy: settings.auto_resolve_strategy,
                            latency_ms: candidate.latency_ms,
                            connected_since: chrono::Utc::now(),
                        };

                        let mut mgr = v2ray_rs_process::ProcessManager::new(
                            binary_path.clone(),
                            config_path,
                            pid_path.clone(),
                            Some(geodata_dir.clone()),
                        );

                        match mgr.start_with_connection(Some(meta.clone())).await {
                            Ok(()) => {
                                input_sender.emit(AppMsg::ProcessStateConnection(
                                    ProcessState::Running,
                                    Some(meta.clone()),
                                ));
                                connected_meta = Some(meta);
                            }
                            Err(e) => {
                                last_error = Some(e.to_string());
                                mgr.shutdown().await;
                                continue;
                            }
                        }

                        let mut event_rx = mgr.subscribe();
                        let log_sender = input_sender.clone();
                        let mut log_rx = mgr.subscribe();
                        tokio::spawn(async move {
                            while let Ok(event) = log_rx.recv().await {
                                if let ProcessEvent::LogLine(line) = event {
                                    log_sender.emit(AppMsg::ProcessLogLine(line.content));
                                }
                            }
                        });

                        loop {
                            tokio::select! {
                                Some(cmd) = cmd_rx.recv() => {
                                    match cmd {
                                        ProcessCmd::Stop => {
                                            mgr.shutdown().await;
                                            input_sender.emit(AppMsg::ProcessStateConnection(
                                                ProcessState::Stopped,
                                                None,
                                            ));
                                            return;
                                        }
                                    }
                                }
                                result = event_rx.recv() => {
                                    match result {
                                        Ok(ProcessEvent::StateChanged { to, connection, .. }) => {
                                            let is_error = matches!(to, ProcessState::Error(_));
                                            input_sender.emit(AppMsg::ProcessStateConnection(to, connection));
                                            if is_error {
                                                break;
                                            }
                                        }
                                        Ok(ProcessEvent::ProcessExited { .. }) => {
                                            let _ = mgr.wait_and_handle_exit().await;
                                            let state = mgr.state();
                                            input_sender.emit(AppMsg::ProcessStateConnection(state, None));
                                            if mgr.state() != ProcessState::Running {
                                                break;
                                            }
                                        }
                                        Ok(_) => {}
                                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                                        Err(broadcast::error::RecvError::Closed) => break,
                                    }
                                }
                            }
                        }

                        if connected_meta.is_some() {
                            break;
                        }
                    }

                    if connected_meta.is_none() {
                        let msg = last_error.unwrap_or_else(|| "All candidates failed".into());
                        input_sender.emit(AppMsg::ProcessStateConnection(
                            ProcessState::Error(msg),
                            None,
                        ));
                    }
                });

                self.process_handle = Some(ProcessHandle { cmd_tx });
            }
            AppMsg::Disconnect => {
                if let Some(handle) = self.process_handle.take() {
                    self.apply_state(&ProcessState::Stopping);
                    let _ = handle.cmd_tx.try_send(ProcessCmd::Stop);
                } else {
                    self.show_toast("Not connected");
                }
            }
            AppMsg::ProcessStateConnection(state, connection) => {
                let stopped = matches!(state, ProcessState::Stopped | ProcessState::Error(_));
                if stopped {
                    self.process_handle = None;
                    self.logs_page.emit(LogsMsg::SetRunning(false));
                }
                if connection.is_some() {
                    self.connection_status = connection;
                    if let Some(meta) = &self.connection_status {
                        self.settings.last_success = Some(LastSuccessMetadata {
                            subscription_id: meta.subscription_id,
                            node_index: meta.node_index,
                            connected_at: meta.connected_since,
                        });
                        if let Err(e) =
                            v2ray_rs_core::persistence::save_settings(&self.paths, &self.settings)
                        {
                            log::error!("save settings: {e}");
                        }
                    }
                } else if matches!(state, ProcessState::Stopped | ProcessState::Error(_)) {
                    self.connection_status = None;
                }
                if let Err(e) =
                    persistence::save_connection_state(&self.paths, &self.connection_status)
                {
                    log::error!("save connection state: {e}");
                }
                self.apply_state(&state);
                if matches!(state, ProcessState::Stopped) && self.reconnect_pending {
                    self.reconnect_pending = false;
                    sender.input(AppMsg::Connect);
                }
            }
            AppMsg::ProcessLogLine(line) => {
                self.logs_page.emit(LogsMsg::AppendLine(line));
            }
            AppMsg::CloseRequested => {
                if self.settings.minimize_to_tray {
                    self.window.set_visible(false);
                } else {
                    if let Some(handle) = self.process_handle.take() {
                        let _ = handle.cmd_tx.try_send(ProcessCmd::Stop);
                    }
                    self.window.destroy();
                }
            }
            AppMsg::TrayShowWindow => {
                self.window.set_visible(true);
                self.window.present();
            }
            AppMsg::TrayQuit => {
                if let Some(handle) = self.process_handle.take() {
                    let _ = handle.cmd_tx.try_send(ProcessCmd::Stop);
                }
                self.window.destroy();
            }
            AppMsg::OpenPreferences => {
                let paths = self.paths.clone();
                let settings = self.settings.clone();
                let window = self.window.clone();
                let s = sender.input_sender().clone();
                crate::preferences::show_preferences(
                    &window,
                    &paths,
                    &settings,
                    move |new_settings| {
                        s.emit(AppMsg::SettingsChanged(new_settings));
                    },
                );
            }
        }
    }
}

fn setup_tray_polling(sender: relm4::Sender<AppMsg>) {
    glib::timeout_add_local(TRAY_POLL_INTERVAL, move || {
        if let Ok(guard) = TRAY_HANDLE.lock()
            && let Some(ref handle) = *guard
        {
            while let Some(action) = handle.try_recv_action() {
                match action {
                    TrayAction::ShowWindow => sender.emit(AppMsg::TrayShowWindow),
                    TrayAction::Quit => sender.emit(AppMsg::TrayQuit),
                    TrayAction::Connect => sender.emit(AppMsg::Connect),
                    TrayAction::Disconnect => sender.emit(AppMsg::Disconnect),
                }
            }
        }
        glib::ControlFlow::Continue
    });
}

const APP_ID: &str = "com.github.v2ray-rs";

fn install_icon_for_compositor() {
    let data_dir = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share"))
        });

    let Some(data_dir) = data_dir else { return };
    let res_prefix = format!("/{}/icons/hicolor", APP_ID.replace('.', "/"));

    let installed = install_resource_icon(
        &data_dir,
        &res_prefix,
        "scalable/apps",
        &format!("{APP_ID}.svg"),
    ) | install_resource_icon(
        &data_dir,
        &res_prefix,
        "symbolic/apps",
        &format!("{APP_ID}-symbolic.svg"),
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

    std::fs::create_dir_all(&icon_dir).is_ok() && std::fs::write(&icon_path, &svg).is_ok()
}

pub fn run() {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("failed to install rustls crypto provider");

    let paths = AppPaths::new().expect("failed to determine XDG directories");

    let settings = v2ray_rs_core::persistence::load_settings(&paths).unwrap_or_default();
    crate::i18n::init(settings.language);

    let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
    let _rt_guard = rt.enter();

    let (event_tx, event_rx) = broadcast::channel::<ProcessEvent>(EVENT_CHANNEL_CAPACITY);
    if let Ok(mut guard) = TRAY_EVENT_TX.lock() {
        *guard = Some(event_tx);
    }

    let tray_handle = rt.block_on(async {
        let notifier = v2ray_rs_tray::Notifier::new(settings.notifications_enabled);
        v2ray_rs_tray::TrayService::spawn(event_rx, notifier)
            .await
            .ok()
    });

    if let Some(handle) = tray_handle
        && let Ok(mut guard) = TRAY_HANDLE.lock()
    {
        *guard = Some(handle);
    }

    let resource_bytes =
        glib::Bytes::from_static(include_bytes!(concat!(env!("OUT_DIR"), "/icons.gresource")));
    let resource =
        gtk::gio::Resource::from_data(&resource_bytes).expect("failed to load icon resource");
    gtk::gio::resources_register(&resource);
    let app = adw::Application::builder().application_id(APP_ID).build();

    app.connect_startup(|_| {
        if let Some(display) = gtk::gdk::Display::default() {
            let theme = gtk::IconTheme::for_display(&display);
            theme.add_resource_path("/com/github/v2ray-rs/icons");
        }
        install_icon_for_compositor();
        gtk::Window::set_default_icon_name(APP_ID);
    });

    app.connect_activate(|app| {
        if let Some(window) = app.active_window() {
            window.set_visible(true);
            window.present();
        }
    });

    let relm_app = RelmApp::from_app(app);
    relm_app.run::<App>(paths);

    if let Ok(mut guard) = TRAY_HANDLE.lock()
        && let Some(handle) = guard.take()
    {
        rt.block_on(handle.shutdown());
    }
    if let Ok(mut guard) = TRAY_EVENT_TX.lock() {
        guard.take();
    }
}
