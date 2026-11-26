//! Runs the generator on `assets/cursors/` and reads back the two themes it writes.

use std::{fs, path::PathBuf, process::Command};

use cursor_theme::{Drawing, SHAPES, SIZES, SPIN_FRAMES, Variant, canvas, spin_delay};

#[test]
fn every_shape_exists_at_every_size_with_its_hotspot() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = std::env::temp_dir().join(format!("cursor-theme-{}", std::process::id()));
    let _ = fs::remove_dir_all(&out);
    let status = Command::new(env!("CARGO_BIN_EXE_cursor-theme"))
        .arg("build")
        .arg(root.join("assets/cursors"))
        .arg(&out)
        .status()
        .unwrap();
    assert!(status.success());
    for variant in Variant::ALL {
        let theme = out.join(variant.theme());
        let index = fs::read_to_string(theme.join("index.theme")).unwrap();
        assert!(
            index.starts_with("[Icon Theme]\nName=SherlockOS"),
            "{index}"
        );
        assert!(index.contains("Inherits=default"));
        for shape in SHAPES {
            let svg = fs::read_to_string(root.join(format!("assets/cursors/{}.svg", shape.name)))
                .unwrap();
            let drawing = Drawing::read(&svg).unwrap();
            let bytes = fs::read(theme.join("cursors").join(shape.name)).unwrap();
            let images = xcursor::parser::parse_xcursor(&bytes).unwrap();
            let frames = if drawing.spin.is_some() {
                SPIN_FRAMES
            } else {
                1
            };
            assert_eq!(
                images.len(),
                SIZES.len() * frames as usize,
                "{}",
                shape.name
            );
            for size in SIZES {
                let at_size: Vec<_> = images.iter().filter(|i| i.size == size).collect();
                assert_eq!(at_size.len(), frames as usize, "{} at {size}", shape.name);
                for image in at_size {
                    assert_eq!((image.width, image.height), (canvas(size), canvas(size)));
                    let hotspot = drawing.hotspot_at(size);
                    assert_eq!(
                        (image.xhot, image.yhot),
                        hotspot,
                        "{} at {size}",
                        shape.name
                    );
                    let delay = if frames > 1 {
                        spin_delay().as_millis() as u32
                    } else {
                        0
                    };
                    assert_eq!(image.delay, delay);
                    // Something is drawn under the hotspot: the tip is where it points. The
                    // ring of the waiting cursor is hollow at its center.
                    let at = 4 * (hotspot.1 * image.width + hotspot.0) as usize;
                    if shape.name != "wait" {
                        assert!(image.pixels_rgba[at + 3] > 0, "{} at {size}", shape.name);
                    }
                }
            }
            for alias in shape.aliases {
                let link = theme.join("cursors").join(alias);
                assert_eq!(fs::read_link(&link).unwrap(), PathBuf::from(shape.name));
            }
        }
    }
    let _ = fs::remove_dir_all(&out);
}
