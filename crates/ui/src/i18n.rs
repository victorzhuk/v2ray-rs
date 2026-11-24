use gettextrs::{
    LocaleCategory, bind_textdomain_codeset, bindtextdomain, gettext, setlocale, textdomain,
};
use std::path::{Path, PathBuf};
use v2ray_rs_core::models::Language;

const DOMAIN: &str = "v2ray-rs";

/// Initializes message localization for the process before any worker, GTK, or
/// Tokio threads are spawned.
fn messages_locale(language: Language) -> &'static str {
    match language {
        Language::English => "en_US.UTF-8",
        Language::Russian => "ru_RU.UTF-8",
    }
}

pub fn init_process_locale(language: Language) {
    let lang = match language {
        Language::English => "en_US",
        Language::Russian => "ru_RU",
    };
    // SAFETY: called at startup in `bootstrap::try_run` before any background
    // threads (`WorkspaceStore`, GTK, Tokio, tray) are created.
    unsafe {
        std::env::set_var("LANGUAGE", lang);
        std::env::set_var("LC_MESSAGES", messages_locale(language));
    }
    init(language);
}

pub fn init(language: Language) {
    let locale = messages_locale(language);

    if setlocale(LocaleCategory::LcMessages, locale).is_none() {
        setlocale(LocaleCategory::LcMessages, "C.UTF-8");
    }

    let locale_dir = locale_dir();
    if let Some(dir) = locale_dir.to_str() {
        bindtextdomain(DOMAIN, dir).ok();
    }
    bind_textdomain_codeset(DOMAIN, "UTF-8").ok();
    textdomain(DOMAIN).ok();
}

pub fn tr(msgid: &str) -> String {
    gettext(msgid)
}

fn locale_dir() -> PathBuf {
    let env_dir = std::env::var_os("V2RAY_RS_LOCALE_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf));
    let dev_locale = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../locale");

    resolve_locale_dir(env_dir.as_deref(), exe_dir.as_deref(), &dev_locale)
}

fn resolve_locale_dir(
    env_dir: Option<&Path>,
    exe_dir: Option<&Path>,
    dev_locale: &Path,
) -> PathBuf {
    if let Some(dir) = env_dir
        && dir.is_dir()
    {
        return dir.to_path_buf();
    }

    if let Some(dir) = exe_dir {
        let candidate = dir.join("locale");
        if candidate.is_dir() {
            return candidate;
        }
        if let Some(parent) = dir.parent() {
            let candidate = parent.join("share/locale");
            if candidate.is_dir() {
                return candidate;
            }
        }
    }

    if dev_locale.is_dir() {
        return dev_locale.to_path_buf();
    }

    PathBuf::from("/usr/share/locale")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snprintf_f2(value: f64) -> String {
        let mut buf = [0 as nix::libc::c_char; 32];
        let fmt = c"%.2f";
        // SAFETY: `buf` is 32 bytes and `fmt` is a valid static C format string for one `f64`.
        let written =
            unsafe { nix::libc::snprintf(buf.as_mut_ptr(), buf.len(), fmt.as_ptr(), value) };
        assert!(written > 0);
        // SAFETY: `snprintf` NUL-terminated within `buf`.
        unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) }
            .to_str()
            .unwrap()
            .to_string()
    }

    #[test]
    fn resolve_locale_dir_ignores_a_file_named_env_or_dev_override() {
        let tmp = tempfile::tempdir().unwrap();
        let file_env = tmp.path().join("locale-file");
        std::fs::write(&file_env, b"not a directory").unwrap();
        let exe_dir = tmp.path().join("bin");
        std::fs::create_dir_all(&exe_dir).unwrap();
        let exe_share = tmp.path().join("share/locale");
        std::fs::create_dir_all(&exe_share).unwrap();
        let dev_dir = tmp.path().join("dev-locale");

        // A file where a locale directory was requested is not a catalog:
        // the valid bundled directory keeps serving messages.
        assert_eq!(
            resolve_locale_dir(Some(&file_env), Some(&exe_dir), &dev_dir),
            exe_share
        );

        let file_dev = tmp.path().join("dev-locale-file");
        std::fs::write(&file_dev, b"not a directory").unwrap();
        assert_eq!(
            resolve_locale_dir(Some(&file_env), None, &file_dev),
            PathBuf::from("/usr/share/locale")
        );
    }

    #[test]
    fn resolve_locale_dir_prefers_env_then_exe_then_dev_then_system() {
        let tmp = tempfile::tempdir().unwrap();
        let env_dir = tmp.path().join("env-locale");
        let exe_dir = tmp.path().join("bin");
        let exe_share = tmp.path().join("share/locale");
        let dev_dir = tmp.path().join("dev-locale");

        std::fs::create_dir_all(&env_dir).unwrap();
        std::fs::create_dir_all(&exe_dir).unwrap();
        std::fs::create_dir_all(&exe_share).unwrap();
        std::fs::create_dir_all(&dev_dir).unwrap();

        assert_eq!(
            resolve_locale_dir(Some(&env_dir), Some(&exe_dir), &dev_dir),
            env_dir
        );

        let missing_env = tmp.path().join("missing-env");
        assert_eq!(
            resolve_locale_dir(Some(&missing_env), Some(&exe_dir), &dev_dir),
            exe_share
        );

        let exe_local = exe_dir.join("locale");
        std::fs::create_dir_all(&exe_local).unwrap();
        assert_eq!(
            resolve_locale_dir(None, Some(&exe_dir), &dev_dir),
            exe_local
        );

        assert_eq!(resolve_locale_dir(None, None, &dev_dir), dev_dir);

        let missing_dev = tmp.path().join("missing-dev");
        assert_eq!(
            resolve_locale_dir(None, None, &missing_dev),
            PathBuf::from("/usr/share/locale")
        );
    }

    #[test]
    fn resolve_locale_dir_skips_file_named_locale() {
        let tmp = tempfile::tempdir().unwrap();
        let exe_dir = tmp.path().join("bin");
        std::fs::create_dir_all(&exe_dir).unwrap();
        let fake_locale = exe_dir.join("locale");
        std::fs::write(&fake_locale, b"not a directory").unwrap();

        let exe_share = tmp.path().join("share/locale");
        std::fs::create_dir_all(&exe_share).unwrap();
        let dev_dir = tmp.path().join("dev-locale");
        std::fs::create_dir_all(&dev_dir).unwrap();

        assert_eq!(
            resolve_locale_dir(None, Some(&exe_dir), &dev_dir),
            exe_share
        );

        let fake_share = tmp.path().join("share/locale");
        std::fs::remove_dir_all(&fake_share).unwrap();
        std::fs::write(&fake_share, b"not a directory").unwrap();
        assert_eq!(resolve_locale_dir(None, Some(&exe_dir), &dev_dir), dev_dir);
    }

    #[test]
    fn init_process_locale_sets_lc_messages_env() {
        if std::env::var_os("V2RAY_RS_TEST_I18N_MESSAGES_CHILD").is_none() {
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .env("V2RAY_RS_TEST_I18N_MESSAGES_CHILD", "1")
                .env_remove("LC_ALL")
                .env_remove("LC_MESSAGES")
                .env_remove("LANGUAGE")
                .env("LANG", "C")
                .arg("--exact")
                .arg("i18n::tests::init_process_locale_sets_lc_messages_env")
                .arg("--test-threads=1")
                .status()
                .expect("spawn isolated i18n messages test child");
            assert!(status.success(), "child i18n messages test failed");
            return;
        }

        // A later GTK `setlocale(LC_ALL, "")` rebuilds categories from the
        // environment: LC_MESSAGES must keep the selected language while
        // LC_NUMERIC keeps the C decimal separator.
        init_process_locale(Language::Russian);
        assert_eq!(std::env::var("LC_MESSAGES").as_deref(), Ok("ru_RU.UTF-8"));
        setlocale(LocaleCategory::LcAll, "");
        assert_eq!(tr("Connect"), "Подключить");
        assert_eq!(snprintf_f2(1.5), "1.50");
        assert_eq!(format!("{:.2}", 1.5_f64), "1.50");

        init_process_locale(Language::English);
        assert_eq!(std::env::var("LC_MESSAGES").as_deref(), Ok("en_US.UTF-8"));
        setlocale(LocaleCategory::LcAll, "");
        assert_eq!(tr("Connect"), "Connect");
    }

    #[test]
    fn russian_locale_keeps_dot_decimal_separator_and_translates() {
        if std::env::var_os("V2RAY_RS_TEST_I18N_CHILD").is_none() {
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .env("V2RAY_RS_TEST_I18N_CHILD", "1")
                .env_remove("LC_ALL")
                .env("LANG", "C")
                .arg("--exact")
                .arg("i18n::tests::russian_locale_keeps_dot_decimal_separator_and_translates")
                .arg("--test-threads=1")
                .status()
                .expect("spawn isolated i18n test child");
            assert!(status.success(), "child i18n test failed");
            return;
        }

        // In the isolated single-threaded child process, restore LC_NUMERIC=C
        // first (simulating normal default C numeric locale) and verify
        // `init_process_locale(Language::Russian)` does not mutate LC_NUMERIC.
        setlocale(LocaleCategory::LcNumeric, "C");
        init_process_locale(Language::Russian);

        // Verify C-level LC_NUMERIC still formats floats with `.` rather than `,`.
        let mut buf = [0 as nix::libc::c_char; 32];
        let fmt = c"%.2f";
        // SAFETY: `buf` is 32 bytes and `fmt` is a valid static C format string for one `f64`.
        let written =
            unsafe { nix::libc::snprintf(buf.as_mut_ptr(), buf.len(), fmt.as_ptr(), 1.5_f64) };
        assert!(written > 0);
        // SAFETY: `snprintf` NUL-terminated within `buf`.
        let c_num = unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) }
            .to_str()
            .unwrap();
        assert_eq!(c_num, "1.50", "LC_NUMERIC must keep '.' decimal separator");
        assert_eq!(format!("{:.2}", 1.5_f64), "1.50");

        assert_eq!(tr("Connect"), "Подключить");

        // The recovery surface is translated in the tracked catalog too.
        assert_eq!(
            tr("Cleanup failed; session rules may remain installed"),
            "Очистка не удалась; правила сессии могут остаться установленными"
        );
        assert_eq!(tr("Retry cleanup"), "Повторить очистку");
        assert_eq!(
            tr("Manual recovery required"),
            "Требуется ручное восстановление"
        );
        assert_eq!(
            tr("DPI interception degraded: {detail}. The proxy connection stays up."),
            "Перехват DPI ухудшен: {detail}. Прокси-соединение остаётся активным."
        );
        assert_eq!(
            tr("the session task has exited; the stop request could not be delivered"),
            "задача сессии завершилась; запрос остановки не удалось доставить"
        );
        assert_eq!(
            tr("the session task has exited; the cleanup retry could not be delivered"),
            "задача сессии завершилась; повтор очистки не удалось доставить"
        );
        assert_eq!(
            tr("Quit failed: the connection could not be stopped"),
            "Не удалось выйти: соединение не было остановлено"
        );
    }
}
