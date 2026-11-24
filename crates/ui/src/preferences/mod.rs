use adw::prelude::*;
use relm4::adw;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

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
use routing::{RoutingEditor, build_routing_error_page, build_routing_page};
use system::build_system_page;
// Exported for page-targeted navigation (grant toasts, restart banner),
// mirroring `TUN_PAGE_NAME`; no caller points at the anti-DPI page yet.
#[allow(unused_imports)]
pub(crate) use antidpi::ANTIDPI_PAGE_NAME;
pub(crate) use tun::TUN_PAGE_NAME;
use tun::build_tun_page;

pub(crate) type SettingsCallback = Rc<dyn Fn(AppSettings)>;
pub(crate) type RoutingCallback = Rc<dyn Fn(RoutingRuleSet)>;
/// Reports a committed routing reset. The reset page already persisted the
/// empty set, so the app must adopt it without enqueuing another save.
pub(crate) type RoutingResetCallback = Rc<dyn Fn(RoutingRuleSet)>;
pub(crate) type ToastCallback = Rc<dyn Fn(&str)>;
pub(crate) type SettingsObserver = Rc<dyn Fn(&AppSettings)>;
pub(crate) type SettingsObservers = Rc<RefCell<Vec<SettingsObserver>>>;
pub(crate) type RoutingObservers = Rc<RefCell<Vec<Rc<dyn Fn()>>>>;

pub struct PreferencesHandle {
    pub dialog: adw::PreferencesDialog,
    settings_state: Rc<RefCell<AppSettings>>,
    /// Strong owner of the pages' observers: the settings callback holds only
    /// a weak reference, so dropping the handle releases every page.
    settings_observers: SettingsObservers,
    nodes_observers: RoutingObservers,
    routing_editor: Rc<RoutingEditor>,
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

    /// Settles the display after the app's routing write receipt. Call it
    /// once per receipt with the rules the store holds afterwards: a
    /// successful save passes its committed set, a failed save passes the last
    /// committed set. The app reports the write error itself; this only
    /// settles the rows and the committed preview, so a draft is never
    /// presented as persisted.
    #[allow(dead_code)] // Wired by the app's routing receipt handler.
    pub fn apply_committed_routing(&self, rules: &RoutingRuleSet) {
        self.routing_editor.apply_committed(rules);
    }
}

pub fn show_preferences(
    parent: &adw::ApplicationWindow,
    store: &WorkspaceStore,
    settings: &AppSettings,
    on_settings_changed: impl Fn(u64, AppSettings) + 'static,
    on_routing_changed: impl Fn(RoutingRuleSet) + 'static,
    on_routing_reset: impl Fn(RoutingRuleSet) + 'static,
    on_toast: impl Fn(&str) + 'static,
) -> PreferencesHandle {
    let dialog = adw::PreferencesDialog::new();
    dialog.set_title(&tr("Preferences"));

    let settings_state = Rc::new(RefCell::new(settings.clone()));
    let settings_observers: SettingsObservers = Rc::new(RefCell::new(Vec::new()));
    let settings_revision = Rc::new(Cell::new(0_u64));
    // Warm bootstrap publishes routing rules before the dialog opens, so this
    // reads the shared snapshot instead of loading from disk on the GTK thread.
    let routing_result = store.routing_rules_snapshot();
    let routing_state = Rc::new(RefCell::new(
        routing_result
            .as_ref()
            .map(|rules| (**rules).clone())
            .unwrap_or_default(),
    ));
    let on_settings_changed = Rc::new(on_settings_changed);
    let settings_cb = settings_commit_callback(
        on_settings_changed,
        settings_revision.clone(),
        Rc::downgrade(&settings_observers),
    );
    let routing_observers: RoutingObservers = Rc::new(RefCell::new(Vec::new()));
    let routing_cb: RoutingCallback = Rc::new(on_routing_changed);
    // The reset page persists its own empty set, so its report must not
    // re-enter the edit callback and enqueue a second write.
    let routing_reset_cb: RoutingResetCallback = Rc::new(on_routing_reset);
    let routing_editor = RoutingEditor::new(
        routing_state.clone(),
        routing_cb.clone(),
        routing_observers.clone(),
    );
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
        Ok(_) => build_routing_page(store, &settings_state, &routing_editor),
        Err(err) => build_routing_error_page(store, &routing_reset_cb, err.to_string()),
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
        routing_editor,
        settings_revision,
    }
}

/// Builds the dialog's accepted-edit callback. The observer set is held
/// weakly: the pages' handlers own this callback strongly, and each observer
/// holds its rows (and their widgets), so a strong reference here would keep
/// every page alive after the dialog closes. The save reaches the app first,
/// whatever the dialog's state; only the re-render of live rows is skipped.
fn settings_commit_callback(
    on_settings_changed: Rc<dyn Fn(u64, AppSettings)>,
    revision: Rc<Cell<u64>>,
    observers: Weak<RefCell<Vec<SettingsObserver>>>,
) -> SettingsCallback {
    Rc::new(move |settings| {
        // Tag the accepted edit with the dialog's monotonic revision so a
        // failed write can be reconciled only when it is still the latest
        // accepted edit.
        let next = revision.get() + 1;
        revision.set(next);
        on_settings_changed(next, settings.clone());
        notify_settings_observers(&observers, &settings);
    })
}

/// Fans an accepted snapshot out to the dialog's live rows. A dropped
/// observer set means the dialog is closed: no widget update applies, and
/// the app's copy of the save stands.
fn notify_settings_observers(
    observers: &Weak<RefCell<Vec<SettingsObserver>>>,
    settings: &AppSettings,
) {
    let Some(observers) = observers.upgrade() else {
        return;
    };
    let list: Vec<_> = observers.borrow().iter().cloned().collect();
    for observer in list {
        observer(settings);
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
    fn accepted_save_reaches_the_app_after_the_dialog_is_gone() {
        let observers: SettingsObservers = Rc::new(RefCell::new(Vec::new()));
        let rendered = Rc::new(RefCell::new(Vec::new()));
        let rendered_sink = rendered.clone();
        subscribe_settings(&observers, move |settings| {
            rendered_sink.borrow_mut().push(settings.antidpi.enabled);
        });

        let saved = Rc::new(RefCell::new(Vec::new()));
        let saved_sink = saved.clone();
        let on_changed: Rc<dyn Fn(u64, AppSettings)> = Rc::new(move |revision, settings| {
            saved_sink
                .borrow_mut()
                .push((revision, settings.antidpi.enabled));
        });
        let revision = Rc::new(Cell::new(0_u64));
        let cb = settings_commit_callback(on_changed, revision.clone(), Rc::downgrade(&observers));

        let mut settings = AppSettings::default();
        settings.antidpi.enabled = true;
        cb(settings);
        assert_eq!(*saved.borrow(), vec![(1, true)]);
        assert_eq!(*rendered.borrow(), vec![true]);
        assert_eq!(revision.get(), 1);

        // The dialog closed: the handle's observer set is gone while the
        // page's handlers still own the callback until they are destroyed.
        drop(observers);
        let mut settings = AppSettings::default();
        settings.antidpi.queue = 42;
        cb(settings);

        // The app kept the accepted save; no widget update was attempted.
        assert_eq!(*saved.borrow(), vec![(1, true), (2, false)]);
        assert_eq!(*rendered.borrow(), vec![true]);
        assert_eq!(revision.get(), 2);
    }

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
