//! A teammate's hands over its page on Linux, as macOS draws them
//! (`hands.rs`): the computer viewer's arrow, the teammate's name beside it,
//! and a ring where it clicks. A drawing area of GTK's own in the page's
//! overlay draws them with cairo, never in the page's document, so the page
//! cannot see, move or hide them; the overlay passes every click and scroll
//! through it to what is below. All of it runs on the main thread, from
//! `with_webview`.

use super::linux::overlay_of;
use gtk::cairo;
use gtk::glib;
use gtk::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::f64::consts::PI;
use std::rc::Rc;
use std::time::{Duration, Instant};
use webkit2gtk::WebView;

/// How long the pointer takes to reach what the teammate acts on; the act
/// waits this long, so the person sees it arrive first.
pub(super) const MOVE_SECONDS: f64 = 0.28;
const APPEAR_SECONDS: f64 = 0.15;
const FADE_SECONDS: f64 = 0.4;
const RING_SECONDS: f64 = 0.45;

/// The computer viewer's arrow, in its 22 by 30 box, tip first.
const ARROW: [(f64, f64); 4] = [(2.5, 1.5), (2.5, 23.0), (8.6, 17.6), (19.0, 15.4)];

/// What is drawn now, and since when.
#[derive(Default)]
struct Drawn {
    name: String,
    /// Where the pointer moves from and to, and when it set off.
    from: Option<(f64, f64)>,
    to: Option<(f64, f64)>,
    moved: Option<Instant>,
    /// The pointer's opacity from, to, since and for how long.
    fade: (f64, f64, Option<Instant>, f64),
    rings: Vec<((f64, f64), Instant)>,
    ticking: bool,
}

impl Drawn {
    fn opacity(&self, now: Instant) -> f64 {
        let (from, to, since, length) = self.fade;
        match since {
            Some(since) => {
                let t = (now - since).as_secs_f64() / length;
                from + (to - from) * t.clamp(0.0, 1.0)
            }
            None => to,
        }
    }

    fn at(&self, now: Instant) -> Option<(f64, f64)> {
        let to = self.to?;
        let (Some(from), Some(moved)) = (self.from, self.moved) else {
            return Some(to);
        };
        let t = ((now - moved).as_secs_f64() / MOVE_SECONDS).clamp(0.0, 1.0);
        // Ease in and out, as Core Animation's.
        let eased = t * t * (3.0 - 2.0 * t);
        Some((
            from.0 + (to.0 - from.0) * eased,
            from.1 + (to.1 - from.1) * eased,
        ))
    }

    fn fade_to(&mut self, to: f64, length: f64, now: Instant) {
        let from = self.opacity(now);
        self.fade = (from, to, Some(now), length);
    }

    /// Whether anything still moves, so the next frame is wanted.
    fn moving(&self, now: Instant) -> bool {
        let fading = self
            .fade
            .2
            .is_some_and(|since| (now - since).as_secs_f64() < self.fade.3);
        let travelling = self
            .moved
            .is_some_and(|moved| (now - moved).as_secs_f64() < MOVE_SECONDS);
        fading || travelling || !self.rings.is_empty()
    }
}

struct Hands {
    area: gtk::DrawingArea,
    drawn: Rc<RefCell<Drawn>>,
}

thread_local! {
    static HANDS: RefCell<HashMap<usize, Hands>> = RefCell::default();
}

fn key(view: &WebView) -> usize {
    glib::ObjectType::as_ptr(view) as usize
}

fn draw(drawn: &mut Drawn, cr: &cairo::Context) {
    let now = Instant::now();
    drawn
        .rings
        .retain(|(_, since)| (now - *since).as_secs_f64() < RING_SECONDS);
    for ((x, y), since) in &drawn.rings {
        let t = (now - *since).as_secs_f64() / RING_SECONDS;
        // Ease out.
        let eased = 1.0 - (1.0 - t).powi(3);
        let radius = 15.0 * (0.35 + (1.5 - 0.35) * eased);
        cr.set_source_rgba(0.957, 0.957, 0.965, 0.95 * (1.0 - eased));
        cr.set_line_width(2.0);
        cr.new_path();
        cr.arc(*x, *y, radius, 0.0, 2.0 * PI);
        let _ = cr.stroke();
    }
    let opacity = drawn.opacity(now);
    let Some((x, y)) = drawn.at(now).filter(|_| opacity > 0.0) else {
        return;
    };
    // The arrow, its tip on the point, with a soft shadow below.
    let arrow = |cr: &cairo::Context, dx: f64, dy: f64| {
        cr.new_path();
        for (i, (ax, ay)) in ARROW.iter().enumerate() {
            let (px, py) = (x + ax - ARROW[0].0 + dx, y + ay - ARROW[0].1 + dy);
            if i == 0 {
                cr.move_to(px, py);
            } else {
                cr.line_to(px, py);
            }
        }
        cr.close_path();
    };
    arrow(cr, 0.0, 1.0);
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.35 * opacity);
    let _ = cr.fill();
    arrow(cr, 0.0, 0.0);
    cr.set_source_rgba(0.957, 0.957, 0.965, opacity);
    let _ = cr.fill_preserve();
    cr.set_source_rgba(0.078, 0.078, 0.086, opacity);
    cr.set_line_width(1.6);
    cr.set_line_join(cairo::LineJoin::Round);
    let _ = cr.stroke();
    if drawn.name.is_empty() {
        return;
    }
    // The tag: the name in a dark pill below and right of the tip.
    cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
    cr.set_font_size(11.0);
    let Ok(text) = cr.text_extents(&drawn.name) else {
        return;
    };
    let (width, height) = (text.x_advance().ceil() + 18.0, 20.0);
    let (left, top) = (x + 14.0, y + 18.0);
    let radius = 10.0;
    cr.new_path();
    cr.arc(
        left + width - radius,
        top + radius,
        radius,
        -PI / 2.0,
        PI / 2.0,
    );
    cr.arc(
        left + radius,
        top + radius,
        radius,
        PI / 2.0,
        3.0 * PI / 2.0,
    );
    cr.close_path();
    cr.set_source_rgba(0.078, 0.078, 0.086, 0.9 * opacity);
    let _ = cr.fill_preserve();
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.18 * opacity);
    cr.set_line_width(1.0);
    let _ = cr.stroke();
    cr.set_source_rgba(1.0, 1.0, 1.0, opacity);
    cr.move_to(
        left + (width - text.x_advance()) / 2.0,
        top + height / 2.0 - (text.y_bearing() + text.height() / 2.0),
    );
    let _ = cr.show_text(&drawn.name);
}

/// The hands for this page, made the first time.
fn ensure(view: &WebView) -> Option<(gtk::DrawingArea, Rc<RefCell<Drawn>>)> {
    HANDS.with(|hands| {
        if let Some(hands) = hands.borrow().get(&key(view)) {
            return Some((hands.area.clone(), hands.drawn.clone()));
        }
        let overlay = overlay_of(view)?;
        let area = gtk::DrawingArea::new();
        let drawn = Rc::new(RefCell::new(Drawn::default()));
        {
            let drawn = drawn.clone();
            area.connect_draw(move |_, cr| {
                draw(&mut drawn.borrow_mut(), cr);
                glib::Propagation::Proceed
            });
        }
        // Never hit: every click and scroll goes to what is below. The
        // overlay passes its own window through, but a drawing area has a
        // window of its own inside that one, which takes no input either.
        area.input_shape_combine_region(Some(&cairo::Region::create()));
        area.connect_realize(|area| {
            if let Some(window) = area.window() {
                window.set_pass_through(true);
            }
        });
        overlay.add_overlay(&area);
        overlay.set_overlay_pass_through(&area, true);
        area.show();
        hands.borrow_mut().insert(
            key(view),
            Hands {
                area: area.clone(),
                drawn: drawn.clone(),
            },
        );
        Some((area, drawn))
    })
}

/// Draws frames while anything moves.
fn tick(area: &gtk::DrawingArea, drawn: &Rc<RefCell<Drawn>>) {
    area.queue_draw();
    if std::mem::replace(&mut drawn.borrow_mut().ticking, true) {
        return;
    }
    let (area, drawn) = (area.clone(), drawn.clone());
    glib::timeout_add_local(Duration::from_millis(16), move || {
        area.queue_draw();
        let mut now = drawn.borrow_mut();
        if now.moving(Instant::now()) {
            glib::ControlFlow::Continue
        } else {
            now.ticking = false;
            glib::ControlFlow::Break
        }
    });
}

/// Moves the teammate's pointer to a point in its page's viewport, named,
/// over `MOVE_SECONDS`; from nowhere it appears there instead.
pub(super) fn point(view: WebView, x: f64, y: f64, _viewport: f64, who: &str) {
    let Some((area, drawn)) = ensure(&view) else {
        return;
    };
    {
        let now = Instant::now();
        let mut drawn = drawn.borrow_mut();
        drawn.name = who.to_string();
        let shown = drawn.opacity(now) > 0.5 && drawn.to.is_some();
        if shown {
            drawn.from = drawn.at(now);
            drawn.moved = Some(now);
        } else {
            drawn.from = None;
            drawn.moved = None;
            drawn.fade_to(1.0, APPEAR_SECONDS, now);
        }
        drawn.to = Some((x, y));
    }
    tick(&area, &drawn);
}

/// A ring where the teammate clicks, growing and fading, as the computer's.
pub(super) fn ring(view: WebView, x: f64, y: f64, _viewport: f64) {
    let Some((area, drawn)) = HANDS.with(|hands| {
        hands
            .borrow()
            .get(&key(&view))
            .map(|hands| (hands.area.clone(), hands.drawn.clone()))
    }) else {
        return;
    };
    drawn.borrow_mut().rings.push(((x, y), Instant::now()));
    tick(&area, &drawn);
}

/// Fades the pointer out: the teammate has been still a while, or the
/// person took the page over and has their own.
pub(super) fn hide(view: WebView) {
    let Some((area, drawn)) = HANDS.with(|hands| {
        hands
            .borrow()
            .get(&key(&view))
            .map(|hands| (hands.area.clone(), hands.drawn.clone()))
    }) else {
        return;
    };
    {
        let mut drawn = drawn.borrow_mut();
        drawn.fade_to(0.0, FADE_SECONDS, Instant::now());
        drawn.to = None;
        drawn.from = None;
        drawn.moved = None;
    }
    tick(&area, &drawn);
}

/// Lets go of a closing page's hands: they go with its overlay.
pub(super) fn forget(view: WebView) {
    HANDS.with(|hands| hands.borrow_mut().remove(&key(&view)));
}

/// The overlay keeps the hands over the page whatever the window's size.
pub(super) fn follow(_view: WebView) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pointer_eases_from_where_it_was_to_where_it_goes() {
        let start = Instant::now();
        let drawn = Drawn {
            from: Some((0.0, 0.0)),
            to: Some((100.0, 50.0)),
            moved: Some(start),
            ..Drawn::default()
        };
        assert_eq!(drawn.at(start), Some((0.0, 0.0)));
        let half = drawn
            .at(start + Duration::from_secs_f64(MOVE_SECONDS / 2.0))
            .unwrap();
        assert!((half.0 - 50.0).abs() < 1.0 && (half.1 - 25.0).abs() < 1.0);
        assert_eq!(
            drawn.at(start + Duration::from_secs_f64(MOVE_SECONDS)),
            Some((100.0, 50.0))
        );
        assert!(drawn.moving(start));
        assert!(!drawn.moving(start + Duration::from_secs(1)));
    }

    #[test]
    fn hidden_hands_fade_to_nothing() {
        let start = Instant::now();
        let mut drawn = Drawn::default();
        drawn.fade_to(1.0, APPEAR_SECONDS, start);
        let shown = start + Duration::from_secs(1);
        assert_eq!(drawn.opacity(shown), 1.0);
        drawn.fade_to(0.0, FADE_SECONDS, shown);
        assert_eq!(drawn.opacity(shown + Duration::from_secs(1)), 0.0);
    }
}
