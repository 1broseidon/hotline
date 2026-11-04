//! A teammate's hands over its page, as the Hotline computer's viewer shows
//! them: the same arrow, the teammate's name beside it, and a ring where it
//! clicks. A clear native view over the page draws them in Core Animation
//! layers, never in the page's own document, so the page cannot see, move
//! or hide them, and the view takes no clicks: the person's own reach the
//! page or the shield below it as before. All of it runs on the main thread,
//! from `with_webview`.

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSFont, NSView, NSWindowOrderingMode};
use objc2_core_foundation::{CGFloat, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{CGColor, CGMutablePath, CGPath};
use objc2_foundation::{NSNumber, NSPoint, NSString};
use objc2_quartz_core::{
    CABasicAnimation, CALayer, CAMediaTimingFunction, CAShapeLayer, CATextLayer, CATransaction,
    kCAMediaTimingFunctionEaseInEaseOut, kCAMediaTimingFunctionEaseOut,
};
use objc2_web_kit::WKWebView;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;

/// How long the pointer takes to reach what the teammate acts on; the act
/// waits this long, so the person sees it arrive first.
pub(super) const MOVE_SECONDS: f64 = 0.28;

/// The computer viewer's arrow, in its 22 by 30 box, tip first.
const ARROW: [(f64, f64); 4] = [(2.5, 1.5), (2.5, 23.0), (8.6, 17.6), (19.0, 15.4)];
const ARROW_SIZE: (f64, f64) = (22.0, 30.0);

define_class!(
    /// The clear view the hands are drawn in. It is never hit, so every
    /// click and scroll goes past it to what is below.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "HotlineBrowserHands"]
    struct HandsView;

    impl HandsView {
        #[unsafe(method(hitTest:))]
        fn hit_test(&self, _point: NSPoint) -> *mut NSView {
            std::ptr::null_mut()
        }
    }
);

/// One page's hands: the view, the layer that moves (arrow and tag), and
/// the tag's text.
struct Hands {
    view: Retained<NSView>,
    hand: Retained<CALayer>,
    tag: Retained<CALayer>,
    label: Retained<CATextLayer>,
    name: String,
    /// The last point, in the view's own (bottom-up) coordinates.
    at: Option<CGPoint>,
}

thread_local! {
    static HANDS: RefCell<HashMap<usize, Hands>> = RefCell::default();
}

/// The page `with_webview` hands over.
///
/// SAFETY: the caller is `with_webview`'s closure, on the main thread,
/// where `view` is the live `WKWebView` of a Tauri window.
unsafe fn page(view: *mut c_void) -> &'static WKWebView {
    // SAFETY: as the function's own contract above.
    unsafe { &*view.cast::<WKWebView>() }
}

fn color(r: f64, g: f64, b: f64, a: f64) -> objc2_core_foundation::CFRetained<CGColor> {
    CGColor::new_srgb(r, g, b, a)
}

/// A point in the page's viewport (CSS pixels from its top left, with the
/// viewport's height) as the hands view's own, which counts up from the
/// bottom. The viewport sits below the window's title bar inside the view,
/// as the desk's own input finds it.
fn local(view: &NSView, x: f64, y: f64, viewport: f64) -> CGPoint {
    let height = view.bounds().size.height;
    let top = y + (height - viewport).max(0.0);
    CGPoint::new(x, height - top)
}

/// The hands for this page, made the first time.
fn ensure(main: MainThreadMarker, page: &WKWebView, key: usize) -> bool {
    HANDS.with(|hands| {
        if hands.borrow().contains_key(&key) {
            return true;
        }
        // SAFETY: the page's own superview, on the main thread.
        let Some(parent) = (unsafe { page.superview() }) else {
            return false;
        };
        // SAFETY: NSView's designated initialiser, on a fresh view.
        let view: Retained<HandsView> =
            unsafe { msg_send![HandsView::alloc(main), initWithFrame: page.frame()] };
        let view = Retained::into_super(view);
        view.setWantsLayer(true);
        let Some(root) = view.layer() else {
            return false;
        };
        parent.addSubview_positioned_relativeTo(&view, NSWindowOrderingMode::Above, Some(page));
        let scale = page
            .window()
            .map_or(2.0, |window| window.backingScaleFactor());

        let hand = CALayer::new();
        hand.setBounds(CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(0.0, 0.0)));
        hand.setOpacity(0.0);

        let arrow = CAShapeLayer::new();
        let path = CGMutablePath::new();
        for (i, (x, y)) in ARROW.iter().enumerate() {
            let (x, y) = (*x, ARROW_SIZE.1 - *y);
            // SAFETY: a fresh path and no transform.
            unsafe {
                if i == 0 {
                    CGMutablePath::move_to_point(Some(&path), std::ptr::null(), x, y);
                } else {
                    CGMutablePath::add_line_to_point(Some(&path), std::ptr::null(), x, y);
                }
            }
        }
        CGMutablePath::close_subpath(Some(&path));
        let shape: &CGPath = &path;
        arrow.setPath(Some(shape));
        arrow.setFillColor(Some(&color(0.957, 0.957, 0.965, 1.0)));
        arrow.setStrokeColor(Some(&color(0.078, 0.078, 0.086, 1.0)));
        arrow.setLineWidth(1.6);
        arrow.setLineJoin(&NSString::from_str("round"));
        arrow.setBounds(CGRect::new(
            CGPoint::new(0.0, 0.0),
            CGSize::new(ARROW_SIZE.0, ARROW_SIZE.1),
        ));
        arrow.setAnchorPoint(CGPoint::new(
            ARROW[0].0 / ARROW_SIZE.0,
            (ARROW_SIZE.1 - ARROW[0].1) / ARROW_SIZE.1,
        ));
        arrow.setPosition(CGPoint::new(0.0, 0.0));
        arrow.setShadowOpacity(0.6);
        arrow.setShadowRadius(2.0);
        arrow.setShadowOffset(CGSize::new(0.0, -1.0));
        arrow.setContentsScale(scale);

        let tag = CALayer::new();
        tag.setBackgroundColor(Some(&color(0.078, 0.078, 0.086, 0.9)));
        tag.setBorderColor(Some(&color(1.0, 1.0, 1.0, 0.18)));
        tag.setBorderWidth(1.0);
        tag.setCornerRadius(10.0);
        tag.setAnchorPoint(CGPoint::new(0.0, 1.0));
        tag.setPosition(CGPoint::new(14.0, -18.0));
        tag.setContentsScale(scale);

        let label = CATextLayer::new();
        let font = NSFont::boldSystemFontOfSize(11.0);
        // SAFETY: an NSFont is the CTFont a text layer takes.
        unsafe {
            label.setFont(Some(
                &*(Retained::as_ptr(&font).cast::<objc2_core_foundation::CFType>()),
            ));
        };
        label.setFontSize(11.0);
        label.setForegroundColor(Some(&color(1.0, 1.0, 1.0, 1.0)));
        label.setContentsScale(scale);
        // SAFETY: a plain string, set on the main thread.
        unsafe { label.setAlignmentMode(objc2_quartz_core::kCAAlignmentCenter) };
        tag.addSublayer(&label);

        hand.addSublayer(&arrow);
        hand.addSublayer(&tag);
        root.addSublayer(&hand);
        hands.borrow_mut().insert(
            key,
            Hands {
                view,
                hand,
                tag,
                label,
                name: String::new(),
                at: None,
            },
        );
        true
    })
}

/// Sets the tag's text and sizes it.
fn name(hands: &mut Hands, name: &str) {
    if hands.name == name {
        return;
    }
    hands.name = name.to_string();
    let text = NSString::from_str(name);
    // SAFETY: an NSString is a text layer's string.
    unsafe {
        hands
            .label
            .setString(Some(&*(Retained::as_ptr(&text).cast::<AnyObject>())));
    };
    let size = hands.label.preferredFrameSize();
    let (width, height) = (size.width.ceil() + 18.0, 20.0);
    CATransaction::begin();
    CATransaction::setDisableActions(true);
    hands.tag.setBounds(CGRect::new(
        CGPoint::new(0.0, 0.0),
        CGSize::new(width, height),
    ));
    hands.label.setBounds(CGRect::new(
        CGPoint::new(0.0, 0.0),
        CGSize::new(size.width.ceil(), size.height.ceil()),
    ));
    hands
        .label
        .setPosition(CGPoint::new(width / 2.0, height / 2.0 - 0.5));
    CATransaction::commit();
}

/// Moves the teammate's pointer to a point in its page's viewport, named,
/// over `MOVE_SECONDS`; from nowhere it appears there instead.
pub(super) fn point(view: *mut c_void, x: f64, y: f64, viewport: f64, who: &str) {
    let Some(main) = MainThreadMarker::new() else {
        return;
    };
    // SAFETY: see `page`.
    let page = unsafe { page(view) };
    let key = view as usize;
    if !ensure(main, page, key) {
        return;
    }
    HANDS.with(|hands| {
        let mut hands = hands.borrow_mut();
        let Some(hands) = hands.get_mut(&key) else {
            return;
        };
        name(hands, who);
        let to = local(&hands.view, x, y, viewport);
        let shown = hands.hand.opacity() > 0.5 && hands.at.is_some();
        CATransaction::begin();
        if shown {
            CATransaction::setAnimationDuration(MOVE_SECONDS);
            // SAFETY: Core Animation's own constant.
            let ease = CAMediaTimingFunction::functionWithName(unsafe {
                kCAMediaTimingFunctionEaseInEaseOut
            });
            CATransaction::setAnimationTimingFunction(Some(&ease));
        } else {
            CATransaction::setDisableActions(true);
        }
        hands.hand.setPosition(to);
        CATransaction::commit();
        if !shown {
            CATransaction::begin();
            CATransaction::setAnimationDuration(0.15);
            hands.hand.setOpacity(1.0);
            CATransaction::commit();
        }
        hands.at = Some(to);
    });
}

/// A ring where the teammate clicks, growing and fading, as the computer's.
pub(super) fn ring(view: *mut c_void, x: f64, y: f64, viewport: f64) {
    if MainThreadMarker::new().is_none() {
        return;
    }
    let key = view as usize;
    HANDS.with(|hands| {
        let hands = hands.borrow();
        let Some(hands) = hands.get(&key) else {
            return;
        };
        let Some(root) = hands.view.layer() else {
            return;
        };
        let at = local(&hands.view, x, y, viewport);
        let radius: CGFloat = 15.0;
        let ring = CAShapeLayer::new();
        let rect = CGRect::new(
            CGPoint::new(0.0, 0.0),
            CGSize::new(radius * 2.0, radius * 2.0),
        );
        // SAFETY: a rect and no transform.
        let circle = unsafe { CGPath::with_ellipse_in_rect(rect, std::ptr::null()) };
        ring.setPath(Some(&circle));
        ring.setFillColor(None);
        ring.setStrokeColor(Some(&color(0.957, 0.957, 0.965, 0.95)));
        ring.setLineWidth(2.0);
        ring.setBounds(rect);
        ring.setPosition(at);
        ring.setOpacity(0.0);
        root.insertSublayer_below(&ring, Some(&hands.hand));

        let grow =
            CABasicAnimation::animationWithKeyPath(Some(&NSString::from_str("transform.scale")));
        // SAFETY: NSNumbers, as a scale animation takes them.
        unsafe {
            grow.setFromValue(Some(&NSNumber::new_f64(0.35)));
            grow.setToValue(Some(&NSNumber::new_f64(1.5)));
        }
        let fade = CABasicAnimation::animationWithKeyPath(Some(&NSString::from_str("opacity")));
        // SAFETY: as above.
        unsafe {
            fade.setFromValue(Some(&NSNumber::new_f64(0.95)));
            fade.setToValue(Some(&NSNumber::new_f64(0.0)));
        }
        CATransaction::begin();
        CATransaction::setAnimationDuration(0.45);
        // SAFETY: Core Animation's own constant.
        let ease =
            CAMediaTimingFunction::functionWithName(unsafe { kCAMediaTimingFunctionEaseOut });
        CATransaction::setAnimationTimingFunction(Some(&ease));
        let gone = ring.clone();
        let done = block2::RcBlock::new(move || gone.removeFromSuperlayer());
        // SAFETY: Core Animation calls it once, on the main thread.
        unsafe { CATransaction::setCompletionBlock(Some(&done)) };
        ring.addAnimation_forKey(&grow, None);
        ring.addAnimation_forKey(&fade, None);
        CATransaction::commit();
    });
}

/// Fades the pointer out: the teammate has been still a while, or the
/// person took the page over and has their own.
pub(super) fn hide(view: *mut c_void) {
    if MainThreadMarker::new().is_none() {
        return;
    }
    HANDS.with(|hands| {
        if let Some(hands) = hands.borrow_mut().get_mut(&(view as usize)) {
            CATransaction::begin();
            CATransaction::setAnimationDuration(0.4);
            hands.hand.setOpacity(0.0);
            CATransaction::commit();
            hands.at = None;
        }
    });
}

/// Keeps the hands over the page after the window is resized.
pub(super) fn follow(view: *mut c_void) {
    // SAFETY: see `page`.
    let page = unsafe { page(view) };
    HANDS.with(|hands| {
        if let Some(hands) = hands.borrow().get(&(view as usize)) {
            hands.view.setFrame(page.frame());
        }
    });
}
