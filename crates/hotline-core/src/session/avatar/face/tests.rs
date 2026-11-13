use super::*;
use crate::contract::{McpPolicy, PolicyMode, SkillPolicy};

fn ask(arguments: Value) -> Ask {
    Ask::read(&arguments, None).unwrap()
}

fn made(arguments: Value, current: Option<&AvatarRecipe>) -> Face {
    compose(ask(arguments), current, "ada", FaceMaker::Teammate).unwrap()
}

fn persona(id: &str, bubble: Option<&str>) -> Persona {
    Persona {
        node: None,
        id: id.into(),
        name: "Ada".into(),
        goal: String::new(),
        avatar: None,
        team: None,
        bubble: bubble.map(str::to_string),
        backend_id: "hotline".into(),
        cwd: "/work".into(),
        reach: None,
        model_id: None,
        mode_id: None,
        effort_id: None,
        config_choices: BTreeMap::new(),
        effort_by_model: BTreeMap::new(),
        harness_override: None,
        hop_notice: None,
        mcp_policy: McpPolicy {
            mode: PolicyMode::None,
            server_ids: Vec::new(),
        },
        skill_policy: SkillPolicy::default(),
        background_work: false,
        allowed_senders: Vec::new(),
        web_search_policy: None,
        folders: None,
        computer: None,
        secrets: None,
        voice: None,
        session_checkpoints: Vec::new(),
        last_session_id: None,
        created_at: 1,
        updated_at: 1,
    }
}

/// Every style draws a 512 px square with something in it, and its recipe
/// names every one of its parts, with the ones the seed left off as none.
#[test]
fn every_style_draws_a_square_face_and_a_whole_recipe() {
    for (kept, parsed) in STYLES.iter().zip(PARSED.iter()) {
        let face = made(json!({ "style": kept.id }), None);
        let picture = image::load_from_memory_with_format(&face.png, image::ImageFormat::Png)
            .unwrap()
            .to_rgba8();
        assert_eq!(picture.dimensions(), (EDGE, EDGE), "{}", kept.id);
        let drawn = picture.pixels().filter(|pixel| pixel[3] > 0).count();
        assert!(drawn > (EDGE * EDGE / 10) as usize, "{}: {drawn}", kept.id);
        let names: Vec<&String> = face.recipe.parts.keys().collect();
        let mut expected: Vec<&String> = parsed.parts.iter().map(|part| &part.name).collect();
        expected.sort();
        assert_eq!(names, expected, "{}", kept.id);
        assert!(
            face.recipe.parts.values().any(Option::is_some),
            "{}",
            kept.id
        );
        assert!(!face.recipe.colors.is_empty(), "{}", kept.id);
        assert_eq!(face.recipe.seed, "ada");
        assert_eq!(face.recipe.made_by, FaceMaker::Teammate);
    }
}

/// The same recipe draws the same face, and the recipe a face was made
/// from draws exactly that face again.
#[test]
fn a_recipe_draws_the_same_face_every_time() {
    let one = made(
        json!({ "style": "sprouts", "parts": { "eyes": "wide" } }),
        None,
    );
    let two = made(
        json!({ "style": "sprouts", "parts": { "eyes": "wide" } }),
        None,
    );
    assert_eq!(one.recipe, two.recipe);
    assert_eq!(one.png, two.png);
    assert_eq!(png(&one.recipe).unwrap(), one.png);
    let other = made(
        json!({ "style": "sprouts", "parts": { "eyes": "wide" }, "seed": "grace" }),
        None,
    );
    assert_eq!(other.recipe.parts["eyes"].as_deref(), Some("wide"));
    assert_ne!(other.png, one.png);
}

/// In the same style a call changes only what it names; a part can be left
/// off; a new style, or a new seed, starts afresh.
#[test]
fn the_same_style_keeps_what_is_not_named_and_another_starts_afresh() {
    let first = made(
        json!({ "style": "sprouts", "colors": { "pot": "#69ACDE" } }),
        None,
    );
    assert_eq!(first.recipe.colors["pot"], "#69acde");

    let glasses = made(
        json!({ "style": "sprouts", "parts": { "eyes": "wide", "mouth": "grin" } }),
        Some(&first.recipe),
    );
    for (name, variant) in &glasses.recipe.parts {
        match name.as_str() {
            "eyes" => assert_eq!(variant.as_deref(), Some("wide")),
            "mouth" => assert_eq!(variant.as_deref(), Some("grin")),
            _ => assert_eq!(variant, &first.recipe.parts[name], "{name}"),
        }
    }
    assert_eq!(glasses.not_drawn, Vec::<String>::new());
    assert_eq!(glasses.recipe.colors, first.recipe.colors);
    assert_eq!(glasses.recipe.seed, first.recipe.seed);

    let bare = made(
        json!({ "style": "sprouts", "parts": { "cheeks": "none", "pattern": null } }),
        Some(&glasses.recipe),
    );
    assert_eq!(bare.recipe.parts["cheeks"], None);
    assert_eq!(bare.recipe.parts["pattern"], None);
    assert_eq!(bare.recipe.parts["eyes"].as_deref(), Some("wide"));

    // A colour for a part that is left off is said not to be drawn.
    let plain = made(
        json!({ "style": "sprouts", "parts": { "plant": "grass", "pattern": "none" }, "colors": { "bloom": "#48b8bc" } }),
        None,
    );
    assert_eq!(plain.recipe.parts["pattern"], None);
    assert!(!plain.recipe.colors.contains_key("bloom"));
    assert_eq!(plain.not_drawn, ["the bloom colour: nothing drawn uses it"]);

    let moods = made(json!({ "style": "moods" }), Some(&bare.recipe));
    assert_eq!(moods.recipe.style, "moods");
    assert!(moods.recipe.parts.contains_key("face"));
    assert!(!moods.recipe.parts.contains_key("body"));
    assert_eq!(moods.recipe, made(json!({ "style": "moods" }), None).recipe);

    let reseeded = made(
        json!({ "style": "sprouts", "seed": "grace" }),
        Some(&bare.recipe),
    );
    assert_eq!(
        reseeded.recipe,
        made(json!({ "style": "sprouts", "seed": "grace" }), None).recipe
    );
}

/// Every name is checked against the style and every colour is a #rrggbb;
/// a refusal names what would do.
#[test]
fn unknown_names_and_bad_colours_are_refused_with_what_would_do() {
    for (arguments, said) in [
        (json!({}), "needs a `style`"),
        (
            json!({ "style": "bottts" }),
            "The styles are: pixelbot, moods, sprouts, open-peeps.",
        ),
        (json!({ "style": "adventurer" }), "no style called"),
        (
            json!({ "style": "sprouts", "parts": { "hat": "tall" } }),
            "Its parts are: cheeks, eyes, mouth, pattern, plant, pot",
        ),
        (
            json!({ "style": "sprouts", "parts": { "eyes": "lasers" } }),
            "Use one of: angry, bigPupils",
        ),
        (
            json!({ "style": "sprouts", "parts": { "eyes": 7 } }),
            "has no variant 7",
        ),
        (
            json!({ "style": "sprouts", "parts": ["eyes"] }),
            "is an object",
        ),
        (
            json!({ "style": "sprouts", "colors": { "glow": "#ffffff" } }),
            "Its slots are: background, bloom, ink, plant, pot",
        ),
        (
            json!({ "style": "sprouts", "colors": { "pot": "red" } }),
            "#rrggbb",
        ),
        (
            json!({ "style": "sprouts", "colors": { "pot": "#12345" } }),
            "#rrggbb",
        ),
        (
            json!({ "style": "sprouts", "colors": { "pot": "#gggggg" } }),
            "#rrggbb",
        ),
        (
            json!({ "style": "sprouts", "colors": { "pot": "69acde" } }),
            "#rrggbb",
        ),
        (
            json!({ "style": "sprouts", "seed": "x".repeat(MAX_SEED + 1) }),
            "A seed is",
        ),
        (json!({ "style": "sprouts", "seed": " " }), "A seed is"),
    ] {
        let Err(error) = Ask::read(&arguments, None) else {
            panic!("{arguments} was taken");
        };
        assert!(error.contains(said), "{arguments}: {error}");
    }
    let long = "x".repeat(10_000);
    let error = Ask::read(&json!({ "style": "sprouts", "parts": { long: "a" } }), None)
        .err()
        .unwrap();
    assert!(error.len() < 400, "a refusal repeats a bounded name");
}

/// The desk's face at setup is a creature style wearing the teammate's
/// bubble colour, the rest from its id, and says the desk made it; every
/// creature style is in the pool.
#[test]
fn a_seeded_face_is_a_creature_in_the_bubble_colour() {
    let mut drawn = std::collections::BTreeSet::new();
    for id in [
        "ada",
        "grace",
        "linus",
        "mira",
        "4f1c2b9e-0000-4000-8000-000000000000",
    ] {
        for bubble_name in [None, Some("pink")] {
            let teammate = persona(id, bubble_name);
            let face = seeded(&teammate).unwrap();
            let kept = STYLES
                .iter()
                .find(|kept| kept.id == face.recipe.style)
                .unwrap();
            assert!(!kept.person, "{}", kept.id);
            for (slot, color) in bubble(&teammate).worn_by(kept) {
                assert_eq!(face.recipe.colors[&slot], color, "{id} {slot}");
            }
            drawn.insert(kept.id.to_string());
            assert_eq!(face.recipe.made_by, FaceMaker::Desk);
            assert_eq!(face.recipe.seed, id);
            assert_eq!(seeded(&teammate).unwrap().png, face.png);
        }
    }
    for n in 0..60 {
        drawn.insert(
            seeded(&persona(&format!("teammate-{n}"), None))
                .unwrap()
                .recipe
                .style,
        );
    }
    let creatures: std::collections::BTreeSet<String> = STYLES
        .iter()
        .filter(|kept| !kept.person)
        .map(|kept| kept.id.to_string())
        .collect();
    assert_eq!(drawn, creatures);
}

/// A teammate's own bubble is worked out as the window works it out
/// (`ownBubble` in ui/src/bubbles.ts), and one the person picked wins.
#[test]
fn the_bubble_is_the_windows() {
    // "ada": ((97 * 31 + 100) * 31 + 97) % 7 == 3, the fourth hue: teal.
    let named = |id: &str, picked: Option<&str>| {
        let bubble = bubble(&persona(id, picked));
        (bubble.name, bubble.mark)
    };
    assert_eq!(named("ada", None), ("teal", "#48b8bc"));
    assert_eq!(named("ada", Some("pink")), ("pink", "#c78ec4"));
    assert_eq!(named("ada", Some("grey")), ("grey", "#a0a5ab"));
    // A name the desk does not know is its own again, as in the window.
    assert_eq!(named("ada", Some("gold")), ("teal", "#48b8bc"));
}

/// What `face_options` says is what `make_face` takes: every style it lists
/// draws, and every part, variant and slot it lists for a style is taken.
#[test]
fn the_options_answer_what_make_face_takes() {
    let listed = styles_answer(&persona("ada", None));
    assert_eq!(listed["styles"].as_array().unwrap().len(), STYLES.len());
    assert_eq!(
        listed["bubble"],
        json!({ "name": "teal", "hex": "#48b8bc" })
    );
    assert_eq!(listed["current"], "none: people see your initial");
    for style in listed["styles"].as_array().unwrap() {
        let id = style["style"].as_str().unwrap();
        let answer = style_answer(id, &persona("ada", None)).unwrap();
        let mut parts = Map::new();
        for part in answer["parts"].as_array().unwrap() {
            let variants = part["variants"].as_array().unwrap();
            parts.insert(
                part["part"].as_str().unwrap().into(),
                variants[variants.len() - 1].clone(),
            );
        }
        let mut colors = Map::new();
        for slot in answer["colors"].as_array().unwrap() {
            colors.insert(slot["slot"].as_str().unwrap().into(), json!("#123456"));
        }
        let face = made(
            json!({ "style": id, "parts": parts, "colors": colors }),
            None,
        );
        for (name, variant) in &parts {
            assert_eq!(
                face.recipe.parts[name].as_deref(),
                variant.as_str(),
                "{id} {name}"
            );
        }
        assert!(!face.recipe.colors.is_empty(), "{id}");
        for (name, color) in &face.recipe.colors {
            assert_eq!(color, "#123456", "{id} {name}");
        }
    }
    assert!(style_answer("bottts", &persona("ada", None)).is_err());
    let sprouts = style_answer("sprouts", &persona("ada", None)).unwrap();
    let pattern = sprouts["parts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|part| part["part"] == "pattern")
        .unwrap();
    assert_eq!(pattern["drawnPercent"], 45.0);
}

/// The definitions read whole: no palette value dropped for not being a
/// #rrggbb, and no variant named as the word that leaves a part off.
#[test]
fn the_definitions_are_read_whole() {
    for (kept, parsed) in STYLES.iter().zip(PARSED.iter()) {
        let raw: Value = serde_json::from_str(kept.definition).unwrap();
        for slot in &parsed.slots {
            let values = raw["colors"][&slot.name]["values"]
                .as_array()
                .map_or(0, Vec::len);
            assert_eq!(slot.palette.len(), values, "{} {}", kept.id, slot.name);
        }
        for part in &parsed.parts {
            assert!(!part.variants.is_empty(), "{} {}", kept.id, part.name);
            assert!(
                !part.variants.iter().any(|variant| variant == LEFT_OFF),
                "{} {}",
                kept.id,
                part.name
            );
        }
    }
}

/// A face never moves: DiceBear only writes its animations when asked, and
/// the desk never asks.
#[test]
fn a_face_is_still() {
    for kept in &STYLES {
        let face = made(json!({ "style": kept.id }), None);
        let drawn = svg(&face.recipe).unwrap();
        assert!(!drawn.contains("@keyframes"), "{}", kept.id);
    }
}

/// Nothing here may need credit where it is shown: each style's own
/// definition names its licence, and every one drawn is CC0.
#[test]
fn every_style_drawn_is_cc0() {
    for kept in &STYLES {
        let definition: Value = serde_json::from_str(kept.definition).unwrap();
        assert_eq!(
            definition["meta"]["license"]["name"], "CC0 1.0",
            "{}",
            kept.id
        );
    }
}

/// A struck variant is not offered, not taken, and never filled from a seed;
/// every struck name is one the style has.
#[test]
fn a_struck_variant_is_neither_offered_nor_taken_nor_seeded() {
    let mut struck_any = false;
    for kept in &STYLES {
        let raw: Value = serde_json::from_str(kept.definition).unwrap();
        for (part, variants) in kept.struck {
            for variant in *variants {
                struck_any = true;
                assert!(
                    raw["components"][*part]["variants"].get(*variant).is_some(),
                    "{} has no {part} {variant} to strike",
                    kept.id
                );
                let offered = style_answer(kept.id, &persona("ada", None)).unwrap();
                let listed = offered["parts"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|entry| entry["part"] == *part)
                    .unwrap();
                assert!(
                    !listed["variants"]
                        .as_array()
                        .unwrap()
                        .contains(&json!(variant))
                );
                let refused = Ask::read(
                    &json!({ "style": kept.id, "parts": { *part: variant } }),
                    None,
                );
                assert!(refused.is_err(), "{} {part} {variant} was taken", kept.id);
            }
        }
        let (_, parsed) = style(kept.id).unwrap();
        for seed in (0..200).filter(|_| !kept.struck.is_empty()) {
            let drawn = draw(
                parsed,
                &BTreeMap::new(),
                &BTreeMap::new(),
                &format!("seed-{seed}"),
                false,
            )
            .unwrap()
            .to_json();
            for (part, variants) in kept.struck {
                let chosen = drawn["options"][format!("{part}Variant")]
                    .as_str()
                    .unwrap_or_default();
                assert!(
                    !variants.contains(&chosen),
                    "{} seed-{seed} drew {part} {chosen}",
                    kept.id
                );
            }
        }
    }
    assert!(
        struck_any,
        "pixelbot's unreadable eyes and mouth are struck"
    );
}

/// Pixelbot glows in the bubble's hue, lit and on a screen of that hue, as
/// setup draws it and as `face_options` suggests it, and is framed so its
/// eyes and mouth sit inside the circle.
#[test]
fn pixelbot_glows_in_the_bubble_hue_inside_the_circle() {
    let ada = persona("ada", None);
    let answer = style_answer("pixelbot", &ada).unwrap();
    assert_eq!(
        answer["yourColors"],
        json!({ "glow": "#00f5fd", "background": "#002b2d" })
    );
    let listed = styles_answer(&ada);
    let pixelbot = listed["styles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|style| style["style"] == "pixelbot")
        .unwrap();
    assert_eq!(pixelbot["yourColors"], answer["yourColors"]);
    let grey = persona("ada", Some("grey"));
    assert_eq!(
        style_answer("pixelbot", &grey).unwrap()["yourColors"],
        json!({ "glow": "#ced9e5", "background": "#202429" })
    );
    let open_peeps = style_answer("open-peeps", &ada).unwrap();
    assert!(open_peeps.get("yourColors").is_none());
    // The other creatures sit on the same dark ground of the bubble's hue.
    for (style, slot) in [("moods", "face"), ("sprouts", "pot")] {
        assert_eq!(
            style_answer(style, &ada).unwrap()["yourColors"],
            json!({ slot: "#48b8bc", "background": "#002b2d" }),
            "{style}"
        );
        assert_eq!(
            style_answer(style, &grey).unwrap()["yourColors"]["background"],
            "#202429",
            "{style}"
        );
    }

    // Every face's lit pixels stay inside the circle the window clips it to.
    for eyes in ["blocks", "happy", "round", "soft", "tall", "wide"] {
        for mouth in ["bigSmile", "open", "smile"] {
            let face = made(
                json!({ "style": "pixelbot", "parts": { "eyes": eyes, "mouth": mouth }, "colors": { "glow": "#ff0000", "background": "#000000" } }),
                None,
            );
            let picture = image::load_from_memory_with_format(&face.png, image::ImageFormat::Png)
                .unwrap()
                .to_rgba8();
            let centre = f64::from(EDGE) / 2.0;
            for (x, y, pixel) in picture.enumerate_pixels() {
                if pixel[0] > 200 && pixel[1] < 80 {
                    let distance = (f64::from(x) - centre).hypot(f64::from(y) - centre);
                    assert!(distance < centre, "{eyes} {mouth} lit at {x},{y}");
                }
            }
        }
    }
}

/// A face that fills its square is kept with no alpha channel, so a window
/// can tell from the file that no colour disc belongs behind it; one with a
/// transparent ground keeps its alpha.
#[test]
fn a_face_with_a_ground_is_kept_without_an_alpha_channel() {
    // A PNG's colour type is the byte after its IHDR's size and depth:
    // 2 is RGB, 6 is RGB with alpha.
    let colour_type = |png: &[u8]| png[25];
    for style in ["pixelbot", "moods", "sprouts"] {
        let face = seeded(&persona(&format!("{style}-ada"), None)).unwrap();
        let face = made(json!({ "style": face.recipe.style }), Some(&face.recipe));
        assert_eq!(colour_type(&face.png), 2, "{style}");
        let picture = image::load_from_memory(&face.png).unwrap();
        assert!(!picture.color().has_alpha(), "{style}");
    }
    let open = made(json!({ "style": "open-peeps" }), None);
    assert_eq!(colour_type(&open.png), 6);
    let grounded = made(
        json!({ "style": "open-peeps", "colors": { "background": "#202429" } }),
        None,
    );
    assert_eq!(colour_type(&grounded.png), 2);
}

/// A face kept in a style the desk no longer draws stays; asked for in that
/// style, the teammate is told the style is gone and which there are, and
/// `face_options` says so beside the face it has.
#[test]
fn a_face_in_a_style_no_longer_drawn_is_kept_and_named_gone() {
    let clay = AvatarRecipe {
        style: "clay".into(),
        parts: BTreeMap::from([("eyes".to_string(), Some("googly".to_string()))]),
        colors: BTreeMap::from([("body".to_string(), "#48b8bc".to_string())]),
        seed: "ada".into(),
        made_by: FaceMaker::Teammate,
    };
    let Err(refused) = Ask::read(
        &json!({ "style": "clay", "parts": { "eyes": "big" } }),
        Some(&clay),
    ) else {
        panic!("a change in a style no longer drawn was taken");
    };
    assert_eq!(
        refused,
        "Your face is in the clay style, which Hotline no longer draws. Your picture stays as it is; to change it, make a new face in one of: pixelbot, moods, sprouts, open-peeps."
    );
    // Another style is a new face, from nothing the old one kept.
    let fresh = compose(
        Ask::read(&json!({ "style": "moods" }), Some(&clay)).unwrap(),
        Some(&clay),
        "ada",
        FaceMaker::Teammate,
    )
    .unwrap();
    assert_eq!(fresh.recipe, made(json!({ "style": "moods" }), None).recipe);

    let mut teammate = persona("ada", None);
    teammate.avatar = Some(crate::contract::Avatar {
        hash: "c".repeat(64),
        by: crate::contract::AvatarBy::Own,
        updated_at: "2026-11-13T00:00:00.000Z".into(),
        recipe: Some(clay.clone()),
        poses: None,
    });
    let listed = styles_answer(&teammate);
    assert_eq!(listed["current"], json!(clay));
    assert!(
        listed["currentStyleGone"]
            .as_str()
            .unwrap()
            .starts_with("Your face is in the clay style")
    );
    assert!(
        styles_answer(&persona("ada", None))
            .get("currentStyleGone")
            .is_none()
    );
}

/// The pixels where two pictures of the same face differ.
fn changed(one: &[u8], other: &[u8]) -> usize {
    let one = image::load_from_memory(one).unwrap().to_rgba8();
    let other = image::load_from_memory(other).unwrap().to_rgba8();
    one.pixels()
        .zip(other.pixels())
        .filter(|(a, b)| a != b)
        .count()
}

/// A face in a style that blinks is made with its eyes shut too: the same
/// square, kept the same way (no alpha channel exactly when its picture has
/// none), differing only in a small part of it. Open Peeps does not blink,
/// and neither do pixelbot's one-row `soft` eyes, which its blink leaves as
/// they are.
#[test]
fn a_face_that_blinks_has_an_eyes_shut_picture_kept_like_it() {
    for style in ["pixelbot", "moods", "sprouts"] {
        for seed in ["ada", "bert", "cleo", "dov", "eve"] {
            for colors in [json!({}), json!({ "background": "#002b2d" })] {
                let face = made(
                    json!({ "style": style, "seed": seed, "colors": colors }),
                    None,
                );
                if face.recipe.parts["eyes"].as_deref() == Some("soft") {
                    assert!(face.shut.is_none(), "{style} {seed}");
                    continue;
                }
                let shut = face
                    .shut
                    .as_ref()
                    .unwrap_or_else(|| panic!("{style} {seed}"));
                assert_eq!(
                    image::load_from_memory(shut).unwrap().color(),
                    image::load_from_memory(&face.png).unwrap().color(),
                    "{style} {seed} {colors}"
                );
                let differs = changed(&face.png, shut);
                assert!(
                    differs > 0 && differs < (EDGE * EDGE / 10) as usize,
                    "{style} {seed}: {differs}"
                );
            }
        }
    }
    let soft = made(
        json!({ "style": "pixelbot", "parts": { "eyes": "soft" } }),
        None,
    );
    assert!(soft.shut.is_none());
    for seed in ["ada", "bert", "cleo"] {
        let person = made(json!({ "style": "open-peeps", "seed": seed }), None);
        assert!(person.shut.is_none(), "{seed}");
    }
}

/// Pixelbot's eyes shut are one row of pixels: the bottom row of its eyes
/// stays lit and every row above it goes dark.
#[test]
fn pixelbots_eyes_shut_are_one_row() {
    let face = made(
        json!({ "style": "pixelbot", "parts": { "eyes": "round" }, "colors": { "glow": "#00f5fd", "background": "#002b2d" } }),
        None,
    );
    let open = image::load_from_memory(&face.png).unwrap().to_rgb8();
    let shut = image::load_from_memory(face.shut.as_ref().unwrap())
        .unwrap()
        .to_rgb8();
    // Each band of lit pixel rows is one row of blocks, from the top: three
    // for these eyes, then the mouth's.
    let bands = |picture: &image::RgbImage| -> Vec<(u32, u32)> {
        let mut bands: Vec<(u32, u32)> = Vec::new();
        for y in 0..EDGE {
            if (0..EDGE).any(|x| picture.get_pixel(x, y)[1] > 200) {
                match bands.last_mut() {
                    Some(band) if band.1 + 1 == y => band.1 = y,
                    _ => bands.push((y, y)),
                }
            }
        }
        bands
    };
    let (open, shut) = (bands(&open), bands(&shut));
    assert_eq!(shut.len(), open.len() - 2, "{open:?} {shut:?}");
    assert_eq!(shut[0], open[2], "the eyes' bottom row is the one left");
    assert_eq!(shut[1..], open[3..], "the mouth is as it was");
}
