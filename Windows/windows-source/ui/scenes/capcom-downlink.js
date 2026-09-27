// CAPCOM DOWNLINK BAND — ported 1:1 from
// helloim-home-replacement/src/scenes/capcom.ts (DOWNLINK_BANDS,
// stepDownlinkBand, paintDownlinkBand, downlinkStateLabel, and FLOORS from
// that repo's src/lib/scene.ts). This is the exact math the site's
// ascent-profile scene AND its CapCom mission-control board both drew from —
// see that file's own header for why the two shared it rather than each
// keeping a copy. Ported rather than re-derived so the desktop board's
// waveform is the SAME instrument, not a second invention of it; only the
// language changed (TS -> plain JS, no build step on this app), not the
// numbers.
//
// capcom-board.js (this app's own port of CapcomBoard.tsx/DownlinkBand.tsx)
// is the only consumer today. If ui/scenes/capcom.js (the ported
// ascent-profile scene) ever grows its own downlink strip, it should import
// this file rather than gaining a third copy of the same four functions.

export const DOWNLINK_BANDS = 96;

const AMBER = '255,190,90';
const BLUE = '110,205,255';

/** Ease the 96-bar band toward `level`, same shape used by the scene's own strip. */
export function stepDownlinkBand(band, level, t, dt) {
  const breathe = 0.5 + Math.sin(t * 0.7) * 0.5;
  for (let i = 0; i < band.length; i++) {
    const wave = Math.sin(i * 0.31 + t * 3.1) * 0.45 + Math.sin(i * 0.11 - t * 1.7) * 0.3;
    const target = Math.max(0.03, level * (0.55 + wave * 0.45)) + breathe * 0.012;
    band[i] += (target - band[i]) * Math.min(1, dt * 11);
  }
}

/** Paint the band centred on `y`, spanning `w` px from `x`, bars up to `maxBarH` tall. */
export function paintDownlinkBand(ctx, x, y, w, maxBarH, band) {
  const bw = w / band.length;
  ctx.save();
  ctx.globalCompositeOperation = 'lighter';
  for (let i = 0; i < band.length; i++) {
    const bx = x + i * bw;
    const bh = 3 + band[i] * maxBarH;
    ctx.fillStyle = `rgba(${AMBER},${0.2 + band[i] * 0.7})`;
    ctx.fillRect(bx, y - bh / 2, Math.max(1, bw - 1.4), bh);
  }
  ctx.restore();
  ctx.strokeStyle = `rgba(${BLUE},0.16)`;
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(x, y + 0.5);
  ctx.lineTo(x + w, y + 0.5);
  ctx.stroke();
}

/** The flight-status vocabulary shown in the scene's top strip and reused on the board. */
export function downlinkStateLabel(mode, listening) {
  if (mode === 'thinking') return 'RECOMPUTING';
  if (mode === 'talking') return 'DOWNLINK ACTIVE';
  if (listening) return 'UPLINK OPEN';
  return 'HOLDING';
}

/** Floors per mode — idle sits flat, thinking sits highest (sustained churn, no audio). */
export const FLOORS = { idle: 0.0, listening: 0.06, talking: 0.12, thinking: 0.22 };
