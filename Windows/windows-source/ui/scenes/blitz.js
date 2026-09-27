// blitz.js — COACH BLITZ theme scene: a stadium LED scoreboard, edge to edge.
//
// Ported value-for-value from helloim-v4's src/scenes/blitz.ts — see
// renderer/lib/scene.js's header for the port rationale.
//
// A hard orthogonal dot matrix. Idle runs a slow attract wave; speech drives
// a bar-graph equaliser across the panel and kicks the floodlights.

import { createScene, fillFrame, glow } from './lib/faceScene.js';

export function mount(canvas) {
  return createScene(canvas, (s) => {
    let cell = 12;
    let cols = 0;
    let rows = 0;
    let bars = [];

    const layout = () => {
      cell = Math.max(8, Math.min(16, s.w / 78));
      cols = Math.ceil(s.w / cell) + 1;
      rows = Math.ceil(s.h / cell) + 1;
      bars = new Array(cols).fill(0);
    };

    return {
      layout,
      onBump: (_st, v) => {
        for (let i = 0; i < cols; i++) {
          bars[i] = Math.max(bars[i], v * (0.5 + Math.random() * 0.8));
        }
      },
      draw: (st) => {
        const ctx = st.ctx;
        fillFrame(st, '#080a0c');

        // panel backing
        ctx.fillStyle = '#0c1014';
        ctx.fillRect(0, 0, st.w, st.h);

        // decay the equaliser bars, floor them on the level
        for (let i = 0; i < cols; i++) {
          const target =
            st.level * (0.45 + 0.55 * Math.abs(Math.sin(i * 0.4 + st.t * 6)));
          bars[i] = Math.max(bars[i] * 0.92, target);
        }

        const r = cell * 0.36;
        for (let cxi = 0; cxi < cols; cxi++) {
          const barH = bars[cxi] * rows;
          for (let cyi = 0; cyi < rows; cyi++) {
            const x = cxi * cell + cell / 2;
            const y = cyi * cell + cell / 2;
            const fromBottom = rows - cyi;

            // attract wave keeps the board alive when nothing is happening
            const wave =
              0.10 +
              0.12 * Math.max(0, Math.sin(cxi * 0.3 - st.t * 2.4 + cyi * 0.12));

            let a = wave;
            let col = '255,196,0'; // amber: the resting board
            if (fromBottom <= barH) {
              const heat = fromBottom / Math.max(1, rows);
              a = 0.85;
              col = heat > 0.72 ? '255,64,64' : heat > 0.45 ? '255,180,40' : '80,255,140';
            }
            ctx.fillStyle = `rgba(${col},${a})`;
            ctx.beginPath();
            ctx.arc(x, y, r, 0, Math.PI * 2);
            ctx.fill();
          }
        }

        // floodlights top corners
        glow(st, st.w * 0.12, -20, st.h * (0.5 + st.level * 0.3), 'rgba(255,255,220,0.10)', 0.8);
        glow(st, st.w * 0.88, -20, st.h * (0.5 + st.level * 0.3), 'rgba(255,255,220,0.10)', 0.8);

        // scoreboard bezel — keeps it reading as hardware, not wallpaper
        ctx.strokeStyle = 'rgba(255,255,255,0.06)';
        ctx.lineWidth = 6;
        ctx.strokeRect(3, 3, st.w - 6, st.h - 6);

        if (st.ping > 0.01) {
          ctx.fillStyle =
            st.flareSign < 0
              ? `rgba(255,40,40,${st.ping * 0.2})`
              : `rgba(255,220,120,${st.ping * 0.18})`;
          ctx.fillRect(0, 0, st.w, st.h);
        }
      },
    };
  });
}

