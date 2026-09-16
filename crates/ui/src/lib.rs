pub(crate) mod app;
mod cli;
mod config_preview;
mod connection;
pub(crate) mod failure_streak;
mod geodata_service;
pub(crate) mod health;
pub mod i18n;
pub(crate) mod logging;
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
