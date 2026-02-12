use gettextrs::{LocaleCategory, bindtextdomain, gettext, setlocale, textdomain};
use std::path::Path;
use v2ray_rs_core::models::Language;

const DOMAIN: &str = "v2ray-rs";

pub fn init(language: Language) {
    let locale = match language {
        Language::English => "en_US.UTF-8",
        Language::Russian => "ru_RU.UTF-8",
    };

    setlocale(LocaleCategory::LcAll, locale);

    let locale_dir = locale_dir();
    if let Some(dir) = locale_dir.to_str() {
        bindtextdomain(DOMAIN, dir).ok();
    }

    textdomain(DOMAIN).ok();
}

pub fn switch_language(language: Language) {
    let locale = match language {
        Language::English => "en_US.UTF-8",
        Language::Russian => "ru_RU.UTF-8",
    };
    setlocale(LocaleCategory::LcAll, locale);
}

pub fn tr(msgid: &str) -> String {
    gettext(msgid)
}

fn locale_dir() -> std::path::PathBuf {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()));

    if let Some(dir) = exe_dir {
        let candidate = dir.join("locale");
        if candidate.exists() {
            return candidate;
        }
        let candidate = dir.join("../share/locale");
        if candidate.exists() {
            return candidate;
        }
    }

    let dev_locale = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../locale");
    if dev_locale.exists() {
        return dev_locale;
    }

    "/usr/share/locale".into()
}
