use adw::prelude::*;
use relm4::adw;
use relm4::gtk;
use std::cell::RefCell;
use std::rc::Rc;

use v2ray_rs_core::models::{AppSettings, BackendLogLevel, BackendType, Language};

use super::{SettingsCallback, SettingsObservers, emit, subscribe_settings};

const CONNECTION_LOG_NOTE: &str = "Write accepted connections to the backend log";
const SINGBOX_CONNECTION_LOG_NOTE: &str =
    "sing-box writes connection lines at the info and debug levels";

pub(super) fn build_system_page(
    state: &Rc<RefCell<AppSettings>>,
    cb: &SettingsCallback,
    observers: &SettingsObservers,
) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title("System")
        .icon_name("preferences-system-symbolic")
        .build();

    let s = state.borrow();

    let interface_group = adw::PreferencesGroup::builder().title("Interface").build();

    let lang_row = adw::ComboRow::builder()
        .title("Language")
        .model(&gtk::StringList::new(&["English", "Russian"]))
        .selected(match s.language {
            Language::English => 0,
            Language::Russian => 1,
        })
        .build();
    interface_group.add(&lang_row);
    page.add(&interface_group);

    let integration_group = adw::PreferencesGroup::builder()
        .title("Integration")
        .build();

    let tray_row = adw::SwitchRow::builder()
        .title("Minimize to tray")
        .active(s.minimize_to_tray)
        .build();
    integration_group.add(&tray_row);

    let notif_row = adw::SwitchRow::builder()
        .title("Enable notifications")
        .active(s.notifications_enabled)
        .build();
    integration_group.add(&notif_row);
    page.add(&integration_group);

    drop(s);

    let (diagnostics_group, _, _) = build_diagnostics_group(state, cb, observers);
    page.add(&diagnostics_group);

    {
        let st = state.clone();
        let cb = cb.clone();
        lang_row.connect_selected_notify(move |row| {
            st.borrow_mut().language = match row.selected() {
                1 => Language::Russian,
                _ => Language::English,
            };
            emit(&st, &cb);
        });
    }
    {
        let st = state.clone();
        let cb = cb.clone();
        tray_row.connect_active_notify(move |row| {
            st.borrow_mut().minimize_to_tray = row.is_active();
            emit(&st, &cb);
        });
    }
    {
        let st = state.clone();
        let cb = cb.clone();
        notif_row.connect_active_notify(move |row| {
            st.borrow_mut().notifications_enabled = row.is_active();
            emit(&st, &cb);
        });
    }

    page
}

fn connection_log_note(backend: BackendType) -> (bool, &'static str) {
    match backend {
        BackendType::SingBox => (false, SINGBOX_CONNECTION_LOG_NOTE),
        _ => (true, CONNECTION_LOG_NOTE),
    }
}

fn build_diagnostics_group(
    state: &Rc<RefCell<AppSettings>>,
    cb: &SettingsCallback,
    observers: &SettingsObservers,
) -> (adw::PreferencesGroup, adw::ComboRow, adw::SwitchRow) {
    let group = adw::PreferencesGroup::builder()
        .title("Diagnostics")
        .build();

    let s = state.borrow();
    let levels: Vec<&str> = BackendLogLevel::ALL.iter().map(|l| l.as_str()).collect();
    let level_row = adw::ComboRow::builder()
        .title("Backend log level")
        .model(&gtk::StringList::new(&levels))
        .selected(
            BackendLogLevel::ALL
                .iter()
                .position(|l| *l == s.logging.backend_level)
                .unwrap_or_default() as u32,
        )
        .build();
    group.add(&level_row);

    let (sensitive, note) = connection_log_note(s.backend.backend_type);
    let connection_row = adw::SwitchRow::builder()
        .title("Connection log")
        .subtitle(note)
        .sensitive(sensitive)
        .active(s.logging.connection_log)
        .build();
    group.add(&connection_row);
    drop(s);

    {
        let st = state.clone();
        let cb = cb.clone();
        level_row.connect_selected_notify(move |row| {
            let Some(level) = BackendLogLevel::ALL.get(row.selected() as usize) else {
                return;
            };
            st.borrow_mut().logging.backend_level = *level;
            emit(&st, &cb);
        });
    }
    {
        let st = state.clone();
        let cb = cb.clone();
        connection_row.connect_active_notify(move |row| {
            st.borrow_mut().logging.connection_log = row.is_active();
            emit(&st, &cb);
        });
    }
    {
        let row = connection_row.clone();
        subscribe_settings(observers, move |settings| {
            let (sensitive, note) = connection_log_note(settings.backend.backend_type);
            row.set_sensitive(sensitive);
            row.set_subtitle(note);
        });
    }

    (group, level_row, connection_row)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_log_note_insensitive_for_singbox_only() {
        assert_eq!(
            connection_log_note(BackendType::SingBox),
            (false, SINGBOX_CONNECTION_LOG_NOTE)
        );
        assert_eq!(
            connection_log_note(BackendType::Xray),
            (true, CONNECTION_LOG_NOTE)
        );
        assert_eq!(
            connection_log_note(BackendType::V2ray),
            (true, CONNECTION_LOG_NOTE)
        );
    }

    #[test]
    fn diagnostics_group_tracks_backend_selection() {
        crate::gtk_test::run(|| {
            let state = Rc::new(RefCell::new(AppSettings::default()));
            let cb: SettingsCallback = Rc::new(|_| {});
            let observers: SettingsObservers = Rc::new(RefCell::new(Vec::new()));

            let (_, level_row, switch_row) = build_diagnostics_group(&state, &cb, &observers);
            assert_eq!(level_row.selected(), 1);
            assert!(!switch_row.is_active());
            assert!(switch_row.is_sensitive());
            assert_eq!(switch_row.subtitle().as_deref(), Some(CONNECTION_LOG_NOTE));
            assert_eq!(observers.borrow().len(), 1);

            let notify = |settings: &AppSettings| {
                let list: Vec<_> = observers.borrow().iter().cloned().collect();
                for observer in list {
                    observer(settings);
                }
            };

            let mut singbox = state.borrow().clone();
            singbox.backend.backend_type = BackendType::SingBox;
            singbox.logging.connection_log = true;
            switch_row.set_active(true);
            notify(&singbox);
            assert!(!switch_row.is_sensitive());
            assert!(switch_row.is_active());
            assert!(state.borrow().logging.connection_log);
            assert_eq!(
                switch_row.subtitle().as_deref(),
                Some(SINGBOX_CONNECTION_LOG_NOTE)
            );

            let mut xray = singbox.clone();
            xray.backend.backend_type = BackendType::Xray;
            notify(&xray);
            assert!(switch_row.is_sensitive());
            assert_eq!(switch_row.subtitle().as_deref(), Some(CONNECTION_LOG_NOTE));
        });
    }
}
