use adw::prelude::*;
use relm4::adw;
use std::cell::RefCell;
use std::rc::Rc;

use v2ray_rs_core::backend::{DetectedBackend, backend_name};
use v2ray_rs_core::models::{AppSettings, RoutingRuleSet};

use crate::workspace::WorkspaceStore;

mod dns;
mod network;
mod routing;
mod system;

use dns::build_dns_page;
use network::build_network_page;
use routing::{build_routing_error_page, build_routing_page};
use system::build_system_page;

pub(crate) type SettingsCallback = Rc<dyn Fn(AppSettings)>;
pub(crate) type RoutingCallback = Rc<dyn Fn(RoutingRuleSet)>;
pub(crate) type SettingsObserver = Rc<dyn Fn(&AppSettings)>;
pub(crate) type SettingsObservers = Rc<RefCell<Vec<SettingsObserver>>>;

pub fn show_preferences(
    parent: &adw::ApplicationWindow,
    store: &WorkspaceStore,
    settings: &AppSettings,
    on_settings_changed: impl Fn(AppSettings) + 'static,
    on_routing_changed: impl Fn(RoutingRuleSet) + 'static,
) -> adw::PreferencesDialog {
    let dialog = adw::PreferencesDialog::new();
    dialog.set_title("Preferences");

    let settings_state = Rc::new(RefCell::new(settings.clone()));
    let settings_observers: SettingsObservers = Rc::new(RefCell::new(Vec::new()));
    let routing_result = store.load_routing_rules();
    let routing_state = Rc::new(RefCell::new(
        routing_result.as_ref().cloned().unwrap_or_default(),
    ));
    let on_settings_changed = Rc::new(on_settings_changed);
    let settings_cb: SettingsCallback = {
        let on_settings_changed = on_settings_changed.clone();
        let settings_observers = settings_observers.clone();
        Rc::new(move |settings| {
            on_settings_changed(settings.clone());

            let observers: Vec<_> = settings_observers.borrow().iter().cloned().collect();
            for observer in observers {
                observer(&settings);
            }
        })
    };
    let routing_cb: RoutingCallback = Rc::new(on_routing_changed);

    let system_page = build_system_page(&settings_state, &settings_cb);
    dialog.add(&system_page);

    let network_page = build_network_page(
        &settings_state,
        &settings_cb,
        &settings_observers,
        store.paths(),
    );
    dialog.add(&network_page);

    let routing_page = match routing_result {
        Ok(_) => build_routing_page(store.paths(), &settings_state, &routing_state, &routing_cb),
        Err(err) => build_routing_error_page(store.paths(), err.to_string()),
    };
    dialog.add(&routing_page);

    let dns_page = build_dns_page(&settings_state, &settings_cb, &settings_observers);
    dialog.add(&dns_page);

    dialog.present(Some(parent));
    dialog
}

pub(crate) fn emit(state: &Rc<RefCell<AppSettings>>, cb: &SettingsCallback) {
    cb(state.borrow().clone());
}

pub(crate) fn emit_routing(state: &Rc<RefCell<RoutingRuleSet>>, cb: &RoutingCallback) {
    cb(state.borrow().clone());
}

pub(crate) fn subscribe_settings(
    observers: &SettingsObservers,
    observer: impl Fn(&AppSettings) + 'static,
) {
    observers.borrow_mut().push(Rc::new(observer));
}

pub(crate) fn detected_backend_subtitle(backend: &DetectedBackend) -> String {
    match &backend.version_error {
        Some(err) => format!("{} | unavailable: {err}", backend.binary_path.display()),
        None => backend.binary_path.display().to_string(),
    }
}

pub(crate) fn current_backend_status(
    settings: &AppSettings,
    detected: &[DetectedBackend],
) -> String {
    match &settings.backend.binary_path {
        Some(path) => {
            let is_detected = detected.iter().any(|backend| {
                backend.backend_type == settings.backend.backend_type
                    && backend.binary_path == *path
            });
            if is_detected {
                format!(
                    "Using detected {} at {}",
                    backend_name(settings.backend.backend_type),
                    path.display()
                )
            } else {
                format!(
                    "Using custom {} at {}",
                    backend_name(settings.backend.backend_type),
                    path.display()
                )
            }
        }
        None => "No backend path configured".to_string(),
    }
}

pub(crate) fn clear_preferences_group(group: &adw::PreferencesGroup) {
    let mut child = group.first_child();
    while let Some(c) = child {
        let next = c.next_sibling();
        // Only remove user-added rows, not internal GTK widgets (header box, etc.)
        if c.is::<adw::ActionRow>()
            || c.is::<adw::SwitchRow>()
            || c.is::<adw::EntryRow>()
            || c.is::<adw::SpinRow>()
            || c.is::<adw::ExpanderRow>()
            || c.is::<adw::ComboRow>() {
            group.remove(&c);
        }
        child = next;
    }
}

pub(crate) fn set_current_backend_status(
    status_row: &adw::ActionRow,
    settings: &AppSettings,
    detected: &[DetectedBackend],
) {
    status_row.set_subtitle(&current_backend_status(settings, detected));
}

pub(crate) fn render_detected_backends(
    group: &adw::PreferencesGroup,
    detected: &[DetectedBackend],
    state: &Rc<RefCell<AppSettings>>,
    cb: &SettingsCallback,
    detected_state: &Rc<RefCell<Vec<DetectedBackend>>>,
    custom_status_row: &adw::ActionRow,
) {
    use relm4::gtk;
    use v2ray_rs_core::backend::backend_name;
    use v2ray_rs_core::models::BackendConfig;

    clear_preferences_group(group);

    if detected.is_empty() {
        let row = adw::ActionRow::builder()
            .title("No backend found")
            .subtitle("Install v2ray, xray, or sing-box")
            .sensitive(false)
            .build();
        group.add(&row);
        return;
    }

    let mut first_check: Option<gtk::CheckButton> = None;
    for backend in detected {
        let version_str = backend
            .version
            .as_ref()
            .map(|v| format!("({v})"))
            .unwrap_or_default();
        let is_available = backend.is_available();

        let row = adw::ActionRow::builder()
            .title(format!(
                "{} {}",
                backend_name(backend.backend_type),
                version_str
            ))
            .subtitle(detected_backend_subtitle(backend))
            .activatable(is_available)
            .sensitive(is_available)
            .build();

        let check = gtk::CheckButton::builder()
            .active(
                state.borrow().backend.backend_type == backend.backend_type
                    && state.borrow().backend.binary_path.as_ref() == Some(&backend.binary_path),
            )
            .sensitive(is_available)
            .valign(gtk::Align::Center)
            .build();

        if let Some(ref first) = first_check {
            check.set_group(Some(first));
        } else {
            first_check = Some(check.clone());
        }

        let bt = backend.backend_type;
        let path = backend.binary_path.clone();
        let st = state.clone();
        let cb = cb.clone();
        let detected_state = detected_state.clone();
        let custom_status_row = custom_status_row.clone();
        check.connect_toggled(move |btn| {
            if btn.is_active() {
                let mut settings = st.borrow_mut();
                settings.backend = BackendConfig {
                    backend_type: bt,
                    binary_path: Some(path.clone()),
                    config_output_dir: settings.backend.config_output_dir.clone(),
                };
                let detected = detected_state.borrow();
                set_current_backend_status(&custom_status_row, &settings, &detected);
                drop(detected);
                drop(settings);
                emit(&st, &cb);
            }
        });

        row.add_suffix(&check);
        group.add(&row);
    }
}
