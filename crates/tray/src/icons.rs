use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use ksni::Icon;
use tempfile::TempDir;

const SVG_DISCONNECTED: &str = include_str!("../icons/v2ray-rs-disconnected-symbolic.svg");
const SVG_CONNECTED: &str = include_str!("../icons/v2ray-rs-connected-symbolic.svg");
const SVG_ERROR: &str = include_str!("../icons/v2ray-rs-error-symbolic.svg");

const ICON_SIZE: u32 = 22;

const INDEX_THEME: &str = "\
[Icon Theme]
Name=v2ray-rs
Directories=scalable/status

[scalable/status]
Size=16
MinSize=8
MaxSize=512
Type=Scalable
";

const STATUS_ICON_DIR: &str = "icons/hicolor/symbolic/apps";
const HICOLOR_DIR: &str = "icons/hicolor";
const HICOLOR_INDEX: &str = "icons/hicolor/index.theme";

fn data_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
}

pub fn install_icons() -> bool {
    let Some(data_dir) = data_dir() else { return false };
    ensure_hicolor_index(&data_dir);
    let status_dir = data_dir.join(STATUS_ICON_DIR);
    if fs::create_dir_all(&status_dir).is_err() {
        return false;
    }

    let mut success = true;
    success &= write_if_missing(
        &status_dir,
        "v2ray-rs-disconnected-symbolic.svg",
        SVG_DISCONNECTED,
    );
    success &= write_if_missing(
        &status_dir,
        "v2ray-rs-connected-symbolic.svg",
        SVG_CONNECTED,
    );
    success &= write_if_missing(
        &status_dir,
        "v2ray-rs-error-symbolic.svg",
        SVG_ERROR,
    );

    if success {
        let theme_dir = data_dir.join(HICOLOR_DIR);
        let _ = Command::new("gtk-update-icon-cache")
            .arg("-f")
            .arg("-t")
            .arg(&theme_dir)
            .status();
    }

    success
}

fn ensure_hicolor_index(data_dir: &Path) {
    let index_path = data_dir.join(HICOLOR_INDEX);
    if index_path.exists() {
        return;
    }

    let system_index = Path::new("/usr/share/icons/hicolor/index.theme");
    let contents = fs::read_to_string(system_index).unwrap_or_else(|_| {
        "[Icon Theme]\nName=hicolor\nDirectories=symbolic/apps,scalable/apps,scalable/status\n\n".into()
    });

    if let Some(parent) = index_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(index_path, contents);
}

pub fn setup_icon_theme() -> Option<TempDir> {
    let dir = TempDir::new().ok()?;
    let status_dir = dir.path().join("hicolor/scalable/status");
    fs::create_dir_all(&status_dir).ok()?;

    fs::write(dir.path().join("hicolor/index.theme"), INDEX_THEME).ok()?;
    write_svg(&status_dir, "v2ray-rs-disconnected-symbolic.svg", SVG_DISCONNECTED)?;
    write_svg(&status_dir, "v2ray-rs-connected-symbolic.svg", SVG_CONNECTED)?;
    write_svg(&status_dir, "v2ray-rs-error-symbolic.svg", SVG_ERROR)?;

    Some(dir)
}

fn write_svg(dir: &Path, name: &str, content: &str) -> Option<()> {
    fs::write(dir.join(name), content).ok()
}

fn write_if_missing(dir: &Path, name: &str, content: &str) -> bool {
    let path = dir.join(name);
    if path.exists() {
        return true;
    }
    fs::write(path, content).is_ok()
}

fn render_svg(svg_str: &str) -> Option<Icon> {
    let svg = svg_str.replace("currentColor", "#DEDDDA");

    let opts = resvg::usvg::Options::default();
    let tree = resvg::usvg::Tree::from_str(&svg, &opts).ok()?;

    let size = resvg::tiny_skia::IntSize::from_wh(ICON_SIZE, ICON_SIZE)?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size.width(), size.height())?;

    let sx = size.width() as f32 / tree.size().width();
    let sy = size.height() as f32 / tree.size().height();
    let transform = resvg::tiny_skia::Transform::from_scale(sx, sy);

    resvg::render(&tree, transform, &mut pixmap.as_mut());

    let rgba = pixmap.data();
    let mut argb = Vec::with_capacity(rgba.len());
    for chunk in rgba.chunks_exact(4) {
        argb.push(chunk[3]); // A
        argb.push(chunk[0]); // R
        argb.push(chunk[1]); // G
        argb.push(chunk[2]); // B
    }

    Some(Icon {
        width: size.width() as i32,
        height: size.height() as i32,
        data: argb,
    })
}

pub fn disconnected_pixmap() -> Vec<Icon> {
    render_svg(SVG_DISCONNECTED).into_iter().collect()
}

pub fn connected_pixmap() -> Vec<Icon> {
    render_svg(SVG_CONNECTED).into_iter().collect()
}

pub fn error_pixmap() -> Vec<Icon> {
    render_svg(SVG_ERROR).into_iter().collect()
}
