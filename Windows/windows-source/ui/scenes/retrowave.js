// retrowave.js — VECTOR (retrowave) theme scene: an infinite neon grid
// running to a chrome sun.
//
// Ported value-for-value from helloim-v4's src/scenes/retrowave.ts — see
// renderer/lib/scene.js's header for the port rationale.
//
// Perspective geometry, magenta/cyan, VHS scanlines. Always travelling, so
// idle reads as cruising and speech reads as flooring it.

import { createScene, fillFrame, glow } from './lib/faceScene.js';

export function mount(canvas) {
  return createScene(canvas, (s) => {
    let travel = 0;

    return {
      draw: (st) => {
        const ctx = st.ctx;
        fillFrame(st, '#0a0418');

        const hz = st.h * 0.52; // horizon

        // sky gradient
        const sky = ctx.createLinearGradient(0, 0, 0, hz);
        sky.addColorStop(0, '#12063a');
        sky.addColorStop(0.6, '#3a0a54');
        sky.addColorStop(1, '#7b1359');
        ctx.fillStyle = sky;
        ctx.fillRect(0, 0, st.w, hz);

        // stars
        ctx.save();
        for (let i = 0; i < 70; i++) {
          const x = ((i * 977) % 1000) / 1000 * st.w;
          const y = ((i * 613) % 1000) / 1000 * hz * 0.75;
          const tw = 0.3 + Math.abs(Math.sin(st.t * 0.8 + i)) * 0.5;
          ctx.fillStyle = `rgba(255,255,255,${tw * 0.6})`;
          ctx.fillRect(x, y, 1.6, 1.6);
        }
        ctx.restore();

        // the sun, with its signature horizontal slits
        const sunR = Math.min(st.w, st.h) * 0.19 * (1 + st.level * 0.12);
        const sunY = hz - sunR * 0.1;
        const sg = ctx.createLinearGradient(0, sunY - sunR, 0, sunY + sunR);
        sg.addColorStop(0, '#fff06a');
        sg.addColorStop(0.5, '#ff7a3d');
        sg.addColorStop(1, '#ff2e88');
        ctx.save();
        ctx.beginPath();
        ctx.arc(st.w / 2, sunY, sunR, 0, Math.PI * 2);
        ctx.clip();
        ctx.fillStyle = sg;
        ctx.fillRect(st.w / 2 - sunR, sunY - sunR, sunR * 2, sunR * 2);
        ctx.fillStyle = '#0a0418';
        for (let i = 0; i < 9; i++) {
          const y = sunY - sunR * 0.1 + i * (sunR / 6);
          ctx.fillRect(st.w / 2 - sunR, y, sunR * 2, 2 + i * 0.9);
        }
        ctx.restore();
        glow(st, st.w / 2, sunY, sunR * 2.4, 'rgba(255,60,140,0.35)', 0.6 + st.level * 0.4);

        // ground
        ctx.fillStyle = '#12002a';
        ctx.fillRect(0, hz, st.w, st.h - hz);

        // perspective grid
        travel += st.dt * (0.35 + st.level * 1.9);
        ctx.save();
        ctx.strokeStyle = `rgba(255,60,190,${0.55 + st.level * 0.35})`;
        ctx.lineWidth = 1.4;
        ctx.shadowBlur = 12;
        ctx.shadowColor = '#ff3cbe';
        // verticals converging on the vanishing point
        for (let i = -14; i <= 14; i++) {
          ctx.beginPath();
          ctx.moveTo(st.w / 2 + i * (st.w / 14), st.h);
          ctx.lineTo(st.w / 2 + i * 6, hz);
          ctx.stroke();
        }
        // horizontals with exponential spacing so they rush at the camera
        ctx.strokeStyle = `rgba(80,240,255,${0.5 + st.level * 0.4})`;
        ctx.shadowColor = '#50f0ff';
        for (let i = 0; i < 16; i++) {
          const p = (i + (travel % 1)) / 16;
          const y = hz + Math.pow(p, 2.4) * (st.h - hz) * 1.05;
          if (y > st.h) continue;
          ctx.beginPath();
          ctx.moveTo(0, y);
          ctx.lineTo(st.w, y);
          ctx.stroke();
        }
        ctx.restore();

        // horizon bloom band
        glow(st, st.w / 2, hz, st.w * 0.8, 'rgba(255,80,200,0.18)', 0.8);

        // VHS scanlines + chroma wobble
        ctx.save();
        ctx.globalAlpha = 0.12;
        ctx.fillStyle = '#000';
        for (let y = 0; y < st.h; y += 3) ctx.fillRect(0, y, st.w, 1);
        ctx.restore();
        const tear = Math.sin(st.t * 2.3) * st.level * 12;
        if (Math.abs(tear) > 0.4) {
          ctx.save();
          ctx.globalCompositeOperation = 'lighter';
          ctx.globalAlpha = 0.12;
          ctx.fillStyle = '#00ffe0';
          ctx.fillRect(tear, st.h * 0.4, st.w, 24);
          ctx.fillStyle = '#ff2e88';
          ctx.fillRect(-tear, st.h * 0.62, st.w, 18);
          ctx.restore();
        }

        if (st.ping > 0.01) {
          glow(st, st.w / 2, hz, st.ping * st.w, st.flareSign < 0 ? 'rgba(255,40,40,0.5)' : 'rgba(120,255,240,0.4)', st.ping * 0.6);
        }
      },
    };
  });
}

