// The body of an async function the desk runs in its own content world,
// with `A` and walk.js in front, before an act the person can watch: it
// brings the act's element into view, smoothly when it is off screen, and
// answers its centre for the teammate's pointer to move to; with no ref, the
// middle of the view. It acts on nothing. Returns JSON: { point? }.
const still = async (read) => {
  let last = read();
  for (let i = 0; i < 16; i++) {
    await new Promise((done) => setTimeout(done, 50));
    const now = read();
    if (now === last) return;
    last = now;
  }
};
const middle = { x: innerWidth / 2, y: innerHeight / 2, height: innerHeight };
if (!A.ref) return JSON.stringify({ point: middle });
const el = refs.get(A.ref);
if (!el || !el.isConnected) return JSON.stringify({});
const before = el.getBoundingClientRect();
const seen = before.top >= 0 && before.bottom <= innerHeight && before.left >= 0 && before.right <= innerWidth;
if (!seen) {
  el.scrollIntoView({ block: "center", inline: "center", behavior: "smooth" });
  await still(() => Math.round(el.getBoundingClientRect().top));
  // A scroll act then says it scrolled, not that it was on screen already.
  globalThis.__hotlineBrought = A.ref;
}
const rect = el.getBoundingClientRect();
if (rect.width === 0 && rect.height === 0) return JSON.stringify({});
return JSON.stringify({ point: { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2, height: innerHeight } });
