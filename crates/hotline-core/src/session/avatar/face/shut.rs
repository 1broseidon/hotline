//! A face's blink, frozen: DiceBear's drawing with its `blink` animation on,
//! held still at one moment of the loop, so the desk can keep the face with
//! its eyes shut as a picture of its own.
//!
//! DiceBear writes an animation as CSS: each animated element wrapped in a
//! `<g class="dba-…">` and a `<style>` of `@keyframes` (dicebear-core's
//! renderer emits every rule in one fixed form, the same in every port). A
//! picture is drawn by resvg, which plays no CSS animation, so each wrapper
//! here becomes the plain `opacity` or `transform` its keyframes give at that
//! moment, turned about the wrapper's own box as `transform-box: fill-box`
//! turns it. A rule in a form not read here leaves no frozen drawing rather
//! than a wrong one.

use regex::Regex;
use resvg::usvg;
use std::sync::LazyLock;

/// The animation's stylesheet, the only `<style>` DiceBear writes this way.
static MOTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"<style>@media \(prefers-reduced-motion:no-preference\)\{(.*?)\}</style>")
        .expect("a valid pattern")
});
static KEYFRAMES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"@keyframes ([\w-]+)\{((?:[\d.]+%\{[^}]*\})+)\}").expect("a valid pattern")
});
static FRAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([\d.]+)%\{([^}]*)\}").expect("a valid pattern"));
static DECLARATION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?:transform:(translateX|translateY|rotate|scaleX|scaleY)\((-?[\d.]+)(?:px|deg)?\)|opacity:(-?[\d.]+))(?:;animation-timing-function:([\w-]+))?$",
    )
    .expect("a valid pattern")
});
static RULE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"\.(dba-[\w-]+)\{(?:transform-box:fill-box;transform-origin:(-?[\d.]+)% (-?[\d.]+)%;)?animation:[\d.]+s ([\w-]+) -?[\d.]+s [\w.]+ [\w-]+ [\w-]+ ([\w-]+)\}",
    )
    .expect("a valid pattern")
});

/// One keyframe: where in the loop, in percent, what the track is there,
/// and how it moves on to the next (`linear` or `step-end`).
struct Frame {
    at: f64,
    value: f64,
    easing: String,
}

/// One animated wrapper: its class, the track it animates, the frames of
/// that track, and the point of its own box it turns about, in percent.
struct Track {
    class: String,
    property: String,
    frames: Vec<Frame>,
    origin: Option<(f64, f64)>,
}

/// `svg`, drawn with only its blink playing, held at `at` percent of the
/// blink's loop, with the stylesheet gone. None when the drawing has no
/// animation or one this cannot hold still.
pub(super) fn frozen(svg: &str, at: f64, options: &usvg::Options) -> Option<String> {
    let motion = MOTION.captures(svg)?;
    let tracks = tracks(&motion[1])?;
    if tracks.is_empty() {
        return None;
    }
    let still = MOTION.replace(svg, "").into_owned();
    let boxes = boxes(&still, &tracks, options)?;
    let mut frozen = still;
    for (track, centre) in tracks.iter().zip(boxes) {
        let value = value_at(&track.frames, at)?;
        let held = match (track.property.as_str(), centre) {
            ("opacity", _) => format!(r#"opacity="{value}""#),
            ("translateX", _) => format!(r#"transform="translate({value} 0)""#),
            ("translateY", _) => format!(r#"transform="translate(0 {value})""#),
            ("rotate", Some((x, y))) => format!(r#"transform="rotate({value} {x} {y})""#),
            ("scaleX", Some((x, y))) => {
                format!(
                    r#"transform="translate({x} {y}) scale({value} 1) translate({} {})""#,
                    -x, -y
                )
            }
            ("scaleY", Some((x, y))) => {
                format!(
                    r#"transform="translate({x} {y}) scale(1 {value}) translate({} {})""#,
                    -x, -y
                )
            }
            _ => return None,
        };
        // An opacity track's wrapper carries the element's resting opacity,
        // which the held value replaces.
        let wrapper = Regex::new(&format!(
            r#"<g class="{}"(?: opacity="[^"]*")?"#,
            regex::escape(&track.class)
        ))
        .ok()?;
        if !wrapper.is_match(&frozen) {
            return None;
        }
        frozen = wrapper
            .replace(&frozen, format!("<g {held}").as_str())
            .into_owned();
    }
    Some(frozen)
}

/// The tracks a stylesheet animates, or None when a rule is not in the form
/// DiceBear writes.
fn tracks(css: &str) -> Option<Vec<Track>> {
    let mut keyframes = Vec::new();
    for found in KEYFRAMES.captures_iter(css) {
        let mut frames = Vec::new();
        let mut property = String::new();
        for frame in FRAME.captures_iter(&found[2]) {
            let declaration = DECLARATION.captures(&frame[2])?;
            let (name, value) = match declaration.get(1) {
                Some(name) => (name.as_str(), &declaration[2]),
                None => ("opacity", &declaration[3]),
            };
            property = name.to_string();
            frames.push(Frame {
                at: frame[1].parse().ok()?,
                value: value.parse().ok()?,
                easing: declaration
                    .get(4)
                    .map_or_else(String::new, |easing| easing.as_str().to_string()),
            });
        }
        keyframes.push((found[1].to_string(), property, frames));
    }
    let mut tracks = Vec::new();
    for rule in RULE.captures_iter(css) {
        let (_, property, frames) = keyframes.iter().find(|(name, _, _)| *name == rule[5])?;
        let origin = match (rule.get(2), rule.get(3)) {
            (Some(x), Some(y)) => Some((x.as_str().parse().ok()?, y.as_str().parse().ok()?)),
            _ => None,
        };
        tracks.push(Track {
            class: rule[1].to_string(),
            property: property.clone(),
            frames: frames
                .iter()
                .map(|frame| Frame {
                    at: frame.at,
                    value: frame.value,
                    easing: if frame.easing.is_empty() {
                        rule[4].to_string()
                    } else {
                        frame.easing.clone()
                    },
                })
                .collect(),
            origin,
        });
    }
    Some(tracks)
}

/// For each track that turns about a point, that point in the wrapper's own
/// space: its origin's share of the box round what the wrapper holds. None
/// when a box cannot be found.
fn boxes(
    still: &str,
    tracks: &[Track],
    options: &usvg::Options,
) -> Option<Vec<Option<(f64, f64)>>> {
    if tracks.iter().all(|track| track.origin.is_none()) {
        return Some(vec![None; tracks.len()]);
    }
    let mut named = still.to_string();
    for track in tracks {
        named = named.replace(
            &format!(r#"<g class="{}""#, track.class),
            &format!(r#"<g id="{}""#, track.class),
        );
    }
    let tree = usvg::Tree::from_str(&named, options).ok()?;
    tracks
        .iter()
        .map(|track| match track.origin {
            None => Some(None),
            Some((x, y)) => {
                let bounds = tree.node_by_id(&track.class)?.bounding_box()?;
                Some(Some((
                    f64::from(bounds.left()) + f64::from(bounds.width()) * x / 100.0,
                    f64::from(bounds.top()) + f64::from(bounds.height()) * y / 100.0,
                )))
            }
        })
        .collect()
}

/// The track's value `at` percent into its loop. DiceBear pads every track
/// with frames at 0 and 100, so some segment always holds `at`.
fn value_at(frames: &[Frame], at: f64) -> Option<f64> {
    let last = frames.last()?;
    if at >= last.at {
        return Some(last.value);
    }
    let index = frames
        .windows(2)
        .position(|pair| pair[0].at <= at && at < pair[1].at)?;
    let (from, to) = (&frames[index], &frames[index + 1]);
    match from.easing.as_str() {
        "step-end" => Some(from.value),
        "linear" => Some(from.value + (to.value - from.value) * (at - from.at) / (to.at - from.at)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STILL: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><defs><style>@media (prefers-reduced-motion:no-preference){@keyframes dbk-a-0{0%{transform:scaleY(1)}50%{transform:scaleY(0.2)}100%{transform:scaleY(1)}}@keyframes dbk-a-1{0%{opacity:1;animation-timing-function:step-end}40%{opacity:0;animation-timing-function:step-end}60%{opacity:1}100%{opacity:1}}.dba-a-0{transform-box:fill-box;transform-origin:50% 50%;animation:4s linear 0s infinite normal none dbk-a-0}.dba-a-1{animation:4s linear 0s infinite normal none dbk-a-1}}</style></defs><g class="dba-a-0"><rect x="10" y="20" width="30" height="40"/></g><g class="dba-a-1" opacity=".5"><circle cx="70" cy="70" r="5"/></g></svg>"#;

    /// A squash is held about the wrapper's own centre, interpolated between
    /// its frames, and a stepped opacity holds the frame it stepped to,
    /// replacing the resting opacity; the stylesheet goes.
    #[test]
    fn a_blink_is_held_at_its_moment() {
        let frozen = frozen(STILL, 45.0, &usvg::Options::default()).unwrap();
        assert!(!frozen.contains("<style>"), "{frozen}");
        assert!(
            frozen.contains(r#"<g transform="translate(25 40) scale(1 0.28"#),
            "{frozen}"
        );
        assert!(frozen.contains(") translate(-25 -40)\">"), "{frozen}");
        assert!(frozen.contains(r#"<g opacity="0">"#), "{frozen}");
        assert!(!frozen.contains(r#"opacity=".5""#), "{frozen}");
    }

    /// A drawing with no animation, or one in a form not read here, has no
    /// frozen moment.
    #[test]
    fn nothing_is_held_that_is_not_a_blink_in_dicebears_form() {
        let options = usvg::Options::default();
        assert!(
            frozen(
                r#"<svg xmlns="http://www.w3.org/2000/svg"/>"#,
                50.0,
                &options
            )
            .is_none()
        );
        let eased = STILL.replace(
            "animation:4s linear 0s infinite normal none dbk-a-0",
            "animation:4s ease 0s infinite normal none dbk-a-0",
        );
        assert!(frozen(&eased, 45.0, &options).is_none());
    }
}
