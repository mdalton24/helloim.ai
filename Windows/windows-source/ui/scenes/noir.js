// noir.js — NOIR theme scene: a cyan/blue HUD reticle.
//
// Ported value-for-value from the helloim-site's src/scenes/noir.ts (round
// 80 rebuild, 2026-09-07) — see lib/faceScene.js's header for the port
// rationale. This REPLACES the previous noir.js, which was "rain on an
// office window, venetian blind light" — Mark's own instruction was a
// direction change away from that look entirely, not a retouch of it, so
// nothing from the old file carries forward.
//
// NOIR — "Hard-Boiled Gumshoe", rebuilt (round 80) as a DIRECTION CHANGE per
// Mark's own instruction: no longer rain + venetian blinds. It is now a
// futuristic cyan/blue HUD, built against noir-spec.md and the source of
// truth it points at (a real HTML/CSS mockup,
// noir_visualizer_blue.md).
// That markup was rendered standalone in a real browser before writing a
// line of this file (saved: NOIR-css-reference.png) rather than guessed from
// reading the CSS — the ◉ glyph's heavy text-shadow blooms it into a solid
// glowing disc with a bright rim, which is not obvious from the stylesheet
// alone and only confirmed by looking.
//
// THE ELEMENTS, read straight off .noir-core/.noir-ring/.noir-symbol:
//   - a glowing circular core: dark radial centre, thin bright cyan rim,
//     soft outer glow (box-shadow in the source, drawn as layered radial
//     gradients here)
//   - two static concentric rings around it: one solid faint ring, one
//     dashed dimmer ring further out
//   - a rotating partial-ring accent: roughly 3/4 of a circle, bright cyan
//     at the top, fading toward a faint tail, with its own glow — the one
//     element that turns
//   - a centre reticle: a glowing circle-in-circle (the ◉), no text under it
//     — the source's "NOIR ACTIVE" label is a literal HUD caption and is
//     DROPPED per this house's no-on-canvas-text rule; the spec's own
//     instruction is explicit that only the glyph stays
//   - a faint horizontal scanline overlay and a vignette, dark blue-black
//     ground with two soft radial glows
//
// REACTIVE, mapped onto this app's own contract (mode/level/ping/flareSign
// via lib/faceScene.js — identical shape to the site's):
//   - idle: slow accent-ring rotation, gentle core breathing, scanlines
//     drift very slowly.
//   - listening: steadier and a touch brighter — reading, not idling.
//   - thinking: rotation speeds up, a second counter-rotating tick ring
//     appears ("more ring complexity"), core brightens.
//   - talking: st.level drives core/rim brightness and the accent ring's
//     glow smoothly — a HUD reading answering amplitude, never a bar
//     equalizer.
//   - flare(true): one confident lock-on sweep — a thin ring travelling
//     outward from the reticle through the static rings, then gone.
//   - flare(false): a quick double-flicker on the core — a stutter, not a
//     colour change; this HUD has one palette and does not break it for an
//     error.
//
// Distinct from every sibling scene by construction: it is the only one
// built from concentric circles and straight radial ticks rather than
// orbits, wisps, ribbons or a dot-cloud, and it is the only scene on a pure
// cyan/blue monochrome palette.
//
// Every colour on screen derives from CONFIG below.

import { createScene, fillFrame, glow } from './lib/faceScene.js';

const CONFIG = {
  dark: '#02070c',
  darkMid: '#04111a',
  cyan: '#22b8ff',
  cyan2: '#78dcff',
  blue: '#0789d8',
  line: '#22b8ff', // rgba(34,184,255,.72) as a hex + alpha pair below
  rim: '#64d7ff', // rgba(100,215,255,.72)
  coreFill: '#0b4362', // rgba(11,67,98,.38)
};

function hexToRgb(hex) {
  const n = parseInt(hex.slice(1), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}
function rgba(hex, a) {
  const [r, g, b] = hexToRgb(hex);
  return `rgba(${r},${g},${b},${Math.min(1, Math.max(0, a))})`;
}
function mixWhite(hex, amt) {
  const [r, g, b] = hexToRgb(hex);
  const m = (c) => Math.round(c + (255 - c) * amt);
  return '#' + [m(r), m(g), m(b)].map((c) => c.toString(16).padStart(2, '0')).join('');
}

/* THE APPEARANCE OVERRIDE — Mastermind round 95, index.html's dock
 * (applyAppearance() -> window.RiftBrain.setPalette() -> here, via
 * lib/faceScene.js's own forwarding — see that file's comment). Noir was
 * chosen as one of the two scenes to wire for real (the other is koan.js)
 * because its whole identity is already "a pure cyan/blue monochrome
 * palette" (this file's own header) — an aesthetic choice, not information
 * the way board.js's inbound/outbound/alert wiring or capcom.js's telemetry
 * lanes are (see this app's handback report for why those two were left
 * alone). Recolours the SIGNAL tones only (cyan/cyan2/line/rim) — the ones
 * that actually read as "the HUD's colour" — and leaves the near-black
 * ground (dark/darkMid/blue/coreFill) untouched, on purpose: those carry
 * depth and contrast, not hue, and swapping them risked the legibility a
 * five-minute render couldn't fully prove out. CONFIG IS MUTATED IN PLACE,
 * NOT REASSIGNED — every draw() helper above closes over this one object
 * and reads it fresh every frame (confirmed by reading the file: nothing
 * caches CONFIG.cyan etc. into a local at scene-build time), so mutating
 * its properties is picked up on the very next animation frame with no
 * rebuild call needed. `colors` of null (Appearance reset to "Follow skin")
 * restores Noir's own original hexes rather than leaving whatever the last
 * override painted. */
const NOIR_DEFAULT = { cyan: CONFIG.cyan, cyan2: CONFIG.cyan2, line: CONFIG.line, rim: CONFIG.rim };
function applyNoirPalette(colors) {
  if (!colors || !colors.accent) {
    Object.assign(CONFIG, NOIR_DEFAULT);
    return;
  }
  CONFIG.cyan = colors.accent;
  CONFIG.line = colors.accent;
  CONFIG.cyan2 = mixWhite(colors.accent, 0.35);
  CONFIG.rim = mixWhite(colors.accent, 0.25);
}

export function mount(canvas) {
  return createScene(canvas, (s) => {
    let cx = 0;
    let cy = 0;
    let unit = 1;
    let coreR = 1;
    let accentTheta = -0.6; // matches the source's rotate(-35deg) start
    let tickTheta = 0;
    let energy = 0.3;
    let scanOffset = 0;
    let flicker = 0; // >0 while the error stutter plays
    let sweeps = [];

    const layout = () => {
      cx = s.w * 0.5;
      cy = s.h * 0.48;
      unit = Math.min(s.w, s.h);
      coreR = unit * 0.22;
      sweeps = [];
    };

    const drawGround = (ctx) => {
      const w = s.w;
      const h = s.h;
      fillFrame({ ctx, w, h }, CONFIG.dark);
      const lg = ctx.createLinearGradient(0, 0, w, h);
      lg.addColorStop(0, rgba(CONFIG.dark, 1));
      lg.addColorStop(0.48, rgba(CONFIG.darkMid, 1));
      lg.addColorStop(1, rgba(CONFIG.dark, 1));
      ctx.fillStyle = lg;
      ctx.fillRect(0, 0, w, h);
      glow({ ctx, w, h }, w * 0.5, h * 0.45, Math.max(w, h) * 0.5, rgba(CONFIG.blue, 0.16), 1);
      glow({ ctx, w, h }, w * 0.2, h * 0.1, Math.max(w, h) * 0.4, rgba(CONFIG.blue, 0.1), 1);
    };

    // Two static concentric rings, per .noir-core:before/:after — one solid
    // faint, one dashed dimmer, both further out than the core's own rim.
    const drawStaticRings = (ctx, heat) => {
      ctx.save();
      ctx.beginPath();
      ctx.arc(cx, cy, coreR + unit * 0.045, 0, Math.PI * 2);
      ctx.strokeStyle = rgba(CONFIG.cyan, 0.3 * heat);
      ctx.lineWidth = 1;
      ctx.stroke();

      ctx.setLineDash([unit * 0.012, unit * 0.014]);
      ctx.beginPath();
      ctx.arc(cx, cy, coreR + unit * 0.08, 0, Math.PI * 2);
      ctx.strokeStyle = rgba(CONFIG.cyan, 0.16 * heat);
      ctx.lineWidth = 1;
      ctx.stroke();
      ctx.restore();
    };

    // The one element that turns: a partial ring, bright at the top of its
    // own rotation, fading toward a faint tail — the source's noir-ring.
    const drawAccentRing = (ctx, theta, heat, glowAmt) => {
      const r = coreR + unit * 0.02;
      const w = Math.max(2, unit * 0.012);
      ctx.save();
      ctx.lineCap = 'round';
      ctx.shadowColor = rgba(CONFIG.cyan, 0.8 * heat);
      ctx.shadowBlur = unit * 0.03 * (0.6 + glowAmt);
      ctx.beginPath();
      ctx.arc(cx, cy, r, theta, theta + Math.PI * 1.15);
      ctx.strokeStyle = rgba(CONFIG.cyan2, 0.95 * heat);
      ctx.lineWidth = w;
      ctx.stroke();
      ctx.beginPath();
      ctx.arc(cx, cy, r, theta + Math.PI * 1.15, theta + Math.PI * 1.75);
      ctx.strokeStyle = rgba(CONFIG.cyan, 0.35 * heat);
      ctx.lineWidth = w;
      ctx.stroke();
      ctx.restore();
    };

    // Thinking-only: a second, thinner, counter-rotating ring of ticks —
    // "more ring complexity," never a spinner because it never completes a
    // clean single rotation cue on its own (it reads as texture, not motion).
    const drawTickRing = (ctx, theta, heat) => {
      const r = coreR + unit * 0.12;
      const n = 24;
      for (let i = 0; i < n; i++) {
        const a = theta + (i / n) * Math.PI * 2;
        const lit = (i % 3) === 0;
        ctx.strokeStyle = rgba(CONFIG.cyan, (lit ? 0.5 : 0.18) * heat);
        ctx.lineWidth = 1;
        ctx.beginPath();
        ctx.moveTo(cx + Math.cos(a) * r, cy + Math.sin(a) * r);
        ctx.lineTo(cx + Math.cos(a) * (r + unit * 0.015), cy + Math.sin(a) * (r + unit * 0.015));
        ctx.stroke();
      }
    };

    const drawCore = (ctx, heat, breath) => {
      const r = coreR * breath;
      const g = ctx.createRadialGradient(cx, cy, 0, cx, cy, r);
      g.addColorStop(0, rgba(CONFIG.coreFill, 0.55 * heat));
      g.addColorStop(0.6, rgba(CONFIG.dark, 0.85));
      g.addColorStop(1, rgba(CONFIG.dark, 0.95));
      ctx.save();
      ctx.beginPath();
      ctx.arc(cx, cy, r, 0, Math.PI * 2);
      ctx.fillStyle = g;
      ctx.fill();
      // outer soft glow (box-shadow stand-in) + inset glow
      ctx.globalCompositeOperation = 'lighter';
      const outer = ctx.createRadialGradient(cx, cy, r * 0.85, cx, cy, r * 1.5);
      outer.addColorStop(0, rgba(CONFIG.blue, 0.18 * heat));
      outer.addColorStop(1, rgba(CONFIG.blue, 0));
      ctx.fillStyle = outer;
      ctx.beginPath();
      ctx.arc(cx, cy, r * 1.5, 0, Math.PI * 2);
      ctx.fill();
      ctx.globalCompositeOperation = 'source-over';
      // the rim
      ctx.beginPath();
      ctx.arc(cx, cy, r, 0, Math.PI * 2);
      ctx.strokeStyle = rgba(CONFIG.rim, 0.72 * heat);
      ctx.lineWidth = 1;
      ctx.stroke();
      ctx.restore();
    };

    // The ◉ reticle — a glowing circle-in-circle, exactly what the source's
    // heavily-shadowed glyph blooms into once rendered (confirmed by
    // rendering it, not assumed). No text is ever drawn under it.
    const drawReticle = (ctx, heat, lockT) => {
      const r = unit * 0.052 * (1 + lockT * 0.25);
      ctx.save();
      ctx.globalCompositeOperation = 'lighter';
      const halo = ctx.createRadialGradient(cx, cy, 0, cx, cy, r * 3.4);
      halo.addColorStop(0, rgba(CONFIG.cyan2, 0.55 * heat));
      halo.addColorStop(1, rgba(CONFIG.cyan2, 0));
      ctx.fillStyle = halo;
      ctx.beginPath();
      ctx.arc(cx, cy, r * 3.4, 0, Math.PI * 2);
      ctx.fill();

      const disc = ctx.createRadialGradient(cx, cy, 0, cx, cy, r);
      disc.addColorStop(0, `rgba(255,255,255,${Math.min(1, 0.95 * heat).toFixed(3)})`);
      disc.addColorStop(0.65, rgba(CONFIG.cyan2, 0.9 * heat));
      disc.addColorStop(1, rgba(CONFIG.cyan, 0.2));
      ctx.fillStyle = disc;
      ctx.beginPath();
      ctx.arc(cx, cy, r, 0, Math.PI * 2);
      ctx.fill();
      ctx.globalCompositeOperation = 'source-over';
      ctx.beginPath();
      ctx.arc(cx, cy, r * 1.35, 0, Math.PI * 2);
      ctx.strokeStyle = rgba(CONFIG.cyan2, 0.6 * heat);
      ctx.lineWidth = 1.2;
      ctx.stroke();
      ctx.restore();
    };

    const drawScanlines = (ctx, offset) => {
      const w = s.w;
      const h = s.h;
      ctx.save();
      ctx.strokeStyle = rgba(CONFIG.cyan2, 0.035);
      ctx.lineWidth = 1;
      const step = 5;
      ctx.beginPath();
      for (let y = -step + (offset % step); y < h; y += step) {
        ctx.moveTo(0, y);
        ctx.lineTo(w, y);
      }
      ctx.stroke();
      ctx.restore();
    };

    const drawVignette = (ctx) => {
      const w = s.w;
      const h = s.h;
      const vg = ctx.createRadialGradient(w / 2, h / 2, Math.min(w, h) * 0.25, w / 2, h / 2, Math.max(w, h) * 0.72);
      vg.addColorStop(0, 'rgba(0,0,0,0)');
      vg.addColorStop(1, 'rgba(0,2,4,0.55)');
      ctx.fillStyle = vg;
      ctx.fillRect(0, 0, w, h);
    };

    return {
      layout,
      setPalette: applyNoirPalette,
      onBump: (st) => {
        if (st.reduced) return;
        // a soft reading tick — HUD amplitude, never a bar; folded into the
        // continuous level-driven heat in draw() rather than a discrete mark
      },
      onFlare: (st, ok) => {
        if (st.reduced) return;
        if (ok) {
          sweeps.push({ r: coreR * 0.3, a: 1 });
        } else {
          flicker = 0.35;
        }
      },
      draw: (st) => {
        const ctx = st.ctx;

        const targetEnergy =
          st.mode === 'thinking' ? 0.72 : st.mode === 'talking' ? 0.45 + st.level * 0.5 : st.mode === 'listening' ? 0.48 : 0.32;
        energy += (targetEnergy - energy) * Math.min(1, st.dt * 2.2);

        if (flicker > 0 && !st.reduced) flicker = Math.max(0, flicker - st.dt * 2.4);
        const flickerOn = flicker > 0 && Math.floor(flicker * 18) % 2 === 0;

        const rotSpeed = (st.mode === 'thinking' ? 0.34 : st.mode === 'listening' ? 0.14 : 0.11) * (flickerOn ? 0 : 1);
        accentTheta += rotSpeed * st.dt;
        tickTheta -= (st.mode === 'thinking' ? 0.2 : 0.08) * st.dt;
        scanOffset += 6 * st.dt;

        const heat = (0.55 + energy * 0.55) * (flickerOn ? 0.25 : 1);
        const breath = 1 + Math.sin((st.t * Math.PI * 2) / (st.mode === 'thinking' ? 3.4 : 5.5)) * (0.02 + energy * 0.012);

        drawGround(ctx);
        drawStaticRings(ctx, heat);
        if (st.mode === 'thinking') drawTickRing(ctx, tickTheta, heat);
        drawAccentRing(ctx, accentTheta, heat, energy);
        drawCore(ctx, heat, breath);

        // lock-on sweeps (flare true): a thin ring travelling outward
        sweeps = sweeps.filter((sw) => sw.a > 0.02 && sw.r < unit * 0.6);
        if (!st.reduced) {
          for (const sw of sweeps) {
            sw.r += unit * 0.6 * st.dt;
            sw.a -= st.dt * 1.1;
            ctx.save();
            ctx.globalCompositeOperation = 'lighter';
            ctx.strokeStyle = rgba(CONFIG.cyan2, Math.max(0, sw.a) * 0.6);
            ctx.lineWidth = 1.4;
            ctx.beginPath();
            ctx.arc(cx, cy, sw.r, 0, Math.PI * 2);
            ctx.stroke();
            ctx.restore();
          }
        }

        drawReticle(ctx, heat, sweeps.length ? sweeps[0].a : 0);
        if (!st.reduced) drawScanlines(ctx, scanOffset);
        drawVignette(ctx);
      },
    };
  });
}
