//! The edges of an application icon, before and after UI-05, zoomed four times.
//!
//! The tests render in software, where iced's SVGs are right; the defect is in iced's wgpu
//! renderer. So « before » is computed here exactly as that renderer does it: resvg's
//! premultiplied pixels at the drawn size rounded up, sampled nearest, blended as straight
//! alpha. « After » is `raster::rasterize` (straight alpha, twice the size in steps) sampled
//! linearly, as the image pipeline draws it. Both over the desktop of each theme, at the size of
//! a dock icon magnified 1.37 times, at scale 1, the author's.

use std::borrow::Cow;
use std::path::PathBuf;

use iced::widget::image::{self, FilterMethod};
use iced::widget::{column, row, text};
use iced::{Pixels, Settings};
use iced_test::simulator::Simulator;
use resvg::{tiny_skia, usvg};
use sherlock_ui::color::{self, Color, Palette};
use sherlock_ui::{metrics, raster, theme, typography};

/// A dock icon magnified on its way: not a whole number of pixels.
const SIDE: f32 = metrics::DOCK_ICON * 1.37;
/// The corner looked at, in screen pixels, and how much it is enlarged.
const CORNER: usize = 24;
const ZOOM: usize = 4;

type Rgb = [f32; 3];

/// The icon's pixels as iced's wgpu renderer uploads them: resvg's premultiplied output at
/// `ceil(SIDE)`.
fn premultiplied(document: &str, side: u32) -> Vec<[f32; 4]> {
    let Ok(tree) = usvg::Tree::from_data(document.as_bytes(), &usvg::Options::default()) else {
        panic!("the icon parses");
    };
    let Some(mut pixmap) = tiny_skia::Pixmap::new(side, side) else {
        panic!("a pixmap");
    };
    let fit = side as f32 / tree.size().width();
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(fit, fit),
        &mut pixmap.as_mut(),
    );
    pixmap
        .pixels()
        .iter()
        .map(|p| [p.red(), p.green(), p.blue(), p.alpha()].map(|c| f32::from(c) / 255.0))
        .collect()
}

fn straight(pixels: &[u8]) -> Vec<[f32; 4]> {
    pixels
        .chunks(4)
        .map(|p| [p[0], p[1], p[2], p[3]].map(|c| f32::from(c) / 255.0))
        .collect()
}

/// Straight-alpha blending, as the image pipeline does (`SrcAlpha`, `OneMinusSrcAlpha`).
fn blend(texel: [f32; 4], below: Rgb) -> Rgb {
    let a = texel[3];
    [0, 1, 2].map(|i| texel[i] * a + below[i] * (1.0 - a))
}

/// The screen pixels of the corner: `texels` (`side` by `side`) drawn over `SIDE` screen
/// pixels, sampled nearest or linearly.
fn corner(texels: &[[f32; 4]], side: usize, linear: bool, below: Rgb) -> Vec<Rgb> {
    let ratio = side as f32 / SIDE;
    let at = |x: usize, y: usize| texels[y.min(side - 1) * side + x.min(side - 1)];
    let mut screen = Vec::new();
    for y in 0..CORNER {
        for x in 0..CORNER {
            let (u, v) = ((x as f32 + 0.5) * ratio, (y as f32 + 0.5) * ratio);
            let texel = if linear {
                let (u, v) = ((u - 0.5).max(0.0), (v - 0.5).max(0.0));
                let (x0, y0) = (u.floor() as usize, v.floor() as usize);
                let (fx, fy) = (u.fract(), v.fract());
                let mix = |a: [f32; 4], b: [f32; 4], t: f32| -> [f32; 4] {
                    [0, 1, 2, 3].map(|i| a[i] + (b[i] - a[i]) * t)
                };
                let top = mix(at(x0, y0), at(x0 + 1, y0), fx);
                let bottom = mix(at(x0, y0 + 1), at(x0 + 1, y0 + 1), fx);
                mix(top, bottom, fy)
            } else {
                at(u.floor() as usize, v.floor() as usize)
            };
            screen.push(blend(texel, below));
        }
    }
    screen
}

fn rgb(c: Color) -> Rgb {
    [c.r, c.g, c.b].map(|v| f32::from(v) / 255.0)
}

/// The corner as RGBA bytes, enlarged `ZOOM` times without smoothing.
fn zoomed(screen: &[Rgb]) -> Vec<u8> {
    let side = CORNER * ZOOM;
    let mut bytes = Vec::with_capacity(side * side * 4);
    for y in 0..side {
        for x in 0..side {
            let p = screen[(y / ZOOM) * CORNER + x / ZOOM];
            bytes.extend(p.map(|c| (c * 255.0).round() as u8));
            bytes.push(255);
        }
    }
    bytes
}

/// Darker than both the tile and the desktop under it by more than `margin`: a fringe.
fn fringes(screen: &[Rgb], tile: Rgb, below: Rgb, margin: f32) -> usize {
    let luma = |c: Rgb| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let floor = luma(tile).min(luma(below)) - margin;
    screen.iter().filter(|p| luma(**p) < floor).count()
}

#[test]
fn the_edges_of_an_icon_are_smooth_without_a_dark_fringe() {
    // The cream tile of Sherlock's icon alone, its radius the icons' (5.28 on 24): the corner
    // holds only the tile's edge, which is what is looked at.
    let document = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\">\
        <rect width=\"24\" height=\"24\" rx=\"5.28\" fill=\"#F4EBE3\"/></svg>";
    let tile = rgb(color::CREAM);
    let before_side = SIDE.ceil() as u32;
    let before = premultiplied(document, before_side);
    let after_side = raster::raster_side(SIDE);
    let Some(after) = raster::rasterize(document.as_bytes(), after_side) else {
        panic!("the icon rasterizes");
    };
    let after = straight(&after);
    for (palette, suffix) in [(&Palette::DARK, ""), (&Palette::LIGHT, "-light")] {
        let below = rgb(palette.desktop);
        let old = corner(&before, before_side as usize, false, below);
        let new = corner(&after, after_side as usize, true, below);
        // On the light desktop the premultiplied edge shows a dark fringe; the straight one
        // stays between the tile and the desktop.
        if suffix == "-light" {
            assert!(
                fringes(&old, tile, below, 0.08) > 2,
                "the defect reproduced"
            );
        }
        assert_eq!(fringes(&new, tile, below, 0.03), 0, "no fringe{suffix}");

        let side = (CORNER * ZOOM) as u32;
        let picture = |screen: &[Rgb]| -> iced::widget::Image<image::Handle> {
            iced::widget::image(image::Handle::from_rgba(side, side, zoomed(screen)))
                .filter_method(FilterMethod::Nearest)
                .width(side as f32)
                .height(side as f32)
        };
        let label = |s: &'static str| text(s).size(typography::SMALL_SIZE);
        let sheet = row![
            column![label("Avant"), picture(&old)].spacing(4),
            column![label("Après"), picture(&new)].spacing(4),
        ]
        .spacing(16)
        .padding(12);
        let page: iced::Element<'_, ()> =
            iced::widget::container(sheet).style(theme::desktop).into();
        color::set_active(if suffix.is_empty() {
            color::Theme::Dark
        } else {
            color::Theme::Light
        });
        let settings = Settings {
            fonts: typography::FONTS.map(Cow::Borrowed).to_vec(),
            default_font: typography::UI,
            default_text_size: Pixels(typography::UI_SIZE),
            ..Settings::default()
        };
        let mut ui = Simulator::with_size(settings, (240.0, 140.0), page);
        let Ok(snapshot) = ui.snapshot(&theme::theme()) else {
            panic!("snapshot");
        };
        let path =
            PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("icon-edges{suffix}.png"));
        let _ =
            std::fs::remove_file(path.with_file_name(format!("icon-edges{suffix}-tiny-skia.png")));
        assert!(matches!(snapshot.matches_image(&path), Ok(true)));
        color::set_active(color::Theme::Dark);
    }
}
