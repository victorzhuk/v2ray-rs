use adw::prelude::*;
use relm4::adw;
use relm4::gtk::glib;
use relm4::prelude::*;

use v2ray_rs_core::backend::{
    DetectedBackend, all_install_guidance, backend_name, detect_all, validate_custom_path,
};
use v2ray_rs_core::models::{AppSettings, BackendConfig, BackendType, SubscriptionSource};

use crate::subscriptions::{SubscriptionSourceInput, subscription_source_from_inputs};

pub struct OnboardingWizard {
    settings: AppSettings,
    detected_backends: Vec<DetectedBackend>,
    detecting_backends: bool,
    backend_detection_error: Option<String>,
    backend_list_container: gtk::Box,
    selected_backend: Option<(BackendType, std::path::PathBuf)>,
    current_page: usize,
    subscription_name: String,
    subscription_url: String,
    subscription_file_path: String,
    custom_backend_type: BackendType,
    custom_backend_path: String,
    custom_backend_status: Option<String>,
    validating_custom_backend: bool,
    custom_validation_generation: u64,
}

#[derive(Debug)]
pub enum WizardMsg {
    NextPage,
    LoadDetectedBackends,
    DetectedBackendsLoaded(Vec<DetectedBackend>),
    DetectedBackendsFailed(String),
    BackendSelected(BackendType, std::path::PathBuf),
    SubscriptionNameChanged(String),
    SubscriptionUrlChanged(String),
    SubscriptionFilePathChanged(String),
    ImportSubscription,
    SkipSubscription,
    CustomBackendTypeChanged(u32),
    CustomBackendPathChanged(String),
    ValidateCustomBackend,
    CustomBackendValidated {
        request_id: u64,
        result: Result<DetectedBackend, String>,
    },
    Complete,
}

#[derive(Debug)]
pub enum WizardOutput {
    Complete {
        settings: AppSettings,
        subscription: Option<(String, SubscriptionSource)>,
    },
}

#[relm4::component(pub)]
impl SimpleComponent for OnboardingWizard {
    type Init = ();
    type Input = WizardMsg;
    type Output = WizardOutput;

    view! {
        gtk::Box {
            set_orientation: gtk::Orientation::Vertical,
            set_vexpand: true,

            gtk::Stack {
                set_vexpand: true,
                set_transition_type: gtk::StackTransitionType::SlideLeftRight,
                set_transition_duration: 300,
                #[watch]
                set_visible_child_name: match model.current_page {
                    0 => "welcome",
                    1 => "backend",
                    2 => "subscription",
                    _ => "complete",
                },

                add_named[Some("welcome")] = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_vexpand: true,
                    set_valign: gtk::Align::Center,

                    adw::StatusPage {
                        set_icon_name: Some("network-vpn-symbolic"),
                        set_title: "Welcome to V2Ray Manager",
                        set_description: Some("A desktop GUI for managing v2ray, xray, and sing-box proxy configurations.\n\nLet's get started with the initial setup."),
                        set_vexpand: true,
                    },

                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_halign: gtk::Align::Center,
                        set_spacing: 12,
                        set_margin_all: 24,

                        gtk::Button {
                            set_label: "Next",
                            add_css_class: "pill",
                            add_css_class: "suggested-action",
                            connect_clicked => WizardMsg::NextPage,
                        },
                    },
                },

                add_named[Some("backend")] = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_vexpand: true,

                    adw::HeaderBar {
                        set_show_end_title_buttons: false,
                    },

                    gtk::ScrolledWindow {
                        set_vexpand: true,
                        set_hscrollbar_policy: gtk::PolicyType::Never,

                        adw::Clamp {
                            set_maximum_size: 600,

                            gtk::Box {
                                set_orientation: gtk::Orientation::Vertical,
                                set_spacing: 24,
                                set_margin_all: 24,

                                adw::StatusPage {
                                    set_icon_name: Some("application-x-executable-symbolic"),
                                    set_title: "Select Backend",
                                    set_description: Some("Choose which proxy backend to use"),
                                },

                                #[name = "backend_list_container"]
                                gtk::Box {
                                    set_orientation: gtk::Orientation::Vertical,
                                },

                                adw::PreferencesGroup {
                                    set_title: "Custom Backend Path",
                                    set_description: Some("Validate and use a manually provided backend binary"),

                                    adw::ComboRow {
                                        set_title: "Backend type",
                                        set_model: Some(&gtk::StringList::new(&["v2ray", "xray", "sing-box"])),
                                        #[watch]
                                        set_selected: model.custom_backend_type.to_index(),
                                        connect_selected_notify[sender] => move |row| {
                                            sender.input(WizardMsg::CustomBackendTypeChanged(row.selected()));
                                        },
                                    },

                                    adw::EntryRow {
                                        set_title: "Binary path",
                                        #[watch]
                                        set_text: &model.custom_backend_path,
                                        connect_changed[sender] => move |entry| {
                                            sender.input(WizardMsg::CustomBackendPathChanged(entry.text().to_string()));
                                        },
                                    },

                                    adw::ActionRow {
                                        set_title: "Custom path status",
                                        #[watch]
                                        set_subtitle: model
                                            .custom_backend_status
                                            .as_deref()
                                            .unwrap_or("Not validated"),
                                    },
                                },

                                gtk::Box {
                                    set_orientation: gtk::Orientation::Horizontal,
                                    set_halign: gtk::Align::Center,
                                    set_spacing: 12,

                                    gtk::Spinner {
                                        #[watch]
                                        set_visible: model.detecting_backends || model.validating_custom_backend,
                                        #[watch]
                                        set_spinning: model.detecting_backends || model.validating_custom_backend,
                                    },

                                    gtk::Button {
                                        set_label: "Validate Custom Path",
                                        add_css_class: "pill",
                                        #[watch]
                                        set_sensitive: !model.custom_backend_path.trim().is_empty() && !model.validating_custom_backend,
                                        connect_clicked => WizardMsg::ValidateCustomBackend,
                                    },

                                    #[name = "backend_next_button"]
                                    gtk::Button {
                                        set_label: "Next",
                                        add_css_class: "pill",
                                        add_css_class: "suggested-action",
                                        #[watch]
                                        set_sensitive: model.selected_backend.is_some() && !model.detecting_backends,
                                        connect_clicked => WizardMsg::NextPage,
                                    },
                                },
                            },
                        },
                    },
                },

                add_named[Some("subscription")] = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_vexpand: true,

                    adw::HeaderBar {
                        set_show_end_title_buttons: false,
                    },

                    gtk::ScrolledWindow {
                        set_vexpand: true,
                        set_hscrollbar_policy: gtk::PolicyType::Never,

                        adw::Clamp {
                            set_maximum_size: 600,

                            gtk::Box {
                                set_orientation: gtk::Orientation::Vertical,
                                set_spacing: 24,
                                set_margin_all: 24,

                                adw::StatusPage {
                                    set_icon_name: Some("folder-download-symbolic"),
                                    set_title: "Import Subscription",
                                    set_description: Some("Add either a proxy subscription URL or a local file path (optional)"),
                                },

                                adw::PreferencesGroup {
                                    adw::EntryRow {
                                        set_title: "Subscription Name",
                                        connect_changed[sender] => move |entry| {
                                            sender.input(WizardMsg::SubscriptionNameChanged(entry.text().to_string()));
                                        },
                                    },

                                    #[name = "subscription_entry"]
                                    adw::EntryRow {
                                        set_title: "Subscription URL",
                                        connect_changed[sender] => move |entry| {
                                            sender.input(WizardMsg::SubscriptionUrlChanged(entry.text().to_string()));
                                        },
                                    },

                                    adw::EntryRow {
                                        set_title: "Local File Path",
                                        connect_changed[sender] => move |entry| {
                                            sender.input(WizardMsg::SubscriptionFilePathChanged(entry.text().to_string()));
                                        },
                                    },
                                },

                                gtk::Box {
                                    set_orientation: gtk::Orientation::Horizontal,
                                    set_halign: gtk::Align::Center,
                                    set_spacing: 12,

                                    gtk::Button {
                                        set_label: "Skip",
                                        add_css_class: "pill",
                                        connect_clicked => WizardMsg::SkipSubscription,
                                    },

                                    gtk::Button {
                                        set_label: "Import",
                                        add_css_class: "pill",
                                        add_css_class: "suggested-action",
                                        #[watch]
                                        set_sensitive: subscription_inputs_valid(&model.subscription_url, &model.subscription_file_path),
                                        connect_clicked => WizardMsg::ImportSubscription,
                                    },
                                },
                            },
                        },
                    },
                },

                add_named[Some("complete")] = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_vexpand: true,
                    set_valign: gtk::Align::Center,

                    adw::StatusPage {
                        set_icon_name: Some("emblem-ok-symbolic"),
                        set_title: "Setup Complete",
                        set_description: Some("You're all set! Click Finish to start using V2Ray Manager."),
                        set_vexpand: true,
                    },

                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_halign: gtk::Align::Center,
                        set_spacing: 12,
                        set_margin_all: 24,

                        gtk::Button {
                            set_label: "Finish",
                            add_css_class: "pill",
                            add_css_class: "suggested-action",
                            connect_clicked => WizardMsg::Complete,
                        },
                    },
                },
            },
        }
    }

    fn init(
        _init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let mut model = OnboardingWizard {
            settings: AppSettings::default(),
            detected_backends: Vec::new(),
            detecting_backends: true,
            backend_detection_error: None,
            backend_list_container: gtk::Box::new(gtk::Orientation::Vertical, 0),
            selected_backend: None,
            current_page: 0,
            subscription_name: String::new(),
            subscription_url: String::new(),
            subscription_file_path: String::new(),
            custom_backend_type: BackendType::Xray,
            custom_backend_path: String::new(),
            custom_backend_status: None,
            validating_custom_backend: false,
            custom_validation_generation: 0,
        };

        let widgets = view_output!();
        model.backend_list_container = widgets.backend_list_container.clone();
        render_wizard_backend_list(
            &model.backend_list_container,
            &model.detected_backends,
            &model.selected_backend,
            sender.clone(),
            model.detecting_backends,
            model.backend_detection_error.as_deref(),
        );
        sender.input(WizardMsg::LoadDetectedBackends);

        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>) {
        match msg {
            WizardMsg::NextPage => {
                self.current_page += 1;
            }
            WizardMsg::LoadDetectedBackends => {
                self.detecting_backends = true;
                self.backend_detection_error = None;
                render_wizard_backend_list(
                    &self.backend_list_container,
                    &self.detected_backends,
                    &self.selected_backend,
                    sender.clone(),
                    self.detecting_backends,
                    self.backend_detection_error.as_deref(),
                );
                spawn_backend_detection(sender);
            }
            WizardMsg::DetectedBackendsLoaded(detected_backends) => {
                self.detected_backends = detected_backends;
                self.detecting_backends = false;
                self.backend_detection_error = None;

                if self.selected_backend.is_none() {
                    self.selected_backend = auto_selected_backend(&self.detected_backends);
                    if let Some((backend_type, binary_path)) = &self.selected_backend {
                        self.settings.backend = BackendConfig {
                            backend_type: *backend_type,
                            binary_path: Some(binary_path.clone()),
                            config_output_dir: None,
                        };
                        self.custom_backend_status = Some(format!(
                            "Using {} at {}",
                            backend_name(*backend_type),
                            binary_path.display()
                        ));
                    }
                }

                render_wizard_backend_list(
                    &self.backend_list_container,
                    &self.detected_backends,
                    &self.selected_backend,
                    sender.clone(),
                    self.detecting_backends,
                    self.backend_detection_error.as_deref(),
                );
            }
            WizardMsg::DetectedBackendsFailed(error) => {
                self.detecting_backends = false;
                self.backend_detection_error = Some(error);
                render_wizard_backend_list(
                    &self.backend_list_container,
                    &self.detected_backends,
                    &self.selected_backend,
                    sender.clone(),
                    self.detecting_backends,
                    self.backend_detection_error.as_deref(),
                );
            }
            WizardMsg::BackendSelected(backend_type, binary_path) => {
                self.selected_backend = Some((backend_type, binary_path.clone()));
                self.custom_backend_status = Some(format!(
                    "Using {} at {}",
                    backend_name(backend_type),
                    binary_path.display()
                ));
                self.settings.backend = BackendConfig {
                    backend_type,
                    binary_path: Some(binary_path),
                    config_output_dir: None,
                };
            }
            WizardMsg::SubscriptionNameChanged(name) => {
                self.subscription_name = name;
            }
            WizardMsg::SubscriptionUrlChanged(url) => {
                self.subscription_url = url;
            }
            WizardMsg::SubscriptionFilePathChanged(path) => {
                self.subscription_file_path = path;
            }
            WizardMsg::ImportSubscription => {
                if subscription_inputs_valid(&self.subscription_url, &self.subscription_file_path) {
                    self.current_page = 3;
                }
            }
            WizardMsg::SkipSubscription => {
                self.current_page = 3;
            }
            WizardMsg::CustomBackendTypeChanged(selected) => {
                self.custom_backend_type =
                    BackendType::from_index(selected).unwrap_or(BackendType::Xray);
                self.custom_validation_generation =
                    self.custom_validation_generation.wrapping_add(1);
                self.validating_custom_backend = false;
                self.custom_backend_status = None;
            }
            WizardMsg::CustomBackendPathChanged(path) => {
                self.custom_backend_path = path;
                self.custom_validation_generation =
                    self.custom_validation_generation.wrapping_add(1);
                self.validating_custom_backend = false;
                self.custom_backend_status = None;
            }
            WizardMsg::ValidateCustomBackend => {
                let path = std::path::PathBuf::from(self.custom_backend_path.trim());
                self.custom_validation_generation =
                    self.custom_validation_generation.wrapping_add(1);
                self.validating_custom_backend = true;
                self.custom_backend_status = Some("Validating custom backend path...".into());
                spawn_custom_backend_validation(
                    sender.clone(),
                    self.custom_validation_generation,
                    path,
                    self.custom_backend_type,
                );
            }
            WizardMsg::CustomBackendValidated { request_id, result } => {
                if request_id != self.custom_validation_generation {
                    return;
                }
                self.validating_custom_backend = false;
                match result {
                    Ok(detected) => {
                        self.custom_backend_status = Some(format!(
                            "Validated {} {}",
                            backend_name(detected.backend_type),
                            detected.version.as_deref().unwrap_or_default()
                        ));
                        self.selected_backend =
                            Some((detected.backend_type, detected.binary_path.clone()));
                        self.settings.backend = BackendConfig {
                            backend_type: detected.backend_type,
                            binary_path: Some(detected.binary_path),
                            config_output_dir: None,
                        };
                        render_wizard_backend_list(
                            &self.backend_list_container,
                            &self.detected_backends,
                            &self.selected_backend,
                            sender.clone(),
                            self.detecting_backends,
                            self.backend_detection_error.as_deref(),
                        );
                    }
                    Err(err) => {
                        self.custom_backend_status = Some(err);
                    }
                }
            }
            WizardMsg::Complete => {
                let mut settings = self.settings.clone();
                settings.onboarding_complete = true;

                let source = subscription_source_from_inputs(
                    &self.subscription_url,
                    &self.subscription_file_path,
                    "",
                )
                .and_then(|input| match input {
                    SubscriptionSourceInput::Url(url) => Some(SubscriptionSource::Url { url }),
                    SubscriptionSourceInput::File(path) => Some(SubscriptionSource::File { path }),
                    SubscriptionSourceInput::Paste(_) => None,
                });
                let subscription = if let Some(source) = source {
                    let name = if self.subscription_name.trim().is_empty() {
                        default_subscription_name(&source).unwrap_or_else(|| "Subscription".into())
                    } else {
                        self.subscription_name.clone()
                    };
                    Some((name, source))
                } else {
                    None
                };

                let _ = sender.output(WizardOutput::Complete {
                    settings,
                    subscription,
                });
            }
        }
    }
}

fn render_wizard_backend_list(
    container: &gtk::Box,
    detected_backends: &[DetectedBackend],
    selected_backend: &Option<(BackendType, std::path::PathBuf)>,
    sender: ComponentSender<OnboardingWizard>,
    detecting_backends: bool,
    backend_detection_error: Option<&str>,
) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }

    if detecting_backends {
        let status = adw::StatusPage::builder()
            .icon_name("system-search-symbolic")
            .title("Scanning Installed Backends")
            .description("Checking common install paths and backend version output.")
            .build();
        container.append(&status);
        return;
    }

    if let Some(error) = backend_detection_error {
        let status = adw::StatusPage::builder()
            .icon_name("dialog-warning-symbolic")
            .title("Backend Scan Failed")
            .description(format!(
                "{error}\n\nYou can still validate a custom backend path below."
            ))
            .build();
        container.append(&status);
        return;
    }

    if detected_backends.is_empty() {
        let status = adw::StatusPage::builder()
            .icon_name("dialog-error-symbolic")
            .title("No Backend Found")
            .description(all_install_guidance())
            .build();
        container.append(&status);
        return;
    }

    let group = adw::PreferencesGroup::builder()
        .title("Detected Backends")
        .build();

    let mut first_check: Option<gtk::CheckButton> = None;
    for backend in detected_backends {
        let (row, check) = create_wizard_backend_row(
            backend,
            selected_backend,
            sender.clone(),
            first_check.as_ref(),
        );
        if first_check.is_none() {
            first_check = Some(check);
        }
        group.add(&row);
    }

    container.append(&group);
}

fn spawn_backend_detection(sender: ComponentSender<OnboardingWizard>) {
    glib::MainContext::default().spawn_local(async move {
        match tokio::task::spawn_blocking(detect_all).await {
            Ok(detected_backends) => {
                sender.input(WizardMsg::DetectedBackendsLoaded(detected_backends))
            }
            Err(err) => sender.input(WizardMsg::DetectedBackendsFailed(err.to_string())),
        }
    });
}

fn spawn_custom_backend_validation(
    sender: ComponentSender<OnboardingWizard>,
    request_id: u64,
    path: std::path::PathBuf,
    backend_type: BackendType,
) {
    glib::MainContext::default().spawn_local(async move {
        let result = tokio::task::spawn_blocking(move || {
            validate_custom_path(&path, backend_type).map_err(|err| err.to_string())
        })
        .await
        .map_err(|err| err.to_string())
        .and_then(|result| result);
        sender.input(WizardMsg::CustomBackendValidated { request_id, result });
    });
}

fn create_wizard_backend_row(
    backend: &DetectedBackend,
    selected: &Option<(BackendType, std::path::PathBuf)>,
    sender: ComponentSender<OnboardingWizard>,
    group_btn: Option<&gtk::CheckButton>,
) -> (adw::ActionRow, gtk::CheckButton) {
    let version_str = backend
        .version
        .as_ref()
        .map(|v| format!("({})", v))
        .unwrap_or_default();
    let is_available = backend.is_available();
    let subtitle = match &backend.version_error {
        Some(err) => format!("{} | unavailable: {err}", backend.binary_path.display()),
        None => backend.binary_path.display().to_string(),
    };

    let row = adw::ActionRow::builder()
        .title(format!(
            "{} {}",
            backend_name(backend.backend_type),
            version_str
        ))
        .subtitle(subtitle)
        .activatable(is_available)
        .sensitive(is_available)
        .build();

    let is_selected = selected
        .as_ref()
        .map(|(bt, path)| *bt == backend.backend_type && *path == backend.binary_path)
        .unwrap_or(false);

    let check = gtk::CheckButton::builder()
        .active(is_selected)
        .sensitive(is_available)
        .valign(gtk::Align::Center)
        .build();

    if let Some(first) = group_btn {
        check.set_group(Some(first));
    }

    let bt = backend.backend_type;
    let path = backend.binary_path.clone();
    check.connect_toggled(move |btn| {
        if btn.is_active() {
            sender.input(WizardMsg::BackendSelected(bt, path.clone()));
        }
    });

    row.add_suffix(&check);
    (row, check)
}

fn extract_host(url: &str) -> Option<String> {
    let after_scheme = url.split("://").nth(1).unwrap_or(url);
    let host = after_scheme.split('/').next()?;
    let host = host.split(':').next()?;
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

fn extract_file_name(path: &str) -> Option<String> {
    let path = std::path::Path::new(path);
    path.file_stem()
        .or_else(|| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
}

fn subscription_inputs_valid(url: &str, file_path: &str) -> bool {
    !url.trim().is_empty() ^ !file_path.trim().is_empty()
}

fn default_subscription_name(source: &SubscriptionSource) -> Option<String> {
    match source {
        SubscriptionSource::Url { url } => extract_host(url),
        SubscriptionSource::File { path } => extract_file_name(path),
    }
}

fn auto_selected_backend(
    detected_backends: &[DetectedBackend],
) -> Option<(BackendType, std::path::PathBuf)> {
    let mut available = detected_backends
        .iter()
        .filter(|backend| backend.is_available());
    let backend = available.next()?;
    if available.next().is_some() {
        return None;
    }
    Some((backend.backend_type, backend.binary_path.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auto_selected_backend_only_picks_single_available() {
        let detected = vec![
            DetectedBackend {
                backend_type: BackendType::Xray,
                binary_path: "/usr/bin/xray".into(),
                version: Some("xray 1.0".into()),
                version_error: None,
            },
            DetectedBackend {
                backend_type: BackendType::V2ray,
                binary_path: "/usr/bin/v2ray".into(),
                version: None,
                version_error: Some("probe failed".into()),
            },
        ];

        assert_eq!(
            auto_selected_backend(&detected),
            Some((BackendType::Xray, "/usr/bin/xray".into()))
        );
    }

    #[test]
    fn test_auto_selected_backend_requires_unique_available_backend() {
        let detected = vec![
            DetectedBackend {
                backend_type: BackendType::Xray,
                binary_path: "/usr/bin/xray".into(),
                version: Some("xray 1.0".into()),
                version_error: None,
            },
            DetectedBackend {
                backend_type: BackendType::SingBox,
                binary_path: "/usr/bin/sing-box".into(),
                version: Some("sing-box 1.0".into()),
                version_error: None,
            },
        ];

        assert!(auto_selected_backend(&detected).is_none());
    }
}
