use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

use ksni::Icon;

const ICON_DISCONNECTED: &[u8] = include_bytes!("../icons/tray-disconnected.png");
const ICON_CONNECTED: &[u8] = include_bytes!("../icons/tray-connected.png");
const ICON_ERROR: &[u8] = include_bytes!("../icons/tray-error.png");

const SVG_DISCONNECTED: &[u8] = include_bytes!("../icons/v2ray-rs-disconnected-symbolic.svg");
const SVG_CONNECTED: &[u8] = include_bytes!("../icons/v2ray-rs-connected-symbolic.svg");
const SVG_ERROR: &[u8] = include_bytes!("../icons/v2ray-rs-error-symbolic.svg");

static ICONS_INSTALLED: OnceLock<bool> = OnceLock::new();

#[allow(dead_code)]
pub const ICON_NAME_CONNECTED: &str = "v2ray-rs-connected-symbolic";
#[allow(dead_code)]
pub const ICON_NAME_DISCONNECTED: &str = "v2ray-rs-disconnected-symbolic";
#[allow(dead_code)]
pub const ICON_NAME_ERROR: &str = "v2ray-rs-error-symbolic";

fn hicolor_status_dir() -> PathBuf {
    let data_home = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
            PathBuf::from(home).join(".local/share")
        });
    data_home
        .join("icons")
        .join("hicolor")
        .join("scalable")
        .join("status")
}

pub fn install_icons() {
    ICONS_INSTALLED.get_or_init(|| {
        let icons_dir = hicolor_status_dir();
        if let Err(e) = fs::create_dir_all(&icons_dir) {
            eprintln!("tray: create icon dir: {e}");
            return false;
        }

        for (name, data) in [
            ("v2ray-rs-disconnected-symbolic.svg", SVG_DISCONNECTED),
            ("v2ray-rs-connected-symbolic.svg", SVG_CONNECTED),
            ("v2ray-rs-error-symbolic.svg", SVG_ERROR),
        ] {
            if let Err(e) = fs::write(icons_dir.join(name), data) {
                eprintln!("tray: write {name}: {e}");
            }
        }

        let hicolor_root = icons_dir
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent());
        if let Some(root) = hicolor_root {
            let _ = std::process::Command::new("gtk-update-icon-cache")
                .arg("-f")
                .arg("-t")
                .arg(root)
                .status();
        }

        true
    });
}

pub fn disconnected() -> Vec<Icon> {
    vec![png_to_icon(ICON_DISCONNECTED)]
}

pub fn connected() -> Vec<Icon> {
    vec![png_to_icon(ICON_CONNECTED)]
}

pub fn error() -> Vec<Icon> {
    vec![png_to_icon(ICON_ERROR)]
}

fn png_to_icon(png_data: &[u8]) -> Icon {
    let mut decoder = png::Decoder::new(png_data);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::ALPHA);
    let mut reader = decoder.read_info().expect("valid png");
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).expect("valid frame");
    buf.truncate(info.buffer_size());

    rgba_to_argb(&mut buf);

    Icon {
        width: info.width as i32,
        height: info.height as i32,
        data: buf,
    }
}

fn rgba_to_argb(data: &mut [u8]) {
    for pixel in data.chunks_exact_mut(4) {
        pixel.rotate_right(1);
    }
}
