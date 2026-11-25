//! Wallpaper: the setting of `wallpaper.conf` (crate `desktop-config`), read again when the
//! file changes, the images decoded on a thread of their own, and their textures, uploaded once
//! per variant and drawn behind everything. Without an image, or if it cannot be read, the
//! background of the Main board (color and discs) shows. See "Wallpapers" in
//! `docs/compositor.md`.

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use desktop_config::wallpaper::{self as config, Choice, Config, Mode, Variant};
use sherlock_ui::wallpapers::ThemeVariant;
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            ImportMem as _,
            element::{
                Kind,
                texture::{TextureBuffer, TextureRenderElement},
            },
            gles::{GlesRenderer, GlesTexture},
        },
    },
    reexports::calloop::{
        LoopHandle,
        channel::{self, Sender},
        timer::{TimeoutAction, Timer},
    },
    utils::{Logical, Physical, Point, Rectangle, Size, Transform},
};

use crate::{State, output::View};

/// How often the settings file is checked for changes: one `stat` every two seconds.
const RELOAD_EVERY: Duration = Duration::from_secs(2);

/// Where the images come from.
#[derive(Debug, Clone, PartialEq, Default)]
enum Source {
    /// The board's background.
    #[default]
    Board,
    /// One image, whatever the screen.
    File(PathBuf),
    /// Variants of one wallpaper, one picked per screen.
    Variants(Vec<Variant>),
}

/// An image decoded by the loading thread, for the setting of `generation`.
struct Decoded {
    generation: u64,
    path: PathBuf,
    image: Result<image::RgbaImage, String>,
}

/// The wallpaper setting and the images waiting to become textures. Renderer-free: the
/// textures live with the renderer, in `Textures`.
#[derive(Default)]
pub struct Wallpaper {
    /// `wallpaper.conf`, and its modification time and length when last read.
    file: Option<PathBuf>,
    stamp: Option<(SystemTime, u64)>,
    data_dirs: Vec<PathBuf>,
    /// Theme the wallpaper follows. Dark until the theme setting (lot D-05) is plugged in
    /// (lot D-05c), through `set_theme`.
    theme: ThemeVariant,
    pub mode: Mode,
    source: Source,
    /// Changes with every new setting: textures and images of the previous one are dropped.
    generation: u64,
    decoded: Vec<(PathBuf, image::RgbaImage)>,
    loading: HashSet<PathBuf>,
    failed: HashSet<PathBuf>,
    sender: Option<Sender<Decoded>>,
}

impl Wallpaper {
    /// Reads the setting of `file`, falls back on the default wallpaper installed in
    /// `data_dirs`, and plugs the loading and the periodic check into the loop.
    pub fn start(
        handle: &LoopHandle<'static, State>,
        file: Option<PathBuf>,
        data_dirs: Vec<PathBuf>,
    ) -> Result<Self, crate::Error> {
        let (sender, receiver) = channel::channel();
        handle
            .insert_source(receiver, |event, _, state| {
                if let channel::Event::Msg(decoded) = event {
                    state.wallpaper.receive(decoded);
                }
            })
            .map_err(|err| err.error)?;
        handle
            .insert_source(Timer::from_duration(RELOAD_EVERY), |_, _, state| {
                state.wallpaper.check();
                TimeoutAction::ToDuration(RELOAD_EVERY)
            })
            .map_err(|err| err.error)?;
        let mut wallpaper = Self {
            file,
            data_dirs,
            sender: Some(sender),
            ..Self::default()
        };
        wallpaper.reload();
        Ok(wallpaper)
    }

    /// The desktop changes theme: a wallpaper that follows it changes to its pendant.
    pub fn set_theme(&mut self, theme: ThemeVariant) {
        if theme != self.theme {
            self.theme = theme;
            self.reload();
        }
    }

    /// Reads the settings file again if it changed, appeared or went away.
    pub fn check(&mut self) {
        let stamp = self.file.as_deref().and_then(stamp);
        if stamp != self.stamp {
            self.reload();
        }
    }

    /// Reads the setting and finds its images. Errors are logged, never fatal: what cannot be
    /// read gives the board's background, or the default wallpaper for a wrong file.
    fn reload(&mut self) {
        self.stamp = self.file.as_deref().and_then(stamp);
        let setting = match &self.file {
            Some(file) => Config::read(file).unwrap_or_else(|err| {
                tracing::warn!("wallpaper: {}: {err}", file.display());
                Config::default()
            }),
            None => Config::default(),
        };
        self.mode = setting.mode;
        self.source = resolve(setting.choice(self.theme), &self.data_dirs);
        self.generation += 1;
        self.decoded.clear();
        self.loading.clear();
        self.failed.clear();
        tracing::info!(mode = %self.mode, source = ?self.source, "wallpaper");
    }

    /// The image to draw on a screen of `size` pixels, if any. Starts loading it if it is not
    /// loaded yet: until then, and if it cannot be read, the board's background shows.
    pub fn image_for(&mut self, size: Size<i32, Physical>) -> Option<PathBuf> {
        let path = match &self.source {
            Source::Board => return None,
            Source::File(path) => path.clone(),
            Source::Variants(variants) => {
                let screen = (size.w.max(1) as u32, size.h.max(1) as u32);
                config::pick(variants, screen, self.mode)?.path.clone()
            }
        };
        if self.failed.contains(&path) {
            return None;
        }
        self.load(&path);
        Some(path)
    }

    /// Decodes `path` on a thread of its own, once: a 4K PNG takes a while, the frames go on.
    fn load(&mut self, path: &Path) {
        let Some(sender) = &self.sender else {
            return;
        };
        if !self.loading.insert(path.to_path_buf()) {
            return;
        }
        let (sender, generation, path) = (sender.clone(), self.generation, path.to_path_buf());
        std::thread::spawn(move || {
            let image = decode(&path);
            // The loop may be gone: the session is closing, nothing to do.
            let _ = sender.send(Decoded {
                generation,
                path,
                image,
            });
        });
    }

    fn receive(&mut self, decoded: Decoded) {
        if decoded.generation != self.generation {
            return;
        }
        match decoded.image {
            Ok(image) => self.decoded.push((decoded.path, image)),
            Err(err) => {
                tracing::warn!("wallpaper: {}: {err}", decoded.path.display());
                self.failed.insert(decoded.path);
            }
        }
    }
}

/// Modification time and length of a file, if it exists.
fn stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let meta = fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// Where the images of a choice are: the user's path, a folder of variants or an image, or the
/// folder of a wallpaper installed in `data_dirs`.
fn resolve(choice: Choice, data_dirs: &[PathBuf]) -> Source {
    let path = match choice {
        Choice::Path(path) => path,
        Choice::Installed(id) => match config::find(&id, data_dirs) {
            Some(dir) => dir,
            None => {
                tracing::info!("wallpaper: {id} is not installed");
                return Source::Board;
            }
        },
    };
    if !path.is_dir() {
        if !path.exists() {
            tracing::warn!("wallpaper: {} does not exist", path.display());
            return Source::Board;
        }
        return Source::File(path);
    }
    match config::variants(&path) {
        Ok(variants) if !variants.is_empty() => Source::Variants(variants),
        Ok(_) => {
            tracing::warn!("wallpaper: no <width>x<height>.png in {}", path.display());
            Source::Board
        }
        Err(err) => {
            tracing::warn!("wallpaper: {}: {err}", path.display());
            Source::Board
        }
    }
}

fn decode(path: &Path) -> Result<image::RgbaImage, String> {
    let image = image::ImageReader::open(path)
        .map_err(|err| err.to_string())?
        .with_guessed_format()
        .map_err(|err| err.to_string())?
        .decode()
        .map_err(|err| err.to_string())?;
    Ok(image.into_rgba8())
}

/// A wallpaper uploaded to the GPU, and its size in pixels.
pub struct Texture {
    buffer: TextureBuffer<GlesTexture>,
    size: (u32, u32),
}

/// The wallpaper textures, one per variant in use, shared by the screens that show it.
#[derive(Default)]
pub struct Textures {
    generation: u64,
    textures: HashMap<PathBuf, Texture>,
}

impl Textures {
    /// Follows the setting: drops the textures of an older one, uploads the decoded images.
    pub fn update(&mut self, renderer: &mut GlesRenderer, wallpaper: &mut Wallpaper) {
        if self.generation != wallpaper.generation {
            self.generation = wallpaper.generation;
            self.textures.clear();
        }
        for (path, image) in wallpaper.decoded.drain(..) {
            let size = image.dimensions();
            let uploaded = renderer.import_memory(
                image.as_raw(),
                Fourcc::Xbgr8888,
                (size.0 as i32, size.1 as i32).into(),
                false,
            );
            match uploaded {
                Ok(texture) => {
                    let buffer =
                        TextureBuffer::from_texture(renderer, texture, 1, Transform::Normal, None);
                    self.textures.insert(path, Texture { buffer, size });
                }
                Err(err) => {
                    tracing::warn!("wallpaper: {}: {err}", path.display());
                    wallpaper.failed.insert(path);
                }
            }
        }
    }

    pub fn get(&self, path: &Path) -> Option<&Texture> {
        self.textures.get(path)
    }

    /// Keeps only the textures drawn in this frame: a variant no screen shows any more goes
    /// away.
    pub fn retain(&mut self, used: &[PathBuf]) {
        self.textures.retain(|path, _| used.contains(path));
    }
}

/// Where a wallpaper of `image` pixels is drawn in the frame for the screen `view`
/// (`mode`), and which part of it (in image pixels).
pub fn placement(
    mode: Mode,
    image: (u32, u32),
    view: &View,
) -> (Rectangle<i32, Physical>, Rectangle<f64, Logical>) {
    let screen = (
        view.frame.size.w.max(0) as u32,
        view.frame.size.h.max(0) as u32,
    );
    let fit = config::fit(mode, image, screen);
    let (x, y) = (fit.dst.x.round() as i32, fit.dst.y.round() as i32);
    let end = (
        (fit.dst.x + fit.dst.w).round() as i32,
        (fit.dst.y + fit.dst.h).round() as i32,
    );
    let dst = Rectangle::new(
        view.frame.loc + Point::from((x, y)),
        (end.0 - x, end.1 - y).into(),
    );
    let src = Rectangle::new((fit.src.x, fit.src.y).into(), (fit.src.w, fit.src.h).into());
    (dst, src)
}

/// The wallpaper element of a screen. The frame is drawn at scale 1: its logical coordinates
/// are its pixels.
pub fn element(texture: &Texture, mode: Mode, view: &View) -> TextureRenderElement<GlesTexture> {
    let (dst, src) = placement(mode, texture.size, view);
    TextureRenderElement::from_texture_buffer(
        dst.loc.to_f64(),
        &texture.buffer,
        None,
        Some(src),
        Some((dst.size.w, dst.size.h).into()),
        Kind::Unspecified,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("compositor-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn view(x: i32, w: i32, h: i32, scale: f64) -> View {
        View {
            area: Rectangle::new((0, 0).into(), (w, h).into()),
            frame: Rectangle::new((x, 0).into(), (w, h).into()),
            scale,
            frame_size: (x + w, h).into(),
        }
    }

    #[test]
    fn cover_fills_each_screen_at_its_scale() {
        // Second screen of the frame, 16:10 at scale 2: 2880 × 1800 pixels from x = 1440.
        let (dst, src) = placement(Mode::Cover, (3840, 2160), &view(1440, 2880, 1800, 2.0));
        assert_eq!(dst, Rectangle::new((1440, 0).into(), (2880, 1800).into()));
        assert_eq!(src.loc.x.round(), 192.0);
        assert_eq!(src.size.w.round(), 3456.0);
        // Contain on a 16:10 screen at scale 1: bands of 45 at the top and the bottom.
        let (dst, src) = placement(Mode::Contain, (1920, 1080), &view(0, 1440, 900, 1.0));
        assert_eq!(dst, Rectangle::new((0, 45).into(), (1440, 810).into()));
        assert_eq!(src.size.w, 1920.0);
    }

    #[test]
    fn the_variant_follows_the_size_of_each_screen_in_pixels() {
        let dir = temp_dir("variants");
        let wallpaper = dir.join("wallpapers/first-clue");
        fs::create_dir_all(&wallpaper).unwrap();
        for name in ["1920x1080.png", "2560x1440.png", "3840x2160.png"] {
            fs::write(wallpaper.join(name), b"").unwrap();
        }
        // No settings file: the default wallpaper, found in the data folder.
        let mut w = Wallpaper {
            data_dirs: vec![dir.clone()],
            ..Wallpaper::default()
        };
        w.reload();
        // 1440 × 900 at scale 1, then the same screen at scale 2.
        assert_eq!(
            w.image_for((1440, 900).into()),
            Some(wallpaper.join("1920x1080.png"))
        );
        assert_eq!(
            w.image_for((2880, 1800).into()),
            Some(wallpaper.join("3840x2160.png"))
        );
        // In the light theme, its pendant, once installed.
        let light = dir.join("wallpapers/first-clue-light");
        fs::create_dir_all(&light).unwrap();
        fs::write(light.join("1920x1080.png"), b"").unwrap();
        w.set_theme(ThemeVariant::Light);
        let image = w.image_for((1440, 900).into());
        assert_eq!(image, Some(light.join("1920x1080.png")));
        w.set_theme(ThemeVariant::Dark);
        let image = w.image_for((1440, 900).into());
        assert_eq!(image, Some(wallpaper.join("1920x1080.png")));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_or_broken_image_gives_the_board() {
        let dir = temp_dir("fallback");
        let file = dir.join("wallpaper.conf");
        // A missing image: the board.
        fs::write(&file, "path = /nonexistent/wall.png\n").unwrap();
        let mut w = Wallpaper {
            file: Some(file.clone()),
            ..Wallpaper::default()
        };
        w.reload();
        assert_eq!(w.image_for((1440, 900).into()), None);
        // An image that does not decode: tried, then the board, without a panic.
        let broken = dir.join("broken.png");
        fs::write(&broken, b"not a PNG").unwrap();
        fs::write(&file, format!("path = {}\n", broken.display())).unwrap();
        w.reload();
        assert_eq!(w.image_for((1440, 900).into()), Some(broken.clone()));
        let image = decode(&broken);
        assert!(image.is_err());
        w.receive(Decoded {
            generation: w.generation,
            path: broken,
            image,
        });
        assert_eq!(w.image_for((1440, 900).into()), None);
        // No default wallpaper installed: the board.
        fs::write(&file, "mode = contain\n").unwrap();
        w.reload();
        assert_eq!(w.mode, Mode::Contain);
        assert_eq!(w.image_for((1440, 900).into()), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_settings_are_read_again_when_they_change() {
        let dir = temp_dir("reload");
        let file = dir.join("wallpaper.conf");
        let image = dir.join("wall.jpg");
        fs::write(&image, b"").unwrap();
        let mut w = Wallpaper {
            file: Some(file.clone()),
            ..Wallpaper::default()
        };
        w.reload();
        let first = w.generation;
        // Nothing changed: nothing read again.
        w.check();
        assert_eq!(w.generation, first);
        fs::write(
            &file,
            format!("path = {}\nmode = center\n", image.display()),
        )
        .unwrap();
        w.check();
        assert_eq!(w.generation, first + 1);
        assert_eq!(
            (w.mode, w.source.clone()),
            (Mode::Center, Source::File(image))
        );
        // A wrong file: logged, and the default setting.
        fs::write(&file, "mode = stretched\n").unwrap();
        w.check();
        assert_eq!(w.mode, Mode::Cover);
        // An image decoded for an older setting is dropped.
        w.receive(Decoded {
            generation: first,
            path: dir.join("old.png"),
            image: Ok(image::RgbaImage::new(1, 1)),
        });
        assert!(w.decoded.is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }
}
