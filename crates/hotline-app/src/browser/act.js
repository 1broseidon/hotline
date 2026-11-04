// The body of an async function the desk runs in its own content world: one
// act on a ref from the last text read. The desk puts `A` and walk.js in
// front of it, so `refs` numbers the page as the read did. `A` is:
// { act, ref, text, values, on, dy, find, timeout }. A click, a check that
// must change and a hover answer the point to act at, in the view's CSS
// pixels with the viewport's height, and the desk sends a real mouse event
// there; the rest act here.
// Returns JSON: { point?, note?, error? }.
const element = () => {
  const el = refs.get(A.ref);
  if (!el || !el.isConnected) return null;
  return el;
};

// How an act names an element: a field by its label, never by its value,
// so a password typed into it never comes back.
const describe = (el) => {
  const field = el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement || el instanceof HTMLSelectElement;
  const kind = el instanceof HTMLInputElement ? `${el.type || "text"} field` : field ? "field" : el.tagName.toLowerCase();
  const name = field ? labelOf(el) : el.innerText || el.getAttribute("aria-label") || el.title || "";
  const text = clean(name).slice(0, 60);
  return `${kind}${text ? ` "${text}"` : ""}`;
};

// Scrolls the element into the middle of the view and answers its centre,
// and what is on top there if something else covers it.
const pointAt = (el) => {
  // The desk asks afterwards whether its click reached the page at all.
  globalThis.__hotlineArrived = false;
  document.addEventListener(A.act === "hover" ? "mouseover" : "click", () => {
    globalThis.__hotlineArrived = true;
  }, { capture: true, once: true });
  el.scrollIntoView({ block: "center", inline: "center", behavior: "instant" });
  const rect = el.getBoundingClientRect();
  if (rect.width === 0 && rect.height === 0) return { error: `${A.ref} is not visible on the page.` };
  const x = rect.left + rect.width / 2;
  const y = rect.top + rect.height / 2;
  const top = document.elementFromPoint(x, y);
  const covered = top && top !== el && !el.contains(top) && !top.contains(el);
  return {
    point: { x, y, height: window.innerHeight },
    note: covered
      ? `${describe(el)} is covered by ${describe(top)}, which takes the click.`
      : `On ${describe(el)}.`,
  };
};

const setValue = (el, value) => {
  const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  const setter = Object.getOwnPropertyDescriptor(proto, "value")?.set;
  if (setter) setter.call(el, value);
  else el.value = value;
  el.dispatchEvent(new Event("input", { bubbles: true }));
  el.dispatchEvent(new Event("change", { bubbles: true }));
};

const wait = (ms) => new Promise((done) => setTimeout(done, ms));

const run = async () => {
  if (A.act === "scroll") {
    if (A.ref) {
      const el = element();
      if (!el) return { error: `${A.ref} is no longer on the page: read it again.` };
      el.scrollIntoView({ block: "center", behavior: "instant" });
      return { note: `Scrolled to ${describe(el)}.` };
    }
    window.scrollBy({ top: A.dy || window.innerHeight * 0.8, behavior: "instant" });
    const end = window.scrollY + window.innerHeight >= document.documentElement.scrollHeight - 2;
    return { note: end ? "At the bottom of the page." : "Scrolled." };
  }
  if (A.act === "wait_for") {
    const until = Date.now() + (A.timeout || 10000);
    while (Date.now() < until) {
      if (A.find && (document.body?.innerText || "").includes(A.find)) return { note: `"${A.find}" is on the page.` };
      if (A.ref) {
        const el = element();
        if (el && el.getClientRects().length > 0) return { note: `${A.ref} is on the page.` };
      }
      await wait(200);
    }
    return { error: `Still not on the page after ${Math.round((A.timeout || 10000) / 1000)} seconds.` };
  }
  if (A.act === "press") {
    // The key itself is sent natively to the focused element.
    return { note: document.activeElement && document.activeElement !== document.body ? `Pressed in ${describe(document.activeElement)}.` : undefined };
  }
  const el = element();
  if (!el) return { error: `${A.ref} is no longer on the page: read it again with text.` };
  if (A.act === "click" || A.act === "hover") return pointAt(el);
  if (A.act === "fill") {
    if (!(el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement) && !el.isContentEditable) {
      return { error: `${A.ref} is ${describe(el)}, not a field.` };
    }
    el.scrollIntoView({ block: "center", behavior: "instant" });
    el.focus();
    if (el.isContentEditable) {
      el.textContent = A.text;
      el.dispatchEvent(new InputEvent("input", { bubbles: true }));
    } else {
      setValue(el, A.text);
    }
    return { note: `${A.text === "" ? "Cleared" : "Filled"} ${describe(el)}.` };
  }
  if (A.act === "select") {
    if (!(el instanceof HTMLSelectElement)) return { error: `${A.ref} is ${describe(el)}, not a select.` };
    const wanted = A.values || [];
    const options = [...el.options];
    const missing = wanted.filter((v) => !options.some((o) => o.value === v || clean(o.text) === v));
    if (missing.length) {
      return { error: `No option ${missing.map((v) => JSON.stringify(v)).join(", ")}. It has: ${options.map((o) => JSON.stringify(clean(o.text))).slice(0, 30).join(", ")}.` };
    }
    for (const option of options) option.selected = wanted.includes(option.value) || wanted.includes(clean(option.text));
    el.dispatchEvent(new Event("input", { bubbles: true }));
    el.dispatchEvent(new Event("change", { bubbles: true }));
    return { note: `Selected ${wanted.join(", ")}.` };
  }
  if (A.act === "check") {
    const checked = "checked" in el ? el.checked : el.getAttribute("aria-checked") === "true";
    if (checked === A.on) return { note: A.on ? "Already checked." : "Already unchecked." };
    return pointAt(el);
  }
  return { error: `No act ${A.act}.` };
};

return JSON.stringify(await run());
