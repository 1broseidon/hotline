//! A teammate's face, drawn on the desk from one of DiceBear's styles, with
//! no image provider and no network: the `face_options` and `make_face`
//! tools, and the face setup gives a new teammate.
//!
//! A face is a recipe ([`AvatarRecipe`]): a style, the variant of each of its
//! parts, the colour of each of its slots, and a seed. The kept PNG is its
//! rendering. Whatever the teammate leaves unchosen is filled from the seed,
//! and the recipe keeps what was filled, so the same recipe draws the same
//! face after the crate changes how it fills one. Asked again in the same
//! style, the stored recipe is the starting point and only what the call
//! names changes; a new style, or a new seed, starts afresh.
//!
//! Only styles whose artwork is CC0 1.0 are here (and enabled in
//! Cargo.toml), so nothing needs credit where a face is shown.

use super::EDGE;
use crate::contract::{AvatarRecipe, FaceMaker, Persona};
use dicebear_core::{Avatar, Style};
use resvg::{tiny_skia, usvg};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::LazyLock;

/// One style the desk draws, as a teammate is told of it.
struct Kept {
    id: &'static str,
    definition: &'static str,
    /// What it draws, in a few words.
    draws: &'static str,
    person: bool,
    /// How its parts' variants are named: `named` (`googly`, `antenna`), or
    /// numbered (`variant17`), where only the picture says what one is.
    names: &'static str,
    /// The slot a face the desk seeds at setup takes the teammate's bubble
    /// colour in. Only creature styles with named parts are seeded.
    seeded_in: Option<&'static str>,
}

/// The styles, creatures first. Each reads as a character in a 28 px circle.
const STYLES: [Kept; 15] = [
    Kept {
        id: "clay",
        definition: dicebear_styles::CLAY,
        draws: "a soft clay creature on a pale ground",
        person: false,
        names: "named",
        seeded_in: Some("body"),
    },
    Kept {
        id: "critters",
        definition: dicebear_styles::CRITTERS,
        draws: "a flat, bold monster on a deep ground",
        person: false,
        names: "named",
        seeded_in: Some("body"),
    },
    Kept {
        id: "moods",
        definition: dicebear_styles::MOODS,
        draws: "a rounded face showing a mood",
        person: false,
        names: "named",
        seeded_in: Some("face"),
    },
    Kept {
        id: "sprouts",
        definition: dicebear_styles::SPROUTS,
        draws: "a potted plant with a face",
        person: false,
        names: "named",
        seeded_in: Some("pot"),
    },
    Kept {
        id: "voxel-bot",
        definition: dicebear_styles::VOXEL_BOT,
        draws: "a blocky 3D robot",
        person: false,
        names: "named",
        seeded_in: Some("body"),
    },
    Kept {
        id: "marbles",
        definition: dicebear_styles::MARBLES,
        draws: "a glossy marble with a face, and hair or a hat",
        person: false,
        names: "named",
        seeded_in: Some("sphere"),
    },
    Kept {
        id: "cats",
        definition: dicebear_styles::CATS,
        draws: "a cat sitting in a small scene",
        person: false,
        names: "named",
        seeded_in: Some("collar"),
    },
    Kept {
        id: "dogs",
        definition: dicebear_styles::DOGS,
        draws: "a dog sitting in a small scene",
        person: false,
        names: "named",
        seeded_in: Some("collar"),
    },
    Kept {
        id: "thumbs",
        definition: dicebear_styles::THUMBS,
        draws: "a simple rounded shape with a face",
        person: false,
        names: "numbered",
        seeded_in: None,
    },
    Kept {
        id: "open-peeps",
        definition: dicebear_styles::OPEN_PEEPS,
        draws: "a hand-drawn person, head and shoulders",
        person: true,
        names: "named",
        seeded_in: None,
    },
    Kept {
        id: "voxel-art",
        definition: dicebear_styles::VOXEL_ART,
        draws: "a blocky 3D person",
        person: true,
        names: "named",
        seeded_in: None,
    },
    Kept {
        id: "cutouts",
        definition: dicebear_styles::CUTOUTS,
        draws: "a face cut from coloured paper",
        person: true,
        names: "named",
        seeded_in: None,
    },
    Kept {
        id: "pixel-art",
        definition: dicebear_styles::PIXEL_ART,
        draws: "a pixel-art person",
        person: true,
        names: "partly numbered",
        seeded_in: None,
    },
    Kept {
        id: "lorelei",
        definition: dicebear_styles::LORELEI,
        draws: "a head drawn in black line",
        person: true,
        names: "numbered",
        seeded_in: None,
    },
    Kept {
        id: "notionists",
        definition: dicebear_styles::NOTIONISTS,
        draws: "a person drawn in black line, to the waist",
        person: true,
        names: "numbered",
        seeded_in: None,
    },
];

/// What a call names for a part to leave it off; `null` does the same. No
/// style has a variant by this name.
const LEFT_OFF: &str = "none";

/// The ids of the styles, for the tool's schema.
pub(crate) fn style_ids() -> Vec<&'static str> {
    STYLES.iter().map(|kept| kept.id).collect()
}

/// The longest seed, and the longest name of a part, a variant or a slot.
const MAX_SEED: usize = 200;
const MAX_NAME: usize = 64;

/// The bubble colours by name, as the window draws a bubble's mark in its
/// light palette (ui/src/tokens.css, `oklch(72% 0.10 <hue>)`, grey at 0.01).
const BUBBLE_MARKS: [(&str, &str); 7] = [
    ("grey", "#a0a5ab"),
    ("lime", "#a5ab5f"),
    ("green", "#6db78a"),
    ("teal", "#48b8bc"),
    ("blue", "#69acde"),
    ("violet", "#9e9ce1"),
    ("pink", "#c78ec4"),
];

/// One of a style's parts: its variants, how often it is drawn when left
/// to the seed, in percent, and the other part it sits on when only some of
/// that part's variants hold it (a clay `top` sits on most `body` variants,
/// not all).
struct Part {
    name: String,
    variants: Vec<String>,
    chance: f64,
    only_with: Option<(String, Vec<String>)>,
}

/// One of a style's colour slots, with the palette the seed picks from;
/// `background`, which every style has, has none unless the style gives it
/// one, and is transparent when unchosen.
struct Slot {
    name: String,
    palette: Vec<String>,
}

struct Parsed {
    style: Style,
    parts: Vec<Part>,
    slots: Vec<Slot>,
}

/// Each style read once: reading one checks it against DiceBear's schema.
/// Its parts and palettes are read from the definition itself, which the
/// crate does not expose.
static PARSED: LazyLock<Vec<Parsed>> =
    LazyLock::new(|| STYLES.iter().map(|kept| parse(kept.definition)).collect());

fn parse(definition: &str) -> Parsed {
    let style = Style::from_str(definition).expect("a style shipped in dicebear-styles reads");
    let raw: Value =
        serde_json::from_str(definition).expect("a style shipped in dicebear-styles is JSON");
    let components = raw["components"].as_object().cloned().unwrap_or_default();
    let parts = components
        .iter()
        .filter(|(_, component)| component.get("extends").is_none())
        .map(|(name, component)| Part {
            name: name.clone(),
            variants: component["variants"]
                .as_object()
                .map(|variants| variants.keys().cloned().collect())
                .unwrap_or_default(),
            chance: component["probability"].as_f64().unwrap_or(100.0),
            only_with: only_with(&raw["canvas"], &components, name),
        })
        .collect();
    let mut slots: Vec<Slot> = raw["colors"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(name, color)| Slot {
            name: name.clone(),
            palette: color["values"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter_map(hex)
                .collect(),
        })
        .collect();
    if !slots.iter().any(|slot| slot.name == "background") {
        slots.push(Slot {
            name: "background".to_string(),
            palette: Vec::new(),
        });
    }
    Parsed {
        style,
        parts,
        slots,
    }
}

/// The part `name` sits on and that part's variants which hold it, when it
/// is not on the canvas itself and some of them do not.
fn only_with(
    canvas: &Value,
    components: &Map<String, Value>,
    name: &str,
) -> Option<(String, Vec<String>)> {
    if holds(canvas, name) {
        return None;
    }
    components.iter().find_map(|(parent, component)| {
        let variants = component["variants"].as_object()?;
        let holding: Vec<String> = variants
            .iter()
            .filter(|(_, variant)| holds(variant, name))
            .map(|(variant, _)| variant.clone())
            .collect();
        (!holding.is_empty() && holding.len() < variants.len()).then(|| (parent.clone(), holding))
    })
}

/// Whether a piece of a definition places the component `name`.
fn holds(piece: &Value, name: &str) -> bool {
    match piece {
        Value::Object(fields) => {
            (fields.get("type").and_then(Value::as_str) == Some("component")
                && fields.get("name").and_then(Value::as_str) == Some(name))
                || fields.values().any(|field| holds(field, name))
        }
        Value::Array(items) => items.iter().any(|item| holds(item, name)),
        _ => false,
    }
}

/// A style by its id, or the refusal that names the ones there are.
fn style(id: &str) -> Result<(&'static Kept, &'static Parsed), String> {
    STYLES
        .iter()
        .zip(PARSED.iter())
        .find(|(kept, _)| kept.id == id)
        .ok_or_else(|| {
            let ids: Vec<&str> = STYLES.iter().map(|kept| kept.id).collect();
            format!(
                "There is no style called “{}”. The styles are: {}.",
                clipped(id),
                ids.join(", ")
            )
        })
}

/// `#rrggbb`, lowercased; nothing else is a colour here.
fn hex(value: &str) -> Option<String> {
    let digits = value.strip_prefix('#')?;
    (digits.len() == 6 && digits.chars().all(|digit| digit.is_ascii_hexdigit()))
        .then(|| format!("#{}", digits.to_ascii_lowercase()))
}

/// A name from the call, cut short enough to repeat in a refusal.
fn clipped(name: &str) -> String {
    name.chars().take(MAX_NAME).collect()
}

/// What a `make_face` call asks for, checked against its style.
pub(crate) struct Ask {
    style: String,
    parts: BTreeMap<String, Option<String>>,
    colors: BTreeMap<String, String>,
    seed: Option<String>,
}

impl Ask {
    /// The call's arguments, every name checked against the style, every
    /// colour a `#rrggbb` and every string bounded. A refusal names what is
    /// allowed, so the next call can differ.
    pub(crate) fn read(arguments: &Value) -> Result<Self, String> {
        let id = arguments
            .get("style")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| "make_face needs a `style`; face_options lists them.".to_string())?;
        let (kept, parsed) = style(id)?;
        let mut parts = BTreeMap::new();
        for (name, variant) in object(arguments, "parts")? {
            let part = parsed
                .parts
                .iter()
                .find(|part| part.name == *name)
                .ok_or_else(|| {
                    let names: Vec<&str> =
                        parsed.parts.iter().map(|part| part.name.as_str()).collect();
                    format!(
                        "{} has no part called “{}”. Its parts are: {}.",
                        kept.id,
                        clipped(name),
                        names.join(", ")
                    )
                })?;
            let variant = match variant {
                Value::Null => None,
                Value::String(none) if none == LEFT_OFF => None,
                Value::String(variant) if part.variants.contains(variant) => Some(variant.clone()),
                other => {
                    return Err(format!(
                        "{}'s {} has no variant {}. Use one of: {}; or \"none\" to leave it off.",
                        kept.id,
                        part.name,
                        clipped(&other.to_string()),
                        part.variants.join(", ")
                    ));
                }
            };
            parts.insert(part.name.clone(), variant);
        }
        let mut colors = BTreeMap::new();
        for (name, value) in object(arguments, "colors")? {
            let slot = parsed
                .slots
                .iter()
                .find(|slot| slot.name == *name)
                .ok_or_else(|| {
                    let names: Vec<&str> =
                        parsed.slots.iter().map(|slot| slot.name.as_str()).collect();
                    format!(
                        "{} has no colour slot called “{}”. Its slots are: {}.",
                        kept.id,
                        clipped(name),
                        names.join(", ")
                    )
                })?;
            let color = value.as_str().and_then(hex).ok_or_else(|| {
                format!(
                    "The {} colour must be a #rrggbb hex colour, such as #69acde; {} is not.",
                    slot.name,
                    clipped(&value.to_string())
                )
            })?;
            colors.insert(slot.name.clone(), color);
        }
        let seed = match arguments.get("seed") {
            None | Some(Value::Null) => None,
            Some(Value::String(seed))
                if !seed.trim().is_empty() && seed.chars().count() <= MAX_SEED =>
            {
                Some(seed.clone())
            }
            Some(_) => return Err(format!("A seed is a string of 1 to {MAX_SEED} characters.")),
        };
        Ok(Self {
            style: kept.id.to_string(),
            parts,
            colors,
            seed,
        })
    }
}

/// The object under `key`, or none; anything else there is refused.
fn object<'a>(arguments: &'a Value, key: &str) -> Result<Vec<(&'a String, &'a Value)>, String> {
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Object(entries)) => Ok(entries.iter().collect()),
        Some(_) => Err(format!(
            "`{key}` is an object of names to values, such as {{\"eyes\": \"happy\"}}."
        )),
    }
}

/// A face: its recipe, the PNG it draws, and what the call chose that is
/// not in the picture, because the part it belongs on is not drawn.
pub(crate) struct Face {
    pub(crate) recipe: AvatarRecipe,
    pub(crate) png: Vec<u8>,
    pub(crate) not_drawn: Vec<String>,
}

/// The face `ask` makes. In the style of `current` with its seed (or none
/// named), `current`'s parts and colours stand but for what `ask` names;
/// otherwise it starts afresh, from `ask`'s seed or `default_seed`.
pub(crate) fn compose(
    ask: Ask,
    current: Option<&AvatarRecipe>,
    default_seed: &str,
    made_by: FaceMaker,
) -> Result<Face, String> {
    let (_, parsed) = style(&ask.style)?;
    let kept = current.filter(|recipe| {
        recipe.style == ask.style && ask.seed.as_ref().is_none_or(|seed| *seed == recipe.seed)
    });
    let seed = ask
        .seed
        .or_else(|| kept.map(|recipe| recipe.seed.clone()))
        .unwrap_or_else(|| default_seed.to_string());
    let mut parts = BTreeMap::new();
    let mut colors = BTreeMap::new();
    if let Some(recipe) = kept {
        // A part or slot a later crate dropped is not carried on.
        parts.extend(recipe.parts.iter().filter_map(|(name, variant)| {
            let part = parsed.parts.iter().find(|part| part.name == *name)?;
            variant
                .as_ref()
                .is_none_or(|variant| part.variants.contains(variant))
                .then(|| (name.clone(), variant.clone()))
        }));
        colors.extend(
            recipe
                .colors
                .iter()
                .filter(|(name, _)| parsed.slots.iter().any(|slot| slot.name == **name))
                .map(|(name, color)| (name.clone(), color.clone())),
        );
    }
    let asked_parts = ask.parts;
    let asked_colors = ask.colors;
    parts.extend(asked_parts.clone());
    colors.extend(asked_colors.clone());
    let drawn = draw(parsed, &parts, &colors, &seed)?;
    let resolved = &drawn.to_json()["options"];
    let recipe = AvatarRecipe {
        style: ask.style,
        parts: parsed
            .parts
            .iter()
            .map(|part| {
                let variant = first(&resolved[format!("{}Variant", part.name)]);
                (part.name.clone(), variant)
            })
            .collect(),
        colors: parsed
            .slots
            .iter()
            .filter_map(|slot| {
                let color = first(&resolved[format!("{}Color", slot.name)])?;
                Some((slot.name.clone(), hex(&color)?))
            })
            .collect(),
        seed,
        made_by,
    };
    let mut not_drawn = Vec::new();
    for (name, variant) in asked_parts {
        let Some(variant) = variant else { continue };
        if recipe.parts.get(&name) != Some(&Some(variant.clone())) {
            let part = parsed.parts.iter().find(|part| part.name == name);
            let place = part
                .and_then(|part| part.only_with.as_ref())
                .map(|(parent, holding)| {
                    format!(": it shows only with {parent} {}", holding.join(", "))
                })
                .unwrap_or_default();
            not_drawn.push(format!("{name} {variant}{place}"));
        }
    }
    for name in asked_colors.keys() {
        if !recipe.colors.contains_key(name) {
            not_drawn.push(format!("the {name} colour: nothing drawn uses it"));
        }
    }
    let png = png(&recipe)?;
    Ok(Face {
        recipe,
        png,
        not_drawn,
    })
}

/// A resolved option's value: a string, or the first of a list.
fn first(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Array(items) => items.first().and_then(Value::as_str).map(str::to_string),
        _ => None,
    }
}

/// DiceBear's drawing of the chosen parts and colours, the rest from `seed`.
fn draw(
    parsed: &Parsed,
    parts: &BTreeMap<String, Option<String>>,
    colors: &BTreeMap<String, String>,
    seed: &str,
) -> Result<Avatar, String> {
    let mut options = Map::new();
    options.insert("seed".into(), json!(seed));
    for (name, variant) in parts {
        match variant {
            Some(variant) => {
                options.insert(format!("{name}Variant"), json!(variant));
                options.insert(format!("{name}Probability"), json!(100));
            }
            None => {
                options.insert(format!("{name}Probability"), json!(0));
            }
        }
    }
    for (name, color) in colors {
        options.insert(format!("{name}Color"), json!([color]));
    }
    Avatar::new(&parsed.style, Value::Object(options))
        .map_err(|_| "The face could not be drawn.".to_string())
}

/// The recipe as SVG.
pub(crate) fn svg(recipe: &AvatarRecipe) -> Result<String, String> {
    let (_, parsed) = style(&recipe.style)?;
    let drawn = draw(parsed, &recipe.parts, &recipe.colors, &recipe.seed)?;
    Ok(drawn.to_svg().to_string())
}

/// The recipe as it is kept: the drawing filling a 512 px square, as a PNG,
/// transparent wherever the style is.
pub(crate) fn png(recipe: &AvatarRecipe) -> Result<Vec<u8>, String> {
    let svg = svg(recipe)?;
    let failed = || "The face could not be drawn.".to_string();
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

/// The face setup gives a teammate at once: a creature style with named
/// parts picked from its id, in its bubble colour where the style has a
/// slot for it, the rest from its id. Seeded by the desk, so the teammate
/// knows to make its own.
pub(crate) fn seeded(persona: &Persona) -> Result<Face, String> {
    let pool: Vec<&Kept> = STYLES
        .iter()
        .filter(|kept| kept.seeded_in.is_some())
        .collect();
    let digest = Sha256::digest(persona.id.as_bytes());
    let kept = pool[usize::from(digest[0]) % pool.len()];
    let (_, color) = bubble(persona);
    let (_, parsed) = style(kept.id)?;
    // A slot named for an optional part (a cat's collar) is only drawn with
    // it, so the part is put on.
    let parts = parsed
        .parts
        .iter()
        .filter(|part| Some(part.name.as_str()) == kept.seeded_in)
        .map(|part| {
            let variant = &part.variants[usize::from(digest[1]) % part.variants.len()];
            (part.name.clone(), Some(variant.clone()))
        })
        .collect();
    let ask = Ask {
        style: kept.id.to_string(),
        parts,
        colors: kept
            .seeded_in
            .map(|slot| (slot.to_string(), color.to_string()))
            .into_iter()
            .collect(),
        seed: None,
    };
    compose(ask, None, &persona.id, FaceMaker::Desk)
}

/// The colour a teammate's bubbles are, by name and as `#rrggbb`: the one
/// the person picked, else its own, from its id as the window works it out
/// (`ownBubble` in ui/src/bubbles.ts).
pub(crate) fn bubble(persona: &Persona) -> (&'static str, &'static str) {
    let named = persona.bubble.as_deref().and_then(|name| {
        BUBBLE_MARKS
            .iter()
            .find(|(bubble, _)| *bubble == name)
            .copied()
    });
    named.unwrap_or_else(|| {
        let mut hash: u64 = 0;
        for unit in persona.id.encode_utf16() {
            hash = (hash * 31 + u64::from(unit)) % 1_000_003;
        }
        // Face hue 0 is gold, which has no bubble of its own: grey. The
        // rest are the bubbles in hue order.
        let index = usize::try_from(hash % 7).expect("less than seven");
        BUBBLE_MARKS[index]
    })
}

/// `face_options` with no style: every style, the teammate's face now, and
/// its bubble colour.
pub(crate) fn styles_answer(persona: &Persona) -> Value {
    let (name, color) = bubble(persona);
    let styles: Vec<Value> = STYLES
        .iter()
        .map(|kept| {
            json!({
                "style": kept.id,
                "draws": kept.draws,
                "kind": if kept.person { "person" } else { "creature" },
                "parts": kept.names,
            })
        })
        .collect();
    let current = match &persona.avatar {
        Some(avatar) => match &avatar.recipe {
            Some(recipe) => json!(recipe),
            None => json!("a picture that is not a face from these styles"),
        },
        None => json!("none: people see your initial"),
    };
    json!({
        "styles": styles,
        "current": current,
        "bubble": { "name": name, "hex": color },
        "note": "Ask with a style for its parts and colours. Numbered parts only say what they are once you see the picture make_face returns.",
    })
}

/// `face_options` for one style: its parts with their variants, how often
/// an optional part is drawn when left to the seed, and its colour slots
/// with their palettes.
pub(crate) fn style_answer(id: &str) -> Result<Value, String> {
    let (kept, parsed) = style(id)?;
    let parts: Vec<Value> = parsed
        .parts
        .iter()
        .map(|part| {
            let mut entry = json!({ "part": part.name, "variants": part.variants });
            if part.chance < 100.0 {
                entry["drawnPercent"] = json!(part.chance);
            }
            if let Some((parent, holding)) = &part.only_with {
                entry["onlyWith"] = json!({ parent: holding });
            }
            entry
        })
        .collect();
    let colors: Vec<Value> = parsed
        .slots
        .iter()
        .map(|slot| json!({ "slot": slot.name, "palette": slot.palette }))
        .collect();
    Ok(json!({
        "style": kept.id,
        "draws": kept.draws,
        "kind": if kept.person { "person" } else { "creature" },
        "parts": parts,
        "colors": colors,
        "note": "A part you leave out of make_face is filled from the seed; \"none\" leaves it off. drawnPercent is how often an optional part is drawn when left to the seed; onlyWith names the variants of another part it can sit on. A colour takes any #rrggbb, not only its palette; background is transparent unless chosen or given a palette.",
    }))
}

#[cfg(test)]
mod tests;
