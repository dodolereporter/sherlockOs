//! Variants of a wallpaper at the usual definitions, made from its original when the image is
//! built (`wallpaper-variants`): cut to the screen ratio, reduced with Lanczos, then brought
//! back to 8 bits with a light dither, so that dark gradients do not show bands. A variant at
//! the size of the original (3840 × 2160 from a 4K original) is the original itself, untouched.

use image::{
    DynamicImage, Rgb32FImage, RgbImage,
    imageops::{self, FilterType},
};

use crate::wallpaper::{Mode, fit};

/// The definitions generated for each wallpaper: 1080p, 1440p, 2160p.
pub const SIZES: [(u32, u32); 3] = [(1920, 1080), (2560, 1440), (3840, 2160)];

/// Seed of the dither noise: the same original always gives the same variants.
const SEED: u64 = 0x5EED_0D04;

/// The variant of `original` at `size`: the middle of the original cut to the ratio of `size`
/// (as `cover` would show it), resized, then dithered to 8 bits. When the cut is already at
/// `size`, it is returned as is: nothing to resample, nothing to dither. An original smaller
/// than `size` is enlarged the same way, which only happens with an image of the user's.
pub fn render(original: &DynamicImage, size: (u32, u32)) -> RgbImage {
    let src = fit(Mode::Cover, (original.width(), original.height()), size).src;
    let (x, y) = (src.x.round() as u32, src.y.round() as u32);
    let (w, h) = (src.w.round().max(1.0) as u32, src.h.round().max(1.0) as u32);
    let cropped = original.crop_imm(x, y, w, h);
    if (w, h) == size {
        return cropped.to_rgb8();
    }
    dither(&imageops::resize(
        &cropped.to_rgb32f(),
        size.0,
        size.1,
        FilterType::Lanczos3,
    ))
}

/// Brings channels in [0, 1] back to 8 bits with triangular noise of one step: the error of
/// rounding becomes fine grain instead of steps, and the mean of each area is kept.
pub fn dither(image: &Rgb32FImage) -> RgbImage {
    let mut noise = Noise(SEED);
    let data = image
        .as_raw()
        .iter()
        .map(|v| {
            let v = v * 255.0 + noise.next() + noise.next() - 1.0;
            v.round().clamp(0.0, 255.0) as u8
        })
        .collect();
    // Same width and height, three channels: the buffer has the right length.
    RgbImage::from_raw(image.width(), image.height(), data).unwrap_or_default()
}

/// xorshift64*: enough for a dither, without a dependency.
struct Noise(u64);

impl Noise {
    /// Uniform in [0, 1).
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32 / (1u64 << 24) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;

    #[test]
    fn the_dither_keeps_a_dark_gradient_smooth() {
        // From level 10 to level 12 over 512 pixels: rounded, three flat bands.
        let (w, h) = (512, 64);
        let level = |x: u32| (10.0 + 2.0 * x as f32 / (w - 1) as f32) / 255.0;
        let ramp = Rgb32FImage::from_fn(w, h, |x, _| Rgb([level(x); 3]));
        let out = dither(&ramp);
        // Each group of 16 columns keeps the mean of the ramp, within a tenth of a step.
        for x0 in (0..w).step_by(16) {
            let values = (x0..x0 + 16).flat_map(|x| (0..h).map(move |y| (x, y)));
            let mean = values
                .map(|(x, y)| f32::from(out.get_pixel(x, y)[0]))
                .sum::<f32>()
                / (16 * h) as f32;
            let expected = (x0..x0 + 16).map(|x| level(x) * 255.0).sum::<f32>() / 16.0;
            assert!(
                (mean - expected).abs() < 0.1,
                "x {x0}: {mean} for {expected}"
            );
        }
        // The grain stays within one step of the value.
        assert!(out.pixels().all(|p| (9..=13).contains(&p[0])));
        // And it is reproducible.
        assert_eq!(dither(&ramp), out);
    }

    #[test]
    fn a_variant_has_its_size_and_the_original_ratio() {
        // 1672 × 941, the author's originals: barely wider than 16:9.
        let original = DynamicImage::ImageRgb8(RgbImage::from_pixel(167, 94, Rgb([14, 26, 46])));
        let variant = render(&original, (192, 108));
        assert_eq!(variant.dimensions(), (192, 108));
        // A flat color stays flat, apart from the grain.
        assert!(variant.pixels().all(|p| p[2].abs_diff(46) <= 1));
        // At the size of the original: the original, pixel for pixel.
        let original = RgbImage::from_fn(192, 108, |x, y| Rgb([x as u8, y as u8, 7]));
        let same = render(&DynamicImage::ImageRgb8(original.clone()), (192, 108));
        assert_eq!(same, original);
    }
}
