//! A teammate's picture drawn on this machine, with no image provider and no
//! network: one of DiceBear's styles, drawn from a seed, as SVG for the window
//! to offer and as the kept 512 px PNG once picked.
//!
//! The same style and seed always draw the same picture, so a candidate the
//! person was shown is the picture that is kept. Only styles whose artwork
//! is CC0 1.0 are here (and enabled in Cargo.toml), so nothing needs credit
//! where a picture is shown.

use super::EDGE;
use crate::contract::AvatarCandidate;
use dicebear_core::{Avatar, Style};
use resvg::{tiny_skia, usvg};
use serde_json::json;
use std::sync::LazyLock;

/// The styles the desk draws, by DiceBear's id, chosen for reading as a
/// character, or a clear mark, in a 28 px circle. The first is the default.
const STYLES: [(&str, &str); 12] = [
    ("lorelei", dicebear_styles::LORELEI),
    ("notionists", dicebear_styles::NOTIONISTS),
    ("open-peeps", dicebear_styles::OPEN_PEEPS),
    ("thumbs", dicebear_styles::THUMBS),
    ("pixel-art", dicebear_styles::PIXEL_ART),
    ("voxel-bot", dicebear_styles::VOXEL_BOT),
    ("clay", dicebear_styles::CLAY),
    ("critters", dicebear_styles::CRITTERS),
    ("moods", dicebear_styles::MOODS),
    ("sprouts", dicebear_styles::SPROUTS),
    ("cutouts", dicebear_styles::CUTOUTS),
    ("marbles", dicebear_styles::MARBLES),
];

/// The style a teammate gets when setup offers a picture and no provider
/// can draw one: line drawn heads on a transparent ground, so the
/// teammate's own colour (the disc its initial sits on) shows through, as
/// in the house style.
pub(crate) const DEFAULT: &str = STYLES[0].0;

/// The longest seed taken from the wire.
const MAX_SEED: usize = 200;

/// Each style read once: reading one checks it against DiceBear's schema.
static PARSED: LazyLock<Vec<Style>> = LazyLock::new(|| {
    STYLES
        .iter()
        .map(|(_, definition)| {
            Style::from_str(definition).expect("a style shipped in dicebear-styles reads")
        })
        .collect()
});

/// The picture as DiceBear draws it.
pub(crate) fn svg(style: &str, seed: &str) -> Result<String, String> {
    let index = STYLES
        .iter()
        .position(|(id, _)| *id == style)
        .ok_or_else(|| format!("Hotline does not draw the style “{style}”."))?;
    if seed.is_empty() || seed.chars().count() > MAX_SEED {
        return Err(format!("A seed is 1 to {MAX_SEED} characters."));
    }
    let avatar = Avatar::new(&PARSED[index], json!({ "seed": seed }))
        .map_err(|_| "The picture could not be drawn.".to_string())?;
    Ok(avatar.to_svg().to_string())
}

/// The picture as it is kept: the drawing filling a 512 px square, as a PNG,
/// transparent wherever the style is.
pub(crate) fn png(style: &str, seed: &str) -> Result<Vec<u8>, String> {
    let svg = svg(style, seed)?;
    let failed = || "The picture could not be drawn.".to_string();
    // DiceBear's styles embed no images; refusing any keeps a drawing from
    // ever reading a file.
    let options = usvg::Options {
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_str(&svg, &options).map_err(|_| failed())?;
    let mut pixmap = tiny_skia::Pixmap::new(EDGE, EDGE).ok_or_else(failed)?;
    let size = tree.size();
    let edge = EDGE as f32;
    let fit = tiny_skia::Transform::from_scale(edge / size.width(), edge / size.height());
    resvg::render(&tree, fit, &mut pixmap.as_mut());
    pixmap.encode_png().map_err(|_| failed())
}

/// One picture per style for the person to choose from, all from one seed
/// that the teammate and the round make, so asking again answers the same
/// set and the next round a fresh one.
pub(crate) fn candidates(persona_id: &str, round: u32) -> Vec<AvatarCandidate> {
    let seed = format!("{persona_id}:{round}");
    STYLES
        .iter()
        .filter_map(|(style, _)| {
            Some(AvatarCandidate {
                style: (*style).to_string(),
                seed: seed.clone(),
                svg: svg(style, &seed).ok()?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_style_and_seed_draw_the_same_picture() {
        for (style, _) in STYLES {
            assert_eq!(svg(style, "ada").unwrap(), svg(style, "ada").unwrap());
            assert_ne!(svg(style, "ada").unwrap(), svg(style, "grace").unwrap());
        }
        assert_eq!(png(DEFAULT, "ada").unwrap(), png(DEFAULT, "ada").unwrap());
    }

    #[test]
    fn every_style_draws_a_square_png_with_something_in_it() {
        for (style, _) in STYLES {
            let png = png(style, "ada").unwrap();
            let picture = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
                .unwrap()
                .to_rgba8();
            assert_eq!(picture.dimensions(), (EDGE, EDGE), "{style}");
            let drawn = picture.pixels().filter(|pixel| pixel[3] > 0).count();
            assert!(drawn > (EDGE * EDGE / 10) as usize, "{style}: {drawn}");
        }
    }

    #[test]
    fn only_a_listed_style_and_a_bounded_seed_are_drawn() {
        for style in ["", "bottts", "adventurer", "../lorelei", "Lorelei"] {
            let error = svg(style, "ada").unwrap_err();
            assert!(error.contains("does not draw"), "{style}: {error}");
        }
        for seed in [String::new(), "x".repeat(MAX_SEED + 1)] {
            assert!(svg(DEFAULT, &seed).unwrap_err().starts_with("A seed is"));
        }
        assert!(svg(DEFAULT, &"x".repeat(MAX_SEED)).is_ok());
    }

    #[test]
    fn candidates_are_one_per_style_and_a_round_changes_them() {
        let first = candidates("ada", 0);
        assert_eq!(first.len(), STYLES.len());
        assert_eq!(first, candidates("ada", 0));
        let next = candidates("ada", 1);
        for (one, other) in first.iter().zip(&next) {
            assert_eq!(one.style, other.style);
            assert_ne!(one.svg, other.svg, "{}", one.style);
        }
        assert_ne!(first[0].svg, candidates("grace", 0)[0].svg);
        for candidate in &first {
            assert_eq!(
                svg(&candidate.style, &candidate.seed).unwrap(),
                candidate.svg
            );
        }
    }

    /// Nothing here may need credit where it is shown: each style's own
    /// definition names its licence, and every one drawn is CC0.
    #[test]
    fn every_style_drawn_is_cc0() {
        for (style, definition) in STYLES {
            let definition: serde_json::Value = serde_json::from_str(definition).unwrap();
            assert_eq!(definition["meta"]["license"]["name"], "CC0 1.0", "{style}");
        }
    }
}
