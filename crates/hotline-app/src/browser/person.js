// The body of an async function the desk runs in its own content world: the
// person's own input from a seat away from the desk, on a browser they have
// taken over. The desk puts `A` in front of it: { kind, x, y, dx, dy, text }.
// A point is in the visible page's CSS pixels.
//   tap:    answers the page's size, and listens for the click the desk
//           then sends as a real mouse event at the point;
//   click:  the same click made by the page's own script, when the real
//           one did not arrive;
//   scroll: scrolls what scrolls under the point, else the page;
//   text:   types where the focus is.
// Returns JSON: { width, height, error? }.
const size = { width: window.innerWidth, height: window.innerHeight };
const inside = (x, y) => x >= 0 && y >= 0 && x <= innerWidth && y <= innerHeight;

// Whether `el` can move `by` pixels further along an axis.
const room = (el, by, vertical) => {
  if (!by) return false;
  const style = getComputedStyle(el);
  const overflow = vertical ? style.overflowY : style.overflowX;
  const page = el === document.scrollingElement || el === document.documentElement || el === document.body;
  if (!page && !/(auto|scroll|overlay)/.test(overflow)) return false;
  const at = vertical ? el.scrollTop : el.scrollLeft;
  const most = vertical ? el.scrollHeight - el.clientHeight : el.scrollWidth - el.clientWidth;
  return by > 0 ? at < most - 1 : at > 0;
};

const scroller = (from) => {
  for (let el = from; el && el !== document.documentElement; el = el.parentElement) {
    if (room(el, A.dy, true) || room(el, A.dx, false)) return el;
  }
  return document.scrollingElement || document.documentElement;
};

const run = () => {
  if (A.kind === "tap") {
    if (!inside(A.x, A.y)) return { error: "That point is not on the page." };
    globalThis.__hotlineArrived = false;
    document.addEventListener("click", () => {
      globalThis.__hotlineArrived = true;
    }, { capture: true, once: true });
    return {};
  }
  if (A.kind === "click") {
    const el = document.elementFromPoint(A.x, A.y);
    if (el) {
      el.focus?.();
      el.click();
    }
    return {};
  }
  if (A.kind === "scroll") {
    const under = A.x == null ? null : document.elementFromPoint(A.x, A.y);
    scroller(under).scrollBy({ left: A.dx || 0, top: A.dy || 0, behavior: "instant" });
    return {};
  }
  if (A.kind === "text") {
    const el = document.activeElement;
    if (!el || el === document.body || el === document.documentElement) {
      return { error: "Nothing on the page has the focus: tap a field first." };
    }
    if (el instanceof HTMLIFrameElement) {
      return { error: "That field is inside a frame, which typing from away does not reach yet." };
    }
    const field = el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement;
    if (!field && !el.isContentEditable) {
      return { error: "What has the focus does not take text: tap a field first." };
    }
    // As if typed: the page hears beforeinput and input, and its own
    // undo has it.
    let typed = false;
    try {
      typed = document.execCommand("insertText", false, A.text);
    } catch {}
    if (!typed && field) {
      let start = null;
      try {
        start = el.selectionStart;
      } catch {}
      if (start === null) {
        el.value += A.text;
      } else {
        el.setRangeText(A.text, start, el.selectionEnd, "end");
      }
      el.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: A.text }));
    } else if (!typed) {
      return { error: "That field did not take the text." };
    }
    return {};
  }
  return { error: `No input ${A.kind}.` };
};

return JSON.stringify({ ...size, ...run() });
