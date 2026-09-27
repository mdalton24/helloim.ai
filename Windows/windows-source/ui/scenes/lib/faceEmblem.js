// faceEmblem.js — the Jarvis face scene's centre mark, live on canvas.
//
// SECOND-HOP PORT, same reasoning as ./faceScene.js: this is
// helloim-app's renderer/lib/emblem.js (itself a value-for-value port of
// helloim-v4's src/lib/emblem.ts's paintEmblem()) with the
// window.HelloimEmblem global replaced by a named export — every ring, arc
// and colour below is unchanged.
//
// USED ONLY BY scenes/jarvis.js (the ported face scene). This app already
// has its OWN, unrelated emblem renderer inside scenes/board.js
// (drawEmblem, lifted from the real board and driven by hover/hoverable
// state this simpler mark does not have) — the two are deliberately
// separate files with no shared state, because they draw two different
// things: board.js's is the live board's reactive mark, this is the
// simpler non-reactive DOOR-EMBLEM form also used on /login and /account.
// Do not merge them.
const CYAN = [79, 214, 224]; // PAL.inbound on the board
const GOLD = [255, 195, 107]; // PAL.outbound on the board
const TAU = Math.PI * 2;

const rgba = (c, a) => `rgba(${c[0]},${c[1]},${c[2]},${a})`;

function ring(ctx, cx, cy, r, from, to, width, col, alpha) {
  ctx.beginPath();
  ctx.arc(cx, cy, r, from, to);
  ctx.strokeStyle = rgba(col, alpha);
  ctx.lineWidth = width;
  ctx.stroke();
}

/**
 * Paints the emblem centred at (cx, cy) with outer radius r. Caller owns
 * clearing/painting the rest of the frame — this only draws the mark.
 *
 * opts: { spin?: radians (advance yourself, e.g. from scene time — the
 * emblem owns no clock), ink?: [r,g,b] (defaults to the board's cyan; swap
 * to alert red on a failure flare), label?: string }
 */
function paintEmblem(ctx, cx, cy, r, opts) {
  opts = opts || {};
  const s = opts.spin || 0;
  const ink = opts.ink || CYAN;
  // Default to no wordmark. A caller that wants text passes opts.label
  // explicitly; the hardcoded 'J.A.R.V.I.S' default was a latent violation of
  // the rule that the word JARVIS never reaches canvas (board.js took the same
  // fix, swapping the identical hardcoded wordmark for the live assistant name).
  const label = opts.label || '';

  ctx.save();
  ctx.lineCap = 'butt';

  // 1. outermost hairline, two small breaks
  ring(ctx, cx, cy, r * 0.973, s * 0.2 + 0.1, s * 0.2 + Math.PI - 0.06, Math.max(1, r * 0.01), ink, 0.4);
  ring(ctx, cx, cy, r * 0.973, s * 0.2 + Math.PI + 0.1, s * 0.2 + TAU - 0.06, Math.max(1, r * 0.01), ink, 0.4);

  // 2. two heavier arcs, opposite each other
  ring(ctx, cx, cy, r * 0.893, -s * 0.3 - 0.55, -s * 0.3 + 0.55, Math.max(1, r * 0.017), ink, 0.62);
  ring(ctx, cx, cy, r * 0.893, -s * 0.3 + Math.PI - 0.4, -s * 0.3 + Math.PI + 0.4, Math.max(1, r * 0.017), ink, 0.52);

  // 3. a dense fine tick arc over one quadrant
  if (r > 24) {
    ctx.lineWidth = Math.max(0.6, r * 0.007);
    for (let i = 0; i < 34; i++) {
      const a = s * 0.45 + Math.PI * 0.72 + (i / 34) * Math.PI * 0.56;
      ctx.strokeStyle = rgba(ink, 0.42);
      ctx.beginPath();
      ctx.moveTo(cx + Math.cos(a) * r * 0.787, cy + Math.sin(a) * r * 0.787);
      ctx.lineTo(cx + Math.cos(a) * r * 0.853, cy + Math.sin(a) * r * 0.853);
      ctx.stroke();
    }
  }

  // 4. small square blocks along an arc — the detail that reads as data, not decoration
  if (r > 24) {
    for (let i = 0; i < 26; i++) {
      const a = -s * 0.6 - Math.PI * 0.3 + (i / 26) * Math.PI * 1.15;
      const rr = r * 0.727;
      const sz = r * (i % 4 === 0 ? 0.03 : 0.02);
      ctx.save();
      ctx.translate(cx + Math.cos(a) * rr, cy + Math.sin(a) * rr);
      ctx.rotate(a);
      ctx.fillStyle = rgba(ink, 0.46);
      ctx.fillRect(-sz / 2, -sz / 2, sz, sz);
      ctx.restore();
    }
  }

  // 5. dotted ring, the other way, smaller and sparser
  if (r > 16) {
    for (let i = 0; i < 44; i++) {
      const a = s * 0.75 + (i / 44) * TAU;
      const rr = r * 0.617;
      ctx.fillStyle = rgba(ink, 0.32);
      ctx.beginPath();
      ctx.arc(cx + Math.cos(a) * rr, cy + Math.sin(a) * rr, Math.max(0.6, r * 0.008), 0, TAU);
      ctx.fill();
    }
  }

  // 6. broken hairline just outside the face
  for (let i = 0; i < 3; i++) {
    const a0 = -s * 0.22 + i * (TAU / 3) + 0.16;
    ring(ctx, cx, cy, r * 0.543, a0, a0 + TAU / 3 - 0.32, Math.max(1, r * 0.007), ink, 0.36);
  }

  // 6b. the amber arcs — the board's second voice, gold instead of cyan, load-bearing colour
  const g = s * 0.9;
  ring(ctx, cx, cy, r * 0.803, g, g + Math.PI * 0.52, Math.max(1, r * 0.017), GOLD, 0.72);
  ring(ctx, cx, cy, r * 0.803, g + Math.PI * 0.72, g + Math.PI * 0.86, Math.max(1, r * 0.017), GOLD, 0.5);
  ring(ctx, cx, cy, r * 0.677, -s * 0.65 + Math.PI * 0.4, -s * 0.65 + Math.PI * 0.74, Math.max(1, r * 0.013), GOLD, 0.58);
  for (let i = 0; i < 3; i++) {
    const a = g + Math.PI * 0.1 + i * Math.PI * 0.16;
    ctx.fillStyle = rgba(GOLD, 0.75);
    ctx.beginPath();
    ctx.arc(cx + Math.cos(a) * r * 0.803, cy + Math.sin(a) * r * 0.803, Math.max(1, r * 0.014), 0, TAU);
    ctx.fill();
  }

  // 7. the bright inner circle — the one crisp, fully closed line
  ring(ctx, cx, cy, r * 0.45, 0, TAU, Math.max(1, r * 0.013), ink, 0.8);

  // 8. the face, knocked back so the wordmark stays readable over the rings
  ctx.fillStyle = 'rgba(3,9,16,0.52)';
  ctx.beginPath();
  ctx.arc(cx, cy, r * 0.443, 0, TAU);
  ctx.fill();

  // 9. the wordmark — measured and shrunk to fit, skipped below a legibility floor
  const baseSize = r * 0.135;
  if (baseSize >= 5) {
    const fitW = r * 0.74;
    let fs = Math.max(6, baseSize);
    ctx.font = `500 ${fs}px ui-monospace,"SF Mono",Menlo,Consolas,"DejaVu Sans Mono",monospace`;
    const measured = ctx.measureText(label).width;
    if (measured > fitW) {
      fs = Math.max(5, fs * (fitW / measured));
      ctx.font = `500 ${fs}px ui-monospace,"SF Mono",Menlo,Consolas,"DejaVu Sans Mono",monospace`;
    }
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.fillStyle = 'rgba(226,244,255,0.9)';
    ctx.fillText(label, cx, cy);
  }

  ctx.restore();
}

export { paintEmblem };
