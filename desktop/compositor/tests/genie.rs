//! The genie drawn for real, without a screen (lot D-19): Mesa's software renderer (llvmpipe),
//! reached through an EGL device of its own, draws `genie.frag` into a texture, halfway between
//! a window and its dock icon. The image is checked pixel by pixel and saved as
//! `genie-half.png` in Cargo's temporary folder of the integration tests
//! (`target/tmp`), next to `genie-steps.png`, the whole way under a dock. Without Mesa's EGL the test has nothing to draw with: it says so, and fails
//! on the CI, which installs it.

#[cfg(test)]
mod tests {
    use compositor::genie::{Genie, GenieElement, Program};
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

    /// The dock, opaque here: its glass over the desktop color, and the icon in it, drawn in
    /// front of the genie as the compositor does since lot Q-14.
    fn dock(icon: Rectangle<i32, Physical>) -> [(Rectangle<i32, Physical>, color::Color); 2] {
        let glass = color::DESKTOP.mix(color::CREAM, color::DOCK_GLASS.a);
        let margin = 8;
        let dock = Rectangle::new(
            (icon.loc.x - 64, icon.loc.y - margin).into(),
            (icon.size.w + 128, icon.size.h + 2 * margin).into(),
        );
        [(dock, glass), (icon, color::TILE_LAB)]
    }

    /// Draws `genie` over the desktop color, then what is in `front` of it, and reads the image
    /// back.
    fn draw(
        renderer: &mut GlesRenderer,
        window: GlesTexture,
        genie: &Genie,
        front: &[(Rectangle<i32, Physical>, color::Color)],
    ) -> Vec<u8> {
        let program = Program::new(renderer).unwrap();
        let element = GenieElement::new(&program, window, genie, genie.alpha());
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
            let dst = element.geometry(Scale::from(1.0));
            let damage = [Rectangle::from_size(dst.size)];
            element
                .draw(&mut frame, element.src(), dst, &damage, &[])
                .unwrap();
            for (rect, color) in front {
                frame.clear(color.premultiplied().into(), &[*rect]).unwrap();
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
    fn the_genie_is_drawn_halfway_into_its_icon() {
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
            icon: Rectangle::new((360.0, 256.0).into(), (32.0, 32.0).into()),
            progress: 0.5,
        };
        let bytes = draw(&mut renderer, window.clone(), &genie, &[]);
        let path = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("genie-half.png");
        let size = (SCREEN.0 as u32, SCREEN.1 as u32);
        image::save_buffer(&path, &bytes, size.0, size.1, image::ColorType::Rgba8).unwrap();
        eprintln!("genie halfway: {}", path.display());

        let desktop = rgba(color::DESKTOP);
        // The title bar along the top row, the body near the bottom one, both at the place
        // `Genie::row` gives.
        let (y, left, right) = genie.row(0.05);
        assert!(near(
            pixel(&bytes, (left + right) / 2.0, y),
            rgba(color::CREAM)
        ));
        let (y, left, right) = genie.row(0.97);
        let body = pixel(&bytes, left + 0.55 * (right - left), y);
        assert!(near(body, rgba(color::BLUE_DARK)), "{body:?}");
        // Just outside the narrowed bottom: the desktop.
        assert!(near(pixel(&bytes, left - 3.0, y), desktop));
        assert!(near(pixel(&bytes, right + 3.0, y), desktop));
        // Above the window, which has slid down, and in the icon, not reached yet.
        assert!(near(pixel(&bytes, 160.0, 22.0), desktop));
        assert!(near(pixel(&bytes, 376.0, 280.0), desktop));

        // The whole way, side by side, for the eye: `genie-steps.png`, under the dock.
        let front = dock(genie.icon.to_i32_round());
        let steps = [0.0, 0.2, 0.4, 0.6, 0.8, 0.95];
        let frames: Vec<Vec<u8>> = steps
            .iter()
            .map(|&progress| {
                let step = Genie { progress, ..genie };
                draw(&mut renderer, window.clone(), &step, &front)
            })
            .collect();
        // Under the dock: at the end, only the icon shows where the window went.
        let last = frames.last().unwrap();
        let (y, left, right) = Genie {
            progress: 0.95,
            ..genie
        }
        .row(0.5);
        assert!(near(
            pixel(last, (left + right) / 2.0, y),
            rgba(color::TILE_LAB)
        ));
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
