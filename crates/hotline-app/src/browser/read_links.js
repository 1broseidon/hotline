// The body of an async function the desk runs in its own content world:
// every visible link on the page, once each, as `- text -> href` lines.
// Returns JSON: { title, url, text }.
const clean = (s) => (s || "").replace(/\s+/g, " ").trim();
const seen = new Set();
const lines = [];
for (const a of document.querySelectorAll("a[href]")) {
  if (!a.href || a.href.startsWith("javascript:")) continue;
  const rect = a.getBoundingClientRect();
  if (rect.width === 0 && rect.height === 0) continue;
  const text = clean(a.innerText || a.getAttribute("aria-label") || a.title).slice(0, 200);
  const key = text + "\u0000" + a.href;
  if (seen.has(key)) continue;
  seen.add(key);
  lines.push(`- ${text || "(no text)"} -> ${a.href}`);
}
return JSON.stringify({ title: document.title, url: location.href, text: lines.join("\n") });
