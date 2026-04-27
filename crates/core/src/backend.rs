use std::fmt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use thiserror::Error;

use crate::models::BackendType;

#[derive(Error, Debug)]
pub enum BackendError {
    #[error(
        "no supported backend found.\n\nInstall one of the following:\n  v2ray:    sudo pacman -S v2ray       (Arch) | sudo apt install v2ray (Debian/Ubuntu)\n  xray:     https://github.com/XTLS/Xray-core/releases\n  sing-box: sudo pacman -S sing-box    (Arch) | https://github.com/SagerNet/sing-box/releases"
    )]
    NoneFound,
    #[error("multiple supported backends found; choose one explicitly")]
    MultipleAvailable,
    #[error("binary not found at {path}")]
    NotFound { path: PathBuf },
    #[error("binary at {path} is not executable")]
    NotExecutable { path: PathBuf },
    #[error("failed to run {path}: {reason}")]
    ExecutionFailed { path: PathBuf, reason: String },
    #[error("failed to detect version for {path}: {reason}")]
    VersionDetectionFailed { path: PathBuf, reason: String },
}

#[derive(Debug, Clone)]
pub struct DetectedBackend {
    pub backend_type: BackendType,
    pub binary_path: PathBuf,
    pub version: Option<String>,
    pub version_error: Option<String>,
}

impl fmt::Display for DetectedBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = backend_name(self.backend_type);
        match (&self.version, &self.version_error) {
            (Some(v), _) => write!(f, "{name} ({v}) at {}", self.binary_path.display()),
            (None, Some(err)) => {
                write!(
                    f,
                    "{name} unavailable at {}: {err}",
                    self.binary_path.display()
                )
            }
            (None, None) => write!(f, "{name} at {}", self.binary_path.display()),
        }
    }
}

impl DetectedBackend {
    pub fn is_available(&self) -> bool {
        self.version_error.is_none()
    }
}

pub fn backend_name(bt: BackendType) -> &'static str {
    match bt {
        BackendType::V2ray => "v2ray",
        BackendType::Xray => "xray",
        BackendType::SingBox => "sing-box",
    }
}

fn well_known_paths(bt: BackendType) -> [PathBuf; 2] {
    let name = backend_name(bt);
    [
        PathBuf::from(format!("/usr/bin/{name}")),
        PathBuf::from(format!("/usr/local/bin/{name}")),
    ]
}

fn is_executable(path: &Path) -> bool {
    path.metadata()
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var("PATH").ok()?;
    path_var.split(':').find_map(|dir| {
        let candidate = PathBuf::from(dir).join(name);
        is_executable(&candidate).then_some(candidate)
    })
}

fn detect_version(path: &Path) -> Result<String, BackendError> {
    const MAX_RETRIES: u32 = 5;
    let output = {
        let mut last_busy_error = None;
        let mut command_result = None;
        for attempt in 0..MAX_RETRIES {
            match Command::new(path).arg("version").output() {
                Ok(output) => {
                    command_result = Some(output);
                    break;
                }
                Err(err) if err.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                    last_busy_error = Some(err);
                    if attempt + 1 < MAX_RETRIES {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                }
                Err(err) => {
                    return Err(BackendError::ExecutionFailed {
                        path: path.to_path_buf(),
                        reason: err.to_string(),
                    });
                }
            }
        }
        match command_result {
            Some(output) => output,
            None => {
                let err = last_busy_error.unwrap_or_else(|| {
                    std::io::Error::other("version detection failed without output")
                });
                return Err(BackendError::ExecutionFailed {
                    path: path.to_path_buf(),
                    reason: err.to_string(),
                });
            }
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(BackendError::VersionDetectionFailed {
            path: path.to_path_buf(),
            reason: if stderr.is_empty() {
                format!("exit code {}", output.status)
            } else {
                stderr
            },
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() {
        return Err(BackendError::VersionDetectionFailed {
            path: path.to_path_buf(),
            reason: "empty version output".into(),
        });
    }

    Ok(stdout.lines().next().unwrap_or(&stdout).to_string())
}

fn version_error_message(err: BackendError) -> String {
    match err {
        BackendError::ExecutionFailed { reason, .. }
        | BackendError::VersionDetectionFailed { reason, .. } => reason,
        other => other.to_string(),
    }
}

fn detected_from_path(bt: BackendType, path: PathBuf) -> DetectedBackend {
    match detect_version(&path) {
        Ok(version) => DetectedBackend {
            backend_type: bt,
            binary_path: path,
            version: Some(version),
            version_error: None,
        },
        Err(err) => DetectedBackend {
            backend_type: bt,
            binary_path: path,
            version: None,
            version_error: Some(version_error_message(err)),
        },
    }
}

fn detect_binary(bt: BackendType) -> Option<PathBuf> {
    for path in well_known_paths(bt) {
        if path.exists() && is_executable(&path) {
            return Some(path);
        }
    }
    find_in_path(backend_name(bt))
}

pub fn detect_single(bt: BackendType) -> Result<DetectedBackend, BackendError> {
    let path = detect_binary(bt).ok_or(BackendError::NotFound {
        path: PathBuf::from(backend_name(bt)),
    })?;
    Ok(detected_from_path(bt, path))
}

pub fn detect_all() -> Vec<DetectedBackend> {
    [BackendType::V2ray, BackendType::Xray, BackendType::SingBox]
        .into_iter()
        .filter_map(|bt| detect_single(bt).ok())
        .collect()
}

pub fn auto_select() -> Result<DetectedBackend, BackendError> {
    let mut available: Vec<_> = detect_all()
        .into_iter()
        .filter(DetectedBackend::is_available)
        .collect();
    match available.len() {
        0 => Err(BackendError::NoneFound),
        1 => Ok(available.remove(0)),
        _ => Err(BackendError::MultipleAvailable),
    }
}

pub fn detect_all_or_error() -> Result<Vec<DetectedBackend>, BackendError> {
    let available = detect_all();
    if available.is_empty() {
        Err(BackendError::NoneFound)
    } else {
        Ok(available)
    }
}

pub fn validate_custom_path(path: &Path, bt: BackendType) -> Result<DetectedBackend, BackendError> {
    if !path.exists() {
        return Err(BackendError::NotFound {
            path: path.to_path_buf(),
        });
    }
    if !is_executable(path) {
        return Err(BackendError::NotExecutable {
            path: path.to_path_buf(),
        });
    }
    let version = detect_version(path)?;
    Ok(DetectedBackend {
        backend_type: bt,
        binary_path: path.to_path_buf(),
        version: Some(version),
        version_error: None,
    })
}

pub fn install_guidance(bt: BackendType) -> &'static str {
    match bt {
        BackendType::V2ray => {
            "Install v2ray:\n\
             \x20 Arch Linux: sudo pacman -S v2ray\n\
             \x20 Debian/Ubuntu: sudo apt install v2ray\n\
             \x20 Manual: https://github.com/v2fly/v2ray-core/releases"
        }
        BackendType::Xray => {
            "Install xray:\n\
             \x20 Arch Linux: yay -S xray (AUR)\n\
             \x20 Script: bash -c \"$(curl -L https://github.com/XTLS/Xray-install/raw/main/install-release.sh)\"\n\
             \x20 Manual: https://github.com/XTLS/Xray-core/releases"
        }
        BackendType::SingBox => {
            "Install sing-box:\n\
             \x20 Arch Linux: sudo pacman -S sing-box\n\
             \x20 Manual: https://github.com/SagerNet/sing-box/releases"
        }
    }
}

pub fn all_install_guidance() -> String {
    [BackendType::V2ray, BackendType::Xray, BackendType::SingBox]
        .into_iter()
        .map(install_guidance)
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_backend_name() {
        assert_eq!(backend_name(BackendType::V2ray), "v2ray");
        assert_eq!(backend_name(BackendType::Xray), "xray");
        assert_eq!(backend_name(BackendType::SingBox), "sing-box");
    }

    #[test]
    fn test_well_known_paths() {
        let paths = well_known_paths(BackendType::V2ray);
        assert_eq!(paths[0], PathBuf::from("/usr/bin/v2ray"));
        assert_eq!(paths[1], PathBuf::from("/usr/local/bin/v2ray"));

        let paths = well_known_paths(BackendType::SingBox);
        assert_eq!(paths[0], PathBuf::from("/usr/bin/sing-box"));
        assert_eq!(paths[1], PathBuf::from("/usr/local/bin/sing-box"));
    }

    #[test]
    fn test_is_executable() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path();
        fs::set_permissions(path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!is_executable(path));

        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(is_executable(path));
    }

    #[test]
    fn test_validate_custom_path_not_found() {
        let result = validate_custom_path(Path::new("/nonexistent/binary"), BackendType::Xray);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, BackendError::NotFound { .. }));
    }

    #[test]
    fn test_validate_custom_path_not_executable() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path();
        fs::set_permissions(path, fs::Permissions::from_mode(0o644)).unwrap();

        let result = validate_custom_path(path, BackendType::Xray);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, BackendError::NotExecutable { .. }));
    }

    #[test]
    fn test_validate_custom_path_executable_but_not_real_binary() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();

        let result = validate_custom_path(path, BackendType::Xray);
        assert!(result.is_err());
    }

    #[test]
    fn test_detect_single_nonexistent() {
        let _ = detect_single(BackendType::V2ray);
    }

    #[test]
    fn test_detect_all_no_panic() {
        let _ = detect_all();
    }

    #[test]
    fn test_auto_select_no_panic() {
        let _ = auto_select();
    }

    #[test]
    fn test_install_guidance_not_empty() {
        assert!(!install_guidance(BackendType::V2ray).is_empty());
        assert!(!install_guidance(BackendType::Xray).is_empty());
        assert!(!install_guidance(BackendType::SingBox).is_empty());
    }

    #[test]
    fn test_all_install_guidance_contains_all_backends() {
        let guidance = all_install_guidance();
        assert!(guidance.contains("v2ray"));
        assert!(guidance.contains("xray"));
        assert!(guidance.contains("sing-box"));
    }

    #[test]
    fn test_error_display_none_found() {
        let err = BackendError::NoneFound;
        let msg = err.to_string();
        assert!(msg.contains("no supported backend found"));
        assert!(msg.contains("v2ray"));
        assert!(msg.contains("xray"));
        assert!(msg.contains("sing-box"));
    }

    #[test]
    fn test_error_display_not_found() {
        let err = BackendError::NotFound {
            path: PathBuf::from("/usr/bin/v2ray"),
        };
        assert!(err.to_string().contains("/usr/bin/v2ray"));
    }

    #[test]
    fn test_error_display_not_executable() {
        let err = BackendError::NotExecutable {
            path: PathBuf::from("/usr/bin/v2ray"),
        };
        assert!(err.to_string().contains("not executable"));
    }

    #[test]
    fn test_detected_backend_display() {
        let d = DetectedBackend {
            backend_type: BackendType::Xray,
            binary_path: PathBuf::from("/usr/bin/xray"),
            version: Some("Xray 1.8.4".into()),
            version_error: None,
        };
        let s = d.to_string();
        assert!(s.contains("xray"));
        assert!(s.contains("1.8.4"));
        assert!(s.contains("/usr/bin/xray"));
    }

    #[test]
    fn test_detected_backend_display_no_version() {
        let d = DetectedBackend {
            backend_type: BackendType::SingBox,
            binary_path: PathBuf::from("/usr/local/bin/sing-box"),
            version: None,
            version_error: None,
        };
        let s = d.to_string();
        assert!(s.contains("sing-box"));
        assert!(s.contains("/usr/local/bin/sing-box"));
    }

    #[test]
    fn test_detected_backend_display_unavailable() {
        let d = DetectedBackend {
            backend_type: BackendType::V2ray,
            binary_path: PathBuf::from("/usr/bin/v2ray"),
            version: None,
            version_error: Some("permission denied".into()),
        };
        let s = d.to_string();
        assert!(s.contains("unavailable"));
        assert!(s.contains("permission denied"));
    }

    #[test]
    fn test_mock_script_detection() {
        let dir = tempfile::TempDir::new().unwrap();
        let script_path = dir.path().join("test-backend");
        fs::write(&script_path, "#!/bin/sh\necho \"TestBackend v1.0.0\"\n").unwrap();
        fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).unwrap();

        let result = validate_custom_path(&script_path, BackendType::V2ray);
        let detected = result.unwrap();
        assert_eq!(detected.backend_type, BackendType::V2ray);
        assert_eq!(detected.binary_path, script_path);
        assert_eq!(detected.version.as_deref(), Some("TestBackend v1.0.0"));
        assert!(detected.version_error.is_none());
    }

    #[test]
    fn test_mock_script_version_failure() {
        let dir = tempfile::TempDir::new().unwrap();
        let script_path = dir.path().join("bad-backend");
        fs::write(&script_path, "#!/bin/sh\nexit 1\n").unwrap();
        fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).unwrap();

        let result = validate_custom_path(&script_path, BackendType::Xray);
        assert!(matches!(
            result,
            Err(BackendError::VersionDetectionFailed { .. })
        ));
    }
}
