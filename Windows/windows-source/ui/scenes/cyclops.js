// cyclops.js — CYCLOPS theme scene: one unblinking eye in an empty room.
//
// Ported value-for-value from helloim-v4's src/scenes/cyclops.ts — see
// renderer/lib/scene.js's header for the port rationale.
//
// The sparsest scene in the set: a single object on a near-empty field,
// almost no motion at idle beyond a slow dilation. Menace comes from
// restraint.

import { createScene, fillFrame, glow } from './lib/faceScene.js';

export function mount(canvas) {
  return createScene(canvas, (s) => {
    let blinkAt = 9;
    let blink = 0;

    return {
      onFlare: () => {
        blink = 1;
      },
      draw: (st) => {
        const ctx = st.ctx;
        fillFrame(st, '#050506');

        const cx = st.w / 2;
        const cy = st.h / 2;
        const R = Math.min(st.w, st.h) * 0.17;

        // the room: faint horizontal panel lines, very low contrast
        ctx.save();
        ctx.strokeStyle = 'rgba(255,255,255,0.025)';
        ctx.lineWidth = 1;
        for (let y = 0; y < st.h; y += 46) {
          ctx.beginPath();
          ctx.moveTo(0, y);
          ctx.lineTo(st.w, y);
          ctx.stroke();
        }
        ctx.restore();

        // rare, deliberate blink
        if (st.t > blinkAt) {
          blink = 1;
          blinkAt = st.t + 6 + Math.random() * 9;
        }
        blink = Math.max(0, blink - st.dt * 3.2);
        const lid = Math.sin(blink * Math.PI); // 0..1..0

        // dilation: breathes slowly at idle, tightens when thinking
        const dilate =
          1 +
          Math.sin(st.t * 0.6) * 0.03 +
          st.level * 0.22 -
          (st.mode === 'thinking' ? 0.06 : 0);

        // housing
        ctx.save();
        ctx.fillStyle = '#0a0a0c';
        ctx.strokeStyle = 'rgba(255,255,255,0.07)';
        ctx.lineWidth = 2;
        ctx.beginPath();
        ctx.arc(cx, cy, R * 1.9, 0, Math.PI * 2);
        ctx.fill();
        ctx.stroke();
        ctx.restore();

        // concentric iris rings
        for (let i = 6; i >= 1; i--) {
          const rr = R * dilate * (0.35 + i * 0.12);
          ctx.strokeStyle = `rgba(255,${40 + i * 6},${30 + i * 4},${0.05 + i * 0.03})`;
          ctx.lineWidth = 1 + i * 0.2;
          ctx.beginPath();
          ctx.arc(cx, cy, rr, 0, Math.PI * 2);
          ctx.stroke();
        }

        // lens
        const lens = ctx.createRadialGradient(cx, cy, 0, cx, cy, R * dilate);
        const heat = 0.55 + st.level * 0.45;
        lens.addColorStop(0, `rgba(255,240,230,${0.9 * heat})`);
        lens.addColorStop(0.25, `rgba(255,90,60,${0.95 * heat})`);
        lens.addColorStop(0.7, `rgba(150,14,10,${0.9 * heat})`);
        lens.addColorStop(1, 'rgba(30,0,0,0.95)');
        ctx.fillStyle = lens;
        ctx.beginPath();
        ctx.arc(cx, cy, R * dilate, 0, Math.PI * 2);
        ctx.fill();

        glow(st, cx, cy, R * (3 + st.level * 2.2), 'rgba(255,60,40,0.22)', 0.5 + st.level * 0.5);

        // thinking: a single scanning sweep line crossing the lens
        if (st.mode === 'thinking') {
          const y = cy - R + ((st.t * 90) % (R * 2));
          ctx.save();
          ctx.globalCompositeOperation = 'lighter';
          ctx.strokeStyle = 'rgba(255,180,150,0.5)';
          ctx.lineWidth = 1.5;
          ctx.beginPath();
          ctx.moveTo(cx - R, y);
          ctx.lineTo(cx + R, y);
          ctx.stroke();
          ctx.restore();
        }

        // listening: a widening ring, once every couple of seconds
        if (st.listening) {
          const p = (st.t % 2.2) / 2.2;
          ctx.strokeStyle = `rgba(255,120,90,${(1 - p) * 0.4})`;
          ctx.lineWidth = 1.5;
          ctx.beginPath();
          ctx.arc(cx, cy, R * (2 + p * 3), 0, Math.PI * 2);
          ctx.stroke();
        }

        // eyelid
        if (lid > 0.001) {
          ctx.fillStyle = '#050506';
          const h = R * 2.2 * lid;
          ctx.fillRect(cx - R * 2, cy - R * 2.2, R * 4, h);
          ctx.fillRect(cx - R * 2, cy + R * 2.2 - h, R * 4, h);
        }

        if (st.ping > 0.01) {
          glow(st, cx, cy, st.ping * Math.max(st.w, st.h) * 0.4, st.flareSign < 0 ? 'rgba(255,30,20,0.7)' : 'rgba(255,150,110,0.4)', st.ping * 0.6);
        }
      },
    };
  });
}

