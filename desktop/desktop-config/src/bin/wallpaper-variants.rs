//! Generates the variants of the wallpapers, when the image is built or for development.
//!
//! `wallpaper-variants <originals folder> <output folder>` reads every `<id>.jpg` and
//! `<id>.png` of the first folder (`assets/wallpapers/`) and writes
//! `<output>/<id>/<width>x<height>.png` at each definition of `SIZES`. The originals are not
//! touched. Running it again overwrites the variants: the result is the same, byte for byte.

use std::{
    error::Error,
    fs::{self, File},
    io::{BufWriter, Write as _},
    path::{Path, PathBuf},
    time::Instant,
};

use desktop_config::variants::{SIZES, render};
use image::{
    ExtendedColorType, ImageEncoder as _,
    codecs::png::{CompressionType, FilterType, PngEncoder},
};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let [input, output] = args.as_slice() else {
        return Err("usage: wallpaper-variants <originals folder> <output folder>".into());
    };
    let mut originals: Vec<PathBuf> = fs::read_dir(input)
        .map_err(|err| format!("{}: {err}", input.display()))?
        .filter_map(|entry| Some(entry.ok()?.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "jpg" || e == "png"))
        .collect();
    originals.sort();
    if originals.is_empty() {
        return Err(format!("no original in {}", input.display()).into());
    }
    let mut out = std::io::stdout().lock();
    for original in &originals {
        let started = Instant::now();
        let id = original
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| format!("{}: name is not UTF-8", original.display()))?;
        let image =
            image::open(original).map_err(|err| format!("{}: {err}", original.display()))?;
        let dir = output.join(id);
        fs::create_dir_all(&dir).map_err(|err| format!("{}: {err}", dir.display()))?;
        for size in SIZES {
            let path = dir.join(format!("{}x{}.png", size.0, size.1));
            write_png(&path, &render(&image, size))
                .map_err(|err| format!("{}: {err}", path.display()))?;
        }
        writeln!(out, "{id}: {:.1} s", started.elapsed().as_secs_f32())?;
    }
    Ok(())
}

/// Writes an RGB PNG, compressed as much as the encoder can: the variants go into the image
/// of the system.
fn write_png(path: &Path, image: &image::RgbImage) -> Result<(), Box<dyn Error>> {
    let mut file = BufWriter::new(File::create(path)?);
    PngEncoder::new_with_quality(&mut file, CompressionType::Best, FilterType::Adaptive)
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            ExtendedColorType::Rgb8,
        )?;
    file.flush()?;
    Ok(())
}
