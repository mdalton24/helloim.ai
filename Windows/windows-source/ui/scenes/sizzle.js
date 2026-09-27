// sizzle.js — SIZZLE theme scene: a screaming-hot flat-top griddle from above.
//
// Ported value-for-value from helloim-v4's src/scenes/sizzle.ts — see
// renderer/lib/scene.js's header for the port rationale.
//
// Dense, granular, edge-to-edge. Idles at a low shimmer with the occasional
// pop; when it speaks the whole surface sears. The angry one.

import { createScene, fillFrame, glow, rnd } from './lib/faceScene.js';

export function mount(canvas) {
  return createScene(canvas, (s) => {
    let embers = [];
    let grain = [];
    let acc = 0;

    const layout = () => {
      const n = Math.round((s.w * s.h) / 900);
      grain = new Array(n);
      for (let i = 0; i < n; i++) {
        grain[i] = { x: Math.random() * s.w, y: Math.random() * s.h, a: rnd(0.02, 0.12) };
      }
      embers = [];
    };

    const spawnEmber = (n, hot) => {
      for (let i = 0; i < n; i++) {
        const max = rnd(0.5, 1.6);
        embers.push({
          x: Math.random() * s.w,
          y: s.h * rnd(0.35, 1.05),
          vx: rnd(-24, 24),
          vy: rnd(-90, -30) * (hot ? 1.6 : 1),
          life: 0,
          max,
          r: rnd(0.8, hot ? 3.4 : 2.2),
        });
      }
      if (embers.length > 420) embers.splice(0, embers.length - 420);
    };

    return {
      layout,
      onBump: (_st, v) => spawnEmber(6 + Math.round(v * 22), true),
      onFlare: (_st, ok) => spawnEmber(ok ? 40 : 70, true),
      draw: (st) => {
        const ctx = st.ctx;
        fillFrame(st, '#140805');

        // heat gradient: cooler at the top, molten toward the bottom
        const g = ctx.createLinearGradient(0, 0, 0, st.h);
        g.addColorStop(0, '#1b0a06');
        g.addColorStop(0.55, `rgba(90,22,8,${0.75 + st.level * 0.25})`);
        g.addColorStop(1, `rgba(${180 + st.level * 60},${60 + st.level * 70},20,${0.55 + st.level * 0.4})`);
        ctx.fillStyle = g;
        ctx.fillRect(0, 0, st.w, st.h);

        // rolling heat bands
        ctx.save();
        ctx.globalCompositeOperation = 'lighter';
        for (let i = 0; i < 4; i++) {
          const y = ((st.t * (18 + i * 9) + i * 140) % (st.h + 240)) - 120;
          const band = ctx.createLinearGradient(0, y - 90, 0, y + 90);
          band.addColorStop(0, 'rgba(0,0,0,0)');
          band.addColorStop(0.5, `rgba(255,${110 + i * 20},40,${0.05 + st.level * 0.09})`);
          band.addColorStop(1, 'rgba(0,0,0,0)');
          ctx.fillStyle = band;
          ctx.fillRect(0, y - 90, st.w, 180);
        }
        ctx.restore();

        // griddle grain, jittering with heat
        ctx.save();
        for (const p of grain) {
          const j = Math.sin(st.t * 3 + p.x * 0.05) * (0.6 + st.level * 2.2);
          ctx.fillStyle = `rgba(255,${180 + Math.random() * 60},120,${p.a * (0.4 + st.level)})`;
          ctx.fillRect(p.x, p.y + j, 1.2, 1.2);
        }
        ctx.restore();

        // char lines from the grill bars — structure, always present
        ctx.save();
        ctx.globalAlpha = 0.25;
        ctx.strokeStyle = '#2a0f08';
        ctx.lineWidth = 10;
        for (let x = -40; x < st.w + 40; x += 64) {
          ctx.beginPath();
          ctx.moveTo(x, 0);
          ctx.lineTo(x + 40, st.h);
          ctx.stroke();
        }
        ctx.restore();

        // embers — 1.2/s idle rising hard when talking
        acc += (1.2 + st.level * 26) * st.dt;
        while (acc >= 1) {
          spawnEmber(1, false);
          acc -= 1;
        }
        ctx.save();
        ctx.globalCompositeOperation = 'lighter';
        for (let i = embers.length - 1; i >= 0; i--) {
          const e = embers[i];
          e.life += st.dt;
          if (e.life > e.max) {
            embers.splice(i, 1);
            continue;
          }
          e.x += e.vx * st.dt;
          e.y += e.vy * st.dt;
          e.vy += 26 * st.dt;
          const f = 1 - e.life / e.max;
          ctx.fillStyle = `rgba(255,${Math.round(120 + f * 120)},${Math.round(40 * f)},${f})`;
          ctx.beginPath();
          ctx.arc(e.x, e.y, e.r * f, 0, Math.PI * 2);
          ctx.fill();
        }
        ctx.restore();

        // the flame front when it is actually shouting
        if (st.level > 0.05) {
          glow(st, st.w / 2, st.h * 1.02, Math.max(st.w, st.h) * (0.35 + st.level * 0.5), 'rgba(255,140,30,0.5)', st.level);
        }
        if (st.ping > 0.01) {
          glow(st, st.w / 2, st.h / 2, st.ping * st.w * 0.7, st.flareSign < 0 ? 'rgba(255,40,20,0.6)' : 'rgba(255,200,80,0.5)', st.ping * 0.6);
        }
      },
    };
  });
}

