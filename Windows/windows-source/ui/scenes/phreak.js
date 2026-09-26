// phreak.js — PHREAK theme scene: glyph rain down a phosphor terminal.
//
// Ported value-for-value from helloim-v4's src/scenes/phreak.ts — see
// renderer/lib/scene.js's header for the port rationale.
//
// The densest text field in the set. Idle drizzles; thinking accelerates
// the columns; speaking lights the leading glyph of every column white-hot.

import { createScene, fillFrame, rnd } from './lib/faceScene.js';

const GLYPHS = 'アカサタナハマヤラワ0123456789ABCDEFXYZ<>/\\|=+-*#$%&@';

export function mount(canvas) {
  return createScene(canvas, (s) => {
    let cols = [];
    let size = 14;

    const layout = () => {
      size = Math.max(11, Math.min(18, s.w / 90));
      const n = Math.ceil(s.w / size);
      cols = new Array(n);
      for (let i = 0; i < n; i++) {
        const len = 8 + ((Math.random() * 22) | 0);
        cols[i] = {
          y: rnd(-s.h, 0),
          sp: rnd(60, 260),
          len,
          chars: Array.from({ length: len }, () => GLYPHS[(Math.random() * GLYPHS.length) | 0]),
        };
      }
    };

    return {
      layout,
      draw: (st) => {
        const ctx = st.ctx;
        // trail decay rather than a hard clear — the phosphor persistence
        fillFrame(st, 'rgba(0,0,0,1)');
        ctx.fillStyle = '#000806';
        ctx.fillRect(0, 0, st.w, st.h);

        ctx.font = `${size}px ui-monospace, "SFMono-Regular", Menlo, monospace`;
        ctx.textBaseline = 'top';

        const speedK = 1 + st.level * 3.2 + (st.mode === 'thinking' ? 1.4 : 0);

        for (let i = 0; i < cols.length; i++) {
          const c = cols[i];
          c.y += c.sp * st.dt * speedK;
          if (c.y - c.len * size > st.h) {
            c.y = -rnd(0, st.h * 0.6);
            c.sp = rnd(60, 260);
          }
          const x = i * size;
          for (let j = 0; j < c.len; j++) {
            const y = c.y - j * size;
            if (y < -size || y > st.h) continue;
            if (Math.random() < 0.02) c.chars[j] = GLYPHS[(Math.random() * GLYPHS.length) | 0];
            const fade = 1 - j / c.len;
            if (j === 0) {
              ctx.fillStyle = st.level > 0.15 ? '#eafff2' : '#b9ffd8';
            } else {
              ctx.fillStyle = `rgba(0,${Math.round(190 + st.level * 60)},${Math.round(110 + st.level * 40)},${fade * (0.5 + st.level * 0.4)})`;
            }
            ctx.fillText(c.chars[j], x, y);
          }
        }

        // status line, bottom left — the scene is a terminal, so it talks
        ctx.fillStyle = 'rgba(120,255,190,0.55)';
        ctx.font = `${Math.max(10, size * 0.8)}px ui-monospace, Menlo, monospace`;
        const label =
          st.mode === 'thinking'
            ? '> resolving...'
            : st.mode === 'talking'
              ? '> transmitting'
              : st.listening
                ? '> listening'
                : '> idle';
        ctx.fillText(label + (Math.floor(st.t * 2) % 2 ? '_' : ' '), 14, st.h - size * 2);

        // CRT scanlines + curvature vignette
        ctx.save();
        ctx.globalAlpha = 0.14;
        ctx.fillStyle = '#000';
        for (let y = 0; y < st.h; y += 3) ctx.fillRect(0, y, st.w, 1);
        ctx.restore();
        const v = ctx.createRadialGradient(st.w / 2, st.h / 2, Math.min(st.w, st.h) * 0.25, st.w / 2, st.h / 2, Math.max(st.w, st.h) * 0.7);
        v.addColorStop(0, 'rgba(0,0,0,0)');
        v.addColorStop(1, 'rgba(0,0,0,0.75)');
        ctx.fillStyle = v;
        ctx.fillRect(0, 0, st.w, st.h);

        if (st.ping > 0.01) {
          ctx.fillStyle = st.flareSign < 0 ? `rgba(255,40,40,${st.ping * 0.22})` : `rgba(60,255,170,${st.ping * 0.16})`;
          ctx.fillRect(0, 0, st.w, st.h);
        }
      },
    };
  });
}

