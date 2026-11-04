//! A whole page for the teammate to look at (`screenshot` with
//! `full_page`): scrolled from the top a viewport at a time, a picture of
//! each, and the scroll put back where it was.
//!
//! Sticky and fixed elements (a header, a cookie banner) would sit on every
//! picture over the text below them, so they show on the first only: after
//! it they are hidden, and shown again with the scroll.

use super::Native;
use hotline_core::browser::{DeskBrowser, FullPage};
use serde::Deserialize;

/// Notes where the page was scrolled to, to go back there after.
const BEGIN: &str =
    "globalThis.__hotlineFullPage = { x: scrollX, y: scrollY, hidden: [] }; return '{}';";

/// Hides what stays in place as the page scrolls, keeping each element's
/// own inline visibility to give back.
const HIDE: &str = "
const kept = globalThis.__hotlineFullPage;
for (const el of document.querySelectorAll('body *')) {
  const position = getComputedStyle(el).position;
  if (position !== 'fixed' && position !== 'sticky') continue;
  kept.hidden.push([el, el.style.getPropertyValue('visibility'), el.style.getPropertyPriority('visibility')]);
  el.style.setProperty('visibility', 'hidden', 'important');
}
return '{}';";

/// Shows the hidden elements again and scrolls back to where the page was.
const RESTORE: &str = "
const kept = globalThis.__hotlineFullPage;
if (!kept) return '{}';
for (const [el, value, priority] of kept.hidden) el.style.setProperty('visibility', value, priority);
window.scrollTo({ left: kept.x, top: kept.y, behavior: 'instant' });
delete globalThis.__hotlineFullPage;
return '{}';";

/// Where the page is after a scroll to `top`, once it has drawn there. A
/// page in the background may never draw a frame, so a short wait stands
/// in for one.
fn scroll_to(top: f64) -> String {
    format!(
        "window.scrollTo({{ left: scrollX, top: {top}, behavior: 'instant' }});
await Promise.race([
  new Promise(drawn => requestAnimationFrame(() => requestAnimationFrame(drawn))),
  new Promise(waited => setTimeout(waited, 150)),
]);
return JSON.stringify({{ top: scrollY, viewport: innerHeight, height: document.documentElement.scrollHeight }});"
    )
}

#[derive(Deserialize)]
struct Place {
    top: f64,
    viewport: f64,
    height: f64,
}

impl Place {
    fn at_bottom(&self) -> bool {
        self.top + self.viewport >= self.height - 1.0
    }
}

/// The page from the top as at most `most` pictures. The scroll and the
/// hidden elements are put back whether or not the pictures were taken.
pub(super) async fn look(
    native: &Native,
    persona_id: &str,
    most: usize,
) -> Result<FullPage, String> {
    native.run(persona_id, BEGIN.to_string()).await?;
    let looked = pictures(native, persona_id, most).await;
    let restored = native.run(persona_id, RESTORE.to_string()).await;
    let page = looked?;
    restored?;
    Ok(page)
}

async fn pictures(native: &Native, persona_id: &str, most: usize) -> Result<FullPage, String> {
    let mut place = moved(native, persona_id, 0.0).await?;
    let mut pictures = Vec::new();
    loop {
        pictures.push(native.screenshot(persona_id).await?);
        if place.at_bottom() || pictures.len() == most {
            break;
        }
        if pictures.len() == 1 {
            native.run(persona_id, HIDE.to_string()).await?;
        }
        let next = moved(native, persona_id, place.top + place.viewport).await?;
        // A page that scrolls inside a box of its own does not move: what
        // was seen is all a scroll of the window shows.
        if next.top <= place.top {
            return Ok(FullPage {
                pictures,
                more: false,
            });
        }
        place = next;
    }
    Ok(FullPage {
        more: !place.at_bottom(),
        pictures,
    })
}

async fn moved(native: &Native, persona_id: &str, top: f64) -> Result<Place, String> {
    let json = native.run(persona_id, scroll_to(top)).await?;
    serde_json::from_str(&json)
        .map_err(|error| format!("The page's answer was unreadable: {error}"))
}
