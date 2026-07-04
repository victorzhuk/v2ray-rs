pub(crate) mod app;
mod cli;
mod connection;
mod geodata_service;
pub mod i18n;
mod logs;
mod nodes;
mod preferences;
mod subscriptions;
mod wizard;
mod workspace;

pub use app::run;
pub use workspace::WorkspaceStore;

pub(crate) fn active_window() -> Option<relm4::gtk::Window> {
    use relm4::gtk::prelude::{Cast, GtkApplicationExt};
    relm4::gtk::gio::Application::default()
        .and_then(|app| app.downcast::<relm4::gtk::Application>().ok())
        .and_then(|app| app.active_window())
}
