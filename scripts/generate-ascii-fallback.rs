// Run: cargo run --release --bin generate-ascii-fallback -- 24
// Writes assets/flarion-mark.ascii.txt with a monochrome mark rendered at the
// requested width (in terminal cells; height is derived from aspect ratio,
// doubled for half-block density).
fn main() {
    let arg = std::env::args().nth(1).expect("usage: generate-ascii-fallback <cells-wide>");
    let cells_wide: u32 = arg.parse().expect("cells-wide u32");

    let svg = std::fs::read("assets/flarion-mark.svg").expect("read svg");
    let opt = resvg::usvg::Options::default();
    let tree = resvg::usvg::Tree::from_data(&svg, &opt).expect("parse svg");
    let view = tree.size();
    let aspect = view.height() / view.width();
    let pw = cells_wide;
    let ph = (pw as f32 * aspect).round() as u32 * 2;
    let mut pm = tiny_skia::Pixmap::new(pw, ph).expect("pixmap");
    resvg::render(&tree, tiny_skia::Transform::from_scale(
        pw as f32 / view.width(), ph as f32 / view.height(),
    ), &mut pm.as_mut());

    let mut out = String::new();
    for y in (0..ph).step_by(2) {
        for x in 0..pw {
            let i = ((y * pw + x) * 4 + 3) as usize;
            let filled = pm.data().get(i).copied().unwrap_or(0) >= 64;
            out.push(if filled { '#' } else { ' ' });
        }
        out.push('\n');
    }
    std::fs::write("assets/flarion-mark.ascii.txt", out).expect("write");
    eprintln!("wrote assets/flarion-mark.ascii.txt ({cells_wide} cells wide)");
}
