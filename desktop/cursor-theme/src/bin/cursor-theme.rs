//! Generates the cursor themes from the SVGs of `assets/cursors/`, and the board that shows
//! them.
//!
//! - `cursor-theme build <svg folder> <output folder>` writes `<output>/nom-os/` and
//!   `<output>/nom-os-light/`: `index.theme`, one Xcursor file per shape in `cursors/`, with an
//!   image per size of `SIZES` (a turn of images per size for the waiting cursors), and a link
//!   per alias.
//! - `cursor-theme board <svg folder> <output.png> [names] [sizes]` draws the shapes `names`
//!   (comma separated, every shape by default) at the `sizes` (24,48,72 by default), the dark
//!   theme on the dark desktop and the light one on the light desktop, with each 24 px drawing
//!   enlarged five times.

use std::{error::Error, fs, io::Write as _, path::Path};

use cursor_theme::{
    Drawing, GRID, SHAPES, SIZES, SPIN_FRAMES, Shape, Variant, canvas, hex, index_theme, margin,
    spin_delay,
    xcursor::{self, Image},
};
use resvg::{
    tiny_skia::{FilterQuality, Pixmap, PixmapPaint, Transform},
    usvg,
};
use sherlock_ui::color::Palette;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["build", svgs, output] => build(Path::new(svgs), Path::new(output)),
        ["board", svgs, output, rest @ ..] if rest.len() <= 2 => {
            let shapes = match rest.first() {
                Some(names) => names
                    .split(',')
                    .map(|n| cursor_theme::shape(n).ok_or(format!("unknown shape {n}")))
                    .collect::<std::result::Result<Vec<_>, _>>()?,
                None => SHAPES.iter().collect(),
            };
            let sizes = match rest.get(1) {
                Some(sizes) => sizes
                    .split(',')
                    .map(str::parse)
                    .collect::<std::result::Result<Vec<u32>, _>>()?,
                None => vec![24, 48, 72],
            };
            board(Path::new(svgs), &shapes, &sizes)?.save_png(output)?;
            Ok(())
        }
        _ => Err(
            "usage: cursor-theme build <svg folder> <output folder>\n       \
                  cursor-theme board <svg folder> <output.png> [names] [sizes]"
                .into(),
        ),
    }
}

/// The SVG of `shape` and what it says of itself.
fn source(svgs: &Path, shape: &Shape) -> Result<(String, Drawing)> {
    let path = svgs.join(format!("{}.svg", shape.name));
    let svg = fs::read_to_string(&path).map_err(|err| format!("{}: {err}", path.display()))?;
    let drawing = Drawing::read(&svg).map_err(|err| format!("{}: {err}", path.display()))?;
    Ok((svg, drawing))
}

/// `svg` in `variant`, turned by `angle`, rendered for the nominal `size`. Fails if the
/// drawing goes past the image: it would be cut.
fn render(
    (svg, drawing): &(String, Drawing),
    variant: Variant,
    angle: f32,
    size: u32,
) -> Result<Pixmap> {
    let tree = usvg::Tree::from_str(
        &variant.paint(svg, drawing, angle),
        &usvg::Options::default(),
    )?;
    let side = canvas(size);
    let mut pixmap = Pixmap::new(side, side).ok_or("empty image")?;
    let scale = size as f32 / GRID as f32;
    let at = margin(size) as f32;
    let place = Transform::from_row(scale, 0.0, 0.0, scale, at, at);
    resvg::render(&tree, place, &mut pixmap.as_mut());
    let alpha = |x: u32, y: u32| pixmap.pixel(x, y).map_or(0, |p| u32::from(p.alpha()));
    let last = side - 1;
    if (0..side).any(|i| alpha(i, 0) + alpha(i, last) + alpha(0, i) + alpha(last, i) > 0) {
        return Err(format!("the drawing touches the edge of its {side} px image").into());
    }
    Ok(pixmap)
}

/// Every image of the file of a shape: per size, one image, or a turn of images.
fn images(source: &(String, Drawing), variant: Variant) -> Result<Vec<Image>> {
    let delay = spin_delay().as_millis() as u32;
    let turn: Vec<(f32, u32)> = match source.1.spin {
        Some(_) => (0..SPIN_FRAMES)
            .map(|i| (360.0 * i as f32 / SPIN_FRAMES as f32, delay))
            .collect(),
        None => vec![(0.0, 0)],
    };
    let mut images = Vec::new();
    for size in SIZES {
        for &(angle, delay) in &turn {
            let pixmap = render(source, variant, angle, size)?;
            let pixels = pixmap
                .data()
                .chunks_exact(4)
                .map(|p| u32::from_be_bytes([p[3], p[0], p[1], p[2]]))
                .collect();
            images.push(Image {
                size,
                width: pixmap.width(),
                height: pixmap.height(),
                hotspot: source.1.hotspot_at(size),
                delay,
                pixels,
            });
        }
    }
    Ok(images)
}

fn build(svgs: &Path, output: &Path) -> Result<()> {
    let mut out = std::io::stdout().lock();
    for variant in Variant::ALL {
        let theme = output.join(variant.theme());
        let cursors = theme.join("cursors");
        if cursors.exists() {
            fs::remove_dir_all(&cursors)?;
        }
        fs::create_dir_all(&cursors)?;
        fs::write(theme.join("index.theme"), index_theme(variant))?;
        for shape in SHAPES {
            let source = source(svgs, shape)?;
            let images =
                images(&source, variant).map_err(|err| format!("{}: {err}", shape.name))?;
            fs::write(cursors.join(shape.name), xcursor::encode(&images))?;
            for alias in shape.aliases {
                std::os::unix::fs::symlink(shape.name, cursors.join(alias))?;
            }
        }
        writeln!(out, "{}: {} shapes", theme.display(), SHAPES.len())?;
    }
    Ok(())
}

/// Space around each drawing on the board, and the enlargement of the 24 px drawings.
const PAD: u32 = 16;
const ZOOM: u32 = 5;
const LABEL: u32 = 120;
const HEADER: u32 = 56;

/// Width of the board's column for `size`: room for the drawing and for its label.
fn cell(size: u32) -> u32 {
    (canvas(size) + PAD).max(72)
}

/// The board: a panel per desktop theme with its cursors, a row per shape, a column per size,
/// and the 24 px drawing enlarged.
fn board(svgs: &Path, shapes: &[&Shape], sizes: &[u32]) -> Result<Pixmap> {
    let zoom = canvas(GRID) * ZOOM;
    let row = sizes
        .iter()
        .map(|&s| canvas(s))
        .max()
        .unwrap_or(0)
        .max(zoom)
        + PAD;
    let panel_w = LABEL + sizes.iter().map(|&s| cell(s)).sum::<u32>() + zoom + 2 * PAD;
    let height = HEADER + row * shapes.len() as u32 + PAD;
    let width = 2 * panel_w;

    // Backgrounds, discs and labels, drawn as an SVG.
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" \
         font-family=\"Inter, DejaVu Sans, sans-serif\" font-size=\"13\">"
    );
    for (i, palette) in [Palette::DARK, Palette::LIGHT].iter().enumerate() {
        let x0 = i as u32 * panel_w;
        let text = hex(palette.text_secondary);
        svg += &format!(
            "<svg x=\"{x0}\" width=\"{panel_w}\" height=\"{height}\"><rect width=\"100%\" \
             height=\"100%\" fill=\"{}\"/>",
            hex(palette.desktop)
        );
        for (disc, (cx, cy, r)) in palette.desktop_discs.iter().zip([
            (-0.2, 1.1, 0.75),
            (0.9, -0.3, 0.6),
            (1.0, 0.85, 0.4),
        ]) {
            let (w, h) = (panel_w as f32, height as f32);
            svg += &format!(
                "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"{}\"/>",
                cx * w,
                cy * h,
                r * w.max(h),
                hex(*disc)
            );
        }
        let mut x = LABEL;
        for size in sizes {
            svg += &format!(
                "<text x=\"{}\" y=\"34\" fill=\"{text}\" text-anchor=\"middle\">{size}</text>",
                x + cell(*size) / 2
            );
            x += cell(*size);
        }
        svg += &format!(
            "<text x=\"{}\" y=\"34\" fill=\"{text}\" text-anchor=\"middle\">24 × {ZOOM}</text>",
            x + PAD + zoom / 2
        );
        for (r, shape) in shapes.iter().enumerate() {
            svg += &format!(
                "<text x=\"{PAD}\" y=\"{}\" fill=\"{text}\">{}</text>",
                HEADER + r as u32 * row + row / 2 + 4,
                shape.name
            );
        }
        svg += "</svg>";
    }
    svg += "</svg>";
    let mut options = usvg::Options::default();
    options.fontdb_mut().load_system_fonts();
    let tree = usvg::Tree::from_str(&svg, &options)?;
    let mut pixmap = Pixmap::new(width, height).ok_or("empty board")?;
    resvg::render(&tree, Transform::identity(), &mut pixmap.as_mut());

    // The cursors, pixel for pixel, each centered in its cell.
    for (panel, variant) in Variant::ALL.into_iter().enumerate() {
        let x0 = panel as u32 * panel_w;
        for (r, shape) in shapes.iter().enumerate() {
            let source = source(svgs, shape)?;
            let y = HEADER + r as u32 * row;
            let mut x = x0 + LABEL;
            for &size in sizes {
                let image = render(&source, variant, 0.0, size)?;
                let side = canvas(size);
                pixmap.draw_pixmap(
                    (x + (cell(size) - side) / 2) as i32,
                    (y + (row - side) / 2) as i32,
                    image.as_ref(),
                    &PixmapPaint::default(),
                    Transform::identity(),
                    None,
                );
                x += cell(size);
            }
            let small = render(&source, variant, 0.0, GRID)?;
            let paint = PixmapPaint {
                quality: FilterQuality::Nearest,
                ..PixmapPaint::default()
            };
            let (zx, zy) = (x + PAD, y + (row - zoom) / 2);
            pixmap.draw_pixmap(
                0,
                0,
                small.as_ref(),
                &paint,
                Transform::from_row(ZOOM as f32, 0.0, 0.0, ZOOM as f32, zx as f32, zy as f32),
                None,
            );
        }
    }
    Ok(pixmap)
}
