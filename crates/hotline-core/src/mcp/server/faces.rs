//! `face_options` and `make_face`: a teammate draws its own face from one of
//! DiceBear's styles on the desk ([`crate::session::avatar::face`]). Neither
//! reads a file or makes a network call; `make_face` writes only the
//! teammate's own picture, as `set_avatar` does, and refuses over one the
//! person chose.
use super::{FACE_OPTIONS, MAKE_FACE, schema};
use crate::session::avatar::face::style_ids;
use rmcp::model::Tool;
use serde_json::json;

pub(super) fn options_descriptor() -> Tool {
    Tool::new(
        FACE_OPTIONS,
        "The styles your face can be drawn in, each with what it draws, whether it draws a person or a creature and whether its parts are named; your face now; and your bubble colour by name and hex. With `style`, that style's parts with their variants, how often an optional part is drawn, and its colour slots with their palettes. Read it before make_face.",
        schema(json!({
            "type": "object",
            "properties": {
                "style": { "type": "string", "enum": style_ids(), "description": "One style, for its parts and colours. Leave it out for the list." },
            },
            "additionalProperties": false,
        })),
    )
}

pub(super) fn make_descriptor() -> Tool {
    Tool::new(
        MAKE_FACE,
        "Draw your own face and make it your picture. Build it from your name, your goal and your bubble colour (face_options gives the styles, their parts and that colour). Name a style, and the parts and colours you choose; the rest is filled from a seed. Asked for a change, keep your style and name only what changes: in the same style everything you do not name stays as it is. Switch style only when the person tells you to; a new style or a new seed starts afresh. Change your picture only when the person asks, or at setup. Returns the picture for you to look at; call again to adjust it. If the person chose your current picture, this refuses.",
        schema(json!({
            "type": "object",
            "properties": {
                "style": { "type": "string", "enum": style_ids() },
                "parts": { "type": "object", "additionalProperties": { "type": "string" }, "description": "A part's name to the variant to draw, as face_options lists them, or \"none\" to leave it off." },
                "colors": { "type": "object", "additionalProperties": { "type": "string", "pattern": "^#[0-9a-fA-F]{6}$" }, "description": "A colour slot's name to a #rrggbb colour, from its palette or any other." },
                "seed": { "type": "string", "minLength": 1, "maxLength": 200, "description": "Only to try a different face in a style: what is not chosen is filled from it. Defaults to your id." },
            },
            "required": ["style"],
            "additionalProperties": false,
        })),
    )
}
