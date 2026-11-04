// The body of an async function the desk runs in its own content world, out
// of the page's sight: the page as lines a model can read. Headings are
// `#` lines, list items `-` lines, and every link and control is marked in
// place with a ref the next phase acts on. A password field never says its
// value. Returns JSON: { title, url, text }.
const LIMIT = 200;
const clean = (s) => (s || "").replace(/\s+/g, " ").trim();
const short = (s) => {
  s = clean(s);
  return s.length > LIMIT ? s.slice(0, LIMIT) + "…" : s;
};
const quote = (s) => JSON.stringify(short(s));
// A link on this site is its path; one elsewhere, its whole address.
const where = (a) =>
  a.origin === location.origin ? a.pathname + a.search + a.hash : a.href;

const refs = new Map();
globalThis.__hotlineRefs = refs;
let count = 0;
const ref = (el) => {
  const id = "e" + ++count;
  refs.set(id, new WeakRef(el));
  return id;
};

const hidden = (el) => {
  if (el.hidden || el.getAttribute("aria-hidden") === "true") return true;
  const style = getComputedStyle(el);
  return style.display === "none" || style.visibility === "hidden";
};

const labelOf = (el) => {
  const aria = el.getAttribute("aria-label");
  if (clean(aria)) return aria;
  const by = el.getAttribute("aria-labelledby");
  if (by) {
    const text = by
      .split(/\s+/)
      .map((id) => document.getElementById(id)?.innerText || "")
      .join(" ");
    if (clean(text)) return text;
  }
  if (el.id) {
    const label = document.querySelector('label[for="' + CSS.escape(el.id) + '"]');
    if (label && clean(label.innerText)) return label.innerText;
  }
  const wrap = el.closest("label");
  if (wrap && clean(wrap.innerText)) return wrap.innerText;
  return (
    el.getAttribute("placeholder") ||
    el.getAttribute("title") ||
    el.getAttribute("name") ||
    ""
  );
};

const SKIP = new Set([
  "SCRIPT", "STYLE", "NOSCRIPT", "TEMPLATE", "SVG", "HEAD", "META", "LINK",
  "IFRAME", "CANVAS", "OBJECT", "EMBED",
]);
const BLOCK = new Set([
  "P", "DIV", "SECTION", "ARTICLE", "MAIN", "HEADER", "FOOTER", "NAV", "ASIDE",
  "UL", "OL", "TABLE", "THEAD", "TBODY", "TR", "BR", "HR", "FORM", "FIELDSET",
  "DL", "DT", "DD", "PRE", "BLOCKQUOTE", "FIGURE", "FIGCAPTION", "DETAILS",
  "SUMMARY", "DIALOG", "BODY",
]);

const lines = [];
let line = "";
const flush = () => {
  const text = clean(line);
  if (text) lines.push(text);
  line = "";
};
const put = (text) => {
  line += " " + text + " ";
};

const control = (el) => {
  const tag = el.tagName;
  const role = (el.getAttribute("role") || "").toLowerCase();
  if (tag === "A" && el.href) {
    const text = el.innerText || el.getAttribute("aria-label") || el.title || where(el);
    return `[link ${quote(text)} ${ref(el)} -> ${where(el)}]`;
  }
  if (tag === "BUTTON" || role === "button") {
    return `[button ${quote(el.innerText || labelOf(el))} ${ref(el)}${el.disabled ? " disabled" : ""}]`;
  }
  if (tag === "INPUT") {
    const type = (el.type || "text").toLowerCase();
    if (type === "hidden") return "";
    if (["submit", "button", "reset", "image"].includes(type)) {
      return `[button ${quote(el.value || labelOf(el) || type)} ${ref(el)}]`;
    }
    if (type === "checkbox" || type === "radio") {
      return `[${type} ${quote(labelOf(el))} ${ref(el)}${el.checked ? " checked" : ""}]`;
    }
    if (type === "password") {
      return `[password ${quote(labelOf(el))} ${ref(el)}${el.value ? " filled" : ""}]`;
    }
    const value = el.value ? ` value=${quote(el.value)}` : "";
    return `[${type === "text" ? "textbox" : type} ${quote(labelOf(el))} ${ref(el)}${value}]`;
  }
  if (tag === "TEXTAREA") {
    const value = el.value ? ` value=${quote(el.value)}` : "";
    return `[textbox ${quote(labelOf(el))} ${ref(el)}${value}]`;
  }
  if (tag === "SELECT") {
    const chosen = [...el.selectedOptions].map((o) => clean(o.text)).join(", ");
    return `[select ${quote(labelOf(el))} ${ref(el)}${chosen ? " value=" + quote(chosen) : ""}]`;
  }
  if (["checkbox", "radio", "switch", "tab", "menuitem", "option", "link"].includes(role)) {
    const state = el.getAttribute("aria-checked") === "true" || el.getAttribute("aria-selected") === "true";
    return `[${role} ${quote(el.innerText || labelOf(el))} ${ref(el)}${state ? " checked" : ""}]`;
  }
  return null;
};

const walk = (node) => {
  if (node.nodeType === Node.TEXT_NODE) {
    line += node.nodeValue;
    return;
  }
  if (node.nodeType !== Node.ELEMENT_NODE) return;
  const el = node;
  const tag = el.tagName;
  if (SKIP.has(tag) || hidden(el)) return;
  if (/^H[1-6]$/.test(tag)) {
    flush();
    lines.push("#".repeat(Number(tag[1])) + " " + short(el.innerText));
    return;
  }
  const marked = control(el);
  if (marked !== null) {
    if (marked) put(marked);
    return;
  }
  if (tag === "IMG") {
    const alt = clean(el.getAttribute("alt"));
    if (alt) put(`[image ${quote(alt)}]`);
    return;
  }
  const block = BLOCK.has(tag);
  if (tag === "LI") {
    flush();
    line = "- ";
  } else if (tag === "TD" || tag === "TH") {
    line += " | ";
  } else if (block) {
    flush();
  }
  for (const child of el.childNodes) walk(child);
  if (el.shadowRoot) for (const child of el.shadowRoot.childNodes) walk(child);
  if (block || tag === "LI") flush();
};

if (document.body) walk(document.body);
flush();
return JSON.stringify({ title: document.title, url: location.href, text: lines.join("\n") });
