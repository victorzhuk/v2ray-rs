use adw::prelude::*;
use relm4::adw;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use v2ray_rs_core::backend::{DetectedBackend, backend_name};
use v2ray_rs_core::models::{AppSettings, RoutingRuleSet};

use crate::i18n::tr;
use crate::workspace::WorkspaceStore;

mod antidpi;
mod dns;
mod network;
mod routing;
mod system;
mod tun;

use antidpi::build_antidpi_page;
use dns::build_dns_page;
use network::build_network_page;
use routing::{build_routing_error_page, build_routing_page};
use system::build_system_page;
// Exported for page-targeted navigation (grant toasts, restart banner),
// mirroring `TUN_PAGE_NAME`; no caller points at the anti-DPI page yet.
#[allow(unused_imports)]
pub(crate) use antidpi::ANTIDPI_PAGE_NAME;
pub(crate) use tun::TUN_PAGE_NAME;
use tun::build_tun_page;

pub(crate) type SettingsCallback = Rc<dyn Fn(AppSettings)>;
pub(crate) type RoutingCallback = Rc<dyn Fn(RoutingRuleSet)>;
pub(crate) type ToastCallback = Rc<dyn Fn(&str)>;
pub(crate) type SettingsObserver = Rc<dyn Fn(&AppSettings)>;
pub(crate) type SettingsObservers = Rc<RefCell<Vec<SettingsObserver>>>;
pub(crate) type RoutingObservers = Rc<RefCell<Vec<Rc<dyn Fn()>>>>;

pub struct PreferencesHandle {
    pub dialog: adw::PreferencesDialog,
    settings_state: Rc<RefCell<AppSettings>>,
    settings_observers: SettingsObservers,
    nodes_observers: RoutingObservers,
    /// Monotonic revision of accepted edits, bumped once per settings emit.
    settings_revision: Rc<Cell<u64>>,
}

impl PreferencesHandle {
    /// Restores the dialog's settings to the last committed snapshot and
    /// re-renders the rows through the observers, but only when `rejected`
    /// is still the dialog's latest accepted edit revision: a newer pending
    /// edit is never overwritten. Returns whether the reconciliation ran.
    pub fn reconcile_settings_if_current(&self, committed: &AppSettings, rejected: u64) -> bool {
        if !rejected_write_reconciles(self.settings_revision.get(), rejected) {
            return false;
        }
        reconcile_settings(&self.settings_state, &self.settings_observers, committed);
        true
    }

    pub fn refresh_nodes(&self) {
        let observers: Vec<_> = self.nodes_observers.borrow().iter().cloned().collect();
        for observer in observers {
            observer();
        }
    }
}

pub fn show_preferences(
    parent: &adw::ApplicationWindow,
    store: &WorkspaceStore,
    settings: &AppSettings,
    on_settings_changed: impl Fn(u64, AppSettings) + 'static,
    on_routing_changed: impl Fn(RoutingRuleSet) + 'static,
    on_toast: impl Fn(&str) + 'static,
) -> PreferencesHandle {
    let dialog = adw::PreferencesDialog::new();
    dialog.set_title(&tr("Preferences"));

    let settings_state = Rc::new(RefCell::new(settings.clone()));
    let settings_observers: SettingsObservers = Rc::new(RefCell::new(Vec::new()));
    let settings_revision = Rc::new(Cell::new(0_u64));
    let routing_result = store.load_routing_rules();
    let routing_state = Rc::new(RefCell::new(
        routing_result.as_ref().cloned().unwrap_or_default(),
    ));
    let on_settings_changed = Rc::new(on_settings_changed);
    let settings_cb: SettingsCallback = {
        let on_settings_changed = on_settings_changed.clone();
        let settings_observers = settings_observers.clone();
        let settings_revision = settings_revision.clone();
        Rc::new(move |settings| {
            // Tag the accepted edit with the dialog's monotonic revision so
            // a failed write can be reconciled only when it is still the
            // latest accepted edit.
            let revision = settings_revision.get() + 1;
            settings_revision.set(revision);
            on_settings_changed(revision, settings.clone());

            let observers: Vec<_> = settings_observers.borrow().iter().cloned().collect();
            for observer in observers {
                observer(&settings);
            }
        })
    };
    let routing_observers: RoutingObservers = Rc::new(RefCell::new(Vec::new()));
    let routing_cb: RoutingCallback = {
        let on_routing_changed = Rc::new(on_routing_changed);
        let routing_observers = routing_observers.clone();
        Rc::new(move |rules: RoutingRuleSet| {
            on_routing_changed(rules);
            let observers: Vec<_> = routing_observers.borrow().iter().cloned().collect();
            for observer in observers {
                observer();
            }
        })
    };
    let toast_cb: ToastCallback = Rc::new(on_toast);

    let system_page = build_system_page(
        &settings_state,
        &settings_cb,
        &settings_observers,
        &toast_cb,
    );
    dialog.add(&system_page);

    let network_page = build_network_page(
        &settings_state,
        &settings_cb,
        &settings_observers,
        store.paths(),
        &toast_cb,
    );
    dialog.add(&network_page);

    let routing_page = match routing_result {
        Ok(_) => build_routing_page(store, &settings_state, &routing_state, &routing_cb),
        Err(err) => build_routing_error_page(store, &routing_cb, err.to_string()),
    };
    dialog.add(&routing_page);

    let nodes_observers: RoutingObservers = Rc::new(RefCell::new(Vec::new()));
    let dns_page = build_dns_page(
        store,
        &settings_state,
        &settings_cb,
        &settings_observers,
        &routing_state,
        &routing_observers,
        &nodes_observers,
    );
    dialog.add(&dns_page);

    let tun_page = build_tun_page(
        &settings_state,
        &settings_cb,
        &settings_observers,
        &toast_cb,
    );
    dialog.add(&tun_page);

    let (antidpi_page, _antidpi_rows) = build_antidpi_page(
        &settings_state,
        &settings_cb,
        &settings_observers,
        &toast_cb,
    );
    dialog.add(&antidpi_page);

    dialog.present(Some(parent));
    PreferencesHandle {
        dialog,
        settings_state,
        settings_observers,
        nodes_observers,
        settings_revision,
    }
}

/// Whether a failed write may reconcile the dialog's rows: the rejected
/// write's revision must still be the dialog's latest accepted edit, so a
/// newer pending edit is never overwritten and no second error is reported.
pub(crate) fn rejected_write_reconciles(latest_accepted: u64, rejected: u64) -> bool {
    latest_accepted == rejected
}

/// Restores the dialog's settings state to the last committed snapshot and
/// notifies the rows' observers so they re-render. The commit callback is
/// deliberately not invoked: the failed write already reported one error,
/// and re-emitting would schedule a second save of the restored snapshot.
pub(crate) fn reconcile_settings(
    state: &Rc<RefCell<AppSettings>>,
    observers: &SettingsObservers,
    committed: &AppSettings,
) {
    *state.borrow_mut() = committed.clone();
    let list: Vec<_> = observers.borrow().iter().cloned().collect();
    for observer in list {
        observer(committed);
    }
}

pub(crate) fn emit(state: &Rc<RefCell<AppSettings>>, cb: &SettingsCallback) {
    // Bound first: as a temporary the borrow would outlive the call, and the
    // observers it fans out to are free to take the state mutably.
    let snapshot = state.borrow().clone();
    cb(snapshot);
}

pub(crate) fn emit_routing(state: &Rc<RefCell<RoutingRuleSet>>, cb: &RoutingCallback) {
    let snapshot = state.borrow().clone();
    cb(snapshot);
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
            || c.is::<adw::ComboRow>()
        {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejected_older_write_with_newer_edit_pending_is_skipped() {
        // Flush A (revision 2) failed after the dialog already accepted
        // edit B (revision 3): the dialog must keep B, and no second error
        // toast is due for the skipped reconciliation.
        assert!(!rejected_write_reconciles(3, 2));
        // The rejected write is still the latest accepted edit: reconcile.
        assert!(rejected_write_reconciles(3, 3));
        // A failure before any edit is accepted reconciles too.
        assert!(rejected_write_reconciles(0, 0));
    }
}
