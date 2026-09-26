// Transferred from the website; rendering values preserved.
import { createScene, fillFrame, glow } from './lib/faceScene.js';
/**
 * NOVA — "Wayfinder".
 *
 * Her character is the guide: calm, precise, already a step ahead. So the scene
 * is not a face and not weather — it is an INSTRUMENT. A holographic navigation
 * ring floating in deep blue: a bearing scale that drifts, arc segments that
 * hold their station, a slow sweep that finds the next heading, and a polar
 * waveform in the middle that is her actual voice.
 *
 *   idle       — the instrument breathes, bearing drifts, sweep is dim
 *   listening  — capture ripples travel outward through the rings
 *   thinking   — the sweep speeds up and the segments counter-rotate
 *   talking    — the polar ring blooms, the heading locks, the core brightens
 *
 * Everything is drawn in Nova's electric cyan-blue on near-black navy so it
 * reads as "the vibe of the guide" rather than a portrait of her.
 */
const BINS = 132;
/** Radius a word-impulse ripple starts at, in CSS px. */
const BUMP_SEED_R = 40;
export function mount(canvas) {
    return createScene(canvas, () => {
        const bins = Array.from({ length: BINS }, (_, i) => ({
            v: 0,
            target: 0,
            seed: (i * 97.13) % 6.283,
        }));
        let ripples = [];
        let sweep = 0; // radians
        let bearing = 0; // slow drift of the outer scale
        let lock = 0; // 0..1, how "locked on" the instrument is (rises when talking)
        /* let, NOT const -- Mastermind round 95, the dock's Appearance
           control (Vance's round-95 follow-up asked for Bella and Nova to
           be wired too, "do what's clean"). Every rgba(${ACCENT},...)/
           rgba(${DEEP},...) call below reads these two bindings fresh on
           every draw() call (template-literal interpolation happens per
           call, nothing here caches the resolved string), so reassigning
           them in setPalette() below is picked up on the very next frame —
           the same live-mutation shape noir.js's CONFIG object and koan.js's
           own `let ACCENT` already use, aimed at two plain strings instead
           of an object because that is what this scene already had. */
        let ACCENT = '150,205,255';
        let DEEP = '30,120,255';
        const NOVA_DEFAULT_ACCENT = ACCENT;
        const NOVA_DEFAULT_DEEP = DEEP;
        function hexToTriplet(hex) {
            const n = parseInt(hex.slice(1), 16);
            return `${(n >> 16) & 255},${(n >> 8) & 255},${n & 255}`;
        }
        // A darker companion tone, not a second accent to pick — DEEP was
        // never its own colour choice in the original (30,120,255 reads as
        // a deeper, more saturated version of 150,205,255, not an
        // unrelated hue), so a straightforward scale-toward-black keeps
        // that relationship without inventing a second knob nobody asked
        // for.
        function darken(triplet, f) {
            return triplet.split(',').map((c) => Math.round(Number(c) * f)).join(',');
        }
        function applyNovaPalette(colors) {
            if (!colors || !colors.accent) {
                ACCENT = NOVA_DEFAULT_ACCENT;
                DEEP = NOVA_DEFAULT_DEEP;
                return;
            }
            ACCENT = hexToTriplet(colors.accent);
            DEEP = darken(ACCENT, 0.55);
        }
        return {
            setPalette: applyNovaPalette,
            draw: (st) => {
                const ctx = st.ctx;
                const { w, h, level, dt } = st;
                const cx = w / 2;
                const cy = h * 0.52;
                const R = Math.min(w, h) * 0.33;
                /* ---------------- field ---------------- */
                fillFrame(st, '#02060f');
                const wash = ctx.createRadialGradient(cx, cy, 0, cx, cy, Math.max(w, h) * 0.75);
                wash.addColorStop(0, `rgba(12,52,102,${0.55 + level * 0.3})`);
                wash.addColorStop(0.55, 'rgba(5,18,40,0.75)');
                wash.addColorStop(1, '#01040a');
                ctx.fillStyle = wash;
                ctx.fillRect(0, 0, w, h);
                // faint measured grid — the instrument sits on graph space
                ctx.save();
                ctx.strokeStyle = `rgba(${DEEP},0.055)`;
                ctx.lineWidth = 1;
                const gap = Math.max(34, Math.min(w, h) / 14);
                ctx.beginPath();
                for (let x = (cx % gap) - gap; x < w + gap; x += gap) {
                    ctx.moveTo(Math.round(x) + 0.5, 0);
                    ctx.lineTo(Math.round(x) + 0.5, h);
                }
                for (let y = (cy % gap) - gap; y < h + gap; y += gap) {
                    ctx.moveTo(0, Math.round(y) + 0.5);
                    ctx.lineTo(w, Math.round(y) + 0.5);
                }
                ctx.stroke();
                ctx.restore();
                /* ---------------- state easing ---------------- */
                const speaking = st.mode === 'talking';
                lock += ((speaking ? 1 : st.mode === 'thinking' ? 0.45 : 0.12) - lock) * Math.min(1, dt * 3);
                bearing += dt * (0.08 + level * 0.5);
                sweep += dt * (st.mode === 'thinking' ? 1.9 : 0.42 + level * 1.2);
                if (st.listening && st.frame % 46 === 0)
                    ripples.push({ r: R * 0.28, a: 0.5 });
                for (const rp of ripples) {
                    rp.r += dt * Math.min(w, h) * 0.34;
                    rp.a -= dt * 0.42;
                }
                ripples = ripples.filter((r) => r.a > 0.01).slice(-6);
                const breathe = 0.5 + Math.sin(st.t * 0.55) * 0.5;
                /* ---------------- HUD horizon ---------------- */
                ctx.save();
                ctx.strokeStyle = `rgba(${ACCENT},${0.1 + lock * 0.14})`;
                ctx.lineWidth = 1;
                ctx.beginPath();
                ctx.moveTo(0, cy + 0.5);
                ctx.lineTo(cx - R * 1.22, cy + 0.5);
                ctx.moveTo(cx + R * 1.22, cy + 0.5);
                ctx.lineTo(w, cy + 0.5);
                ctx.stroke();
                // end caps
                ctx.strokeStyle = `rgba(${ACCENT},${0.18 + lock * 0.3})`;
                ctx.beginPath();
                ctx.moveTo(cx - R * 1.22, cy - 7);
                ctx.lineTo(cx - R * 1.22, cy + 7);
                ctx.moveTo(cx + R * 1.22, cy - 7);
                ctx.lineTo(cx + R * 1.22, cy + 7);
                ctx.stroke();
                ctx.restore();
                /* ---------------- outer bearing scale ---------------- */
                ctx.save();
                ctx.translate(cx, cy);
                ctx.rotate(bearing * 0.12);
                for (let i = 0; i < 96; i++) {
                    const a = (i / 96) * Math.PI * 2;
                    const major = i % 8 === 0;
                    const cardinal = i % 24 === 0;
                    const len = cardinal ? 16 : major ? 10 : 4.5;
                    const alpha = cardinal ? 0.55 + level * 0.4 : major ? 0.3 + level * 0.25 : 0.14;
                    ctx.strokeStyle = `rgba(${ACCENT},${alpha})`;
                    ctx.lineWidth = cardinal ? 2 : 1;
                    const x0 = Math.cos(a) * R;
                    const y0 = Math.sin(a) * R;
                    const x1 = Math.cos(a) * (R + len);
                    const y1 = Math.sin(a) * (R + len);
                    ctx.beginPath();
                    ctx.moveTo(x0, y0);
                    ctx.lineTo(x1, y1);
                    ctx.stroke();
                }
                ctx.restore();
                /* ---------------- station arcs (counter-rotating) ---------------- */
                const arcRing = (radius, segs, span, spin, alpha, wide) => {
                    ctx.save();
                    ctx.translate(cx, cy);
                    ctx.rotate(spin);
                    ctx.strokeStyle = `rgba(${ACCENT},${alpha})`;
                    ctx.lineWidth = wide;
                    ctx.lineCap = 'round';
                    for (let i = 0; i < segs; i++) {
                        const a0 = (i / segs) * Math.PI * 2;
                        ctx.beginPath();
                        ctx.arc(0, 0, radius, a0, a0 + span);
                        ctx.stroke();
                    }
                    ctx.restore();
                };
                arcRing(R * 0.93, 3, 1.45, -bearing * 0.35, 0.2 + level * 0.35, 2);
                arcRing(R * 0.84, 6, 0.62, bearing * 0.55, 0.13 + level * 0.3, 1.2);
                arcRing(R * 0.7, 2, 2.1, -bearing * 0.22 + 0.9, 0.16 + lock * 0.3, 1);
                /* ---------------- sweep: the guide looking ahead ---------------- */
                ctx.save();
                ctx.translate(cx, cy);
                ctx.globalCompositeOperation = 'lighter';
                const TRAIL = 22;
                for (let i = 0; i < TRAIL; i++) {
                    const a = sweep - i * 0.035;
                    const fade = (1 - i / TRAIL) ** 2 * (0.1 + lock * 0.22 + level * 0.2);
                    ctx.strokeStyle = `rgba(${ACCENT},${fade})`;
                    ctx.lineWidth = 1.5;
                    ctx.beginPath();
                    ctx.moveTo(Math.cos(a) * R * 0.2, Math.sin(a) * R * 0.2);
                    ctx.lineTo(Math.cos(a) * R * 0.96, Math.sin(a) * R * 0.96);
                    ctx.stroke();
                }
                // the marker riding the outer ring
                const mx = Math.cos(sweep) * R;
                const my = Math.sin(sweep) * R;
                ctx.fillStyle = `rgba(220,240,255,${0.6 + level * 0.4})`;
                ctx.beginPath();
                ctx.arc(mx, my, 2.6 + level * 2.4, 0, Math.PI * 2);
                ctx.fill();
                ctx.restore();
                glow(st, cx + mx, cy + my, 26 + level * 40, `rgba(${ACCENT},${0.22 + level * 0.35})`);
                /* ---------------- listening ripples ---------------- */
                for (const rp of ripples) {
                    ctx.strokeStyle = `rgba(${ACCENT},${rp.a * 0.5})`;
                    ctx.lineWidth = 1.2;
                    ctx.beginPath();
                    ctx.arc(cx, cy, rp.r, 0, Math.PI * 2);
                    ctx.stroke();
                }
                /* ---------------- polar voice ring ---------------- */
                const base = R * (0.44 + breathe * 0.012);
                const reach = R * 0.3;
                for (let i = 0; i < BINS; i++) {
                    const b = bins[i];
                    const wave = Math.sin(b.seed + st.t * 2.3) * 0.5 +
                        Math.sin(b.seed * 1.7 - st.t * 3.1) * 0.32 +
                        Math.sin(b.seed * 0.6 + st.t * 1.1) * 0.18;
                    b.target = Math.max(0, level * (0.42 + wave * 0.58)) + 0.035 + breathe * 0.02;
                    b.v += (b.target - b.v) * Math.min(1, dt * 9);
                }
                const ringPoint = (i, scale) => {
                    const a = (i / BINS) * Math.PI * 2 - Math.PI / 2;
                    const r = base + bins[i % BINS].v * reach * scale;
                    return [cx + Math.cos(a) * r, cy + Math.sin(a) * r];
                };
                ctx.save();
                ctx.globalCompositeOperation = 'lighter';
                // filled body
                const body = ctx.createRadialGradient(cx, cy, base * 0.25, cx, cy, base + reach);
                body.addColorStop(0, `rgba(${DEEP},${0.28 + level * 0.3})`);
                body.addColorStop(1, 'rgba(20,90,190,0)');
                ctx.fillStyle = body;
                ctx.beginPath();
                for (let i = 0; i <= BINS; i++) {
                    const [x, y] = ringPoint(i, 1);
                    if (i === 0)
                        ctx.moveTo(x, y);
                    else
                        ctx.lineTo(x, y);
                }
                ctx.closePath();
                ctx.fill();
                // bright rim
                ctx.strokeStyle = `rgba(${ACCENT},${0.55 + level * 0.45})`;
                ctx.lineWidth = 1.8;
                ctx.beginPath();
                for (let i = 0; i <= BINS; i++) {
                    const [x, y] = ringPoint(i, 1);
                    if (i === 0)
                        ctx.moveTo(x, y);
                    else
                        ctx.lineTo(x, y);
                }
                ctx.closePath();
                ctx.stroke();
                // ghost rim, mirrored — gives the ring depth without a second colour
                ctx.strokeStyle = `rgba(${DEEP},${0.18 + level * 0.3})`;
                ctx.lineWidth = 1;
                ctx.beginPath();
                for (let i = 0; i <= BINS; i++) {
                    const [x, y] = ringPoint(BINS - i, 1.5);
                    if (i === 0)
                        ctx.moveTo(x, y);
                    else
                        ctx.lineTo(x, y);
                }
                ctx.closePath();
                ctx.stroke();
                ctx.restore();
                /* ---------------- core ---------------- */
                const coreR = R * (0.055 + level * 0.05) * (0.92 + breathe * 0.12);
                glow(st, cx, cy, R * (0.45 + level * 0.4), `rgba(${ACCENT},${0.16 + level * 0.3})`);
                ctx.fillStyle = `rgba(235,247,255,${0.8 + level * 0.2})`;
                ctx.beginPath();
                ctx.arc(cx, cy, coreR, 0, Math.PI * 2);
                ctx.fill();
                // heading chevron — always pointing forward, brightens as she speaks
                ctx.save();
                ctx.translate(cx, cy - R * 1.06);
                ctx.strokeStyle = `rgba(${ACCENT},${0.4 + lock * 0.5})`;
                ctx.lineWidth = 2;
                ctx.lineJoin = 'round';
                ctx.beginPath();
                ctx.moveTo(-11, 8);
                ctx.lineTo(0, -6 - level * 5);
                ctx.lineTo(11, 8);
                ctx.stroke();
                ctx.restore();
                /* ---------------- events ---------------- */
                if (st.ping > 0.01) {
                    const ok = st.flareSign >= 0;
                    ctx.strokeStyle = ok
                        ? `rgba(190,230,255,${st.ping * 0.55})`
                        : `rgba(255,120,95,${st.ping * 0.6})`;
                    ctx.lineWidth = 2;
                    ctx.beginPath();
                    ctx.arc(cx, cy, base + (1 - st.ping) * R * 0.9, 0, Math.PI * 2);
                    ctx.stroke();
                }
                // vignette keeps the instrument the only thing you look at
                const vig = ctx.createRadialGradient(cx, cy, R * 0.9, cx, cy, Math.max(w, h) * 0.78);
                vig.addColorStop(0, 'rgba(0,0,0,0)');
                vig.addColorStop(1, 'rgba(0,0,0,0.72)');
                ctx.fillStyle = vig;
                ctx.fillRect(0, 0, w, h);
            },
            // a spoken word nudges a ripple out through the rings, on top of the
            // polar bloom the level already drives
            onBump: (_s, v) => {
                if (v > 0.35 && ripples.length < 6)
                    ripples.push({ r: BUMP_SEED_R, a: 0.22 + v * 0.2 });
            },
            onFlare: () => {
                ripples = [];
            },
            onModeChange: (_s, mode) => {
                if (mode === 'idle')
                    ripples = [];
            },
        };
    });
}
export default mount;
