use adw::prelude::*;
use relm4::adw;
use relm4::prelude::*;

use v2ray_rs_core::backend::{DetectedBackend, all_install_guidance, backend_name, detect_all};
use v2ray_rs_core::models::{AppSettings, BackendConfig, BackendType};
use v2ray_rs_core::persistence::AppPaths;

pub struct OnboardingWizard {
    _paths: AppPaths,
    settings: AppSettings,
    _detected_backends: Vec<DetectedBackend>,
    selected_backend: Option<(BackendType, std::path::PathBuf)>,
    current_page: usize,
    subscription_name: String,
    subscription_url: String,
}

#[derive(Debug)]
pub enum WizardMsg {
    NextPage,
    BackendSelected(BackendType, std::path::PathBuf),
    SubscriptionNameChanged(String),
    SubscriptionUrlChanged(String),
    ImportSubscription,
    SkipSubscription,
    Complete,
}

#[derive(Debug)]
pub enum WizardOutput {
    Complete {
        settings: AppSettings,
        subscription: Option<(String, String)>,
    },
}

#[relm4::component(pub)]
impl SimpleComponent for OnboardingWizard {
    type Init = AppPaths;
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

                                gtk::Box {
                                    set_orientation: gtk::Orientation::Horizontal,
                                    set_halign: gtk::Align::Center,
                                    set_spacing: 12,

                                    #[name = "backend_next_button"]
                                    gtk::Button {
                                        set_label: "Next",
                                        add_css_class: "pill",
                                        add_css_class: "suggested-action",
                                        #[watch]
                                        set_sensitive: model.selected_backend.is_some(),
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
                                    set_description: Some("Add your proxy subscription URL (optional)"),
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
                                        set_sensitive: !model.subscription_url.is_empty(),
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
        paths: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let detected_backends = detect_all();

        let model = OnboardingWizard {
            _paths: paths,
            settings: AppSettings::default(),
            _detected_backends: detected_backends.clone(),
            selected_backend: None,
            current_page: 0,
            subscription_name: String::new(),
            subscription_url: String::new(),
        };

        let widgets = view_output!();

        if detected_backends.is_empty() {
            let status = adw::StatusPage::builder()
                .icon_name("dialog-error-symbolic")
                .title("No Backend Found")
                .description(all_install_guidance())
                .build();
            widgets.backend_list_container.append(&status);
        } else {
            let group = adw::PreferencesGroup::builder()
                .title("Detected Backends")
                .build();

            let mut first_check: Option<gtk::CheckButton> = None;
            for backend in &detected_backends {
                let (row, check) =
                    create_wizard_backend_row(backend, &None, sender.clone(), first_check.as_ref());
                if first_check.is_none() {
                    first_check = Some(check);
                }
                group.add(&row);
            }

            widgets.backend_list_container.append(&group);
        }

        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>) {
        match msg {
            WizardMsg::NextPage => {
                self.current_page += 1;
            }
            WizardMsg::BackendSelected(backend_type, binary_path) => {
                self.selected_backend = Some((backend_type, binary_path.clone()));
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
            WizardMsg::ImportSubscription => {
                if !self.subscription_url.is_empty() {
                    self.current_page = 3;
                }
            }
            WizardMsg::SkipSubscription => {
                self.current_page = 3;
            }
            WizardMsg::Complete => {
                let mut settings = self.settings.clone();
                settings.onboarding_complete = true;

                let subscription = if !self.subscription_url.is_empty() {
                    let name = if self.subscription_name.trim().is_empty() {
                        extract_host(&self.subscription_url)
                            .unwrap_or_else(|| "Subscription".into())
                    } else {
                        self.subscription_name.clone()
                    };
                    Some((name, self.subscription_url.clone()))
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

    let row = adw::ActionRow::builder()
        .title(format!(
            "{} {}",
            backend_name(backend.backend_type),
            version_str
        ))
        .subtitle(backend.binary_path.display().to_string())
        .activatable(true)
        .build();

    let is_selected = selected
        .as_ref()
        .map(|(bt, _)| *bt == backend.backend_type)
        .unwrap_or(false);

    let check = gtk::CheckButton::builder()
        .active(is_selected)
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
