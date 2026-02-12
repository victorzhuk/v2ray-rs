use ksni::Icon;

const SVG_DISCONNECTED: &str = include_str!("../icons/v2ray-rs-disconnected-symbolic.svg");
const SVG_CONNECTED: &str = include_str!("../icons/v2ray-rs-connected-symbolic.svg");
const SVG_ERROR: &str = include_str!("../icons/v2ray-rs-error-symbolic.svg");

const ICON_SIZE: u32 = 22;

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
