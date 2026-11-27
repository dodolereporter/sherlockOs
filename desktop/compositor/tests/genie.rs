//! The genie drawn for real, without a screen (lots D-19, D-19b): Mesa's software renderer
//! (llvmpipe), reached through an EGL device of its own, draws `genie.frag` into a texture,
//! halfway between a window and its dock icon, then at eight steps of the way under a
//! translucent dock. The images are checked pixel by pixel and saved in Cargo's temporary
//! folder of the integration tests (`target/tmp`): `genie-half.png` and `genie-steps.png`.
//! Without Mesa's EGL the test has nothing to draw with: it says so, and fails on the CI, which
//! installs it.

#[cfg(test)]
mod tests {
    use compositor::genie::{Genie, GenieElement, Program, Side};
    use sherlock_ui::color;
    use smithay::{
        backend::{
            allocator::Fourcc,
            egl::{EGLContext, EGLDevice, EGLDisplay},
            renderer::{
                Bind, ExportMem, Frame as _, ImportMem, Offscreen, Renderer,
                element::{Element, RenderElement},
                gles::{GlesRenderer, GlesTexture},
            },
        },
        utils::{Physical, Rectangle, Scale, Transform},
    };

    const SCREEN: (i32, i32) = (480, 300);
    const WINDOW: (i32, i32) = (240, 160);
    /// Height of the title bar of the drawn window.
    const TITLE: i32 = 30;
    /// Distance from the icon to the top of the dock's glass.
    const GLASS: i32 = 8;

    /// A renderer on Mesa's software device, if there is one.
    fn software_renderer() -> Option<GlesRenderer> {
        let software = |d: &EGLDevice| {
            d.extensions()
                .iter()
                .any(|e| e == "EGL_MESA_device_software")
        };
        let device = EGLDevice::enumerate().ok()?.find(software)?;
        // SAFETY: nothing else in this process creates EGL displays, so none of them can be
        // terminated behind smithay's back.
        let display = unsafe { EGLDisplay::new(device) }.ok()?;
        let context = EGLContext::new(&display).ok()?;
        // SAFETY: the context was just created and is used by this thread only.
        unsafe { GlesRenderer::new(context) }.ok()
    }

    fn rgba(c: color::Color) -> [u8; 4] {
        c.premultiplied().map(|v| (v * 255.0).round() as u8)
    }

    /// The window: a title bar, then a body crossed by a grid, which shows the deformation. Colors
    /// far from the desktop's, so that the image reads well.
    fn window_pixels() -> Vec<u8> {
        let (title, body, grid) = (
            rgba(color::CREAM),
            rgba(color::BLUE_DARK),
            rgba(color::BLUE_LIGHT),
        );
        (0..WINDOW.1)
            .flat_map(|y| {
                (0..WINDOW.0).flat_map(move |x| match (y < TITLE, x % 24 < 2 || y % 24 < 2) {
                    (true, _) => title,
                    (false, true) => grid,
                    (false, false) => body,
                })
            })
            .collect()
    }

    fn pixel(bytes: &[u8], x: f64, y: f64) -> [u8; 4] {
        let i = 4 * (y.round() as usize * SCREEN.0 as usize + x.round() as usize);
        [bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]
    }

    fn near(a: [u8; 4], b: [u8; 4]) -> bool {
        a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 3)
    }

    /// The dock: its translucent glass, `GLASS` around the icon toward the window and below it,
    /// and the icon in it, drawn in front of the genie as the compositor does since lot Q-14.
    fn dock(icon: Rectangle<i32, Physical>) -> [(Rectangle<i32, Physical>, color::Color); 2] {
        let glass = Rectangle::new(
            (icon.loc.x - 64, icon.loc.y - GLASS).into(),
            (icon.size.w + 128, icon.size.h + 2 * GLASS).into(),
        );
        [(glass, color::DOCK_GLASS), (icon, color::TILE_LAB)]
    }

    /// Draws `genie` over the desktop color, if any of it shows, then what is in `front` of it,
    /// blended, and reads the image back.
    fn draw(
        renderer: &mut GlesRenderer,
        window: GlesTexture,
        genie: Option<&Genie>,
        front: &[(Rectangle<i32, Physical>, color::Color)],
    ) -> Vec<u8> {
        let program = Program::new(renderer).unwrap();
        let mut target: GlesTexture = renderer
            .create_buffer(Fourcc::Abgr8888, SCREEN.into())
            .unwrap();
        let full = Rectangle::from_size(SCREEN.into());
        {
            let mut framebuffer = renderer.bind(&mut target).unwrap();
            let mut frame = renderer
                .render(&mut framebuffer, SCREEN.into(), Transform::Normal)
                .unwrap();
            frame
                .clear(color::DESKTOP.premultiplied().into(), &[full])
                .unwrap();
            if let Some((genie, area)) = genie.and_then(|g| Some((g, g.bounds()?))) {
                let element = GenieElement::new(&program, window, (genie, area), genie.alpha());
                let dst = element.geometry(Scale::from(1.0));
                let damage = [Rectangle::from_size(dst.size)];
                element
                    .draw(&mut frame, element.src(), dst, &damage, &[])
                    .unwrap();
            }
            for (rect, color) in front {
                let damage = [Rectangle::from_size(rect.size)];
                frame
                    .draw_solid(*rect, &damage, color.premultiplied().into())
                    .unwrap();
            }
            frame.finish().unwrap().wait().unwrap();
        }
        let framebuffer = renderer.bind(&mut target).unwrap();
        let region = Rectangle::from_size(SCREEN.into());
        let mapping = renderer
            .copy_framebuffer(&framebuffer, region, Fourcc::Abgr8888)
            .unwrap();
        renderer.map_texture(&mapping).unwrap().to_vec()
    }

    #[test]
    fn the_genie_goes_into_its_icon_behind_the_glass() {
        let Some(mut renderer) = software_renderer() else {
            assert!(
                std::env::var_os("CI").is_none(),
                "the CI has Mesa's EGL (libegl-mesa0): its software device must be found"
            );
            eprintln!("no software EGL device (Mesa's llvmpipe): the genie is not drawn");
            return;
        };
        let window = renderer
            .import_memory(&window_pixels(), Fourcc::Abgr8888, WINDOW.into(), false)
            .unwrap();
        let genie = Genie {
            window: Rectangle::new((40.0, 20.0).into(), (240.0, 160.0).into()),
            icon: Rectangle::new((360.0, 248.0).into(), (32.0, 32.0).into()),
            side: Side::Bottom,
            glass: f64::from(GLASS),
            progress: 0.5,
        };
        let bytes = draw(&mut renderer, window.clone(), Some(&genie), &[]);
        let path = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("genie-half.png");
        let size = (SCREEN.0 as u32, SCREEN.1 as u32);
        image::save_buffer(&path, &bytes, size.0, size.1, image::ColorType::Rgba8).unwrap();
        eprintln!("genie halfway: {}", path.display());

        let desktop = rgba(color::DESKTOP);
        // The title bar along the top row, the body further down, both where `Genie::point`
        // puts them.
        let (x, y) = genie.point((0.5, 0.05)).unwrap();
        assert!(near(pixel(&bytes, x, y), rgba(color::CREAM)));
        let (x, y) = genie.point((0.55, 0.5)).unwrap();
        let body = pixel(&bytes, x, y);
        assert!(near(body, rgba(color::BLUE_DARK)), "{body:?}");
        // Just outside its narrowed sides: the desktop.
        let (left, y) = genie.point((0.0, 0.5)).unwrap();
        let (right, _) = genie.point((1.0, 0.5)).unwrap();
        assert!(near(pixel(&bytes, left - 3.0, y), desktop));
        assert!(near(pixel(&bytes, right + 3.0, y), desktop));
        // Above the window, which has moved down, and past the glass's edge: the desktop.
        assert!(near(pixel(&bytes, 160.0, 22.0), desktop));
        assert!(near(pixel(&bytes, 376.0, 244.0), desktop));
        assert!(near(pixel(&bytes, 376.0, 264.0), desktop));

        // The whole way under the dock, for the eye: `genie-steps.png`. Nothing of the window
        // shows through the glass nor under it: the dock is as without the window.
        let icon = genie.icon.to_i32_round();
        let front = dock(icon);
        let alone = draw(&mut renderer, window.clone(), None, &front);
        let glass = front[0].0;
        let steps = [0.0, 0.15, 0.3, 0.45, 0.6, 0.72, 0.84, 0.95];
        let frames: Vec<Vec<u8>> = steps
            .iter()
            .map(|&progress| {
                let step = Genie { progress, ..genie };
                let frame = draw(&mut renderer, window.clone(), Some(&step), &front);
                for y in glass.loc.y..SCREEN.1 {
                    for x in 0..SCREEN.0 {
                        let (x, y) = (f64::from(x), f64::from(y));
                        let (got, want) = (pixel(&frame, x, y), pixel(&alone, x, y));
                        assert!(near(got, want), "{progress}: ({x}, {y}) {got:?} {want:?}");
                    }
                }
                frame
            })
            .collect();
        // On the way the window has shown just above the glass, as wide as the icon there.
        let late = Genie {
            progress: 0.72,
            ..genie
        };
        let (x, y) = late.point((0.5, 0.05)).unwrap();
        assert!(y < f64::from(glass.loc.y) && near(pixel(&frames[5], x, y), rgba(color::CREAM)));
        let row = 4 * SCREEN.0 as usize;
        let strip: Vec<u8> = (0..SCREEN.1 as usize)
            .flat_map(|y| {
                frames
                    .iter()
                    .flat_map(move |f| f[y * row..(y + 1) * row].to_vec())
            })
            .collect();
        let path = path.with_file_name("genie-steps.png");
        let width = size.0 * steps.len() as u32;
        image::save_buffer(&path, &strip, width, size.1, image::ColorType::Rgba8).unwrap();
        eprintln!("genie, step by step: {}", path.display());
    }
}
