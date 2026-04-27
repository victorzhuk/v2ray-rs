use adw::prelude::*;
use relm4::adw;
use relm4::gtk;
use std::cell::RefCell;
use std::rc::Rc;

use v2ray_rs_core::models::{AppSettings, Language};

use super::{SettingsCallback, emit};

pub(super) fn build_system_page(
    state: &Rc<RefCell<AppSettings>>,
    cb: &SettingsCallback,
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
