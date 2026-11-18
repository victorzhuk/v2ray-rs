use super::*;
pub(super) fn tray_available() -> bool {
    TRAY_HANDLE
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().map(|_| ()))
        .is_some()
}

/// Whether the session has a StatusNotifier host that will actually display a
/// tray icon. Wayland compositors without an AppIndicator host accept the item
/// registration but never render it, so hiding the window there would strand a
/// running process with no way back.
pub(super) fn status_notifier_host_registered() -> bool {
    let Ok(connection) =
        gtk::gio::bus_get_sync(gtk::gio::BusType::Session, None::<&gtk::gio::Cancellable>)
    else {
        return false;
    };

    for name in [
        "org.kde.StatusNotifierWatcher",
        "org.freedesktop.StatusNotifierWatcher",
    ] {
        let arguments = (
            "org.kde.StatusNotifierWatcher",
            "IsStatusNotifierHostRegistered",
        );
        let reply = connection.call_sync(
            Some(name),
            "/StatusNotifierWatcher",
            "org.freedesktop.DBus.Properties",
            "Get",
            Some(&arguments.to_variant()),
            Some(glib::VariantTy::new("(v)").expect("valid variant type")),
            gtk::gio::DBusCallFlags::NONE,
            1_000,
            None::<&gtk::gio::Cancellable>,
        );
        if let Ok(reply) = reply
            && let Some((value,)) = reply.get::<(glib::Variant,)>()
            && let Some(registered) = value.get::<bool>()
        {
            return registered;
        }
    }

    false
}

pub(super) fn update_tray_notification_setting(enabled: bool) {
    if let Ok(mut guard) = TRAY_HANDLE.lock()
        && let Some(handle) = guard.as_mut()
    {
        handle.set_notifications_enabled(enabled);
    }
}

pub(super) fn notify_tray(summary: &str, body: &str) {
    if let Ok(guard) = TRAY_HANDLE.lock()
        && let Some(handle) = guard.as_ref()
    {
        handle.notify(summary, body);
    }
}

pub(super) fn cleanup_orphaned_backend(paths: &AppPaths) -> std::io::Result<bool> {
    let pid_file = PidFile::new(paths.pid_file_path());
    pid_file.check_and_kill_orphaned()
}

/// The PID file is removed on every clean stop and crash exit, so finding one
/// at startup means the previous run ended without cleaning up.
pub(super) fn record_unclean_previous_run(paths: &AppPaths) -> bool {
    let pid_file = PidFile::new(paths.pid_file_path());
    if !paths.pid_file_path().exists() {
        return false;
    }
    let pid = match pid_file.read() {
        Ok(Some(record)) => record.pid.to_string(),
        _ => "unknown".to_string(),
    };
    let killed = cleanup_orphaned_backend(paths).unwrap_or_else(|err| {
        log::warn!("failed to clean orphaned backend process: {err}");
        false
    });
    match RotatingFileWriter::open(paths.logs_dir().join("backend.log"), DEFAULT_MAX_BYTES) {
        Ok(log) => log.append_line(
            "notice",
            &format!("unclean previous run left backend pid={pid} killed={killed}"),
        ),
        Err(err) => log::warn!("open backend log: {err}"),
    }
    killed
}

pub(super) struct RecoveryFailure {
    pub(super) session: Option<TunSession>,
    pub(super) timed_out: bool,
    /// Journal file the operator must remove once kernel cleanup is verified.
    pub(super) journal_file: Option<String>,
}

pub(super) fn recovery_hint(session: &TunSession) -> String {
    format!(
        "{} interface {}",
        recover_flag(session.backend).trim_start_matches("--"),
        session.iface
    )
}

pub(super) fn recover_flag(backend: BackendType) -> &'static str {
    match backend {
        BackendType::SingBox => "--singbox",
        _ => "--xray",
    }
}

/// Name of the journal file the loaded record lives in:
/// `load_tun_session_result` prefers the versioned file, so its presence
/// identifies the loaded path.
pub(super) fn unverifiable_journal_file(paths: &AppPaths) -> String {
    let path = if paths.tun_session_path().exists() {
        paths.tun_session_path()
    } else {
        paths.legacy_tun_session_path()
    };
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// Retires the journal after a clean stop only when it is safe to discard:
/// ownership proven, phase Complete, and no kernel resources recorded. A
/// journal left by a failed recovery has unfinished work in it and must
/// survive — it is the only evidence of leaked kernel state, and deleting it
/// would permanently block later TUN starts. An absent journal, a load
/// error, missing ownership, and every unfinished phase keep the file.
pub(super) fn retire_stopped_tun_journal(
    paths: &AppPaths,
) -> Result<(), v2ray_rs_core::persistence::PersistenceError> {
    let Some(session) = v2ray_rs_core::persistence::load_tun_session_result(paths)? else {
        return Ok(());
    };
    let retirable = session.ownership.as_ref().is_some_and(|ownership| {
        ownership.phase == v2ray_rs_core::persistence::TunPhase::Complete
            && ownership.resources.is_empty()
            && ownership.pending.is_none()
    });
    if retirable {
        v2ray_rs_core::persistence::clear_tun_session(paths)
    } else {
        Ok(())
    }
}

/// If a TUN session marker is present, run the route helper's recovery pass
/// and clear the marker only after successful recovery. Failed cleanup remains
/// pending across launches without preventing the desktop from opening.
pub(super) async fn recover_tun_session(
    paths: &AppPaths,
    helper: &std::path::Path,
) -> Result<(), RecoveryFailure> {
    let session = v2ray_rs_core::persistence::load_tun_session_result(paths).map_err(|error| {
        log::warn!("read TUN recovery journal: {error}");
        RecoveryFailure {
            session: None,
            timed_out: false,
            journal_file: None,
        }
    })?;
    let Some(session) = session else {
        return Ok(());
    };
    let ownership = session.ownership.as_ref();
    // Retire a journal that records no kernel state before the epoch gate: a
    // finished record from a previous boot must not refuse every later launch
    // forever. Retiring only removes or replaces the journal file; it never
    // mutates kernel state.
    if v2ray_rs_core::persistence::retirable_tun_session(&session) {
        return v2ray_rs_core::persistence::clear_tun_session(paths).map_err(|_| RecoveryFailure {
            session: Some(session),
            timed_out: false,
            journal_file: None,
        });
    }
    if ownership.is_none_or(|ownership| ownership.verify_epoch().is_err()) {
        let journal_file = unverifiable_journal_file(paths);
        log::warn!(
            "TUN ownership cannot be established. Explicit operator recovery is required. \
             Unverifiable journal: {journal_file}"
        );
        return Err(RecoveryFailure {
            session: Some(session),
            timed_out: false,
            journal_file: Some(journal_file),
        });
    }
    if session.backend != BackendType::Xray {
        log::warn!(
            "sing-box recovery has no verifiable ownership. Explicit operator recovery is required"
        );
        return Err(RecoveryFailure {
            session: Some(session),
            timed_out: false,
            journal_file: None,
        });
    }
    let args = [
        "recover".to_string(),
        recover_flag(session.backend).to_string(),
        "--iface".to_string(),
        session.iface.clone(),
        "--journal".to_string(),
        paths.tun_session_path().to_string_lossy().into_owned(),
        "--session-id".to_string(),
        session
            .ownership
            .as_ref()
            .expect("verified ownership")
            .session_id
            .to_string(),
    ];
    let run = v2ray_rs_process::run_helper(helper, &args, v2ray_rs_process::HELPER_TIMEOUT).await;
    let outcome = match &run.result {
        Ok(()) => {
            log::info!("recovered leftover TUN state on {}", session.iface);
            "recover ok".to_string()
        }
        Err(err) => {
            log::warn!("tun {err}");
            err.clone()
        }
    };
    match RotatingFileWriter::open(paths.logs_dir().join("backend.log"), DEFAULT_MAX_BYTES) {
        Ok(log) => {
            for line in &run.output {
                log.append_line("helper", line);
            }
            log.append_line("helper", &outcome);
        }
        Err(err) => log::warn!("open backend log: {err}"),
    }
    let result = run.result.and_then(|()| {
        v2ray_rs_core::persistence::clear_tun_session(paths).map_err(|err| {
            log::warn!("clear recovered TUN session marker: {err}");
            err.to_string()
        })
    });
    match result {
        Ok(()) => Ok(()),
        Err(_) => Err(RecoveryFailure {
            session: Some(session),
            timed_out: run.timed_out,
            journal_file: None,
        }),
    }
}

pub(super) fn install_icon_for_compositor(profile: &AppProfile) {
    let data_dir = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share"))
        });

    let Some(data_dir) = data_dir else { return };
    let app_id = profile.app_id();
    let res_prefix = format!("/{}/icons/hicolor", app_id.replace('.', "/"));

    let installed = install_resource_icon(
        &data_dir,
        &res_prefix,
        "scalable/apps",
        &format!("{app_id}.svg"),
    ) | install_resource_icon(
        &data_dir,
        &res_prefix,
        "symbolic/apps",
        &format!("{app_id}-symbolic.svg"),
    );

    if installed {
        let theme_dir = data_dir.join("icons/hicolor");
        let _ = std::process::Command::new("gtk-update-icon-cache")
            .arg("-f")
            .arg("-t")
            .arg(&theme_dir)
            .spawn();
    }
}

pub(super) fn install_resource_icon(
    data_dir: &std::path::Path,
    res_prefix: &str,
    subdir: &str,
    filename: &str,
) -> bool {
    let icon_dir = data_dir.join(format!("icons/hicolor/{subdir}"));
    let icon_path = icon_dir.join(filename);

    let Ok(svg) = gtk::gio::resources_lookup_data(
        &format!("{res_prefix}/{subdir}/{filename}"),
        gtk::gio::ResourceLookupFlags::NONE,
    ) else {
        return false;
    };

    // Refresh an already-installed copy that drifted from the resource: the
    // user-local hicolor dir shadows the packaged icon, so a stale file here
    // would keep overriding fixed icons from newer builds.
    if let Ok(existing) = std::fs::read(&icon_path)
        && existing == svg
    {
        return false;
    }

    if let Err(err) = std::fs::create_dir_all(&icon_dir) {
        log::debug!("create icon dir {icon_dir:?}: {err}");
        return false;
    }
    if let Err(err) = std::fs::write(&icon_path, &svg) {
        log::debug!("write icon {icon_path:?}: {err}");
        return false;
    }
    true
}

pub(super) fn onboarding_persist_error_message(err: &str) -> String {
    format!("Failed to save settings: {err}")
}

pub fn run() {
    if let Err(err) = try_run() {
        eprintln!("v2ray-rs startup failed: {err}");
        std::process::exit(1);
    }
}

pub(super) fn reset_instance_preserving_lock(
    paths: &AppPaths,
    profile: &AppProfile,
    confirm: bool,
) -> Result<(), InstanceError> {
    if *profile == AppProfile::Production && !confirm {
        return Err(InstanceError::ResetProductionDenied);
    }

    remove_dir_contents_except(paths.config_dir(), None)?;
    remove_dir_contents_except(paths.data_dir(), None)?;
    remove_dir_contents_except(paths.cache_dir(), None)?;
    remove_dir_contents_except(paths.runtime_dir(), Some(&paths.instance_lock_path()))?;
    remove_dir_contents_except(paths.state_dir(), None)?;

    paths.ensure_dirs()?;

    Ok(())
}

pub(super) fn remove_dir_contents_except(
    dir: &std::path::Path,
    skip: Option<&std::path::Path>,
) -> Result<(), InstanceError> {
    if !dir.exists() {
        return Ok(());
    }

    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if skip.is_some_and(|s| path == s) {
            continue;
        }
        if path.is_dir() {
            std::fs::remove_dir_all(&path)?;
        } else {
            std::fs::remove_file(&path)?;
        }
    }

    Ok(())
}

pub(super) fn try_run() -> Result<(), String> {
    // clap sends --help/--version to stdout with exit code 0 and real errors to
    // stderr; `print` and `exit_code` already know which. Routing the stdout
    // cases through the startup-failure path printed help text to stderr and
    // exited 1.
    let cli_args = CliArgs::try_parse().unwrap_or_else(|e: clap::error::Error| {
        let _ = e.print();
        std::process::exit(e.exit_code());
    });

    let profile = AppProfile::resolve(cli_args.profile.as_deref(), &StdEnv)
        .map_err(|e| format!("invalid profile: {e}"))?;

    let overrides = PathOverrides::resolve(&cli_args.paths(), &StdEnv);

    let paths = AppPaths::with_overrides(profile.clone(), &overrides)
        .map_err(|err| format!("failed to determine XDG directories: {err}"))?;

    paths
        .ensure_dirs()
        .map_err(|err| format!("failed to create directories: {err}"))?;

    crate::logging::init_logging(&paths);
    crate::logging::install_panic_hook();
    log::info!(
        "v2ray-rs {} starting, profile '{}'",
        env!("CARGO_PKG_VERSION"),
        profile.qualifier()
    );

    let settings = match v2ray_rs_core::persistence::load_settings(&paths) {
        Ok(settings) => settings,
        Err(err) => {
            log::warn!("load settings during startup: {err}");
            AppSettings::default()
        }
    };
    // gettext's setlocale and env LANGUAGE are not thread-safe, so this must
    // run before WorkspaceStore, GTK, Tokio, or the tray service spawn threads.
    // A language change needs a restart.
    crate::i18n::init_process_locale(settings.language);

    let app_id = profile.app_id();
    let app = adw::Application::builder().application_id(&app_id).build();
    // Default profile paths can rely on GApplication's session-bus uniqueness:
    // a second launch activates the primary window and exits 0. Directory
    // overrides need the per-profile lock instead because the application ID
    // alone does not identify the directory set.
    if !overrides.has_directory_overrides() {
        match app.register(None::<&gtk::gio::Cancellable>) {
            Ok(()) if app.is_remote() => {
                app.run_with_args::<&str>(&[]);
                return Ok(());
            }
            Ok(()) => {}
            Err(err) => {
                log::warn!("session-bus registration failed, using the instance lock: {err}");
            }
        }
    }

    let _lock = InstanceLock::acquire(&paths).map_err(|e| {
        if let v2ray_rs_core::instance::InstanceError::LockHeld { pid, profile: p } = e {
            let pid_label = pid.map_or_else(|| "unknown".to_string(), |n| n.to_string());
            eprintln!(
                "Another instance is already running (PID {pid_label}), profile '{p}' is locked."
            );
            std::process::exit(75);
        }
        format!("failed to acquire instance lock: {e}")
    })?;

    if cli_args.reset_instance {
        reset_instance_preserving_lock(&paths, &profile, cli_args.i_understand)
            .map_err(|e| format!("failed to reset instance: {e}"))?;
        println!("Instance reset successfully.");
        return Ok(());
    }

    let mut stamp = InstanceStamp::load_or_create(&paths)
        .map_err(|e| format!("failed to load instance stamp: {e}"))?;

    let compatibility = check_compatibility(&stamp, &profile);
    match compatibility {
        CompatibilityResult::Match => {
            log::info!("Instance stamp is compatible");
        }
        CompatibilityResult::NeedsForwardMigration => {
            log::warn!(
                "Instance stamp needs forward migration (schema version {} < {}), continuing",
                stamp.schema_version,
                v2ray_rs_core::instance::CURRENT_SCHEMA_VERSION
            );
        }
        CompatibilityResult::IncompatibleProfile => {
            return Err(format!(
                "Instance profile '{}' is incompatible with current profile '{}'. Reset instance with --reset-instance to continue.",
                stamp.profile,
                profile.qualifier()
            ));
        }
        CompatibilityResult::IncompatibleAppId => {
            return Err(format!(
                "Instance app_id '{}' is incompatible with current app_id '{}'. Reset instance with --reset-instance to continue.",
                stamp.app_id,
                profile.app_id()
            ));
        }
        CompatibilityResult::TooNew => {
            return Err(format!(
                "Instance schema version {} is newer than current {}. Downgrade the application or reset instance with --reset-instance to continue.",
                stamp.schema_version,
                v2ray_rs_core::instance::CURRENT_SCHEMA_VERSION
            ));
        }
    }

    stamp
        .update_started(&paths)
        .map_err(|e| format!("failed to update instance stamp: {e}"))?;

    if let Err(err) = rustls::crypto::ring::default_provider().install_default() {
        log::debug!("rustls crypto provider already installed or unavailable: {err:?}");
    }

    let rt = tokio::runtime::Runtime::new()
        .map_err(|err| format!("failed to create tokio runtime: {err}"))?;
    let _rt_guard = rt.enter();

    let (event_tx, event_rx) = broadcast::channel::<ProcessEvent>(EVENT_CHANNEL_CAPACITY);
    if let Ok(mut guard) = TRAY_EVENT_TX.lock() {
        *guard = Some(event_tx);
    }

    let should_install_icons =
        profile == AppProfile::Production || overrides.install_icons.unwrap_or(false);
    let tray_action_rx = if should_install_icons {
        let (tray_tx, tray_rx) = tokio::sync::mpsc::unbounded_channel::<TrayAction>();
        let notifier = v2ray_rs_tray::Notifier::new(settings.notifications_enabled);
        let data_dir = paths.data_dir().to_path_buf();
        match rt.block_on(async {
            v2ray_rs_tray::TrayService::spawn_with_data_dir(
                event_rx,
                notifier,
                move |action| {
                    let _ = tray_tx.send(action);
                },
                &data_dir,
            )
            .await
        }) {
            Ok(handle) => {
                if let Ok(mut guard) = TRAY_HANDLE.lock() {
                    *guard = Some(handle);
                }
                Some(tray_rx)
            }
            Err(err) => {
                log::warn!("failed to start tray service: {err}");
                None
            }
        }
    } else {
        None
    };

    let resource_bytes =
        glib::Bytes::from_static(include_bytes!(concat!(env!("OUT_DIR"), "/icons.gresource")));
    let resource = gtk::gio::Resource::from_data(&resource_bytes)
        .map_err(|err| format!("failed to load icon resource: {err}"))?;
    gtk::gio::resources_register(&resource);

    // Register our CLI options with GTK so it doesn't complain about "unknown options"
    app.add_main_option(
        "profile",
        glib::Char(0),
        glib::OptionFlags::NONE,
        glib::OptionArg::String,
        "Runtime profile (production, development, test, custom:name)",
        Some("PROFILE"),
    );
    app.add_main_option(
        "config-dir",
        glib::Char(0),
        glib::OptionFlags::NONE,
        glib::OptionArg::String,
        "Override config directory",
        Some("PATH"),
    );
    app.add_main_option(
        "data-dir",
        glib::Char(0),
        glib::OptionFlags::NONE,
        glib::OptionArg::String,
        "Override data directory",
        Some("PATH"),
    );
    app.add_main_option(
        "cache-dir",
        glib::Char(0),
        glib::OptionFlags::NONE,
        glib::OptionArg::String,
        "Override cache directory",
        Some("PATH"),
    );
    app.add_main_option(
        "runtime-dir",
        glib::Char(0),
        glib::OptionFlags::NONE,
        glib::OptionArg::String,
        "Override runtime directory",
        Some("PATH"),
    );
    app.add_main_option(
        "state-dir",
        glib::Char(0),
        glib::OptionFlags::NONE,
        glib::OptionArg::String,
        "Override state directory",
        Some("PATH"),
    );
    app.add_main_option(
        "reset-instance",
        glib::Char(0),
        glib::OptionFlags::NONE,
        glib::OptionArg::None,
        "Reset instance data for current profile",
        None,
    );
    app.add_main_option(
        "install-icons",
        glib::Char(0),
        glib::OptionFlags::NONE,
        glib::OptionArg::None,
        "Install icons even for non-production profiles",
        None,
    );
    app.add_main_option(
        "i-understand",
        glib::Char(0),
        glib::OptionFlags::NONE,
        glib::OptionArg::None,
        "Confirm destructive operations",
        None,
    );

    let app_id_clone = app_id.clone();
    let profile_clone = profile.clone();
    app.connect_startup(move |_| {
        if let Some(display) = gtk::gdk::Display::default() {
            let theme = gtk::IconTheme::for_display(&display);
            theme.add_resource_path("/com/github/v2ray-rs/icons");
        }
        if should_install_icons {
            install_icon_for_compositor(&profile_clone);
        }
        gtk::Window::set_default_icon_name(&app_id_clone);
    });

    app.connect_activate(|app| {
        if let Some(window) = app.active_window() {
            window.set_visible(true);
            window.present();
        }
    });

    let relm_app = RelmApp::from_app(app);
    relm_app.run::<App>(AppInit {
        paths,
        tray_action_rx,
    });

    if let Ok(mut guard) = TRAY_HANDLE.lock()
        && let Some(handle) = guard.take()
    {
        rt.block_on(handle.shutdown());
    }
    if let Ok(mut guard) = TRAY_EVENT_TX.lock() {
        guard.take();
    }

    Ok(())
}
