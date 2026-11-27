//! What a hostile SVG costs to draw (Q-38): documents within `picture::MAX_SVG_BYTES` that
//! would make resvg work hard, drawn at the size of a bubble's tile through the same renderer
//! as the bubbles. Run with `--nocapture` to read the measures.

// In a test module: the helpers may unwrap like the tests (clippy.toml).
#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use iced::widget::svg;
    use iced_test::simulator::Simulator;
    use notifications::picture;
    use sherlock_ui::notifications as ui;
    use sherlock_ui::theme;

    /// `<use>` fanning out: each group draws the one before it `fan` times, `depth` deep.
    pub fn fan_out(fan: usize, depth: usize) -> String {
        let mut svg = String::from(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" \
             viewBox=\"0 0 20 20\"><defs><g id=\"g0\"><rect width=\"1\" height=\"1\"/></g>",
        );
        for level in 1..=depth {
            svg.push_str(&format!("<g id=\"g{level}\">"));
            for _ in 0..fan {
                svg.push_str(&format!("<use xlink:href=\"#g{}\"/>", level - 1));
            }
            svg.push_str("</g>");
        }
        svg.push_str(&format!("</defs><use xlink:href=\"#g{depth}\"/></svg>"));
        svg
    }

    /// A blur as wide as it can be asked, on a region far larger than the picture.
    pub fn huge_blur() -> String {
        String::from(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 20 20\"><filter id=\"f\" \
             x=\"-100000\" y=\"-100000\" width=\"200000\" height=\"200000\" \
             filterUnits=\"userSpaceOnUse\"><feGaussianBlur stdDeviation=\"10000\"/></filter>\
             <rect width=\"20\" height=\"20\" filter=\"url(#f)\"/></svg>",
        )
    }

    /// The time to draw `document` at the tile's size, given up after `limit`.
    fn draw(document: String, limit: Duration) -> Option<Duration> {
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let start = Instant::now();
            let handle = svg::Handle::from_memory(document.into_bytes());
            let tile: iced::Element<'_, ()> = svg(handle).width(ui::TILE).height(ui::TILE).into();
            let mut screen: Simulator<'_, ()> =
                Simulator::with_size(notifications::settings(), (ui::TILE, ui::TILE), tile);
            let _ = screen.snapshot(&theme::theme());
            let _ = sender.send(start.elapsed());
        });
        receiver.recv_timeout(limit).ok()
    }

    /// `count` shapes side by side.
    pub fn flat(count: usize) -> String {
        let mut svg =
            String::from("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 20 20\">");
        for i in 0..count {
            svg.push_str(&format!(
                "<rect x=\"{}\" width=\"1\" height=\"1\"/>",
                i % 20
            ));
        }
        svg.push_str("</svg>");
        svg
    }

    #[test]
    fn a_hostile_svg_is_refused_and_an_accepted_one_draws_quickly() {
        let limit = Duration::from_secs(60);
        // Refused before drawing, though within the size limit; what they would cost is
        // printed (1.2 s for 8^6 shapes in a debug build).
        for (name, document) in [
            ("fan-out 8^6", fan_out(8, 6)),
            ("fan-out 2^20", fan_out(2, 20)),
            ("flat 5000", flat(5000)),
        ] {
            assert!(document.len() as u64 <= picture::MAX_SVG_BYTES, "{name}");
            assert!(picture::check_svg(document.as_bytes()).is_err(), "{name}");
            if name == "fan-out 8^6" {
                eprintln!("{name}, refused, would take: {:?}", draw(document, limit));
            }
        }
        // The worst accepted: as many elements and uses as allowed, and a blur as wide as
        // can be asked.
        for (name, document) in [
            ("flat 4000", flat(4000)),
            // 3 × 5 nested uses and the one that shows them: 16, 243 shapes.
            ("fan-out 3^5", fan_out(3, 5)),
            ("huge blur", huge_blur()),
        ] {
            assert!(picture::check_svg(document.as_bytes()).is_ok(), "{name}");
            let took = draw(document, limit);
            eprintln!("{name}: {took:?}");
            assert!(took.is_some(), "{name} never ended");
        }
    }
}
