// jarvis.js — JARVIS theme scene: the HUD ring and its emblem, alone.
//
// CHANGED 2026-08-31 — after seeing 0.2.7 running on Windows, the call was
// to drop the background and keep only the ring. The ring itself
// (paintEmblem() — the board's own DOOR-EMBLEM mark, faithfully ported, see
// renderer/lib/emblem.js) stayed; the circuit board underneath it
// — substrate wash, the orthogonal trace lattice, vias, IC packages, and the
// cyan/gold/red signal packets that used to travel the traces — was noise
// at this size. All of that is gone from this file now, not hidden
// behind a flag: dead geometry left in "just in case" is exactly the loose
// end this house's rules say not to leave.
//
// What replaced it: a flat dark field (C.bg, unchanged) plus a slow, very
// subtle radial wash centred on the emblem itself, breathing gently with
// st.level. Three options were on the table — flat colour, a subtle wash, or
// literally nothing behind the ring — and flat-alone was rejected because a
// ring on a perfectly featureless rectangle reads as adrift rather than as
// the one thing in the frame; "literally nothing" isn't achievable here
// either, since the canvas context is alpha:false and something must occupy
// every pixel. The wash is the same glow() helper and the same cyan the
// board's own IC-package glow used to use, so a streamed token (still
// bump()'d from overlay.js's onDelta — see that file) still has somewhere
// to land instead of the signal simply vanishing along with the board that
// used to carry it.
//
// The emblem's own reactivity is untouched: spin rate still follows
// st.mode (idle/listening/thinking/talking) and its ink still turns alert
// red under a failure flare — see drawCenterEmblem() below, ported as-is
// from before this change. The flare ring from pulse()/flare() is likewise
// untouched: it was always independent of the board.
//
// Ported originally from helloim-v4's src/scenes/jarvis.ts — see
// renderer/lib/scene.js's header for the full port note. This file has now
// diverged from that source (the marketing site's full-width jarvis scene
// still runs the circuit board; only this 420×280 overlay panel had it
// removed), which is why this change touches only this file.

import { createScene, fillFrame, glow } from './lib/faceScene.js';
import { paintEmblem } from './lib/faceEmblem.js';

const C = {
  bg: '#03080c',
  inbound: 'rgb(79, 214, 224)',
  alert: 'rgb(255, 59, 48)',
};

export function mount(canvas, name) {
  // `name` is accepted to match the scene mount() interface but deliberately
  // unused — this theme's identity is the emblem's own mark, and nothing here
  // may put a word (least of all JARVIS) on the canvas.
  return createScene(canvas, (s) => {
    /* THE CENTREPIECE IS THE ACTUAL EMBLEM, not a silkscreened name — this
       theme's whole identity is the board's own mark rendered at the
       centre of the scene.

       paintEmblem() is a faithful port of the board's DOOR-EMBLEM form —
       the same non-reactive rendering already used on /login, /account and
       the phone board, so this reads as the same mark everywhere it
       appears, not a fourth interpretation of it. See renderer/lib/emblem.js
       for why that form and not the board's live, hover-and-audio-driven
       drawEmblem().

       Sized at ~0.22 of the smaller dimension — unchanged by the
       background removal above; this was always sized to read as the
       centre of the scene, not as filling it. `name` is ignored
       deliberately: this theme's identity is the emblem's own wordmark,
       not whatever face wears the theme. */
    const drawCenterEmblem = (ctx, st) => {
      const r = Math.min(s.w, s.h) * 0.22;
      if (r < 4) return;
      const rate = st.mode === 'thinking' ? 2.6 : st.mode === 'listening' ? 1.6 : st.mode === 'talking' ? 1.3 : 0.45;
      const spin = st.t * 0.16 * rate;
      const ink = st.flareSign < 0 && st.ping > 0.05 ? [255, 59, 48] : [79, 214, 224];
      paintEmblem(ctx, s.w / 2, s.h / 2, r, { spin, ink });
    };

    return {
      draw: (st) => {
        const ctx = st.ctx;
        fillFrame(st, C.bg);

        const lvl = st.level;

        // The wash the board's removal left behind — see this file's header
        // for why flat-alone wasn't enough. Anchored on the emblem's own
        // centre and radius so it reads as the ring's presence, not a
        // second, competing shape; alpha stays low enough that it never
        // approaches "background artwork" again.
        const R = Math.min(s.w, s.h) * 0.22;
        glow(st, s.w / 2, s.h / 2, R * (2.6 + lvl * 1.4), 'rgba(79,214,224,0.12)', 0.3 + lvl * 0.35);

        // flare ring from pulse()/flare()
        if (st.ping > 0.01) {
          ctx.strokeStyle = st.flareSign < 0 ? C.alert : C.inbound;
          ctx.globalAlpha = st.ping * 0.6;
          ctx.lineWidth = 2;
          ctx.beginPath();
          ctx.arc(s.w / 2, s.h / 2, (1 - st.ping) * Math.max(s.w, s.h) * 0.6, 0, Math.PI * 2);
          ctx.stroke();
          ctx.globalAlpha = 1;
        }

        drawCenterEmblem(ctx, st);
      },
    };
  });
}

