// The masthead mark, at work. A port of the window's own working glyph
// (ui/src/ui/Glyph.tsx and ui/src/ui/receiver.ts): the same poses, the
// same handset, the same line. Here it runs one turn on a loop: thinking,
// reading, searching, editing, running, the reply down the line, the
// hang-up and the wink, then a rest on the cradle.
(() => {
  const svg = document.getElementById("mglyph"); if (!svg) return;

  // receiver.ts: the brow, the small drawing the working glyph moves.
  const G = { gap: 3.4, thick: 3.8, lips: 156, sag: 0 };
  const LEFT = 20, RIGHT = 44, Y = 30, EYE = 10.5;
  const rad = (d) => (d * Math.PI) / 180;
  const n = (v) => +v.toFixed(2);
  const at = (cx, r, d) => [cx + r * Math.cos(rad(d)), Y - r * Math.sin(rad(d))];
  const pt = (p) => `${n(p[0])} ${n(p[1])}`;
  const receiverPath = (g) => {
    const r = EYE + g.gap, R = r + g.thick, e = g.thick / 2;
    const join = (Math.asin(Math.min(1, (R - g.sag) / R)) * 180) / Math.PI;
    const oL = at(LEFT, R, g.lips), iL = at(LEFT, r, g.lips), oR = at(RIGHT, R, 180 - g.lips), iR = at(RIGHT, r, 180 - g.lips);
    return `M${pt(oL)}A${n(R)} ${n(R)} 0 0 1 ${pt(at(LEFT, R, join))}` +
      `L${pt(at(RIGHT, R, 180 - join))}A${n(R)} ${n(R)} 0 0 1 ${pt(oR)}` +
      `A${n(e)} ${n(e)} 0 0 1 ${pt(iR)}A${n(r)} ${n(r)} 0 0 0 ${RIGHT} ${n(Y - r)}` +
      `L${LEFT} ${n(Y - r)}A${n(r)} ${n(r)} 0 0 0 ${pt(iL)}A${n(e)} ${n(e)} 0 0 1 ${pt(oL)}Z`;
  };
  const receiverLine = (g, count) => {
    const r = EYE + g.gap, mid = r + g.thick / 2, arc = mid * rad(g.lips - 90), span = RIGHT - LEFT, total = 2 * arc + span;
    const points = [];
    for (let i = 0; i < count; i++) {
      const s = (total * i) / (count - 1);
      if (s < arc) points.push(at(LEFT, mid, g.lips - (s / mid) * (180 / Math.PI)));
      else if (s < arc + span) { const x = LEFT + (s - arc); points.push([x, Y - mid + (g.sag / 2) * Math.sin((Math.PI * (x - LEFT)) / span)]); }
      else points.push(at(RIGHT, mid, 90 - ((s - arc - span) / mid) * (180 / Math.PI)));
    }
    const top = Y - r - g.thick, bottom = Y - mid * Math.sin(rad(g.lips)) + g.thick / 2;
    return { points, centre: [32, (top + bottom) / 2] };
  };
  svg.querySelector(".g-receiver").setAttribute("d", receiverPath(G));

  // Glyph.tsx: the poses.
  const REST = { pupil: 0, pupilY: 0, eyeL: 1, eyeR: 1, wide: 1, body: 1, rot: 0, dy: 0, lift: 0, tilt: 0, hx: 0, hy: 0, ht: 0 };
  const DARTS = [-1, 0.6, -0.3, 1, -0.8, 0.2, 0.9, -0.6];
  const KEYS = [0, 0.12, 0.22, 0.4, 0.49, 0.63, 0.71, 0.86];
  const PHRASE = 1.15, HOP = 0.62, LINE_POINTS = 80, MORPH = 0.5, REEL = 0.4, DROP = 0.3, CLUNK = 0.12, WINK = 0.56;
  const LINE_WIDTH = G.thick;
  const ease = (p) => (p < 0.5 ? 4 * p ** 3 : 1 - (-2 * p + 2) ** 3 / 2);
  const easeIn = (p) => p ** 3;
  const clamp01 = (p) => Math.max(0, Math.min(1, p));
  const winkAt = (p) => (p < 0.36 ? 1 - 0.92 * ease(p / 0.36) : p < 0.45 ? 0.08 : p < 0.78 ? 0.08 + 0.92 * ease((p - 0.45) / 0.33) : 1);
  const hop = (t) => {
    const p = (((t % HOP) + HOP) % HOP) / HOP;
    if (p < 0.2) { const k = ease(p / 0.2); return { dy: 1.4 * k, body: 1 - 0.16 * k }; }
    if (p < 0.72) { const q = (p - 0.2) / 0.52; return { dy: 1.4 - 7.4 * Math.sin(q * Math.PI) - 1.4 * q, body: q < 0.3 ? 0.84 + 0.24 * ease(q / 0.3) : 1.08 - 0.08 * ease((q - 0.3) / 0.7) }; }
    const q = (p - 0.72) / 0.28; return { dy: 1.2 * Math.sin(q * Math.PI), body: 1 - 0.12 * Math.sin(q * Math.PI) };
  };
  const poseOf = (phase, t) => {
    switch (phase) {
      case "thinking": return { ...REST, rot: -3.6 * Math.sin(t * 0.7), dy: 2.2 * Math.sin(t * 0.5), pupil: 1.8 * Math.sin(t * 0.42), pupilY: -2.2, lift: 8, tilt: -16 + 3 * Math.sin(t * 0.6) };
      case "read": { const p = (t / 1.7) % 1; const x = p < 0.82 ? -1 + (p / 0.82) * 2 : 1 - ((p - 0.82) / 0.18) * 2; return { ...REST, pupil: x * 2.8, pupilY: 1.3, eyeL: 0.62, eyeR: 0.62, lift: 3 }; }
      case "search": {
        const step = t / 0.42, k = Math.floor(step) % DARTS.length, prev = DARTS[(k + DARTS.length - 1) % DARTS.length];
        const d = prev + (DARTS[k] - prev) * ease(clamp01((step - Math.floor(step)) / 0.28));
        return { ...REST, pupil: d * 3, pupilY: DARTS[(k + 3) % DARTS.length] * 1.4, rot: d * 5, lift: 5, tilt: -d * 9 };
      }
      case "edit": {
        const c = t % PHRASE; let tap = 0, side = 1, keys = 0;
        for (let i = 0; i < KEYS.length && c >= KEYS[i]; i++) { keys = i + 1; side = i % 2 ? -1 : 1; tap = Math.exp(-(c - KEYS[i]) / 0.05); }
        return { ...REST, pupil: -2.6 + (5.2 * keys) / KEYS.length, pupilY: 1.5, body: 1 - 0.06 * tap, dy: 0.7 * tap, rot: 1.3 * side * tap, lift: 2.5, hx: 3, tilt: 20, hy: -0.4 * tap };
      }
      case "execute": { const h = hop(t), lag = hop(t - 0.07); return { ...REST, body: h.body, dy: h.dy, pupilY: -0.4, lift: 4, tilt: -3, hy: lag.dy - h.dy }; }
      default: return { ...REST };
    }
  };
  const voiceLine = (t) => {
    const pts = [];
    for (let i = 0; i < LINE_POINTS; i++) { const u = i / (LINE_POINTS - 1), x = 10 + 44 * u; pts.push([x, 10 + 2.6 * Math.sin(Math.PI * u) ** 0.6 * Math.sin((2 * Math.PI * x) / 9 - 2 * Math.PI * 1.4 * t)]); }
    return pts;
  };
  const watching = (pose, t, m) => { pose.pupilY = -1.8 * m; pose.pupil = 2.2 * Math.sin(t * 2.4) * m; pose.dy = 1.2 * m; };
  const HANDSET = receiverLine(G, LINE_POINTS);
  const heldLine = (lift, tilt, hx) => {
    const [cx, cy] = HANDSET.centre, a = (tilt * Math.PI) / 180, c = Math.cos(a), s = Math.sin(a);
    return HANDSET.points.map(([x, y]) => [cx + (x - cx) * c - (y - cy) * s + hx, cy + (x - cx) * s + (y - cy) * c - lift]);
  };
  const mix = (a, b, m) => a.map(([x, y], i) => [x + (b[i][0] - x) * m, y + (b[i][1] - y) * m]);
  // At rest the toad sits alone, as the app's own chrome wears it; the handset is only in hand during a turn.
  let hand = false;
  const set = (sel, name, v) => svg.querySelector(sel).setAttribute(name, v);
  const paint = (pose, held, line) => {
    const w = pose.wide, f = (v) => Math.max(0.001, v).toFixed(3);
    set(".g-all", "transform", `translate(0 ${pose.dy.toFixed(2)}) rotate(${pose.rot.toFixed(2)} 32 34)`);
    set(".g-body", "transform", `scale(1 ${f(pose.body)})`);
    set(".g-eyeL", "transform", `scale(${f(w)} ${f(pose.eyeL * w)})`);
    set(".g-eyeR", "transform", `scale(${f(w)} ${f(pose.eyeR * w)})`);
    set(".g-pupils", "transform", `translate(${pose.pupil.toFixed(2)} ${pose.pupilY.toFixed(2)}) scale(1 ${f(Math.max(pose.eyeL, pose.eyeR) * w)})`);
    set(".g-hand", "transform", `translate(${held.hx.toFixed(2)} ${(pose.hy - held.lift).toFixed(2)})`);
    set(".g-receiver", "transform", `rotate(${(held.tilt + pose.ht).toFixed(2)})`);
    set(".g-hand", "opacity", line || !hand ? "0" : "1");
    set(".g-line", "opacity", line ? "1" : "0");
    if (line) { set(".g-line", "d", `M${line.points.map(([x, y]) => `${x.toFixed(2)} ${y.toFixed(2)}`).join("L")}`); set(".g-line", "stroke-width", line.width.toFixed(2)); }
  };

  if (matchMedia("(prefers-reduced-motion: reduce)").matches) { paint({ ...REST }, { lift: 0, tilt: 0, hx: 0 }, null); return; }

  // One turn, on a loop. Each kind of work opens with a blink, as in the window.
  const TURN = [["rest", 1.8], ["thinking", 3.0], ["read", 3.4], ["search", 2.6], ["edit", 3.45], ["execute", 2.48], ["writing", 3.2], ["landed", REEL + DROP + CLUNK + WINK + 0.1]];
  const LEN = TURN.reduce((a, [, d]) => a + d, 0);
  const pin = new URLSearchParams(location.search).get("mt"); // ?mt=seconds holds one moment, for frame checks
  const held = { lift: 0, tilt: 0, hx: 0 };
  let entry = { ...held }, spoken = 0, current = null, last = 0, blinkAt = -1, nextBlink = 0.7;
    const draw = (clock, dt) => {
    const tl = clock % LEN;
    let acc = 0, phase = "rest", t = 0;
    for (const [p, d] of TURN) { if (tl < acc + d) { phase = p; t = tl - acc; break; } acc += d; }
    hand = phase !== "rest";
    if (phase !== current) { current = phase; entry = { ...held }; if (phase !== "writing" && phase !== "landed" && phase !== "rest") blinkAt = clock; }
    if (clock > nextBlink) { blinkAt = clock; nextBlink = clock + 2.4 + Math.random() * 4; }
    const gap = clock - blinkAt, blink = gap >= 0 && gap < 0.18 ? 0.08 + 0.92 * Math.abs(gap / 0.09 - 1) : 1;
    const pose = poseOf(phase, t);
    pose.eyeL = Math.min(pose.eyeL, blink); pose.eyeR = Math.min(pose.eyeR, blink);
    const k = 1 - Math.exp(-dt / 0.08);
    let line = null;
    if (phase === "writing") {
      const m = ease(clamp01(t / MORPH));
      line = { points: mix(heldLine(entry.lift, entry.tilt, entry.hx), voiceLine(t), m), width: LINE_WIDTH };
      watching(pose, t, m); spoken = t;
    } else if (phase === "landed") {
      if (t < REEL) {
        const m = ease(t / REEL);
        line = { points: mix(voiceLine(spoken + t), heldLine(3, 0, 0), m), width: LINE_WIDTH };
        watching(pose, spoken + t, 1 - m);
        Object.assign(held, { lift: 3, tilt: 0, hx: 0 });
      } else {
        const c = t - REEL;
        held.lift = c < DROP ? 3 * (1 - easeIn(c / DROP)) : 0;
        const d = c - DROP;
        if (d >= 0 && d < CLUNK) pose.body = 1 - 0.1 * Math.sin((d / CLUNK) * Math.PI);
        else if (d >= CLUNK && d < CLUNK + WINK) pose.eyeL = Math.min(pose.eyeL, winkAt((d - CLUNK) / WINK));
      }
    } else if (phase === "search" && t > 0.25) {
      Object.assign(held, { lift: pose.lift, tilt: pose.tilt, hx: pose.hx });
    } else {
      held.lift += (pose.lift - held.lift) * k; held.tilt += (pose.tilt - held.tilt) * k; held.hx += (pose.hx - held.hx) * k;
    }
    paint(pose, held, line);
  };
  if (pin !== null) { for (let c = 0; c <= +pin; c += 0.05) draw(c, 0.05); return; }
  // 20 frames a second on a timer, as in the window. When the page opens the
  // mark only sends its reply down the line, hangs up and winks (under five
  // seconds, so it needs no pause control); pointing at it plays the whole turn.
  let t0 = performance.now(), timer = 0;
  const tick = () => {
    const clock = (performance.now() - t0) / 1000;
    if (clock >= LEN) { last = 0; current = null; hand = false; paint({ ...REST }, { lift: 0, tilt: 0, hx: 0 }, null); timer = 0; return; }
    timer = setTimeout(tick, 50);
    if (document.hidden) return;
    draw(clock, Math.min(0.1, clock - last)); last = clock;
  };
  const play = (from = 0) => { if (timer) return; t0 = performance.now() - from * 1000; last = 0; Object.assign(held, { lift: 0, tilt: 0, hx: 0 }); nextBlink = from + 0.7; tick(); };
  const brand = svg.closest("a");
  if (brand) { brand.addEventListener("pointerenter", () => play()); brand.addEventListener("focus", () => play()); }
  play(TURN.slice(0, -2).reduce((a, [, d]) => a + d, 0));
})();
