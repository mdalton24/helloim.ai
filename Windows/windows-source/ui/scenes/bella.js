// bella.js — BELLA theme scene: a luminous particle sphere on near-black.
//
// Ported value-for-value from helloim-v4's src/scenes/bella.ts — see
// renderer/lib/scene.js's header for the port rationale.
//
// Organic, breathing, singular. The opposite of Jarvis in every axis that
// matters: sparse field, curved geometry, one object, pill-round chrome.
//
// Rendered as a projected point cloud on 2D canvas (no WebGL dependency)
// with an aurora wash behind it standing in for the background shader pass.

import { createScene, fillFrame, glow } from './lib/faceScene.js';

const CANVAS_BG = '#0b0d10';
const ACCENT = '#52ffa5';

export function mount(canvas) {
  return createScene(canvas, (s) => {
    let pts = [];
    let rot = 0;

    const layout = () => {
      // Sparse relative to Jarvis on purpose: this is one object, not a field.
      const n = Math.max(700, Math.min(2600, Math.round((s.w * s.h) / 900)));
      pts = new Array(n);
      for (let i = 0; i < n; i++) {
        // fibonacci sphere for even coverage
        const k = i + 0.5;
        const phi = Math.acos(1 - (2 * k) / n);
        const theta = Math.PI * (1 + Math.sqrt(5)) * k;
        pts[i] = { u: theta, v: phi };
      }
    };

    return {
      layout,
      draw: (st) => {
        const ctx = st.ctx;
        fillFrame(st, CANVAS_BG);

        const cx = st.w / 2;
        const cy = st.h / 2;
        const R = Math.min(st.w, st.h) * 0.31;

        // --- aurora background wash ------------------------------------
        ctx.save();
        ctx.globalCompositeOperation = 'lighter';
        for (let i = 0; i < 3; i++) {
          const a = st.t * (0.08 + i * 0.05) + i * 2.1;
          const gx = cx + Math.cos(a) * st.w * 0.26;
          const gy = cy + Math.sin(a * 0.8) * st.h * 0.22;
          const col = i === 0 ? 'rgba(82,255,165,0.13)' : i === 1 ? 'rgba(86,140,255,0.12)' : 'rgba(255,120,190,0.08)';
          const g = ctx.createRadialGradient(gx, gy, 0, gx, gy, Math.max(st.w, st.h) * 0.55);
          g.addColorStop(0, col);
          g.addColorStop(1, 'rgba(0,0,0,0)');
          ctx.fillStyle = g;
          ctx.fillRect(0, 0, st.w, st.h);
        }
        ctx.restore();

        // --- breathing -------------------------------------------------
        const breathe = 1 + Math.sin(st.t * 0.9) * 0.018 + st.level * 0.13;
        rot += st.dt * (0.11 + st.level * 0.5);
        const churn = st.mode === 'thinking' ? 1 : 0.25;

        ctx.save();
        ctx.globalCompositeOperation = 'lighter';
        for (let i = 0; i < pts.length; i++) {
          const p = pts[i];
          const wob =
            Math.sin(p.v * 6 + st.t * 1.6 + p.u * 2) * 0.02 * churn +
            st.level * 0.06 * Math.sin(p.u * 3 - st.t * 4);
          const r = R * breathe * (1 + wob);
          const u = p.u + rot;
          const sinV = Math.sin(p.v);
          const x3 = sinV * Math.cos(u);
          const y3 = Math.cos(p.v);
          const z3 = sinV * Math.sin(u);
          const depth = (z3 + 1) / 2; // 0 back .. 1 front
          const persp = 1 / (1.9 - z3 * 0.55);
          const x = cx + x3 * r * persp * 1.6;
          const y = cy + y3 * r * persp * 1.6;

          // warm-to-cool gradient down the sphere, mint accent at the equator
          const warm = (y3 + 1) / 2;
          const rr = Math.round(70 + warm * 120 + st.level * 60);
          const gg = Math.round(190 + (1 - warm) * 60);
          const bb = Math.round(150 + warm * 105);
          const alpha = (0.12 + depth * 0.5) * (0.5 + st.level * 0.6);
          ctx.fillStyle = `rgba(${rr},${gg},${bb},${alpha})`;
          const size = (0.7 + depth * 1.5) * (1 + st.level * 0.5);
          ctx.fillRect(x, y, size, size);
        }
        ctx.restore();

        // --- core bloom ------------------------------------------------
        glow(st, cx, cy, R * (1.5 + st.level * 0.7), 'rgba(82,255,165,0.20)', 0.85);
        glow(st, cx, cy, R * 0.42, 'rgba(220,255,240,0.35)', 0.5 + st.level * 0.5);

        // listening: a slow halo ring instead of a churn
        if (st.listening) {
          const pulse = (st.t % 2.4) / 2.4;
          ctx.save();
          ctx.globalCompositeOperation = 'lighter';
          ctx.strokeStyle = ACCENT;
          ctx.globalAlpha = (1 - pulse) * 0.35;
          ctx.lineWidth = 1.5;
          ctx.beginPath();
          ctx.arc(cx, cy, R * (1.15 + pulse * 0.9), 0, Math.PI * 2);
          ctx.stroke();
          ctx.restore();
        }

        if (st.ping > 0.01) {
          glow(
            st,
            cx,
            cy,
            R * (1 + st.ping * 2.4),
            st.flareSign < 0 ? 'rgba(255,90,90,0.5)' : 'rgba(140,255,210,0.5)',
            st.ping * 0.7
          );
        }
      },
    };
  });
}

