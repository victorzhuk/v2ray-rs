//! Joined GTK proof for the quit/disconnect cleanup contract.
//!
//! These tests launch the real `App` against an isolated profile and drive the
//! real widget actions. Only the process manager's host probes and helper path
//! are pinned through the test-only spawn hook in `crate::connection`; the
//! session, its cleanup parking and the retry path are production code.
//!
//! The app's own fields stay private, so the proof reads the authoritative tray
//! snapshot (state, connection, cleanup admission), the live widget tree and
//! the helper call log. Session identity is proven by the ids the helper
//! records for `install-rules` and `remove-rules`.

use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use relm4::component::Connector;
use relm4::prelude::*;
use v2ray_rs_core::models::{AppSettings, BackendType, ManualNode, ProxyNode, ShadowsocksConfig};
use v2ray_rs_core::persistence::AppPaths;
use v2ray_rs_core::profile::AppProfile;
use v2ray_rs_process::{AntiDpiRuntime, HelperAccess, HostProbe, ProcessState};
use v2ray_rs_tray::TraySnapshot;

use super::*;

const STEP: Duration = Duration::from_secs(30);

struct Fixture {
    _tmp: tempfile::TempDir,
    paths: AppPaths,
    helper: PathBuf,
    calls: PathBuf,
    _listener: TcpListener,
}

fn write_executable(path: &Path, script: &str) {
    std::fs::write(path, script).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn fixture(make_body: impl FnOnce(&Path) -> String) -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    let paths = AppPaths::for_profile_in(AppProfile::Test, &root);
    paths.ensure_dirs().unwrap();

    let calls = root.join("helper-calls");
    let helper = root.join("v2ray-rs-antidpi");
    let helper_body = make_body(&root);
    write_executable(
        &helper,
        &format!(
            "#!/bin/sh\necho \"$@\" >> {}\n{helper_body}\n",
            calls.display()
        ),
    );

    let backend = root.join("backend");
    write_executable(
        &backend,
        "#!/bin/sh\n[ \"$1\" = version ] && { echo 'sing-box version 1.13.0'; exit 0; }\n[ \"$1\" = check ] && exit 0\nexec sleep 60\n",
    );

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut settings = AppSettings::default();
    settings.backend.backend_type = BackendType::SingBox;
    settings.backend.binary_path = Some(backend);
    settings.socks_port = listener.local_addr().unwrap().port();
    settings.onboarding_complete = true;
    settings.minimize_to_tray = false;
    settings.notifications_enabled = false;
    settings.auto_update_geodata = false;
    settings.auto_update_subscriptions = false;
    settings.antidpi.enabled = true;
    settings.antidpi.queue = 200;
    v2ray_rs_core::persistence::save_settings(&paths, &settings).unwrap();

    let node = ManualNode::new(ProxyNode::Shadowsocks(ShadowsocksConfig {
        address: "203.0.113.1".into(),
        port: 8388,
        method: "aes-256-gcm".into(),
        password: "secret".into(),
        remark: None,
    }));
    WorkspaceStore::new(paths.clone())
        .save_manual_nodes(&[node])
        .unwrap();

    Fixture {
        _tmp: tmp,
        paths,
        helper,
        calls,
        _listener: listener,
    }
}

/// The first removal fails and every later removal succeeds.
fn fail_once() -> Fixture {
    fixture(|root| {
        let gate = root.join("remove-gate");
        format!(
            "case \"$1\" in \
install-rules) exit 0 ;; \
run) echo 'v2ray-rs-antidpi: ready queue=200'; while :; do sleep 1; done ;; \
remove-rules) [ -f {gate} ] && exit 0; : > {gate}; echo 'cleanup failed' >&2; exit 1 ;; \
esac\nexit 0",
            gate = gate.display()
        )
    })
}

fn pump_until(timeout: Duration, predicate: impl Fn() -> bool) -> bool {
    let ctx = gtk::glib::MainContext::default();
    let start = Instant::now();
    loop {
        while ctx.pending() {
            ctx.iteration(false);
        }
        if predicate() {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn pump_for(duration: Duration) {
    let ctx = gtk::glib::MainContext::default();
    let start = Instant::now();
    while start.elapsed() < duration {
        while ctx.pending() {
            ctx.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn find_button(widget: &gtk::Widget, label: &str) -> Option<gtk::Button> {
    if let Some(button) = widget.downcast_ref::<gtk::Button>()
        && button.label().as_deref() == Some(label)
    {
        return Some(button.clone());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(found) = find_button(&current, label) {
            return Some(found);
        }
        child = current.next_sibling();
    }
    None
}

fn find_cleanup_banner(widget: &gtk::Widget) -> Option<adw::Banner> {
    if let Some(banner) = widget.downcast_ref::<adw::Banner>()
        && banner.title().starts_with("Cleanup failed")
    {
        return Some(banner.clone());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(found) = find_cleanup_banner(&current) {
            return Some(found);
        }
        child = current.next_sibling();
    }
    None
}

fn call_lines(calls: &Path, verb: &str) -> Vec<String> {
    let text = std::fs::read_to_string(calls).unwrap_or_default();
    text.lines()
        .filter(|line| line.starts_with(verb))
        .map(str::to_owned)
        .collect()
}

fn removals(fixture: &Fixture) -> Vec<String> {
    call_lines(&fixture.calls, "remove-rules")
}

fn installs(fixture: &Fixture) -> Vec<String> {
    call_lines(&fixture.calls, "install-rules")
}

fn session_id(line: &str) -> Option<&str> {
    line.split("--session-id ").nth(1).map(str::trim)
}

/// The main action button is the only `gtk::Button` whose child is an
/// `adw::ButtonContent`; its visible label lives on that child, not on the
/// button's own (empty) `label` property. Per-node menus also say "Connect",
/// so resolving by plain label would click the wrong control.
fn action_button(window: &gtk::Window) -> gtk::Button {
    fn scan(widget: &gtk::Widget) -> Option<gtk::Button> {
        if let Some(button) = widget.downcast_ref::<gtk::Button>()
            && button
                .child()
                .is_some_and(|child| child.downcast::<adw::ButtonContent>().is_ok())
        {
            return Some(button.clone());
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            if let Some(found) = scan(&current) {
                return Some(found);
            }
            child = current.next_sibling();
        }
        None
    }
    scan(window.upcast_ref()).expect("main action button")
}

fn action_label(button: &gtk::Button) -> Option<String> {
    button
        .child()
        .and_then(|child| child.downcast::<adw::ButtonContent>().ok())
        .map(|content| content.label().to_string())
}

fn connect(window: &gtk::Window, fixture: &Fixture) {
    // The test hook is one-shot by contract: re-arm it for the session this
    // click starts (the retry path never spawns a second manager).
    arm_manager(fixture);
    let button = action_button(window);
    assert_eq!(action_label(&button).as_deref(), Some("Connect"));
    button.emit_clicked();
}

fn arm_manager(fixture: &Fixture) {
    let helper = fixture.helper.clone();
    crate::connection::inject_test_configure(Box::new(move |mgr| {
        mgr.with_host_probe(HostProbe {
            getcap: helper.clone(),
            helper: helper.clone(),
            helper_access: Some(HelperAccess::Ready),
        })
        .with_antidpi(Some(AntiDpiRuntime {
            helper_path: helper.clone(),
            queue: 200,
            strategies: Vec::new(),
            hostlist: None,
        }))
    }));
}

fn disconnect(window: &gtk::Window) {
    let button = action_button(window);
    assert_eq!(action_label(&button).as_deref(), Some("Disconnect"));
    button.emit_clicked();
}

fn press(window: &gtk::Window, label: &str) {
    let button = find_button(window.upcast_ref(), label).expect(label);
    button.grab_focus();
    button.activate();
}

/// `adw::PreferencesDialog` presents as a floating `AdwDialog`, so it is not
/// a `gtk::Window` toplevel and not reliably a descendant of the main window.
/// It is either embedded under the presenting window or owned by its own
/// toplevel surface, so search both.
fn find_preferences_dialog(widget: &gtk::Widget) -> Option<adw::PreferencesDialog> {
    if let Some(dialog) = widget.downcast_ref::<adw::PreferencesDialog>() {
        return Some(dialog.clone());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(found) = find_preferences_dialog(&current) {
            return Some(found);
        }
        child = current.next_sibling();
    }
    None
}

fn preferences_dialog(window: &gtk::Window) -> Option<adw::PreferencesDialog> {
    if let Some(found) = find_preferences_dialog(window.upcast_ref()) {
        return Some(found);
    }
    for toplevel in gtk::Window::list_toplevels() {
        if toplevel == *window {
            continue;
        }
        if let Some(found) = find_preferences_dialog(&toplevel) {
            return Some(found);
        }
    }
    None
}

fn find_switch_row(widget: &gtk::Widget, title: &str) -> Option<adw::SwitchRow> {
    if let Some(row) = widget.downcast_ref::<adw::SwitchRow>()
        && row.title() == title
    {
        return Some(row.clone());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(found) = find_switch_row(&current, title) {
            return Some(found);
        }
        child = current.next_sibling();
    }
    None
}

/// Diagnoses a failed dialog search: names every toplevel and its first two
/// levels of children.
fn toplevel_dump() -> String {
    fn walk(widget: &gtk::Widget, depth: usize, out: &mut String) {
        out.push_str(&format!(
            "{}{} visible={} mapped={}\n",
            "  ".repeat(depth),
            widget.type_().name(),
            widget.is_visible(),
            widget.is_mapped()
        ));
        if depth >= 2 {
            return;
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            walk(&current, depth + 1, out);
            child = current.next_sibling();
        }
    }
    let mut out = String::new();
    for toplevel in gtk::Window::list_toplevels() {
        walk(&toplevel, 0, &mut out);
    }
    out
}

fn open_anti_dpi(controller: &Connector<App>, window: &gtk::Window) -> adw::SwitchRow {
    // Drive the component's real input path instead of the window action
    // proxy: the action is installed on the Relm4 root widget, and the
    // direct message needs no window-action namespace lookup to succeed.
    controller.sender().emit(AppMsg::OpenPreferences(Some(
        crate::preferences::ANTIDPI_PAGE_NAME,
    )));
    assert!(
        pump_until(STEP, || preferences_dialog(window).is_some()),
        "the preferences dialog did not open: {}",
        toplevel_dump()
    );
    let dialog = preferences_dialog(window).expect("preferences dialog");
    dialog.set_visible_page_name(crate::preferences::ANTIDPI_PAGE_NAME);
    pump_for(Duration::from_millis(50));
    find_switch_row(dialog.upcast_ref(), "Enable anti-DPI").expect("anti-DPI switch")
}

/// Makes the settings write fail without changing the warmed snapshot: the
/// atomic rename cannot replace a directory.
fn block_settings_writes(paths: &AppPaths) {
    let path = paths.settings_path();
    let _ = std::fs::remove_file(&path);
    std::fs::create_dir(&path).unwrap();
}

fn collect_labels(widget: &gtk::Widget, out: &mut Vec<String>) {
    if let Some(label) = widget.downcast_ref::<gtk::Label>() {
        out.push(label.label().to_string());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        collect_labels(&current, out);
        child = current.next_sibling();
    }
}

fn failure_toasts(window: &gtk::Window) -> Vec<String> {
    let mut texts = Vec::new();
    collect_labels(window.upcast_ref(), &mut texts);
    texts.retain(|text| text.contains("Failed to save settings"));
    texts
}

/// Runs the real app on the GTK thread with an isolated profile, a private
/// Tokio runtime and a snapshot receiver standing in for the tray.
fn with_app(
    fixture: Fixture,
    body: impl FnOnce(
        Fixture,
        &Connector<App>,
        &gtk::Window,
        &mut tokio::sync::watch::Receiver<TraySnapshot>,
        &WorkspaceStore,
    ) + Send
    + 'static,
) {
    crate::gtk_test::run(move || {
        // The production bootstrap installs the rustls provider before any
        // HTTP client is built; these tests launch `App` directly, so they
        // must repeat that one-time process initialization.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let _guard = rt.enter();
        let (snapshot_tx, mut snapshot_rx) = tokio::sync::watch::channel(TraySnapshot::initial());
        *TRAY_SNAPSHOT_TX.lock().unwrap() = Some(snapshot_tx);

        // The app no longer loads the workspace on the GTK thread: the store
        // is warmed once here, after this test's fixture files exist, exactly
        // as bootstrap does after the instance lock.
        let store = WorkspaceStore::new(fixture.paths.clone());
        let warm = store.warm().unwrap();
        let controller = App::builder().launch(AppInit {
            paths: fixture.paths.clone(),
            store: store.clone(),
            warm,
            tray_action_rx: None,
        });
        let window = controller.widget();
        let window: &gtk::Window = window.upcast_ref();
        // The production bootstrap presents the window from the application's
        // `activate` handler; a directly launched `App` has no such handler,
        // so the harness supplies the same activation step.
        window.set_visible(true);
        window.present();
        body(fixture, &controller, window, &mut snapshot_rx, &store);

        drop(controller);
        pump_for(Duration::from_millis(50));
        *TRAY_SNAPSHOT_TX.lock().unwrap() = None;
    });
}

#[test]
fn disconnect_cleanup_pending_retry_restores_then_quit_closes() {
    with_app(
        fail_once(),
        |fixture, _controller, window, snapshot, _store| {
            connect(window, &fixture);
            assert!(
                pump_until(STEP, || matches!(
                    snapshot.borrow().state,
                    ProcessState::Running
                )),
                "connect never reached Running: {:?}",
                snapshot.borrow().state
            );
            assert!(!snapshot.borrow().cleanup_pending);

            disconnect(window);
            assert!(
                pump_until(STEP, || snapshot.borrow().cleanup_pending),
                "disconnect never parked the cleanup: {:?}",
                snapshot.borrow().state
            );
            // The parked session stays the only one: no terminal Stopped, no new
            // install, and the failure is presented as the banner's Error.
            assert!(matches!(snapshot.borrow().state, ProcessState::Error(_)));
            assert_eq!(installs(&fixture).len(), 1);
            assert_eq!(removals(&fixture).len(), 1);
            let banner = find_cleanup_banner(window.upcast_ref()).expect("cleanup banner");
            assert!(banner.is_revealed(), "pending cleanup banner is revealed");
            assert!(window.is_visible(), "the window stays open");

            // Connect is refused while the retry owns the session: the control is
            // insensitive and a synthesized click starts no second install.
            let action = action_button(window);
            assert!(
                !action.is_sensitive(),
                "Connect must be unavailable while cleanup is pending"
            );
            connect(window, &fixture);
            pump_for(Duration::from_millis(300));
            assert_eq!(installs(&fixture).len(), 1, "admission stayed closed");

            // The recovery control must remain for the required interval.
            pump_for(Duration::from_secs(5));
            assert!(snapshot.borrow().cleanup_pending, "banner must survive 5s");
            assert!(banner.is_revealed());
            assert!(window.is_visible());
            assert_eq!(removals(&fixture).len(), 1, "one failed removal");

            press(window, "Retry cleanup");
            assert!(
                pump_until(STEP, || {
                    matches!(snapshot.borrow().state, ProcessState::Stopped)
                        && !snapshot.borrow().cleanup_pending
                }),
                "retry never completed: {:?}",
                snapshot.borrow().state
            );
            assert!(
                !banner.is_revealed(),
                "the banner clears on verified cleanup"
            );

            let removes = removals(&fixture);
            assert_eq!(removes.len(), 2, "failed attempt plus retry: {removes:?}");
            let install = installs(&fixture);
            assert_eq!(install.len(), 1, "the retry must not mint a new install");
            let install_id = session_id(&install[0]).expect("install session id");
            for removal in &removes {
                assert_eq!(
                    session_id(removal),
                    Some(install_id),
                    "the retried cleanup owns the same session"
                );
            }

            // Controls are restored: a fresh Connect starts a new session.
            connect(window, &fixture);
            assert!(
                pump_until(STEP, || installs(&fixture).len() == 2),
                "controls were not restored after the retry"
            );
            assert!(pump_until(STEP, || matches!(
                snapshot.borrow().state,
                ProcessState::Running
            )));
            // Clean quit from Running: verified removal, then the window closes.
            window.close();
            assert!(
                pump_until(STEP, || !window.is_visible()),
                "quit never closed the window"
            );
        },
    );
}

#[test]
fn quit_during_pending_cleanup_waits_for_the_same_retry() {
    with_app(
        fail_once(),
        |fixture, _controller, window, snapshot, _store| {
            connect(window, &fixture);
            assert!(pump_until(STEP, || matches!(
                snapshot.borrow().state,
                ProcessState::Running
            )));
            let install_id = session_id(&installs(&fixture)[0])
                .expect("install session id")
                .to_owned();

            disconnect(window);
            assert!(pump_until(STEP, || snapshot.borrow().cleanup_pending));
            assert_eq!(removals(&fixture).len(), 1);

            window.close();
            pump_for(Duration::from_millis(300));
            assert!(
                snapshot.borrow().cleanup_pending,
                "quit is deferred behind the parked cleanup"
            );
            assert!(
                matches!(snapshot.borrow().state, ProcessState::Error(_)),
                "no terminal state before the retry"
            );
            assert!(window.is_visible(), "pending quit keeps the window");
            assert_eq!(
                removals(&fixture).len(),
                1,
                "quit must not enqueue another removal"
            );

            // A second ordinary quit re-presents the same control, no new Stop.
            window.close();
            pump_for(Duration::from_millis(300));
            assert!(window.is_visible());
            assert_eq!(removals(&fixture).len(), 1);

            press(window, "Retry cleanup");
            assert!(
                pump_until(STEP, || !window.is_visible()),
                "pending quit did not close after verified cleanup"
            );
            let removes = removals(&fixture);
            assert_eq!(removes.len(), 2);
            assert_eq!(installs(&fixture).len(), 1);
            for removal in &removes {
                assert_eq!(session_id(removal), Some(install_id.as_str()));
            }
        },
    );
}

#[test]
fn quit_from_running_fail_once_keeps_the_window_until_retry() {
    with_app(
        fail_once(),
        |fixture, _controller, window, snapshot, _store| {
            connect(window, &fixture);
            assert!(pump_until(STEP, || matches!(
                snapshot.borrow().state,
                ProcessState::Running
            )));
            let install_id = session_id(&installs(&fixture)[0])
                .expect("install session id")
                .to_owned();

            // Ordinary quit from Running: the failed first removal must park the
            // quit, not close the window and abandon the session's rules.
            window.close();
            assert!(
                pump_until(STEP, || snapshot.borrow().cleanup_pending),
                "quit did not park the failed cleanup: {:?}",
                snapshot.borrow().state
            );
            assert!(matches!(snapshot.borrow().state, ProcessState::Error(_)));
            assert!(window.is_visible(), "pending quit keeps the window");
            let banner = find_cleanup_banner(window.upcast_ref()).expect("cleanup banner");
            assert!(banner.is_revealed(), "the recovery control is presented");
            assert_eq!(
                installs(&fixture).len(),
                1,
                "quit must not mint a new session"
            );
            assert_eq!(removals(&fixture).len(), 1);
            let removes = removals(&fixture);
            assert_eq!(session_id(&removes[0]), Some(install_id.as_str()));
            assert!(
                !action_button(window).is_sensitive(),
                "Connect must be unavailable while the quit waits"
            );

            // A click during recovery starts nothing, and a repeated ordinary
            // quit re-presents the same control instead of enqueueing another
            // Stop.
            connect(window, &fixture);
            pump_for(Duration::from_millis(300));
            assert_eq!(installs(&fixture).len(), 1);
            assert_eq!(removals(&fixture).len(), 1);
            window.close();
            pump_for(Duration::from_millis(300));
            assert!(window.is_visible());
            assert_eq!(removals(&fixture).len(), 1, "no second stop");

            press(window, "Retry cleanup");
            assert!(
                pump_until(STEP, || !window.is_visible()),
                "pending quit did not close after verified cleanup"
            );
            let removes = removals(&fixture);
            assert_eq!(removes.len(), 2, "failed attempt plus retry: {removes:?}");
            assert_eq!(installs(&fixture).len(), 1);
            for removal in &removes {
                assert_eq!(
                    session_id(removal),
                    Some(install_id.as_str()),
                    "the retried cleanup owns the quit's session"
                );
            }
        },
    );
}

#[test]
fn a_held_quit_barrier_keeps_the_gtk_loop_responsive() {
    with_app(
        fail_once(),
        |_fixture, _controller, window, _snapshot, store| {
            // The writer holds the quit's barrier transaction. The GTK loop
            // must keep processing events and the window must stay open until
            // the ordered write resolves.
            store.test_writer_latch().arm();
            window.close();
            assert!(
                pump_until(STEP, || store.test_writer_latch().is_parked()),
                "the quit barrier never reached the writer"
            );
            assert!(window.is_visible(), "a pending quit keeps the window");
            assert!(store.test_writer_latch().is_parked());

            let idle_ran = std::rc::Rc::new(std::cell::Cell::new(false));
            let flag = idle_ran.clone();
            glib::idle_add_local_once(move || flag.set(true));
            assert!(
                pump_until(STEP, || idle_ran.get()),
                "an unrelated idle event did not run while the quit was parked"
            );
            assert!(
                store.test_writer_latch().is_parked(),
                "the idle proof ran only after the latch was released"
            );
            assert!(window.is_visible());

            store.test_writer_latch().release();
            assert!(
                pump_until(STEP, || !window.is_visible()),
                "the quit did not close after the barrier resolved"
            );
        },
    );
}

#[test]
fn startup_error_after_successful_cleanup_stays_an_error() {
    with_app(
        fixture(|_root| {
            "case \"$1\" in \
install-rules) echo 'install interrupted' >&2; exit 1 ;; \
remove-rules) exit 0 ;; \
esac\nexit 0"
                .to_string()
        }),
        |fixture, _controller, window, snapshot, _store| {
            connect(window, &fixture);
            assert!(
                pump_until(STEP, || matches!(
                    snapshot.borrow().state,
                    ProcessState::Error(_)
                )),
                "startup failure never surfaced: {:?}",
                snapshot.borrow().state
            );
            let ProcessState::Error(detail) = &snapshot.borrow().state else {
                unreachable!()
            };
            assert!(detail.contains("install interrupted"), "{detail}");
            assert!(
                !snapshot.borrow().cleanup_pending,
                "a completed cleanup arms no banner"
            );
            let banner = find_cleanup_banner(window.upcast_ref()).expect("cleanup banner");
            assert!(!banner.is_revealed(), "banner must stay hidden");
            assert_eq!(removals(&fixture).len(), 1);
            assert_eq!(installs(&fixture).len(), 1);
        },
    );
}

#[test]
fn held_save_failure_during_quit_reports_unsaved_edits_and_keeps_the_window() {
    with_app(
        fail_once(),
        |fixture, controller, window, _snapshot, store| {
            let switch = open_anti_dpi(controller, window);
            assert!(switch.is_active());

            // A user edit is accepted and its write reaches the writer, where it
            // is held. The Quit barrier queued behind it is not evidence that the
            // edit became durable.
            store.test_writer_latch().arm();
            switch.set_active(false);
            assert!(
                pump_until(STEP, || store.test_writer_latch().is_parked()),
                "the user save never reached the writer"
            );
            block_settings_writes(&fixture.paths);

            // The modal preferences dialog owns the window's close request, so
            // the user dismisses it before quitting. The accepted edit is still
            // held in the writer, so the quit barrier queues behind it.
            preferences_dialog(window)
                .expect("preferences dialog")
                .close();
            assert!(
                pump_until(STEP, || preferences_dialog(window).is_none()),
                "the preferences dialog did not close"
            );

            window.close();
            pump_for(Duration::from_millis(50));
            assert!(window.is_visible(), "the flush is still pending");

            store.test_writer_latch().release();
            assert!(
                pump_until(STEP, || {
                    failure_toasts(window)
                        .iter()
                        .any(|toast| toast.contains("Your edits were not saved"))
                }),
                "the failed save was not reported: {:?}",
                failure_toasts(window)
            );
            assert!(
                window.is_visible(),
                "a failed save must keep the window and the pending quit"
            );
            let toasts = failure_toasts(window);
            assert_eq!(
                toasts.len(),
                1,
                "the unsaved edit is reported once, not once per receipt: {toasts:?}"
            );

            // The failure was reported once; the next ordinary quit is not
            // blocked by it and closes without minting another removal.
            window.close();
            assert!(
                pump_until(STEP, || !window.is_visible()),
                "the next quit did not close"
            );
            assert_eq!(installs(&fixture).len(), 0);
            assert_eq!(removals(&fixture).len(), 0);
        },
    );
}

#[test]
fn stale_save_failure_does_not_roll_back_the_reopened_dialog() {
    with_app(
        fail_once(),
        |fixture, controller, window, _snapshot, store| {
            let first = open_anti_dpi(controller, window);
            store.test_writer_latch().arm();
            first.set_active(false);
            assert!(
                pump_until(STEP, || store.test_writer_latch().is_parked()),
                "the first user save never reached the writer"
            );
            block_settings_writes(&fixture.paths);

            // Close the dialog that owns the held save, then reopen. The new
            // dialog has a new identity and its revision counter restarts at 0.
            preferences_dialog(window)
                .expect("preferences dialog")
                .close();
            assert!(
                pump_until(STEP, || preferences_dialog(window).is_none()),
                "the preferences dialog did not close"
            );
            let reopened = open_anti_dpi(controller, window);
            assert!(
                reopened.is_active(),
                "the reopened dialog renders the committed settings"
            );
            // The same revision the stale write carries, from the new dialog.
            reopened.set_active(false);

            // Release the failed write and hold the reopened dialog's own save so
            // the stale receipt is the only completion that can run.
            store.test_writer_latch().release();
            store.test_writer_latch().arm();
            assert!(
                pump_until(STEP, || {
                    failure_toasts(window)
                        .iter()
                        .any(|toast| toast.contains("Failed to save settings"))
                }),
                "the stale save failure was not reported"
            );
            assert!(
                !reopened.is_active(),
                "a stale rollback must not touch the reopened dialog"
            );

            // Positive control: the same revision from the current dialog does
            // reconcile to the last committed snapshot.
            assert!(
                pump_until(STEP, || store.test_writer_latch().is_parked()),
                "the reopened dialog's save never reached the writer"
            );
            assert!(
                !reopened.is_active(),
                "the held save must not have been applied"
            );
            store.test_writer_latch().release();
            assert!(
                pump_until(STEP, || reopened.is_active()),
                "the same-revision failure did not restore the committed settings"
            );
        },
    );
}
