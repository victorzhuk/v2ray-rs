use super::grant::*;
use super::probes::*;
use super::verify::*;
use super::*;

/// Test shim over the path-aware parser: the path is the first whitespace
/// token of the `getcap` line, exactly how the parser delimits it.
fn getcap_has_cap(output: &str, cap: &str) -> bool {
    let path = output.trim().split([' ', '\t']).next().unwrap_or_default();
    caps_present(output, Path::new(path), &[cap])
}

#[test]
fn detects_cap_in_getcap_output() {
    assert!(getcap_has_cap(
        "/usr/bin/xray cap_net_admin,cap_net_bind_service,cap_net_raw=ep",
        "cap_net_admin"
    ));
    assert!(getcap_has_cap(
        "/usr/bin/xray cap_net_admin+ep",
        "cap_net_admin"
    ));
    // getcap terminates its line with a newline; the trailing whitespace
    // must not swallow the capset.
    assert!(getcap_has_cap(
        "/usr/bin/xray cap_net_bind_service,cap_net_admin,cap_net_raw=ep\n",
        "cap_net_admin"
    ));
}

#[test]
fn missing_cap_in_getcap_output() {
    assert!(!getcap_has_cap("", "cap_net_admin"));
    assert!(!getcap_has_cap(
        "/usr/bin/xray cap_net_bind_service=ep",
        "cap_net_admin"
    ));
}

#[test]
fn getcap_does_not_match_cap_in_path() {
    // A path component must not be mistaken for a granted capability.
    assert!(!getcap_has_cap(
        "/var/cap_net_admin/xray cap_net_bind_service=ep",
        "cap_net_admin"
    ));
    // ...but the real capability still matches.
    assert!(getcap_has_cap(
        "/var/cap_net_admin/xray cap_net_admin+ep",
        "cap_net_admin"
    ));
}

fn target(path: &str, identity: &str) -> TargetId {
    TargetId {
        path: PathBuf::from(path),
        identity: identity.into(),
    }
}

fn in_place(wrapper: Option<&str>) -> GrantPlan {
    GrantPlan::InPlace {
        backend: target("/usr/bin/xray", "8:1"),
        helper: target("/usr/bin/v2ray-rs-netctl", "8:2"),
        wrapper: wrapper.map(|w| target(w, "8:3")),
        antidpi: None,
        user: "zhuk".into(),
        uid: 1000,
    }
}

fn in_place_with_antidpi() -> GrantPlan {
    GrantPlan::InPlace {
        backend: target("/usr/bin/xray", "8:1"),
        helper: target("/usr/bin/v2ray-rs-netctl", "8:2"),
        wrapper: None,
        antidpi: Some(target("/usr/bin/v2ray-rs-antidpi", "8:4")),
        user: "zhuk".into(),
        uid: 1000,
    }
}

/// Builds a real pinned source from a file of exactly `len` bytes. The
/// temp directory is dropped immediately; the open handle keeps the
/// payload readable, and only `len` reaches the argument vector.
fn pinned(len: usize) -> PinnedSource {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("payload");
    std::fs::write(&path, vec![0u8; len]).unwrap();
    let file = std::fs::File::open(&path).unwrap();
    let meta = file.metadata().unwrap();
    PinnedSource {
        path,
        identity: format!("{}:{}", meta.dev(), meta.ino()),
        len: meta.len(),
        file,
    }
}

fn relocate() -> GrantPlan {
    GrantPlan::Relocate {
        backend: target("/usr/bin/xray", "8:1"),
        dest_dir: PathBuf::from("/usr/local/lib/v2ray-rs"),
        src: pinned(12345),
        dst: PathBuf::from("/usr/local/lib/v2ray-rs/v2ray-rs-netctl"),
        prior_dst: "absent".into(),
        antidpi: None,
        user: "zhuk".into(),
        uid: 1000,
    }
}

fn relocate_with_antidpi() -> GrantPlan {
    GrantPlan::Relocate {
        backend: target("/usr/bin/xray", "8:1"),
        dest_dir: PathBuf::from("/usr/local/lib/v2ray-rs"),
        src: pinned(12345),
        dst: PathBuf::from("/usr/local/lib/v2ray-rs/v2ray-rs-netctl"),
        prior_dst: "absent".into(),
        antidpi: Some(AntidpiRelocate {
            src: pinned(54321),
            dst: PathBuf::from("/usr/local/lib/v2ray-rs/v2ray-rs-antidpi"),
            prior_dst: "8:9".into(),
        }),
        user: "zhuk".into(),
        uid: 1000,
    }
}

fn sel_in_place(wrapper: Option<&str>) -> Selection {
    Selection::InPlace {
        helper: PathBuf::from("/usr/bin/v2ray-rs-netctl"),
        wrapper: wrapper.map(PathBuf::from),
        antidpi: None,
    }
}

fn sel_relocate() -> Selection {
    Selection::Relocate {
        dest_dir: PathBuf::from("/usr/local/lib/v2ray-rs"),
        src: pinned(12345),
        dst: PathBuf::from("/usr/local/lib/v2ray-rs/v2ray-rs-netctl"),
        antidpi: None,
    }
}

fn strs(argv: &[OsString]) -> Vec<&str> {
    argv.iter().map(|a| a.to_str().unwrap()).collect()
}

#[test]
fn inplace_argv_addresses_double_digit_positionals() {
    let plan = in_place(None);
    let argv_raw = grant_argv(&plan);
    let argv = strs(&argv_raw);
    assert_eq!(argv.len(), 4 + 14);
    assert_eq!(
        argv[4..],
        [
            BACKEND_CAPS,
            "/usr/bin/xray",
            HELPER_CAPS,
            "/usr/bin/v2ray-rs-netctl",
            HELPER_GROUP,
            "zhuk",
            "",
            "8:1",
            "8:2",
            "",
            "1000",
            "",
            "",
            "",
        ]
    );
    let script = grant_script(&plan);
    // `$10` unbraced would read as `$1` and a literal `0`.
    assert!(script.contains("\"${10}\"") && script.contains("\"${11}\""));
}

#[test]
fn inplace_argv_carries_wrapper_path_and_identity() {
    let plan = in_place(Some("/usr/bin/v2ray-rs-run"));
    let argv_raw = grant_argv(&plan);
    let argv = strs(&argv_raw);
    assert_eq!(argv.len(), 4 + 14);
    assert_eq!(argv[10], "/usr/bin/v2ray-rs-run");
    assert_eq!(argv[13], "8:3");
}

#[test]
fn inplace_argv_carries_the_antidpi_target() {
    let plan = in_place_with_antidpi();
    let argv_raw = grant_argv(&plan);
    let argv = strs(&argv_raw);
    assert_eq!(argv.len(), 4 + 14);
    assert_eq!(argv[15], "/usr/bin/v2ray-rs-antidpi");
    assert_eq!(argv[16], ANTIDPI_CAPS);
    assert_eq!(argv[17], "8:4");
    // The helper and wrapper positionals are untouched by the trailing
    // anti-DPI entries.
    assert_eq!(argv[10], "");
    assert_eq!(argv[13], "");
}

#[test]
fn inplace_script_provisions_the_antidpi_target() {
    let script = grant_script(&in_place_with_antidpi());
    // Validated under elevation before the first mutation, alongside the
    // helper and wrapper.
    assert!(script.contains(
            "if [ -n \"${12}\" ]; then check_file \"${12}\" \"${14}\"; check_parents \"${12}\"; check_acl \"${12}\"; fi"
        ));
    // Ownership precedes mode precedes capabilities, as for the helper.
    assert!(script.contains("chown \"root:$5\" \"${12}\""));
    assert!(script.contains("chmod 0750 \"${12}\""));
    assert!(script.contains("setcap \"${13}\" \"${12}\""));
    assert!(
        script.find("chown \"root:$5\" \"${12}\"").unwrap()
            < script.find("chmod 0750 \"${12}\"").unwrap()
    );
    assert!(
        script.find("chmod 0750 \"${12}\"").unwrap()
            < script.find("setcap \"${13}\" \"${12}\"").unwrap()
    );
}

#[test]
fn grant_argv_neutralizes_shell_metacharacters_in_path() {
    // A malicious backend path must be passed verbatim as a positional
    // argument, never interpolated into the executed script.
    let mut plan = in_place(Some("/tmp/run; chmod 777 /etc; #"));
    let GrantPlan::InPlace { backend, .. } = &mut plan else {
        unreachable!()
    };
    *backend = target("/tmp/x'; touch /pwned; '", "8:1");
    let argv_raw = grant_argv(&plan);
    assert_eq!(argv_raw[2].to_str().unwrap(), grant_script(&plan));
    assert_eq!(argv_raw[5].to_str().unwrap(), "/tmp/x'; touch /pwned; '");
    assert_eq!(
        argv_raw[10].to_str().unwrap(),
        "/tmp/run; chmod 777 /etc; #"
    );
}

#[test]
fn relocating_grant_copies_then_grants_the_destination() {
    let plan = relocate();
    let argv_raw = grant_argv(&plan);
    let argv = strs(&argv_raw);
    assert_eq!(argv.len(), 4 + 15);
    assert_eq!(
        argv[4..],
        [
            BACKEND_CAPS,
            "/usr/bin/xray",
            "/usr/local/lib/v2ray-rs",
            HELPER_GROUP,
            "/usr/local/lib/v2ray-rs/v2ray-rs-netctl",
            HELPER_CAPS,
            "zhuk",
            "8:1",
            "1000",
            "absent",
            "",
            "",
            "12345",
            "",
            "",
        ]
    );
    // The source is the stdin payload now; it never reaches the shell.
    assert!(!argv.contains(&"/tmp/.mount_abc/usr/bin/v2ray-rs-netctl"));
}

#[test]
fn relocating_grant_carries_the_antidpi_payload() {
    let plan = relocate_with_antidpi();
    let argv_raw = grant_argv(&plan);
    let argv = strs(&argv_raw);
    assert_eq!(argv.len(), 4 + 15);
    assert_eq!(argv[14], "/usr/local/lib/v2ray-rs/v2ray-rs-antidpi");
    assert_eq!(argv[15], ANTIDPI_CAPS);
    assert_eq!(argv[16], "12345");
    assert_eq!(argv[17], "54321");
    // ${15}: the pre-elevation identity of the anti-DPI destination,
    // re-checked in the script like the route helper's.
    assert_eq!(argv[18], "8:9");
    // The anti-DPI source also travels on stdin, never as a path.
    assert!(!argv.contains(&"/tmp/.mount_abc/usr/bin/v2ray-rs-antidpi"));
}

#[test]
fn relocating_grant_keeps_metachar_paths_positional() {
    let mut plan = relocate();
    let GrantPlan::Relocate { dst, .. } = &mut plan else {
        unreachable!()
    };
    *dst = PathBuf::from("/tmp/x\"; touch /pwned; \"");
    let argv_raw = grant_argv(&plan);
    assert_eq!(argv_raw[2].to_str().unwrap(), grant_script(&plan));
    assert_eq!(argv_raw[8].to_str().unwrap(), "/tmp/x\"; touch /pwned; \"");
}

#[test]
fn relocating_script_never_reads_the_source_as_root() {
    let script = grant_script(&relocate());
    // The payload arrives on stdin because root cannot read the FUSE mount
    // the source lives on; any command naming it would fail with EACCES.
    assert!(script.contains("head -c \"${13}\" > \"$tmp_h\""));
    assert!(!script.contains("cp "));
}

#[test]
fn inplace_script_provisions_the_bypass_user() {
    let script = grant_script(&in_place(Some("/usr/bin/v2ray-rs-run")));
    assert!(script.contains("if [ -n \"$7\" ]"));
    assert!(script.contains("getent passwd v2ray-rs-bypass"));
    assert!(script.contains(
            "useradd --system --no-create-home --home-dir /nonexistent --shell /usr/sbin/nologin v2ray-rs-bypass"
        ));
    // The account must exist before the wrapper is made setuid.
    assert!(script.find("useradd --system").unwrap() < script.find("chmod 4750 \"$7\"").unwrap());
}

#[test]
fn inplace_script_applies_setuid_last() {
    let script = grant_script(&in_place(Some("/usr/bin/v2ray-rs-run")));
    let setuid = script.find("chmod 4750 \"$7\"").unwrap();
    // The setuid bit lands only after every capability grant has
    // succeeded: a late failure must not leave a setuid binary without
    // its caps, and chown still precedes because it clears the bit.
    for earlier in [
        "chown \"root:$5\" \"$7\"",
        "setcap \"$3\" \"$4\"",
        "setcap \"${13}\" \"${12}\"",
        "setcap \"$1\" \"$2\"",
    ] {
        assert!(
            script.find(earlier).unwrap() < setuid,
            "{earlier} must precede the setuid bit"
        );
    }
    assert_eq!(script.rfind("chmod 4750"), Some(setuid));
}

#[test]
fn failure_state_names_every_planned_target() {
    let dir = tempfile::TempDir::new().unwrap();
    let helper = dir.path().join("v2ray-rs-netctl");
    std::fs::write(&helper, b"helper").unwrap();
    let antidpi = dir.path().join("v2ray-rs-antidpi");
    std::fs::write(&antidpi, b"antidpi").unwrap();
    let backend = dir.path().join("xray");
    std::fs::write(&backend, b"backend").unwrap();
    let missing_wrapper = dir.path().join("missing-wrapper");
    let backend_caps = required_caps(BACKEND_CAPS);
    let probes = [
        StateProbe::new(
            "route helper",
            &helper,
            Some(0o750),
            Some(&HELPER_REQUIRED_CAPS),
            true,
        ),
        StateProbe::new(
            "setuid wrapper",
            &missing_wrapper,
            Some(0o4750),
            None,
            false,
        ),
        StateProbe::new(
            "anti-DPI helper",
            &antidpi,
            Some(0o750),
            Some(&ANTIDPI_REQUIRED_CAPS),
            true,
        ),
        StateProbe::new("backend", &backend, None, Some(&backend_caps), false),
    ];
    let state = confirmed_grant_state("zhuk", &probes);
    for label in [
        "route helper",
        "setuid wrapper",
        "anti-DPI helper",
        "backend",
    ] {
        assert!(state.contains(label), "{label} missing from: {state}");
    }
    // A fresh unprivileged copy holds none of the grant's postconditions.
    assert!(state.contains("not confirmed"), "{state}");
}

#[test]
fn recovery_location_demands_the_managed_copies() {
    let dir = tempfile::TempDir::new().unwrap();
    let bundled = dir.path().join("v2ray-rs-netctl");
    std::fs::write(&bundled, b"helper").unwrap();
    let relocated = dir.path().join("relocated").join("v2ray-rs-netctl");
    std::fs::create_dir(relocated.parent().unwrap()).unwrap();
    std::fs::write(&relocated, b"helper").unwrap();
    assert_eq!(
        recovery_location_violation(&bundled, Some(&bundled), &relocated),
        None
    );
    assert_eq!(
        recovery_location_violation(&relocated, Some(&bundled), &relocated),
        None
    );
    // Right basename, wrong location (a PATH-style copy).
    let bin = dir.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let other = bin.join("v2ray-rs-netctl");
    std::fs::write(&other, b"helper").unwrap();
    assert_eq!(
        recovery_location_violation(&other, Some(&bundled), &relocated),
        Some("recovery target is neither the packaged nor the relocated route helper".into())
    );
    let wrong_name = dir.path().join("netctl");
    std::fs::write(&wrong_name, b"helper").unwrap();
    assert!(
        recovery_location_violation(&wrong_name, Some(&bundled), &relocated)
            .is_some_and(|detail| detail.contains("v2ray-rs-netctl"))
    );
}

#[test]
fn recovery_refuses_untrusted_targets_without_elevation() {
    let dir = tempfile::TempDir::new().unwrap();
    let bundled = dir.path().join("v2ray-rs-netctl");
    std::fs::write(&bundled, b"helper").unwrap();
    let relocated = dir.path().join("nowhere");
    let refused = |path: &Path| {
        validate_recovery_target_at(path, Some(&bundled), &relocated)
            .expect_err("target must be refused")
            .to_string()
    };
    // PATH-only copy: right name, wrong location.
    let path_only = dir.path().join("bin");
    std::fs::create_dir(&path_only).unwrap();
    let path_only = path_only.join("v2ray-rs-netctl");
    std::fs::write(&path_only, b"helper").unwrap();
    assert!(refused(&path_only).contains("neither the packaged nor the relocated"));
    // Symlink to the managed copy.
    let link = dir.path().join("linked-helper");
    std::os::unix::fs::symlink(&bundled, &link).unwrap();
    assert!(refused(&link).contains("not a non-symlink regular file"));
    // User-writable helper: the file passes the location check as the
    // bundled copy, so ownership and mode bits alone must refuse it.
    let writable_dir = dir.path().join("writable");
    std::fs::create_dir(&writable_dir).unwrap();
    let writable = writable_dir.join("v2ray-rs-netctl");
    std::fs::write(&writable, b"helper").unwrap();
    let mut perms = std::fs::metadata(&writable).unwrap().permissions();
    use std::os::unix::fs::PermissionsExt;
    perms.set_mode(0o664);
    std::fs::set_permissions(&writable, perms).unwrap();
    let writable_refusal = validate_recovery_target_at(&writable, Some(&writable), &relocated)
        .expect_err("target must be refused")
        .to_string();
    assert!(
        writable_refusal.contains("not root-owned")
            || writable_refusal.contains("parent directory is not protected"),
        "{writable_refusal}"
    );
    // Unprivileged fixture: the file itself is not root-owned (and its
    // tempdir parents would not be protected either).
    let refusal = refused(&bundled);
    assert!(
        refusal.contains("not root-owned") || refusal.contains("parent directory is not protected"),
        "{refusal}"
    );
}

#[test]
fn recovery_validates_the_canonical_path_through_a_symlinked_parent() {
    let dir = tempfile::TempDir::new().unwrap();
    let real_dir = dir.path().join("real");
    std::fs::create_dir(&real_dir).unwrap();
    let bundled = real_dir.join("v2ray-rs-netctl");
    std::fs::write(&bundled, b"helper").unwrap();
    let relocated = dir.path().join("nowhere");
    // Caller names the managed copy through a symlinked parent. All
    // checks run on the canonical path, so the location check passes on
    // the alias too and the refusal comes from the (unprivileged) parent
    // chain — proving the alias itself was never the validated target.
    let link_dir = dir.path().join("link");
    std::os::unix::fs::symlink(&real_dir, &link_dir).unwrap();
    let caller_path = link_dir.join("v2ray-rs-netctl");

    let refusal = validate_recovery_target_at(&caller_path, Some(&bundled), &relocated)
        .expect_err("unprivileged fixture parents must be refused")
        .to_string();
    assert!(
        refusal.contains("parent directory is not protected"),
        "canonicalization failed: the refusal named the caller alias instead: {refusal}"
    );

    // Without canonicalization the same alias would fail the location
    // check against the real copy; confirm that check does accept the
    // canonical path in this fixture.
    let canonical_refusal = validate_recovery_target_at(&bundled, Some(&bundled), &relocated)
        .expect_err("unprivileged fixture parents must be refused")
        .to_string();
    assert_eq!(refusal, canonical_refusal);
}

#[test]
fn eligibility_matches_the_grant_planner() {
    // Absent and unmanaged candidates are never eligible: the planner
    // skips the optional helper, and the UI reads that as "no repair to
    // offer".
    assert!(!antidpi_grant_eligible(None));
    let dir = tempfile::TempDir::new().unwrap();
    let unmanaged = dir.path().join("v2ray-rs-antidpi");
    std::fs::write(&unmanaged, b"helper").unwrap();
    assert!(!antidpi_grant_eligible(Some(&unmanaged)));
    assert!(!antidpi_grant_eligible(Some(Path::new("/does/not/exist"))));
    // The bundled copy beside this test executable is managed: once it
    // exists the planner accepts it and accepts nothing else.
    let exe_dir = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let planted = exe_dir.join("v2ray-rs-antidpi");
    std::fs::write(&planted, b"helper").unwrap();
    assert!(antidpi_grant_eligible(Some(&planted)));
    assert!(!antidpi_grant_eligible(Some(&unmanaged)));
    std::fs::remove_file(&planted).unwrap();
}

#[test]
fn relocating_script_targets_the_destination_only() {
    let script = grant_script(&relocate());
    // Attributes are staged on the temp file; ownership clears caps and
    // the setuid bit, so it precedes the mode and the capability grant.
    assert!(script.contains("setcap \"$6\" \"$tmp_h\""));
    assert!(script.contains("chmod 0750 \"$tmp_h\""));
    assert!(
        script.find("chown \"root:$4\" \"$tmp_h\"").unwrap()
            < script.find("chmod 0750 \"$tmp_h\"").unwrap()
    );
    assert!(
        script.find("chmod 0750 \"$tmp_h\"").unwrap()
            < script.find("setcap \"$6\" \"$tmp_h\"").unwrap()
    );
    // Publication is an atomic rename in the destination's own directory.
    assert!(script.contains("mv -f \"$tmp_h\" \"$dst_h\""));
    // No truncating write or unlink ever names a destination directly.
    for banned in ["cat > \"$5\"", "> \"$5\"", "rm -f \"$5\""] {
        assert!(!script.contains(banned), "{banned}");
    }
}

#[test]
fn relocating_script_reads_exactly_the_declared_payload_bytes() {
    let script = grant_script(&relocate());
    // Helper-only plans consume the exact declared count too; the empty
    // ${11} merely skips the second stage.
    assert!(script.contains("[ \"$(stat -c %s \"$tmp_h\")\" = \"${13}\" ]"));
    // Anything left after the declared payloads is a refusal, so a short
    // or excess stdin stream can never publish a wrong binary.
    assert!(script.contains("[ \"$(head -c 1 | wc -c)\" = 0 ]"));
}

#[test]
fn relocating_script_splits_the_payload_for_the_antidpi_target() {
    let script = grant_script(&relocate_with_antidpi());
    // The helper's bytes are consumed by exact count (GNU `head -c` reads
    // exactly N bytes from a pipe), then the anti-DPI binary follows, also
    // by exact count with its own size verification.
    assert!(script.contains("head -c \"${14}\" > \"$tmp_a\""));
    assert!(script.contains("[ \"$(stat -c %s \"$tmp_a\")\" = \"${14}\" ]"));
    // Ownership → mode → capabilities order for the anti-DPI temp file.
    assert!(script.contains("chown \"root:$4\" \"$tmp_a\""));
    assert!(script.contains("chmod 0750 \"$tmp_a\""));
    assert!(script.contains("setcap \"${12}\" \"$tmp_a\""));
    assert!(
        script.find("chown \"root:$4\" \"$tmp_a\"").unwrap()
            < script.find("chmod 0750 \"$tmp_a\"").unwrap()
    );
    assert!(
        script.find("chmod 0750 \"$tmp_a\"").unwrap()
            < script.find("setcap \"${12}\" \"$tmp_a\"").unwrap()
    );
    // The receipts keep the parser's order: helper record first, the
    // optional anti-DPI record last, each the recorded inode identity.
    assert!(script.contains("printf 'antidpi-identity=%s\\n' \"$new_a\""));
    let helper = script.find("printf 'helper-identity=").unwrap();
    let antidpi = script.find("printf 'antidpi-identity=").unwrap();
    assert!(helper < antidpi);
}

#[test]
fn relocating_script_restricts_to_a_dedicated_group() {
    let script = grant_script(&relocate());
    // Never the caller's primary group: where distributions share one, that
    // would hand every local account a cap_net_admin binary.
    assert!(script.contains("groupadd --system \"$4\""));
    assert!(script.contains("chown \"root:$4\" \"$tmp_h\""));
    // Enrollment is unconditional: a session that has not picked up the
    // group yet must not skip the persistent account change.
    assert!(script.contains("usermod -aG \"$4\" -- \"$7\""));
    assert!(!script.contains("id -nG"));
}

#[test]
fn relocating_script_reports_the_installed_identity_last() {
    let script = grant_script(&relocate());
    // The transaction record's printf lines are all redirected to the
    // state file; the helper receipt is the first stdout write and, for
    // a helper-only plan, the last one: only the conditional anti-DPI
    // record may follow.
    let helper_receipt = script.find("printf 'helper-identity=").unwrap();
    let last_record_printf = script[..helper_receipt].rfind("printf '").unwrap();
    assert!(
        script[last_record_printf..helper_receipt].contains("> \"$STATE\""),
        "a printf before the receipts does not target the state file"
    );
    let tail = &script[script.find("printf 'helper-identity=").unwrap()..];
    assert!(tail.trim_end().ends_with("fi"), "{tail}");
    for banned in [
        "chown ", "chmod ", "setcap ", "mv -f ", "head -c", "mktemp", "usermod", "groupadd",
    ] {
        assert!(!tail.contains(banned), "{banned} follows the receipts");
    }
}

#[test]
fn relocating_script_parses_as_posix_sh() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("relocate.sh");
    std::fs::write(&path, grant_script(&relocate_with_antidpi())).unwrap();
    let status = std::process::Command::new("/bin/sh")
        .arg("-n")
        .arg(&path)
        .status()
        .expect("/bin/sh is present on Linux");
    assert!(status.success(), "sh -n rejected the relocating script");
}

#[test]
fn relocating_script_verifies_both_destinations_against_recorded_identities() {
    let script = grant_script(&relocate_with_antidpi());
    // The route helper destination and the anti-DPI destination are both
    // compared to their pre-elevation `device:inode` records, each with
    // its own protected-parent walk.
    assert!(script.contains("check_prior \"$5\" \"${10}\""));
    assert!(script.contains("check_prior \"${11}\" \"${15}\""));
    // Any divergence from the recorded identity refuses, an existing
    // target refuses a recorded `absent`, and a missing target refuses a
    // recorded identity.
    assert!(script.contains(
        "[ \"$(stat -c '%d:%i' \"$1\")\" = \"$2\" ] || refuse \"target identity changed: $1\""
    ));
    assert!(script.contains("[ \"$2\" != absent ]"));
    assert!(script.contains("[ \"$2\" = absent ]"));
}

#[test]
fn relocating_script_publishes_destinations_with_atomic_renames() {
    let script = grant_script(&relocate_with_antidpi());
    assert!(script.contains("mv -f \"$tmp_h\" \"$dst_h\""));
    assert!(script.contains("mv -f \"$tmp_a\" \"$dst_a\""));
    // No truncating write or unlink ever names a destination directly.
    for banned in [
        "cat > \"$5\"",
        "cat > \"${11}\"",
        "> \"$5\"",
        "> \"${11}\"",
        "rm -f \"$5\"",
        "rm -f \"${11}\"",
    ] {
        assert!(!script.contains(banned), "{banned}");
    }
}

#[test]
fn relocating_script_removes_staged_temps_on_any_exit() {
    let script = grant_script(&relocate_with_antidpi());
    assert!(script.contains("trap cleanup EXIT"));
    assert!(script.contains("trap 'exit 1' HUP INT TERM"));
    // The staged payloads live in the transaction directory, and the
    // EXIT cleanup removes the whole directory unless a retained state
    // file records an interrupted grant for the next publisher.
    assert!(script.contains("rm -rf \"$txdir\""));
    assert!(script.contains("if [ -f \"$STATE\" ]"));
}

#[test]
fn relocating_script_prints_helper_identity_before_the_antidpi_record() {
    let script = grant_script(&relocate_with_antidpi());
    let helper = script.find("printf 'helper-identity=").unwrap();
    let antidpi = script.find("printf 'antidpi-identity=").unwrap();
    assert!(helper < antidpi);
    // Both destinations are published before the first receipt, and the
    // anti-DPI record is the script's final statement.
    assert!(script.rfind("mv -f").unwrap() < helper);
    assert!(script[antidpi..].trim_end().ends_with("fi"));
}

#[test]
fn inplace_script_validates_every_target_before_the_first_mutation() {
    let script = grant_script(&in_place(None));
    // Identities recorded pre-elevation are compared under root before
    // anything is modified.
    assert!(script.contains("stat -c '%d:%i'"));
    for gate in [
        "check_file \"$2\" \"$8\"",
        "check_file \"$4\" \"$9\"",
        "usermod -aG \"$5\" -- \"$6\"",
    ] {
        assert!(script.contains(gate), "missing {gate}");
    }
    let validate = script.find("usermod").unwrap();
    assert!(script.find("check_file \"$4\" \"$9\"").unwrap() < validate);
    // Ownership changes precede modes and capability grants.
    assert!(
        script.find("chown \"root:$5\" \"$4\"").unwrap()
            < script.find("chmod 0750 \"$4\"").unwrap()
    );
    assert!(
        script.find("chmod 0750 \"$4\"").unwrap() < script.find("setcap \"$3\" \"$4\"").unwrap()
    );
    // Only a missing group key (getent status 2) may create the group.
    assert!(script.contains("[ \"$rc\" = 2 ]"));
    // The script re-resolves the account and pins the invoking UID.
    assert!(script.contains("id -u \"$6\""));
    // The ACL probe is bounded and repeated under elevation.
    assert!(script.contains("timeout 5 getfacl"));
}

#[test]
fn root_caller_is_refused_before_any_planning() {
    assert!(ensure_not_root(0, 0).is_err());
    assert!(ensure_not_root(0, 1000).is_err());
    assert!(ensure_not_root(1000, 0).is_err());
    assert!(ensure_not_root(1000, 1000).is_ok());
    assert!(matches!(
        ensure_not_root(0, 1000),
        Err(PrivilegeError::RootEnrollment)
    ));
}

#[test]
fn unsafe_target_details_are_exact() {
    let not_a_file = "target is not a non-symlink regular file";
    assert_eq!(
        file_target_violation(true, true, 0, 0o750),
        Some(not_a_file)
    );
    assert_eq!(
        file_target_violation(false, false, 0, 0o750),
        Some(not_a_file)
    );
    assert_eq!(
        file_target_violation(false, true, 1000, 0o750),
        Some("target is not root-owned")
    );
    assert_eq!(
        file_target_violation(false, true, 0, 0o770),
        Some("target is writable by non-root accounts")
    );
    assert_eq!(
        file_target_violation(false, true, 0, 0o757),
        Some("target is writable by non-root accounts")
    );
    assert_eq!(file_target_violation(false, true, 0, 0o750), None);
    assert_eq!(file_target_violation(false, true, 0, 0o4750), None);
}

#[test]
fn unsafe_target_refuses_real_files_owned_by_the_caller() {
    // The test asserts the non-root verdict "target is not root-owned".
    // Running as root the temporary file really is root-owned, so the
    // ownership check cannot be exercised at all.
    if !crate::privilege::caps_check_needed(nix::unistd::geteuid().as_raw()) {
        return;
    }
    let dir = tempfile::TempDir::new().unwrap();
    let file = dir.path().join("helper");
    std::fs::write(&file, b"bin").unwrap();

    let err = validate_mutation_target(&file).unwrap_err();
    assert!(
        matches!(
            &err,
            PrivilegeError::UnsafeGrantTarget { detail, .. }
                if detail == "target is not root-owned"
        ),
        "{err:?}"
    );

    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&file, &link).unwrap();
    let err = validate_mutation_target(&link).unwrap_err();
    assert!(
        matches!(
            &err,
            PrivilegeError::UnsafeGrantTarget { detail, .. }
                if detail == "target is not a non-symlink regular file"
        ),
        "{err:?}"
    );
}

#[test]
fn acl_parser_accepts_plain_and_named_records() {
    assert_eq!(
        parse_access_acl("user::rw-\ngroup::r--\nother::r--\n"),
        Some(AclNamedMask::NoNamed)
    );
    assert_eq!(
        parse_access_acl("user::rwx\nuser:1000:r-x\ngroup::r-x\nmask::r-x\nother::---\n"),
        Some(AclNamedMask::Mask(0o5))
    );
    assert_eq!(
        parse_access_acl("user::rwx\ngroup::r-x\nuser:0:---\nmask::rwx\nother::---\n"),
        Some(AclNamedMask::Mask(0o7))
    );
    // A named entry without a mask record is incomplete.
    assert_eq!(
        parse_access_acl("user::rwx\nuser:1000:r-x\nother::---\n"),
        None
    );
    // Real multi-line base-only ACL with a trailing blank line.
    assert_eq!(
        parse_access_acl("user::rw-\ngroup::r--\nother::---\n\n"),
        Some(AclNamedMask::NoNamed)
    );
    // A named group entry with a real base-only prefix.
    assert_eq!(
        parse_access_acl("user::rwx\ngroup::r-x\ngroup:1000:r-x\nmask::r-x\nother::---\n"),
        Some(AclNamedMask::Mask(0o5))
    );
    // Two-part base records are not a real getfacl shape.
    assert_eq!(parse_access_acl("user:rw-\nother::r--\n"), None);
    // Missing a base record.
    assert_eq!(parse_access_acl("user::rwx\ngroup::r-x\n"), None);
    // Duplicate base record.
    assert_eq!(
        parse_access_acl("user::rwx\nuser::r-x\ngroup::r-x\nother::---\n"),
        None
    );
    // Two-field mask shape is not a real getfacl record.
    assert_eq!(parse_access_acl("user::rwx\nmask:r-x\nother::---\n"), None);
}

#[test]
fn acl_parser_rejects_malformed_records() {
    // Duplicate mask.
    assert_eq!(parse_access_acl("mask::r-x\nmask::r-x\n"), None);
    // Permissions must be exactly three position-valid characters.
    assert_eq!(parse_access_acl("user::rw\n"), None);
    assert_eq!(parse_access_acl("user::xrw\n"), None);
    // Named qualifiers must be numeric.
    assert_eq!(parse_access_acl("user:zhuk:r-x\nmask::r-x\n"), None);
    // Unknown record shape.
    assert_eq!(parse_access_acl("default:user::rwx\n"), None);
    assert_eq!(parse_access_acl("junk\n"), None);
    assert_eq!(parse_access_acl("user:1:2:3\n"), None);
}

#[test]
fn acl_mask_decision_blocks_dormant_named_access() {
    // The helper/wrapper modes need group r-x; an old mask that lacks
    // either bit would be broadened by the mode change.
    assert!(mask_broadens_named_access(0));
    assert!(mask_broadens_named_access(0o4));
    assert!(mask_broadens_named_access(0o1));
    assert!(!mask_broadens_named_access(0o5));
    assert!(!mask_broadens_named_access(0o7));
}

#[test]
fn helper_identity_record_must_be_the_whole_stdout() {
    assert_eq!(
        parse_grant_identities("helper-identity=8:42\n"),
        Some(((8, 42), None))
    );
    // Leading zeros are still plain decimal digits.
    assert_eq!(
        parse_grant_identities("helper-identity=08:042\n"),
        Some(((8, 42), None))
    );
    assert_eq!(parse_grant_identities("helper-identity=8:42"), None);
    assert_eq!(
        parse_grant_identities("helper-identity=8:42\nextra\n"),
        None
    );
    assert_eq!(
        parse_grant_identities("polkit: granted\nhelper-identity=8:42\n"),
        None
    );
    assert_eq!(parse_grant_identities("helper-identity=8:42:9\n"), None);
    assert_eq!(parse_grant_identities("helper-identity=8:0x1f\n"), None);
    assert_eq!(parse_grant_identities("helper-identity=:42\n"), None);
    assert_eq!(parse_grant_identities(""), None);
}

#[test]
fn antidpi_identity_record_is_optional_and_strict() {
    assert_eq!(
        parse_grant_identities("helper-identity=8:42\nantidpi-identity=8:43\n"),
        Some(((8, 42), Some((8, 43))))
    );
    // A plan without an anti-DPI target must not receive a second record.
    assert_eq!(
        parse_grant_identities("helper-identity=8:42\nantidpi-identity=8:43\nextra\n"),
        None
    );
    assert_eq!(parse_grant_identities("helper-identity=8:42\njunk\n"), None);
    assert_eq!(
        parse_grant_identities("helper-identity=8:42\nhelper-identity=8:43\n"),
        None
    );
    assert_eq!(parse_grant_identities("antidpi-identity=8:43\n"), None);
}

#[test]
fn antidpi_caps_require_both_tokens() {
    let both = "/usr/bin/v2ray-rs-antidpi cap_net_admin,cap_net_raw=ep\n";
    assert!(getcap_has_cap(both, "cap_net_admin"));
    assert!(getcap_has_cap(both, "cap_net_raw"));
    let one = "/usr/bin/v2ray-rs-antidpi cap_net_admin=ep\n";
    assert!(getcap_has_cap(one, "cap_net_admin"));
    assert!(!getcap_has_cap(one, "cap_net_raw"));
    let none = "/usr/bin/v2ray-rs-antidpi\n";
    assert!(!getcap_has_cap(none, "cap_net_admin"));
    assert!(!getcap_has_cap(none, "cap_net_raw"));
}

#[test]
fn antidpi_parser_accepts_grouped_separated_and_whole_set_clauses() {
    let path = Path::new("/usr/bin/v2ray-rs-antidpi");
    assert!(antidpi_caps_present(
        "/usr/bin/v2ray-rs-antidpi cap_net_admin,cap_net_raw=ep\n",
        path
    ));
    assert!(antidpi_caps_present(
        "/usr/bin/v2ray-rs-antidpi cap_net_admin=eip cap_net_raw=ep\n",
        path
    ));
    assert!(antidpi_caps_present(
        "/usr/bin/v2ray-rs-antidpi cap_net_raw=eip cap_net_admin=ep\n",
        path
    ));
    // Extra unrelated capabilities never hurt.
    assert!(antidpi_caps_present(
        "/usr/bin/v2ray-rs-antidpi cap_net_bind_service=ep cap_net_admin=ep cap_net_raw=ep\n",
        path
    ));
    // libcap's whole-set clause carries every capability.
    assert!(antidpi_caps_present(
        "/usr/bin/v2ray-rs-antidpi =ep\n",
        path
    ));
}

#[test]
fn antidpi_parser_rejects_permitted_only_flags() {
    let path = Path::new("/usr/bin/v2ray-rs-antidpi");
    assert!(!antidpi_caps_present(
        "/usr/bin/v2ray-rs-antidpi cap_net_admin,cap_net_raw=p\n",
        path
    ));
    assert!(!antidpi_caps_present(
        "/usr/bin/v2ray-rs-antidpi cap_net_admin=p cap_net_raw=ep\n",
        path
    ));
    assert!(!antidpi_caps_present(
        "/usr/bin/v2ray-rs-antidpi cap_net_admin=ep cap_net_raw=i\n",
        path
    ));
    // The whole-set clause without `e` grants nothing effective either.
    assert!(!antidpi_caps_present(
        "/usr/bin/v2ray-rs-antidpi =p\n",
        path
    ));
}

#[test]
fn antidpi_parser_rejects_a_missing_capability() {
    let path = Path::new("/usr/bin/v2ray-rs-antidpi");
    assert!(!antidpi_caps_present(
        "/usr/bin/v2ray-rs-antidpi cap_net_admin=ep\n",
        path
    ));
    assert!(!antidpi_caps_present(
        "/usr/bin/v2ray-rs-antidpi cap_net_raw=ep\n",
        path
    ));
    // `getcap` prints nothing for a file without capabilities.
    assert!(!antidpi_caps_present("", path));
    assert!(!antidpi_caps_present("\n", path));
}

#[test]
fn antidpi_parser_ignores_capability_names_in_the_path() {
    let probed = Path::new("/var/cap_net_admin/cap_net_raw/xray");
    // The names live in the probed path and the capset lacks both.
    assert!(!antidpi_caps_present(
        "/var/cap_net_admin/cap_net_raw/xray cap_net_bind_service=ep\n",
        probed
    ));
    // A line naming a different file is not evidence about this one.
    assert!(!antidpi_caps_present(
        "/other/xray cap_net_admin=ep cap_net_raw=ep\n",
        probed
    ));
    // A path prefix continuing into different characters is another file.
    assert!(!antidpi_caps_present(
        "/usr/bin/v2ray-rs-antidpi2 cap_net_admin=ep cap_net_raw=ep\n",
        Path::new("/usr/bin/v2ray-rs-antidpi")
    ));
}

#[test]
fn antidpi_parser_rejects_malformed_clause_shapes() {
    let path = Path::new("/usr/bin/v2ray-rs-antidpi");
    // No `=` separating names from flags.
    assert!(!antidpi_caps_present(
        "/usr/bin/v2ray-rs-antidpi cap_net_admin cap_net_raw\n",
        path
    ));
    // Unknown flag characters, including case.
    assert!(!antidpi_caps_present(
        "/usr/bin/v2ray-rs-antidpi cap_net_admin,cap_net_raw=epx\n",
        path
    ));
    assert!(!antidpi_caps_present(
        "/usr/bin/v2ray-rs-antidpi cap_net_admin,cap_net_raw=EP\n",
        path
    ));
    // An empty name inside a group.
    assert!(!antidpi_caps_present(
        "/usr/bin/v2ray-rs-antidpi cap_net_admin,,cap_net_raw=ep\n",
        path
    ));
    // A repeated capability never occurs in real libcap output.
    assert!(!antidpi_caps_present(
        "/usr/bin/v2ray-rs-antidpi cap_net_admin=ep cap_net_admin=ep cap_net_raw=ep\n",
        path
    ));
}

#[test]
fn probe_and_verify_reject_a_file_without_antidpi_caps() {
    let dir = tempfile::TempDir::new().unwrap();
    let file = dir.path().join("v2ray-rs-antidpi");
    std::fs::write(&file, b"binary").unwrap();
    // Whatever the environment's getcap answers (caps, nothing, or a
    // missing binary), a plain file must never be accepted.
    assert!(!matches!(probe_antidpi_caps(&file), Ok(true)));
    assert!(verify_antidpi_caps(&file).is_err());
}

#[test]
fn source_must_be_a_regular_file_beside_the_executable() {
    let dir = Path::new("/tmp/.mount_abc/usr/bin");
    let good = dir.join("v2ray-rs-netctl");

    assert!(source_acceptable(&good, dir, false, true));
    // A symlink beside the executable is refused outright: the caller stats
    // the path as given, before canonicalization resolves it away.
    assert!(!source_acceptable(&good, dir, true, true));
    assert!(!source_acceptable(&good, dir, false, false));
    // Anything outside the executable's own directory is refused.
    assert!(!source_acceptable(
        Path::new("/etc/shadow"),
        dir,
        false,
        true
    ));
}

#[test]
fn manual_hint_quotes_every_path_shape() {
    let cases = [
        ("/usr/bin/xray", "'/usr/bin/xray'"),
        ("/opt/my apps/xray", "'/opt/my apps/xray'"),
        ("/opt/x';id;'/xray", r"'/opt/x'\'';id;'\''/xray'"),
        ("/opt/$(id)/xray", "'/opt/$(id)/xray'"),
    ];
    for (path, quoted) in cases {
        let path = Path::new(path);
        let command = manual_command(path, BACKEND_CAPS);
        assert_eq!(command, format!("sudo setcap '{BACKEND_CAPS}' {quoted}"));
        let text = PrivilegeError::Unsupported {
            path: path.to_path_buf(),
            caps: BACKEND_CAPS.to_string(),
        }
        .to_string();
        assert!(text.ends_with(&command), "{text}");
    }
}

#[test]
fn manual_command_format() {
    assert_eq!(
        manual_command(Path::new("/usr/bin/xray"), BACKEND_CAPS),
        "sudo setcap 'cap_net_admin,cap_net_bind_service,cap_net_raw+ep' '/usr/bin/xray'"
    );
}

#[test]
fn mount_lookup_picks_longest_prefix() {
    let mounts = "\
/dev/sda1 / ext4 rw,relatime 0 0
tmpfs /tmp tmpfs rw,nosuid,nodev 0 0
/dev/sda2 /home ext4 rw,relatime 0 0
";
    assert_eq!(
        mount_for_path(mounts, Path::new("/usr/bin/xray")).as_deref(),
        Some("rw,relatime")
    );
    assert!(
        mount_for_path(mounts, Path::new("/tmp/x"))
            .unwrap()
            .contains("nosuid")
    );
    // `/tmpfoo` must not match the `/tmp` mount.
    assert_eq!(
        mount_for_path(mounts, Path::new("/tmpfoo/x")).as_deref(),
        Some("rw,relatime")
    );
}

#[test]
fn mount_lookup_skips_blank_and_malformed_lines() {
    let mounts =
        "\n\ntruncated\n/dev/sda1 / ext4 rw,relatime 0 0\n\ntmpfs /tmp tmpfs rw,nosuid,nodev 0 0\n";
    assert!(
        mount_for_path(mounts, Path::new("/tmp/x"))
            .unwrap()
            .contains("nosuid")
    );
    assert_eq!(
        mount_for_path(mounts, Path::new("/usr/bin/xray")).as_deref(),
        Some("rw,relatime")
    );
}

#[test]
fn nosuid_mount_is_unsupported() {
    let mounts = "tmpfs /mnt/ro tmpfs rw,nosuid 0 0\n/dev/sda1 / ext4 rw 0 0\n";
    assert!(
        mount_for_path(mounts, Path::new("/mnt/ro/xray"))
            .unwrap()
            .split(',')
            .any(|o| o == "nosuid")
    );
}

#[test]
fn nosuid_mount_drives_the_unsupported_error_text() {
    let mounts = "\
/dev/sda1 / ext4 rw 0 0
tmpfs /tmp/.mount_abc tmpfs rw,nosuid,nodev 0 0
";
    let backend = Path::new("/tmp/.mount_abc/usr/bin/xray");
    let opts = mount_for_path(mounts, backend).expect("backend sits under a mount");
    assert!(opts.split(',').any(|o| o == "nosuid"), "{opts}");

    // The manager carries this Display verbatim in its mount-unsupported
    // ProcessError, so the manual setcap wording must survive the trip.
    let text = PrivilegeError::Unsupported {
        path: backend.to_path_buf(),
        caps: BACKEND_CAPS.to_string(),
    }
    .to_string();
    assert!(text.contains("ignores file capabilities"), "{text}");
    assert!(
        text.ends_with(&manual_command(backend, BACKEND_CAPS)),
        "{text}"
    );
}

#[test]
fn preflight_targets_includes_helper_and_optional_wrapper() {
    let backend = Path::new("/usr/bin/xray");
    let helper = Path::new("/usr/bin/v2ray-rs-netctl");
    let wrapper = Path::new("/usr/lib/v2ray-rs/v2ray-rs-run");

    let plan = sel_in_place(None);
    let without_wrapper = preflight_targets(backend, &plan);
    assert_eq!(without_wrapper.len(), 2);
    assert_eq!(without_wrapper[0].0, backend);
    assert_eq!(without_wrapper[0].1, BACKEND_CAPS);
    assert_eq!(without_wrapper[1].0, helper);
    assert_eq!(without_wrapper[1].1, HELPER_CAPS);

    let plan = sel_in_place(Some("/usr/lib/v2ray-rs/v2ray-rs-run"));
    let with_wrapper = preflight_targets(backend, &plan);
    assert_eq!(with_wrapper.len(), 3);
    assert_eq!(with_wrapper[2].0, wrapper);
    assert_eq!(with_wrapper[2].1, HELPER_CAPS);

    let antidpi = Path::new("/usr/bin/v2ray-rs-antidpi");
    let plan = Selection::InPlace {
        helper: PathBuf::from("/usr/bin/v2ray-rs-netctl"),
        wrapper: None,
        antidpi: Some(antidpi.to_path_buf()),
    };
    let with_antidpi = preflight_targets(backend, &plan);
    assert_eq!(with_antidpi.len(), 3);
    assert_eq!(with_antidpi[2].0, antidpi);
    assert_eq!(with_antidpi[2].1, ANTIDPI_CAPS);
}

#[test]
fn relocating_preflight_checks_the_destination_not_the_appdir() {
    let plan = sel_relocate();
    let targets = preflight_targets(Path::new("/usr/bin/xray"), &plan);

    assert_eq!(targets.len(), 2);
    assert_eq!(targets[1].0, Path::new("/usr/local/lib/v2ray-rs"));
    // Preflighting the bundled copy would always abort: its mount is nosuid,
    // which is precisely why the plan relocates.
    assert!(
        !targets
            .iter()
            .any(|(p, _)| p.starts_with("/tmp/.mount_abc"))
    );

    let antidpi_dst = PathBuf::from("/usr/local/lib/v2ray-rs/v2ray-rs-antidpi");
    let plan = Selection::Relocate {
        dest_dir: PathBuf::from("/usr/local/lib/v2ray-rs"),
        src: pinned(1),
        dst: PathBuf::from("/usr/local/lib/v2ray-rs/v2ray-rs-netctl"),
        antidpi: Some(AntidpiRelocate {
            src: pinned(2),
            dst: antidpi_dst.clone(),
            prior_dst: "absent".into(),
        }),
    };
    let targets = preflight_targets(Path::new("/usr/bin/xray"), &plan);
    assert_eq!(targets.len(), 3);
    assert_eq!(targets[2].0, antidpi_dst);
    assert_eq!(targets[2].1, ANTIDPI_CAPS);
}

#[test]
fn mount_lookup_unescapes_octal_in_mount_points() {
    let mounts = "\
/dev/sda1 / ext4 rw,relatime 0 0
/dev/sdb1 /run/media/zhuk/My\\040Drive vfat rw,nosuid,nodev 0 0
";
    // Without un-escaping this falls back to / and reports caps supported.
    assert!(
        mount_for_path(mounts, Path::new("/run/media/zhuk/My Drive/xray"))
            .unwrap()
            .split(',')
            .any(|o| o == "nosuid")
    );
}

#[test]
fn mount_lookup_prefers_the_last_of_duplicate_entries() {
    // Over-mounts are listed in mount order; the effective one is the last.
    let mounts = "\
rootfs / rootfs rw,nosuid 0 0
/dev/sda1 / ext4 rw,relatime 0 0
";
    assert_eq!(
        mount_for_path(mounts, Path::new("/usr/bin/xray")).as_deref(),
        Some("rw,relatime")
    );
}

#[test]
fn mount_lookup_handles_a_destination_that_does_not_exist_yet() {
    let mounts = "/dev/sda1 / ext4 rw,relatime 0 0\n";
    assert_eq!(
        mount_for_path(mounts, Path::new("/usr/local/lib/v2ray-rs/v2ray-rs-netctl")).as_deref(),
        Some("rw,relatime")
    );
}

#[test]
fn probe_error_text_pinpoints_timeout() {
    assert_eq!(
        probe_error_text(&ProbeFailure::Timeout),
        "could not verify TUN capabilities: getcap timed out"
    );
}

#[test]
fn probe_error_text_names_libcap_when_getcap_is_missing() {
    assert_eq!(
        probe_error_text(&ProbeFailure::NotFound),
        "getcap not found; install libcap to use TUN"
    );
}

#[test]
fn probe_error_text_interpolates_trimmed_stderr_on_non_zero_exit() {
    assert_eq!(
        probe_error_text(&ProbeFailure::Exit("exit status 1".into())),
        "could not verify TUN capabilities: getcap exited with exit status 1"
    );
    // An empty stderr still produces the pinned prefix verbatim.
    assert_eq!(
        probe_error_text(&ProbeFailure::Exit(String::new())),
        "could not verify TUN capabilities: getcap exited with "
    );
}

#[test]
fn probe_error_text_says_getcap_could_not_run() {
    let err = std::io::Error::from_raw_os_error(nix::libc::EACCES);
    assert_eq!(
        probe_error_text(&ProbeFailure::Spawn(err.to_string())),
        format!("could not verify TUN capabilities: cannot run getcap: {err}")
    );
}

#[test]
fn unrunnable_getcap_is_a_spawn_failure() {
    let dir = tempfile::TempDir::new().unwrap();
    let probe = probe_net_admin_within(dir.path(), Path::new("/usr/bin/xray"), GETCAP_TIMEOUT);
    assert!(
        matches!(
            probe,
            Err(PrivilegeError::ProbeFailure(ProbeFailure::Spawn(_)))
        ),
        "{probe:?}"
    );
}

fn sh_probe(dir: &Path, script: &str, timeout: Duration) -> Result<bool, PrivilegeError> {
    // `sh` reads the script as the probed path, so no freshly written file
    // is ever exec'd (and no ETXTBSY race with parallel tests).
    let path = dir.join("getcap.sh");
    std::fs::write(&path, script).unwrap();
    probe_net_admin_within(Path::new("/bin/sh"), &path, timeout)
}

#[test]
fn probe_reads_the_capset_from_stdout() {
    let dir = tempfile::TempDir::new().unwrap();
    let probe = sh_probe(dir.path(), "echo \"$0 cap_net_admin+ep\"\n", GETCAP_TIMEOUT);
    assert!(probe.unwrap());
}

#[test]
fn probe_output_drain_is_bounded_by_the_deadline() {
    let dir = tempfile::TempDir::new().unwrap();
    let pid_file = dir.path().join("pid");
    let script = format!("sleep 30 &\necho $! > {}\nexit 0\n", pid_file.display());

    let started = Instant::now();
    let probe = sh_probe(dir.path(), &script, Duration::from_millis(300));
    let elapsed = started.elapsed();

    if let Ok(pid) = std::fs::read_to_string(&pid_file) {
        let pid = nix::unistd::Pid::from_raw(pid.trim().parse().unwrap());
        let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGKILL);
    }
    assert!(
        matches!(
            probe,
            Err(PrivilegeError::ProbeFailure(ProbeFailure::Timeout))
        ),
        "{probe:?}"
    );
    assert!(elapsed < Duration::from_secs(2), "{elapsed:?}");
}

#[test]
fn caps_check_needed_is_false_for_root() {
    assert!(!caps_check_needed(0));
}

#[test]
fn caps_check_needed_is_true_for_a_normal_user() {
    assert!(caps_check_needed(1000));
}

#[test]
fn relocating_script_verifies_staged_payloads_before_any_publish() {
    let script = grant_script(&relocate_with_antidpi());
    // Modes and capability sets are re-read from the staged files before
    // anything is published: `setcap` exits 0 even where the mount
    // silently drops capabilities, so success alone is not evidence.
    assert!(script.contains("check_staged \"$tmp_h\" \"$5\" \"$6\""));
    assert!(script.contains("check_staged \"$tmp_a\" \"${11}\" \"${12}\""));
    assert!(script.contains("getcap \"$1\""));
    assert!(script.contains("stat -c '%a' \"$1\""));
    let publish = script.find("mv -f \"$tmp_h\" \"$dst_h\"").unwrap();
    assert!(script.find("check_staged \"$tmp_h\"").unwrap() < publish);
    assert!(script.find("check_staged \"$tmp_a\"").unwrap() < publish);
}

#[test]
fn relocating_script_sets_backend_caps_before_the_first_rename() {
    let script = grant_script(&relocate());
    let backend_setcap = script.find("setcap \"$1\" \"$2\"").unwrap();
    let publish = script.find("mv -f \"$tmp_h\" \"$dst_h\"").unwrap();
    assert!(backend_setcap < publish);
}

#[test]
fn relocating_script_preserves_prior_inodes_in_rollback_names() {
    let script = grant_script(&relocate_with_antidpi());
    // Hard links into the transaction directory keep the original
    // destination inodes alive without ever unlinking their live names.
    assert!(script.contains("ln \"$dst_h\" \"$backup_h\""));
    assert!(script.contains("ln \"$dst_a\" \"$backup_a\""));
    // Backups only for destinations that exist at that point.
    assert!(script.contains("if [ \"$prior_h\" != absent ]"));
    assert!(script.contains("[ -n \"${11}\" ] && [ \"$prior_a\" != absent ]"));
    // The transaction directory is only discarded after both
    // publications and their identity checks, before the first receipt.
    let helper_receipt = script.find("printf 'helper-identity=").unwrap();
    assert!(script.rfind("stat -c '%d:%i' \"$dst_a\"").unwrap() < helper_receipt);
    assert!(script.rfind("rm -rf \"$txdir\"").unwrap() < helper_receipt);
}

#[test]
fn relocating_script_restores_originals_when_a_publish_fails() {
    let script = grant_script(&relocate_with_antidpi());
    let publish_failed = script.find("publish_failed() {").unwrap();
    let window = &script[publish_failed..script.find("cleanup() {").unwrap()];
    // The original route helper is restored first, then the anti-DPI
    // destination.
    assert!(window.contains("mv -f \"$rb_h\" \"$dst_h\""));
    assert!(window.contains("mv -f \"$rb_a\" \"$dst_a\""));
    assert!(
        window.find("mv -f \"$rb_h\" \"$dst_h\"").unwrap()
            < window.find("mv -f \"$rb_a\" \"$dst_a\"").unwrap()
    );
    // A failed restore keeps the rollback link and reports its path
    // instead of claiming success.
    assert!(window.contains("original route helper preserved at $rb_h"));
    assert!(window.contains("original anti-DPI helper preserved at $rb_a"));
    assert!(window.contains("refuse \"$detail\""));
    // A first install is rolled back by verified inode identity, not by
    // destination name.
    assert!(window.contains("= \"$new_h\" ]"));
    assert!(window.contains("rm -f \"$dst_h\""));
    assert!(window.contains("rm -f \"$dst_a\""));
    // The EXIT trap never touches a rollback link: a failed restore must
    // leave the preserved original for the operator, and the retained
    // state file routes the next grant through recovery instead.
    let cleanup = &script[script.find("cleanup() {").unwrap()..];
    let cleanup_body = &cleanup[..cleanup.find("[ \"$(id -u").unwrap()];
    assert!(!cleanup_body.contains("$rb_h"));
    assert!(!cleanup_body.contains("$rb_a"));
}

#[test]
fn relocating_script_serializes_publishers_and_records_the_transaction() {
    let script = grant_script(&relocate_with_antidpi());
    // Publishers serialize on a root-only lock before any recovery or
    // identity re-check runs.
    assert!(script.contains("GRANT_LOCK=/run/v2ray-rs-grant.lock"));
    assert!(script.contains("flock 9"));
    let lock = script.find("flock 9").unwrap();
    let recover = script.find("recover_interrupted\n").unwrap();
    let record = script.find("printf 'helper-prior=").unwrap();
    let publish = script.find("mv -f \"$tmp_h\" \"$dst_h\"").unwrap();
    assert!(lock < recover && recover < record && record < publish);
    // Interrupted transactions are recovered from the exact recorded
    // directory and never swept by wildcard.
    assert!(script.contains("\"$dest_dir\"/.v2ray-rs-tx.*)"));
    assert!(script.contains("rm -rf \"$tx\""));
    // A destination that matches neither the prior nor the staged
    // identity is reported as a conflict, never overwritten.
    assert!(script.contains("changed during an interrupted grant; left untouched"));
}

/// Mirror of the script's `recover_target` decision table, so the
/// recovery matrix stays readable in Rust; the shell text itself is
/// asserted branch by branch in the script tests above.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Decision {
    Restore,
    Keep,
    RemoveNew,
    Conflict,
}

fn interrupted_transaction_decision(
    backup_present: bool,
    prior: &str,
    staged_new: &str,
    current: Option<&str>,
) -> Decision {
    match (backup_present, prior, current) {
        // A backup of a path recorded absent is a corrupted record.
        (true, "absent", _) => Decision::Conflict,
        (true, _, Some(cur)) if cur == prior || cur == staged_new => Decision::Restore,
        (true, _, Some(_)) => Decision::Conflict,
        // The destination vanished: recreate it from the backup.
        (true, _, None) => Decision::Restore,
        (false, "absent", None) => Decision::Keep,
        (false, "absent", Some(cur)) if cur == staged_new => Decision::RemoveNew,
        (false, "absent", Some(_)) => Decision::Conflict,
        (false, _, Some(cur)) if cur == prior => Decision::Keep,
        (false, _, _) => Decision::Conflict,
    }
}

#[test]
fn interrupted_transaction_recovery_matrix() {
    use Decision::{Conflict, Keep, RemoveNew, Restore};
    // Backed-up prior present: restore over either recorded identity,
    // recreate when the destination vanished, refuse a foreign occupant.
    assert_eq!(
        interrupted_transaction_decision(true, "8:2", "9:7", Some("8:2")),
        Restore
    );
    assert_eq!(
        interrupted_transaction_decision(true, "8:2", "9:7", Some("9:7")),
        Restore
    );
    assert_eq!(
        interrupted_transaction_decision(true, "8:2", "9:7", None),
        Restore
    );
    assert_eq!(
        interrupted_transaction_decision(true, "8:2", "9:7", Some("1:1")),
        Conflict
    );
    // A backup of a path recorded absent is itself a corrupted record.
    assert_eq!(
        interrupted_transaction_decision(true, "absent", "9:7", Some("9:7")),
        Conflict
    );
    // No backup, prior present: keep the untouched original, refuse a
    // replaced or vanished one.
    assert_eq!(
        interrupted_transaction_decision(false, "8:2", "9:7", Some("8:2")),
        Keep
    );
    assert_eq!(
        interrupted_transaction_decision(false, "8:2", "9:7", Some("9:7")),
        Conflict
    );
    assert_eq!(
        interrupted_transaction_decision(false, "8:2", "9:7", None),
        Conflict
    );
    // Prior absent: keep the absence, remove only the verified staged
    // inode, refuse a foreign file.
    assert_eq!(
        interrupted_transaction_decision(false, "absent", "9:7", None),
        Keep
    );
    assert_eq!(
        interrupted_transaction_decision(false, "absent", "9:7", Some("9:7")),
        RemoveNew
    );
    assert_eq!(
        interrupted_transaction_decision(false, "absent", "9:7", Some("1:1")),
        Conflict
    );
}

#[test]
fn relocate_script_failure_paths_avoid_unbound_positionals() {
    let script = grant_script(&relocate_with_antidpi());
    // `publish_failed` and `cleanup` run without arguments, so under
    // `set -u` they must use the top-level destination bindings, never
    // the caller's positional parameters.
    let functions =
        &script[script.find("publish_failed() {").unwrap()..script.find("[ \"$(id -u").unwrap()];
    assert!(!functions.contains("$5"));
    assert!(!functions.contains("${11}"));
    assert!(functions.contains("\"$dst_h\""));
    assert!(functions.contains("\"$dst_a\""));
}

struct ProtectedDir(PathBuf);

impl Drop for ProtectedDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn running_as_root() -> bool {
    !caps_check_needed(nix::unistd::geteuid().as_raw())
}

fn var_lib_protected() -> bool {
    use std::os::unix::fs::PermissionsExt;
    match std::fs::metadata("/var/lib") {
        Ok(meta) => meta.is_dir() && meta.uid() == 0 && meta.permissions().mode() & 0o22 == 0,
        Err(_) => false,
    }
}

fn cap_tools_available() -> bool {
    let dirs = match std::env::var_os("PATH") {
        Some(dirs) => dirs,
        None => return false,
    };
    [
        "getcap", "setcap", "stat", "mktemp", "head", "wc", "chown", "chmod", "dirname", "flock",
        "ln", "readlink",
    ]
    .iter()
    .all(|tool| std::env::split_paths(&dirs).any(|dir| dir.join(tool).is_file()))
        && Path::new("/usr/bin/mv").is_file()
}

/// The script refuses unprotected parent chains, so the fixtures must sit
/// under a root-owned, non-world-writable directory. Only `/var/lib` is
/// reliably that on the target distributions; `/tmp` is 1777.
fn protected_fixture_dir(tag: &str) -> Option<ProtectedDir> {
    if !running_as_root() || !var_lib_protected() || !cap_tools_available() {
        return None;
    }
    let dir = PathBuf::from("/var/lib").join(format!(
        "v2ray-rs-grant-fixture-{}-{tag}",
        std::process::id()
    ));
    std::fs::create_dir(&dir).ok()?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).ok()?;
    if !file_caps_supported(&dir) {
        let _ = std::fs::remove_dir(&dir);
        return None;
    }
    Some(ProtectedDir(dir))
}

fn root_file(path: &Path, bytes: &[u8]) -> String {
    std::fs::write(path, bytes).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644)).unwrap();
    identity_of(path)
}

fn identity_of(path: &Path) -> String {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::symlink_metadata(path).unwrap();
    format!("{}:{}", meta.dev(), meta.ino())
}

fn assert_no_staging_leftovers(dir: &Path) {
    let leftovers: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(".v2ray-rs-"))
        .collect();
    assert!(leftovers.is_empty(), "staging leftovers: {leftovers:?}");
}

/// Runs the generated Relocate script against disposable fixtures. The
/// account gates pass through shims (`id`, `usermod`, `getent`), so no
/// elevation and no real account is ever touched; `mv` is a shim that can
/// fail exactly the publish or restore rename the scenario asks for.
fn run_relocate_script(
    args: [&str; 15],
    payload: &[u8],
    fail_publish_of: &str,
    fail_restore_of: &str,
) -> std::process::Output {
    use std::io::Write as _;
    use std::os::unix::fs::PermissionsExt;

    let shim = tempfile::TempDir::new().unwrap();
    let write_shim = |name: &str, body: &str| {
        let path = shim.path().join(name);
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    };
    write_shim("id", "#!/bin/sh\necho 4242\n");
    write_shim("usermod", "#!/bin/sh\nexit 0\n");
    write_shim("getent", "#!/bin/sh\nexit 0\n");
    write_shim(
        "mv",
        "#!/bin/sh\n\
             if [ \"$1\" = -f ] && [ \"$3\" = \"$MV_FAIL_PUBLISH\" ]; then exit 42; fi\n\
             case \"$2\" in\n\
               */helper-backup|*/antidpi-backup)\n\
                 [ \"$3\" = \"$MV_FAIL_RESTORE\" ] && exit 42 ;;\n\
             esac\n\
             exec /usr/bin/mv \"$@\"\n",
    );
    let path_var = format!(
        "{}:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
        shim.path().display()
    );

    let mut child = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(grant_script(&relocate_with_antidpi()))
        .arg("sh")
        .args(args)
        .env("PATH", path_var)
        .env("MV_FAIL_PUBLISH", fail_publish_of)
        .env("MV_FAIL_RESTORE", fail_restore_of)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(payload).unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn relocate_script_restores_originals_when_the_helper_publish_fails() {
    let Some(fixture) = protected_fixture_dir("helper-publish") else {
        return;
    };
    let root = fixture.0.clone();
    let backend = root.join("xray");
    let backend_identity = root_file(&backend, b"backend");
    let helper_dst = root.join("v2ray-rs-netctl");
    let helper_identity = root_file(&helper_dst, b"OLD-HELPER");
    let antidpi_dst = root.join("v2ray-rs-antidpi");
    let antidpi_identity = root_file(&antidpi_dst, b"OLD-ANTIDPI");

    let helper_payload = b"NEW-HELPER-BYTES".to_vec();
    let antidpi_payload = b"NEW-ANTIDPI-BYTES".to_vec();
    let mut payload = helper_payload.clone();
    payload.extend_from_slice(&antidpi_payload);

    let output = run_relocate_script(
        [
            BACKEND_CAPS,
            backend.to_str().unwrap(),
            root.to_str().unwrap(),
            "root",
            helper_dst.to_str().unwrap(),
            HELPER_CAPS,
            "fixture-user",
            backend_identity.as_str(),
            "4242",
            helper_identity.as_str(),
            antidpi_dst.to_str().unwrap(),
            ANTIDPI_CAPS,
            &helper_payload.len().to_string(),
            &antidpi_payload.len().to_string(),
            antidpi_identity.as_str(),
        ],
        &payload,
        helper_dst.to_str().unwrap(),
        "",
    );

    assert!(
        !output.status.success(),
        "publish failure must exit nonzero"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("destination publish failed"), "{stderr}");
    assert!(!stderr.contains("preserved at"), "{stderr}");
    assert!(output.stdout.is_empty(), "no receipts on failure: {stderr}");
    assert_eq!(std::fs::read(&helper_dst).unwrap(), b"OLD-HELPER".to_vec());
    assert_eq!(identity_of(&helper_dst), helper_identity);
    assert_eq!(
        std::fs::read(&antidpi_dst).unwrap(),
        b"OLD-ANTIDPI".to_vec()
    );
    assert_eq!(identity_of(&antidpi_dst), antidpi_identity);
    assert_no_staging_leftovers(&root);
}

#[test]
fn relocate_script_restores_both_originals_when_the_antidpi_publish_fails() {
    let Some(fixture) = protected_fixture_dir("antidpi-publish-both") else {
        return;
    };
    let root = fixture.0.clone();
    let backend = root.join("xray");
    let backend_identity = root_file(&backend, b"backend");
    let helper_dst = root.join("v2ray-rs-netctl");
    let helper_identity = root_file(&helper_dst, b"OLD-HELPER");
    let antidpi_dst = root.join("v2ray-rs-antidpi");
    let antidpi_identity = root_file(&antidpi_dst, b"OLD-ANTIDPI");

    let helper_payload = b"NEW-HELPER-BYTES".to_vec();
    let antidpi_payload = b"NEW-ANTIDPI-BYTES".to_vec();
    let mut payload = helper_payload.clone();
    payload.extend_from_slice(&antidpi_payload);

    let output = run_relocate_script(
        [
            BACKEND_CAPS,
            backend.to_str().unwrap(),
            root.to_str().unwrap(),
            "root",
            helper_dst.to_str().unwrap(),
            HELPER_CAPS,
            "fixture-user",
            backend_identity.as_str(),
            "4242",
            helper_identity.as_str(),
            antidpi_dst.to_str().unwrap(),
            ANTIDPI_CAPS,
            &helper_payload.len().to_string(),
            &antidpi_payload.len().to_string(),
            antidpi_identity.as_str(),
        ],
        &payload,
        antidpi_dst.to_str().unwrap(),
        "",
    );

    assert!(
        !output.status.success(),
        "publish failure must exit nonzero"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("destination publish failed"), "{stderr}");
    assert!(!stderr.contains("unbound variable"), "{stderr}");
    // Restoration succeeded, so the refusal names no retained rollback.
    assert!(!stderr.contains("preserved at"), "{stderr}");
    assert!(output.stdout.is_empty(), "no receipts on failure: {stderr}");
    // Helper restore ran first: original bytes, original inode.
    assert_eq!(std::fs::read(&helper_dst).unwrap(), b"OLD-HELPER".to_vec());
    assert_eq!(identity_of(&helper_dst), helper_identity);
    // Anti-DPI restore ran too: original bytes, original inode.
    assert_eq!(
        std::fs::read(&antidpi_dst).unwrap(),
        b"OLD-ANTIDPI".to_vec()
    );
    assert_eq!(identity_of(&antidpi_dst), antidpi_identity);
    assert_no_staging_leftovers(&root);
}

#[test]
fn relocate_script_first_install_rollback_leaves_no_new_helper() {
    let Some(fixture) = protected_fixture_dir("antidpi-publish") else {
        return;
    };
    let root = fixture.0.clone();
    let backend = root.join("xray");
    let backend_identity = root_file(&backend, b"backend");
    let helper_dst = root.join("v2ray-rs-netctl");
    let antidpi_dst = root.join("v2ray-rs-antidpi");
    let antidpi_identity = root_file(&antidpi_dst, b"OLD-ANTIDPI");

    let helper_payload = b"NEW-HELPER-BYTES".to_vec();
    let antidpi_payload = b"NEW-ANTIDPI-BYTES".to_vec();
    let mut payload = helper_payload.clone();
    payload.extend_from_slice(&antidpi_payload);

    let output = run_relocate_script(
        [
            BACKEND_CAPS,
            backend.to_str().unwrap(),
            root.to_str().unwrap(),
            "root",
            helper_dst.to_str().unwrap(),
            HELPER_CAPS,
            "fixture-user",
            backend_identity.as_str(),
            "4242",
            "absent",
            antidpi_dst.to_str().unwrap(),
            ANTIDPI_CAPS,
            &helper_payload.len().to_string(),
            &antidpi_payload.len().to_string(),
            antidpi_identity.as_str(),
        ],
        &payload,
        antidpi_dst.to_str().unwrap(),
        "",
    );

    assert!(
        !output.status.success(),
        "publish failure must exit nonzero"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("destination publish failed"), "{stderr}");
    assert!(!stderr.contains("preserved at"), "{stderr}");
    assert!(output.stdout.is_empty(), "no receipts on failure: {stderr}");
    // The route helper had no prior file, so the rollback removes the
    // verified published inode; the anti-DPI original came back.
    assert!(
        !helper_dst.exists(),
        "first-install rollback must remove the published helper"
    );
    assert_eq!(
        std::fs::read(&antidpi_dst).unwrap(),
        b"OLD-ANTIDPI".to_vec()
    );
    assert_eq!(identity_of(&antidpi_dst), antidpi_identity);
    assert_no_staging_leftovers(&root);
}

#[test]
fn relocate_script_reports_a_retained_rollback_when_restore_fails() {
    let Some(fixture) = protected_fixture_dir("restore-fails") else {
        return;
    };
    let root = fixture.0.clone();
    let backend = root.join("xray");
    let backend_identity = root_file(&backend, b"backend");
    let helper_dst = root.join("v2ray-rs-netctl");
    let antidpi_dst = root.join("v2ray-rs-antidpi");
    let antidpi_identity = root_file(&antidpi_dst, b"OLD-ANTIDPI");

    let helper_payload = b"NEW-HELPER-BYTES".to_vec();
    let antidpi_payload = b"NEW-ANTIDPI-BYTES".to_vec();
    let mut payload = helper_payload.clone();
    payload.extend_from_slice(&antidpi_payload);

    let output = run_relocate_script(
        [
            BACKEND_CAPS,
            backend.to_str().unwrap(),
            root.to_str().unwrap(),
            "root",
            helper_dst.to_str().unwrap(),
            HELPER_CAPS,
            "fixture-user",
            backend_identity.as_str(),
            "4242",
            "absent",
            antidpi_dst.to_str().unwrap(),
            ANTIDPI_CAPS,
            &helper_payload.len().to_string(),
            &antidpi_payload.len().to_string(),
            antidpi_identity.as_str(),
        ],
        &payload,
        antidpi_dst.to_str().unwrap(),
        antidpi_dst.to_str().unwrap(),
    );

    assert!(
        !output.status.success(),
        "restore failure must exit nonzero"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("original anti-DPI helper preserved at"),
        "{stderr}"
    );
    let retained = Path::new(stderr.split("preserved at ").nth(1).unwrap().trim());
    assert!(
        retained.is_file(),
        "rollback must be retained: {retained:?}"
    );
    assert_eq!(std::fs::read(retained).unwrap(), b"OLD-ANTIDPI".to_vec());
    assert_eq!(identity_of(retained), antidpi_identity);
    assert!(!antidpi_dst.exists(), "destination must stay empty");
    // The first-installed route helper is rolled back as well: only the
    // verified new inode is removed.
    assert!(
        !helper_dst.exists(),
        "first-install rollback must remove the published helper"
    );
    assert!(output.stdout.is_empty(), "no receipts on failure");
}

#[test]
fn relocate_script_refuses_a_non_root_owned_backend_without_mutating_it() {
    // Skipped under root: there the script would really run and pass.
    if !caps_check_needed(nix::unistd::geteuid().as_raw()) {
        return;
    }
    let dir = tempfile::TempDir::new().unwrap();
    let backend = dir.path().join("xray");
    std::fs::write(&backend, b"backend").unwrap();
    let backend_identity = identity_of(&backend);
    let bytes_before = std::fs::read(&backend).unwrap();

    let payload = b"NEW-HELPER".to_vec();
    let helper_dst = dir.path().join("v2ray-rs-netctl");
    let output = run_relocate_script(
        [
            BACKEND_CAPS,
            backend.to_str().unwrap(),
            dir.path().to_str().unwrap(),
            "root",
            helper_dst.to_str().unwrap(),
            HELPER_CAPS,
            "fixture-user",
            backend_identity.as_str(),
            "4242",
            "absent",
            "",
            "",
            &payload.len().to_string(),
            "",
            "",
        ],
        &payload,
        "",
        "",
    );

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("not root-owned"), "{stderr}");
    assert_eq!(std::fs::read(&backend).unwrap(), bytes_before);
    assert_eq!(identity_of(&backend), backend_identity);
    assert!(!helper_dst.exists());
    assert_no_staging_leftovers(dir.path());
}

#[test]
fn relocate_script_recovers_an_interrupted_first_install() {
    let Some(fixture) = protected_fixture_dir("recover-interrupted") else {
        return;
    };
    use std::os::unix::fs::PermissionsExt;
    let root = fixture.0.clone();
    let backend = root.join("xray");
    let backend_identity = root_file(&backend, b"backend");
    let helper_dst = root.join("v2ray-rs-netctl");
    let antidpi_dst = root.join("v2ray-rs-antidpi");
    let antidpi_identity = root_file(&antidpi_dst, b"OLD-ANTIDPI");
    // A previous publisher died after publishing the route helper of a
    // first install and before finishing: a transaction directory and
    // record are left behind, and the published helper has no prior.
    std::fs::write(&helper_dst, b"INTERRUPTED-HELPER").unwrap();
    std::fs::set_permissions(&helper_dst, std::fs::Permissions::from_mode(0o750)).unwrap();
    let interrupted = identity_of(&helper_dst);
    let txdir = root.join(".v2ray-rs-tx.deadbeef");
    std::fs::create_dir(&txdir).unwrap();
    std::fs::write(
            root.join(".v2ray-rs-grant-state"),
            format!(
                "tx={}\nhelper-dst={}\nhelper-prior=absent\nhelper-new={}\nhelper-len=18\nhelper-mode=750\n",
                txdir.display(),
                helper_dst.display(),
                interrupted
            ),
        )
        .unwrap();

    let helper_payload = b"NEW-HELPER-BYTES".to_vec();
    let antidpi_payload = b"NEW-ANTIDPI-BYTES".to_vec();
    let mut payload = helper_payload.clone();
    payload.extend_from_slice(&antidpi_payload);

    let output = run_relocate_script(
        [
            BACKEND_CAPS,
            backend.to_str().unwrap(),
            root.to_str().unwrap(),
            "root",
            helper_dst.to_str().unwrap(),
            HELPER_CAPS,
            "fixture-user",
            backend_identity.as_str(),
            "4242",
            "absent",
            antidpi_dst.to_str().unwrap(),
            ANTIDPI_CAPS,
            &helper_payload.len().to_string(),
            &antidpi_payload.len().to_string(),
            antidpi_identity.as_str(),
        ],
        &payload,
        "",
        "",
    );

    assert!(
        output.status.success(),
        "recovery must let a fresh grant proceed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    // The interrupted helper was removed under the record before this
    // grant published its own bytes on a fresh inode.
    assert_eq!(std::fs::read(&helper_dst).unwrap(), helper_payload);
    assert_ne!(identity_of(&helper_dst), interrupted);
    assert_eq!(
        std::fs::read(&antidpi_dst).unwrap(),
        b"OLD-ANTIDPI".to_vec()
    );
    assert_eq!(identity_of(&antidpi_dst), antidpi_identity);
    assert!(!root.join(".v2ray-rs-grant-state").exists());
    assert_no_staging_leftovers(&root);
}

#[test]
fn stream_refuses_a_source_that_shortened_after_validation() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("helper");
    std::fs::write(&path, b"0123456789").unwrap();
    let file = std::fs::File::open(&path).unwrap();
    let meta = file.metadata().unwrap();
    let source = PinnedSource {
        path: path.clone(),
        identity: format!("{}:{}", meta.dev(), meta.ino()),
        len: meta.len(),
        file,
    };
    // The same inode rewritten behind the validated handle: the pinned
    // length no longer matches what the handle can deliver, so the
    // stream is refused instead of publishing a different binary.
    std::fs::write(&path, b"short").unwrap();
    let mut sink = Vec::new();
    let err = stream_pinned(&mut sink, &source).expect_err("a changed source must be refused");
    assert!(err.to_string().contains("streamed 5 of 10 bytes"), "{err}");
}

#[test]
fn stream_kills_and_reaps_the_child_on_a_short_source() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("helper");
    std::fs::write(&path, vec![b'x'; 5]).unwrap();
    let file = std::fs::File::open(&path).unwrap();
    let meta = file.metadata().unwrap();
    let source = PinnedSource {
        path,
        identity: format!("{}:{}", meta.dev(), meta.ino()),
        len: meta.len() + 5,
        file,
    };
    let plan = GrantPlan::Relocate {
        backend: target("/usr/bin/xray", "8:1"),
        dest_dir: PathBuf::from("/usr/local/lib/v2ray-rs"),
        src: source,
        dst: PathBuf::from("/usr/local/lib/v2ray-rs/v2ray-rs-netctl"),
        prior_dst: "absent".into(),
        antidpi: None,
        user: "zhuk".into(),
        uid: 1000,
    };
    let mut child = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg("sleep 30")
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let err = stream_payloads(&mut child, &plan).expect_err("a short source must fail the grant");
    assert!(err.to_string().contains("streamed 5 of 10 bytes"), "{err}");
    // The elevated child was terminated and reaped, not left behind.
    assert!(!child.wait().unwrap().success());
}

#[test]
fn stream_kills_and_reaps_the_child_on_a_closed_pipe() {
    // Larger than the pipe buffer, so the write to a child that has
    // already exited must surface EPIPE instead of fitting the buffer.
    let plan = {
        let mut plan = relocate_with_antidpi();
        if let GrantPlan::Relocate { src, .. } = &mut plan {
            *src = pinned(300_000);
        }
        plan
    };
    let mut child = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg("exit 0")
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let err =
        stream_payloads(&mut child, &plan).expect_err("a closed stdin pipe must fail the grant");
    let text = err.to_string();
    assert!(
        text.contains("cannot stream payload") || text.contains("streamed "),
        "{text}"
    );
    // The elevated child is gone either way: the error path already
    // attempted terminate-and-reap (proven against a live child by the
    // short-source test), and this wait reaps whatever remains. Nothing
    // survives holding the payload pipe.
    let _ = child.wait();
}

#[test]
fn relocation_source_refuses_a_writable_bundle() {
    let dir = tempfile::TempDir::new().unwrap();
    let src = dir.path().join("v2ray-rs-netctl");
    std::fs::write(&src, b"helper").unwrap();
    let canonical = std::fs::canonicalize(&src).unwrap();
    let err = source_bundle_trusted(&canonical)
        .expect_err("a writable, caller-owned directory is not a trusted bundle");
    let text = err.to_string();
    assert!(
        text.contains("parent directory is not protected") || text.contains("cannot verify"),
        "{text}"
    );
}

#[test]
fn relocation_source_accepts_a_protected_root_owned_installation() {
    if !running_as_root() || !var_lib_protected() {
        return;
    }
    let dir =
        PathBuf::from("/var/lib").join(format!("v2ray-rs-src-fixture-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let src = dir.join("v2ray-rs-netctl");
    std::fs::write(&src, b"helper").unwrap();
    let canonical = std::fs::canonicalize(&src).unwrap();
    let result = source_bundle_trusted(&canonical);
    let _ = std::fs::remove_dir_all(&dir);
    result.expect("a protected root-owned installation is a trusted bundle");
}

#[test]
fn readonly_mount_option_is_detected_exactly() {
    assert!(readonly_option("ro,nosuid,nodev"));
    assert!(readonly_option("rw,relatime,ro"));
    assert!(!readonly_option("rw,nosuid"));
    assert!(!readonly_option(""));
}
