use gettextrs::{
    LocaleCategory, bind_textdomain_codeset, bindtextdomain, gettext, setlocale, textdomain,
};
use std::path::{Path, PathBuf};
use v2ray_rs_core::models::Language;

const DOMAIN: &str = "v2ray-rs";

/// Initializes message localization for the process before any worker, GTK, or
/// Tokio threads are spawned.
pub fn init_process_locale(language: Language) {
    let lang = match language {
        Language::English => "en_US",
        Language::Russian => "ru_RU",
    };
    // SAFETY: called at startup in `bootstrap::try_run` before any background
    // threads (`WorkspaceStore`, GTK, Tokio, tray) are created.
    unsafe {
        std::env::set_var("LANGUAGE", lang);
    }
    init(language);
}

pub fn init(language: Language) {
    let locale = match language {
        Language::English => "en_US.UTF-8",
        Language::Russian => "ru_RU.UTF-8",
    };

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
        && dir.exists()
    {
        return dir.to_path_buf();
    }

    if let Some(dir) = exe_dir {
        let candidate = dir.join("locale");
        if candidate.exists() {
            return candidate;
        }
        if let Some(parent) = dir.parent() {
            let candidate = parent.join("share/locale");
            if candidate.exists() {
                return candidate;
            }
        }
    }

    if dev_locale.exists() {
        return dev_locale.to_path_buf();
    }

    PathBuf::from("/usr/share/locale")
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }
}
