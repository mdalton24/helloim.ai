// capcom.js — CAPCOM theme scene: an orbital instrument chamber, mission
// control telemetry with a Center Hologram.
//
// Ported value-for-value from the helloim-site's src/scenes/capcom.ts
// (round 84 rebuild, 2026-09-07) — see lib/faceScene.js's header for the
// port rationale. This REPLACES the previous 09-02 capcom.js (three
// scrolling telemetry traces + a small lower-right orbit accent, with
// on-canvas text labels/status strip); the round-84 site rebuild keeps the
// telemetry plane but drops all on-canvas text and moves the orbital
// mechanism to a centred, grown "Center Hologram" per capcom-spec.md.
//
// CAPCOM — an orbital instrument chamber, refined (round 84) toward a
// mission-control CENTER HOLOGRAM per capcom-spec.md / capcom-spec-source.md.
// Same gap as Blitz: the source names a reference image that was never
// attached (checked both locations, neither has it) — this works from the
// prose spec alone, said plainly rather than pretending otherwise.
//
// WHAT CHANGED FROM ROUND 74, and why the rest was left alone: that pass
// already reads well at card size (three legible telemetry traces under
// glass, a graph-paper instrument plane) — the brief says keep that. What
// moved is the orbital mechanism itself: it was a small accent tucked in the
// lower-right corner, and the spec's own "Center Hologram" section asks for
// a mission object on an orbital path AS THE CENTREPIECE, ringed by
// telemetry and status lights. So it moved to centre stage and grew, the
// background instrument plane stayed exactly what it was.
//
// STATE-DRIVEN COLOUR, mapped onto this app's own contract (mode/level/ping/
// flareSign via lib/faceScene.js — identical shape to the site's), per the
// spec's own vocabulary:
//   - NOMINAL (idle/listening): calm cyan + green, steady orbit, no alarms.
//   - WATCH (thinking): an amber ring appears around the hologram in
//     addition to the cyan rings — never replacing them, "amber orbit ring
//     appears" is additive in the source.
//   - OFF-NOMINAL (onFlare(true) here — a notable event, not necessarily a
//     failure): ONE status light and one ring segment flash amber/red and
//     recover — "flashes amber/red WITHOUT taking over the entire
//     interface," so the rest of the hologram stays nominal throughout.
//   - CRITICAL (onFlare(false) — a real failure): the periphery (grid,
//     traces, status lights) dims and the central object goes high-contrast
//     red, per the source's own "peripheral UI dims, central alert becomes
//     dominant" — then recovers, since this app's contract has no
//     persistent alarm state, only a decaying transient.
//   - Talking: the object and rings answer st.level as a confident callout —
//     brightness and orbital speed lift smoothly, never a bar graph.
//
// No text, no labels, no HUD chrome, no "CAPCOM // FLIGHT DIRECTOR" — the
// spec's own header bar and mode-selector are page furniture around the
// visualizer, not the visualizer; this instrument reads by light and motion
// alone, same rule as before. (This is also why the old MET clock/status
// strip/lane labels are gone — the round-84 site source dropped them and
// this port carries that change forward, not just the visual mechanism.)
//
// Every colour on screen derives from C below.

import { createScene, fillFrame, glow } from './lib/faceScene.js';

const C = {
  // round-74: the alpha-only lift on the grid/orbital measured a real but
  // visually-imperceptible gain at true card size (verified by an empirical
  // A/B render, not assumed) — one subtle background step per the design
  // lead's fallback clause, still deep near-black.
  background: '#070e13',
  panel: '#07141b',
  grid: '#12303a',
  vox: '#f4b84b',
  attention: '#44c8ed',
  power: '#57d59a',
  highlight: '#d9f4f6',
  warning: '#ff765c',
  // round-84: the spec's explicit WATCH/CRITICAL colours, kept distinct from
  // `vox` (amber already in use for the telemetry lane) and `warning`
  // (the softer ping wash) so a real off-nominal/critical event is never
  // mistaken for ordinary telemetry colour.
  watch: '#ffb545',
  critical: '#ff3b30',
};

// TIME_SCALE = 0.5 globally, so a 14s wall loop is a 7-unit period in st.t.
const LOOP_T = 7;

function hexRgb(hex) {
  const h = hex.replace('#', '');
  return [parseInt(h.slice(0, 2), 16), parseInt(h.slice(2, 4), 16), parseInt(h.slice(4, 6), 16)];
}
function rgba(hex, a) {
  const [r, g, b] = hexRgb(hex);
  return `rgba(${r},${g},${b},${a})`;
}
function lerpRgba(hexA, hexB, t, a) {
  const [r1, g1, b1] = hexRgb(hexA);
  const [r2, g2, b2] = hexRgb(hexB);
  const r = Math.round(r1 + (r2 - r1) * t);
  const g = Math.round(g1 + (g2 - g1) * t);
  const b = Math.round(b1 + (b2 - b1) * t);
  return `rgba(${r},${g},${b},${a})`;
}

const LANE_COLORS = [C.vox, C.attention, C.power];

export function mount(canvas) {
  return createScene(canvas, (s) => {
    let hist = [[], [], []];
    let cap = 240;
    let acc = 0;
    let prevLevel = 0;
    let orbitTrail = [];
    let burst = 0; // decaying amber-white pulse at the orbital intersection
    let offNominal = 0; // >0 while ONE status light + ring segment flash amber/red
    let offNominalLight = 0; // which status light is flashing, chosen once per event
    let critical = 0; // >0 while the periphery dims and the object goes red

    const layout = () => {
      cap = Math.max(140, Math.round(s.w / 2));
      hist = [[], [], []];
      orbitTrail = [];
    };

    return {
      layout,
      onBump: (_st, v) => {
        if (v > 0.35) burst = Math.max(burst, 0.7);
      },
      onFlare: (_st, ok) => {
        if (ok) {
          // OFF-NOMINAL: one subsystem flashes, the rest of the hologram
          // stays nominal throughout — never a full-interface alarm.
          offNominal = 1;
          offNominalLight = Math.floor(Math.random() * 10);
        } else {
          // CRITICAL: the real failure state — periphery dims, the object
          // itself goes high-contrast red.
          critical = 1;
        }
        burst = ok ? 0.6 : 0.3;
      },
      draw: (st) => {
        const ctx = st.ctx;
        const w = st.w;
        const h = st.h;
        fillFrame(st, C.background);

        const thinking = st.mode === 'thinking';
        const talking = st.mode === 'talking';
        const loop = (st.t % LOOP_T) / LOOP_T;
        // near-imperceptible parallax drift, a couple of px over the loop
        const driftX = Math.sin(loop * Math.PI * 2) * 2.2;
        const driftY = Math.cos(loop * Math.PI * 2) * 1.3;

        // --- chamber vignette, gives the enclosure its depth -----------------
        const vg = ctx.createRadialGradient(w / 2, h * 0.45, Math.min(w, h) * 0.2, w / 2, h * 0.5, Math.max(w, h) * 0.72);
        vg.addColorStop(0, rgba(C.panel, 0.35));
        vg.addColorStop(1, rgba(C.background, 1));
        ctx.fillStyle = vg;
        ctx.fillRect(0, 0, w, h);

        ctx.save();
        ctx.translate(driftX, driftY);

        // --- the recessed instrument plane, viewed with a slight elevated tilt
        const pad = Math.min(w, h) * 0.07;
        const top = h * 0.11;
        const bottom = h * 0.9;
        const inset = (w - pad * 2) * 0.032;
        const TL = { x: pad + inset, y: top };
        const TR = { x: w - pad - inset, y: top };
        const BR = { x: w - pad, y: bottom };
        const BL = { x: pad, y: bottom };

        ctx.save();
        ctx.beginPath();
        ctx.moveTo(TL.x, TL.y);
        ctx.lineTo(TR.x, TR.y);
        ctx.lineTo(BR.x, BR.y);
        ctx.lineTo(BL.x, BL.y);
        ctx.closePath();
        ctx.clip();

        // panel substrate, lit noticeably above black so it reads as a living
        // instrument even at rest — this was the note from the last pass.
        const panelGrad = ctx.createLinearGradient(0, top, 0, bottom);
        panelGrad.addColorStop(0, rgba(C.panel, 0.95));
        panelGrad.addColorStop(1, rgba(C.background, 0.95));
        ctx.fillStyle = panelGrad;
        ctx.fillRect(pad, top, w - pad * 2, bottom - top);

        // ambient backlight — always on, brightens with thinking/talking
        const ambient = 0.16 + st.level * 0.12 + (thinking ? 0.08 : 0);
        glow(st, w * 0.62, top + (bottom - top) * 0.45, Math.max(w, h) * 0.5, rgba(C.attention, 1), ambient * 0.5);
        glow(st, w * 0.3, top + (bottom - top) * 0.6, Math.max(w, h) * 0.35, rgba(C.power, 1), ambient * 0.35);

        // fine graph-paper geometry, raised baseline luminance
        ctx.strokeStyle = rgba(C.grid, 0.4); // round-74: +25% baseline so the graph-paper reads at card size
        ctx.lineWidth = 1;
        const g = 20;
        ctx.beginPath();
        for (let x = pad; x < w - pad; x += g) {
          ctx.moveTo(x, top);
          ctx.lineTo(x, bottom);
        }
        for (let y = top; y < bottom; y += g) {
          ctx.moveTo(pad, y);
          ctx.lineTo(w - pad, y);
        }
        ctx.stroke();
        // thinking: a soft shimmering noise band sweeping across the channels
        if (thinking) {
          const bw = w * (0.18 + 0.06 * Math.sin(st.t * 0.5));
          const bx = pad - bw + ((st.t * 0.28) % 1) * (w - pad * 2 + bw * 2);
          const band = ctx.createLinearGradient(bx, 0, bx + bw, 0);
          band.addColorStop(0, rgba(C.highlight, 0));
          band.addColorStop(0.5, rgba(C.highlight, 0.1));
          band.addColorStop(1, rgba(C.highlight, 0));
          ctx.fillStyle = band;
          ctx.fillRect(bx, top, bw, bottom - top);
        }

        // --- three telemetry traces, shallow illuminated channels under glass
        acc += st.dt;
        while (acc > 1 / 60) {
          acc -= 1 / 60;
          const noise = thinking ? 0.22 : 0.07;
          hist[0].push(st.level * 0.95 + (Math.random() - 0.5) * noise); // vox: micro-noise
          hist[1].push(Math.sin(st.t * 1.1) * 0.4 + (Math.random() - 0.5) * noise * 0.4); // attention: slow controlled wave
          hist[2].push(0.3 + Math.sin(st.t * 0.5) * 0.08 + st.level * 0.18); // power: stable low pulse
          for (const hb of hist) if (hb.length > cap) hb.shift();
        }

        const laneSpan = (bottom - top) * 0.6;
        const laneH = laneSpan / 3.2;
        for (let k = 0; k < 3; k++) {
          const baseY = top + laneH * (k + 0.75);
          const color = LANE_COLORS[k];

          // recessed channel groove
          ctx.fillStyle = rgba(C.grid, 0.28); // round-74: +25% baseline, matches the grid lift above
          ctx.fillRect(pad, baseY - laneH * 0.34, w - pad * 2, laneH * 0.68);
          // resting light in the channel — lit even at idle
          const edgeLit = 0.4 + st.level * (k === 0 ? 0.5 : 0.15) + (talking && k === 0 ? 0.3 : 0);
          ctx.strokeStyle = rgba(color, 0.35 + ambient * 0.4); // round-74: +25% baseline (structure, not the trace itself)
          ctx.lineWidth = 1;
          ctx.beginPath();
          ctx.moveTo(pad, baseY - laneH * 0.34);
          ctx.lineTo(w - pad, baseY - laneH * 0.34);
          ctx.moveTo(pad, baseY + laneH * 0.34);
          ctx.lineTo(w - pad, baseY + laneH * 0.34);
          ctx.stroke();

          ctx.strokeStyle = rgba(color, 0.55 + edgeLit * 0.4);
          ctx.lineWidth = k === 0 ? 2 : 1.3;
          ctx.beginPath();
          const hb = hist[k];
          for (let i = 0; i < hb.length; i++) {
            const x = pad + (i / cap) * (w - pad * 2);
            const y = baseY - hb[i] * laneH * 0.42;
            if (i === 0) ctx.moveTo(x, y);
            else ctx.lineTo(x, y);
          }
          ctx.stroke();

          if (hb.length) {
            const x = pad + ((hb.length - 1) / cap) * (w - pad * 2);
            const y = baseY - hb[hb.length - 1] * laneH * 0.42;
            ctx.fillStyle = rgba(color, 0.9);
            ctx.beginPath();
            ctx.arc(x, y, k === 0 ? 3 : 2.2, 0, Math.PI * 2);
            ctx.fill();
            if (k === 0 && talking) glow(st, x, y, 14, rgba(color, 1), 0.5);
          }
        }

        ctx.restore(); // end panel clip

        // subtle glass edge highlight along the recessed plane
        ctx.strokeStyle = rgba(C.highlight, 0.15 + ambient * 0.2); // round-74: +25% baseline, the panel needs a visible bezel
        ctx.lineWidth = 1;
        ctx.beginPath();
        ctx.moveTo(TL.x, TL.y);
        ctx.lineTo(TR.x, TR.y);
        ctx.lineTo(BR.x, BR.y);
        ctx.lineTo(BL.x, BL.y);
        ctx.closePath();
        ctx.stroke();

        // --- CENTRE HOLOGRAM: the mission object on its orbital path -----------
        // Moved from a lower-right accent to the true centre of the panel, and
        // grown, per the spec's own "Center Hologram" section — everything
        // else above (the graph-paper plane, the three telemetry lanes) is
        // untouched, because it already reads well at rest.
        const ox = (TL.x + TR.x) / 2;
        const oy = (top + bottom) / 2;
        const orx = Math.min(w, h) * 0.15;
        const ory = orx * 0.48;
        const tilt = -0.28;
        const cosT = Math.cos(tilt);
        const sinT = Math.sin(tilt);

        // periphery dim during CRITICAL — confined to the instrument plane,
        // never the object itself, per "peripheral UI dims, central alert
        // becomes dominant."
        if (critical > 0.01) {
          ctx.save();
          ctx.beginPath();
          ctx.moveTo(TL.x, TL.y);
          ctx.lineTo(TR.x, TR.y);
          ctx.lineTo(BR.x, BR.y);
          ctx.lineTo(BL.x, BL.y);
          ctx.closePath();
          ctx.clip();
          ctx.fillStyle = `rgba(3,6,9,${(critical * 0.6).toFixed(3)})`;
          ctx.fillRect(pad, top, w - pad * 2, bottom - top);
          ctx.restore();
        }

        // guide rings — NOMINAL cyan, always present. brighten while thinking.
        const ringLit = 0.32 + ambient * 0.3 + (thinking ? 0.2 : 0);
        for (let ring = 0; ring < 3; ring++) {
          const rscale = 0.62 + ring * 0.22;
          ctx.strokeStyle = rgba(C.attention, ringLit * (1 - ring * 0.18) * (1 - critical * 0.4));
          ctx.lineWidth = 1;
          ctx.beginPath();
          ctx.ellipse(ox, oy, orx * rscale, ory * rscale, tilt, 0, Math.PI * 2);
          ctx.stroke();
        }

        // WATCH ring — amber, additive, appears only while thinking. Never
        // replaces the cyan rings: "amber orbit ring appears" is additive.
        if (thinking) {
          ctx.strokeStyle = rgba(C.watch, 0.4 + ambient * 0.2);
          ctx.lineWidth = 1.4;
          ctx.setLineDash([4, 5]);
          ctx.beginPath();
          ctx.ellipse(ox, oy, orx * 1.14, ory * 1.14, tilt, 0, Math.PI * 2);
          ctx.stroke();
          ctx.setLineDash([]);
        }

        // status lights — a ring of small ticks around the hologram. All
        // nominal green; exactly one flashes amber/red during OFF-NOMINAL,
        // and none of the others are touched — "without taking over the
        // entire interface" is the whole point of this element.
        const lightCount = 10;
        for (let i = 0; i < lightCount; i++) {
          const a = (i / lightCount) * Math.PI * 2;
          const lx = Math.cos(a) * orx * 1.42;
          const ly = Math.sin(a) * ory * 1.42;
          const px2 = ox + lx * cosT - ly * sinT;
          const py2 = oy + lx * sinT + ly * cosT;
          const isFlashing = offNominal > 0.01 && i === offNominalLight;
          const lightCol = isFlashing ? lerpRgba(C.watch, C.critical, 0.5, 1) : rgba(C.power, 1);
          const lightAlpha = isFlashing ? 0.4 + offNominal * 0.6 : (0.3 + ambient * 0.3) * (1 - critical * 0.5);
          ctx.fillStyle = isFlashing ? lightCol : rgba(C.power, lightAlpha);
          if (isFlashing) ctx.globalAlpha = lightAlpha;
          ctx.beginPath();
          ctx.arc(px2, py2, isFlashing ? 2.6 : 1.6, 0, Math.PI * 2);
          ctx.fill();
          ctx.globalAlpha = 1;
        }

        // the central object — cyan/green NOMINAL, shifting to high-contrast
        // red under CRITICAL. Untouched by OFF-NOMINAL (that stays localised
        // to the one status light above).
        const heartbeat = 0.5 + Math.sin(st.t * 1.1) * 0.5;
        const sphereR = orx * 0.24;
        const objectCol = critical > 0.01 ? lerpRgba(C.power, C.critical, critical, 1) : rgba(C.power, 1);
        const sg = ctx.createRadialGradient(ox, oy, 0, ox, oy, sphereR * 2);
        sg.addColorStop(0, critical > 0.01 ? objectCol : rgba(C.power, 0.44 + heartbeat * 0.12 + st.level * 0.15));
        sg.addColorStop(1, rgba(critical > 0.01 ? C.critical : C.power, 0));
        ctx.fillStyle = sg;
        ctx.beginPath();
        ctx.arc(ox, oy, sphereR * 2, 0, Math.PI * 2);
        ctx.fill();

        // the moving light point, tracing its orbit — mission progress
        const angSpeed = (0.4 * Math.PI * 2) / LOOP_T + st.level * 1.4 + (thinking ? 0.6 : 0);
        const ang = st.t * angSpeed;
        const ex = Math.cos(ang) * orx;
        const ey = Math.sin(ang) * ory;
        const px = ox + ex * cosT - ey * sinT;
        const py = oy + ex * sinT + ey * cosT;

        if (thinking) {
          orbitTrail.push({ x: px, y: py, a: 1 });
          if (orbitTrail.length > 18) orbitTrail.shift();
        } else if (orbitTrail.length) {
          orbitTrail.shift();
        }
        // the fading trail, the burst and the ping wash below are all
        // one-shot transients — a reduced-motion freeze must land on the
        // calm base state, never mid-burst, so each is skipped while frozen.
        if (!st.reduced) {
          for (let i = 0; i < orbitTrail.length; i++) {
            const tp = orbitTrail[i];
            tp.a *= 0.9;
            ctx.fillStyle = rgba(C.highlight, tp.a * 0.35);
            ctx.beginPath();
            ctx.arc(tp.x, tp.y, 1.4, 0, Math.PI * 2);
            ctx.fill();
          }
        }

        ctx.fillStyle = rgba(C.highlight, 0.85 + st.level * 0.15);
        ctx.beginPath();
        ctx.arc(px, py, 2.4, 0, Math.PI * 2);
        ctx.fill();
        glow(st, px, py, 10, rgba(C.highlight, 1), 0.35 + st.level * 0.2);

        // strong peaks: a contained amber-white pulse at the orbital intersection
        const delta = st.level - prevLevel;
        prevLevel = st.level;
        if (talking && delta > 0.16) burst = Math.max(burst, Math.min(1, delta * 2.2));
        burst *= Math.pow(0.001, st.dt);
        if (!st.reduced && burst > 0.01) {
          glow(st, ox, oy, orx * (1.2 + burst * 1.4), lerpRgba(C.vox, C.highlight, 0.5, 1), burst * 0.7);
        }
        // OFF-NOMINAL decays quickly (a call-out, not a state change).
        // CRITICAL lingers longer, matching the spec's own "dominant" framing
        // — both are transients on this app's contract either way; there is
        // no persistent alarm slot, so a real sustained failure would need
        // the host to keep calling flare(false), not this scene to latch it.
        offNominal *= Math.pow(0.02, st.dt);
        critical *= Math.pow(0.15, st.dt);

        ctx.restore(); // end parallax drift

        if (!st.reduced && st.ping > 0.01) {
          const ok = st.flareSign >= 0;
          ctx.fillStyle = ok ? rgba(C.power, st.ping * 0.1) : rgba(C.warning, st.ping * 0.12);
          ctx.fillRect(0, 0, w, h);
        }
      },
    };
  });
}
