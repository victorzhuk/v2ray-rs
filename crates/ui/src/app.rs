use std::sync::Mutex;
use std::time::Duration;

use relm4::prelude::*;
use relm4::adw;
use adw::prelude::*;
use gtk::glib;
use tokio::sync::broadcast;

use v2ray_rs_core::config::ConfigWriter;
use v2ray_rs_core::models::AppSettings;
use v2ray_rs_core::persistence::{self, AppPaths};
use v2ray_rs_process::{ProcessEvent, ProcessState};
use v2ray_rs_tray::{TrayAction, TrayHandle};

static TRAY_HANDLE: Mutex<Option<TrayHandle>> = Mutex::new(None);
static TRAY_EVENT_TX: Mutex<Option<broadcast::Sender<ProcessEvent>>> = Mutex::new(None);

const APP_ICON_PNG: &[u8] = include_bytes!("../../../assets/v2ray-rs.png");
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
    connected: bool,
    button_sensitive: bool,
    has_active_nodes: bool,
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
    ProcessStateChanged(ProcessState),
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
        self.subscriptions_page.emit(SubscriptionsMsg::SetLocked(locked));

        if let Ok(guard) = TRAY_EVENT_TX.lock() {
            if let Some(tx) = guard.as_ref() {
                let _ = tx.send(ProcessEvent::StateChanged {
                    from,
                    to: state.clone(),
                });
            }
        }
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

                        pack_start = &gtk::Button {
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
                }
            }
        }
    }

    fn init(paths: Self::Init, root: Self::Root, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let settings = v2ray_rs_core::persistence::load_settings(&paths)
            .unwrap_or_default();

        let show_wizard = !paths.settings_path().exists();

        setup_tray_polling(sender.input_sender().clone());

        let subscriptions_page = SubscriptionsPage::builder()
            .launch((paths.clone(), settings.clone()))
            .forward(sender.input_sender(), |msg| match msg {
                SubscriptionsOutput::ActiveNodesChanged(has) => AppMsg::ActiveNodesChanged(has),
            });

        let logs_page = LogsPage::builder()
            .launch(())
            .detach();

        let wizard = OnboardingWizard::builder()
            .launch(paths.clone())
            .forward(sender.input_sender(), |msg| {
                match msg {
                    crate::wizard::WizardOutput::Complete { settings, subscription } => {
                        AppMsg::OnboardingComplete(settings, subscription)
                    }
                }
            });

        let toast_overlay = adw::ToastOverlay::new();

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
            connected: false,
            button_sensitive: true,
            has_active_nodes,
            toast_overlay: toast_overlay.clone(),
        };

        let toast_overlay = &model.toast_overlay;
        let widgets = view_output!();

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
                    self.subscriptions_page.emit(SubscriptionsMsg::AddSubscription(name, url));
                }
            }
            AppMsg::SettingsChanged(settings) => {
                crate::i18n::switch_language(settings.language);
                if let Err(e) = v2ray_rs_core::persistence::save_settings(&self.paths, &settings) {
                    log::error!("save settings: {e}");
                }
                self.settings = settings;
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

                let subscriptions = persistence::load_subscriptions(&self.paths).unwrap_or_default();
                let nodes: Vec<_> = subscriptions.iter()
                    .filter(|s| s.enabled)
                    .flat_map(|s| s.enabled_nodes().cloned())
                    .collect();

                if nodes.is_empty() {
                    self.show_toast("No enabled proxy nodes — add a subscription first");
                    return;
                }

                let rules = persistence::load_routing_rules(&self.paths).unwrap_or_default();
                let enabled_rules: Vec<_> = rules.enabled_rules().cloned().collect();

                let writer = ConfigWriter::new(&self.settings, &self.paths);
                let config_path = match writer.write_config(&nodes, &enabled_rules, &self.settings) {
                    Ok(path) => path,
                    Err(e) => {
                        self.show_toast(&format!("Config generation failed: {e}"));
                        return;
                    }
                };

                let pid_path = self.paths.data_dir().join("backend.pid");

                self.apply_state(&ProcessState::Starting);
                self.logs_page.emit(LogsMsg::SetRunning(true));
                self.logs_page.emit(LogsMsg::Clear);

                let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<ProcessCmd>(4);
                let input_sender = sender.input_sender().clone();

                tokio::spawn(async move {
                    let mut mgr = v2ray_rs_process::ProcessManager::new(
                        binary_path, config_path, pid_path,
                    );

                    match mgr.start().await {
                        Ok(()) => {
                            input_sender.emit(AppMsg::ProcessStateChanged(
                                ProcessState::Running,
                            ));
                        }
                        Err(e) => {
                            input_sender.emit(AppMsg::ProcessStateChanged(
                                ProcessState::Error(e.to_string()),
                            ));
                            return;
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
                                        input_sender.emit(AppMsg::ProcessStateChanged(
                                            ProcessState::Stopped,
                                        ));
                                        break;
                                    }
                                }
                            }
                            result = event_rx.recv() => {
                                match result {
                                    Ok(ProcessEvent::StateChanged { to, .. }) => {
                                        let is_error = matches!(to, ProcessState::Error(_));
                                        input_sender.emit(AppMsg::ProcessStateChanged(to));
                                        if is_error {
                                            break;
                                        }
                                    }
                                    Ok(ProcessEvent::ProcessExited { .. }) => {
                                        let _ = mgr.wait_and_handle_exit().await;
                                        let state = mgr.state();
                                        input_sender.emit(AppMsg::ProcessStateChanged(state));
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
            AppMsg::ProcessStateChanged(state) => {
                let stopped = matches!(state, ProcessState::Stopped | ProcessState::Error(_));
                if stopped {
                    self.process_handle = None;
                    self.logs_page.emit(LogsMsg::SetRunning(false));
                }
                self.apply_state(&state);
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
                crate::preferences::show_preferences(&window, &paths, &settings, move |new_settings| {
                    s.emit(AppMsg::SettingsChanged(new_settings));
                });
            }
        }
    }
}

fn setup_tray_polling(sender: relm4::Sender<AppMsg>) {
    glib::timeout_add_local(TRAY_POLL_INTERVAL, move || {
        if let Ok(guard) = TRAY_HANDLE.lock() {
            if let Some(ref handle) = *guard {
                while let Some(action) = handle.try_recv_action() {
                    match action {
                        TrayAction::ShowWindow => sender.emit(AppMsg::TrayShowWindow),
                        TrayAction::Quit => sender.emit(AppMsg::TrayQuit),
                        TrayAction::Connect => sender.emit(AppMsg::Connect),
                        TrayAction::Disconnect => sender.emit(AppMsg::Disconnect),
                    }
                }
            }
        }
        glib::ControlFlow::Continue
    });
}

fn install_app_icon() {
    let data_dir = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share")));

    if let Some(data_dir) = data_dir {
        let icon_dir = data_dir.join("icons/hicolor/256x256/apps");
        if std::fs::create_dir_all(&icon_dir).is_ok() {
            let _ = std::fs::write(icon_dir.join("v2ray-rs.png"), APP_ICON_PNG);
        }
    }
}

pub fn run() {
    let paths = AppPaths::new().expect("failed to determine XDG directories");

    let settings = v2ray_rs_core::persistence::load_settings(&paths)
        .unwrap_or_default();
    crate::i18n::init(settings.language);

    let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
    let _rt_guard = rt.enter();

    let (event_tx, event_rx) = broadcast::channel::<ProcessEvent>(EVENT_CHANNEL_CAPACITY);
    if let Ok(mut guard) = TRAY_EVENT_TX.lock() {
        *guard = Some(event_tx);
    }

    let tray_handle = rt.block_on(async {
        let notifier = v2ray_rs_tray::Notifier::new(settings.notifications_enabled);
        v2ray_rs_tray::TrayService::spawn(event_rx, notifier).await.ok()
    });

    if let Some(handle) = tray_handle
        && let Ok(mut guard) = TRAY_HANDLE.lock()
    {
        *guard = Some(handle);
    }

    install_app_icon();

    let app = adw::Application::builder()
        .application_id("com.github.v2ray-rs")
        .build();

    app.connect_startup(|_| {
        gtk::Window::set_default_icon_name("v2ray-rs");
    });

    app.connect_activate(|app| {
        if let Some(window) = app.active_window() {
            window.set_visible(true);
            window.present();
        }
    });

    let relm_app = RelmApp::from_app(app);
    relm_app.run::<App>(paths);

    if let Ok(mut guard) = TRAY_HANDLE.lock() {
        if let Some(handle) = guard.take() {
            rt.block_on(handle.shutdown());
        }
    }
    if let Ok(mut guard) = TRAY_EVENT_TX.lock() {
        guard.take();
    }
}
