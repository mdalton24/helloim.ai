// THE JARVIS SCENE — VENDORED FROM THE REAL BOARD, NOT A READING OF IT.
//
// This canvas was asked to match ai.markdalton.com three times and got a
// lookalike each time — a from-scratch 491-line file sharing the board's
// vocabulary (chip, traces, packets, cyan/gold/red) and none of its actual
// code, easing or constants. This file is the fix: every drawing and
// animation function below is LIFTED, not rewritten, out of the real
// board's own source file — same numbers, same easing, same draw order,
// same comments, because it is the same code.
//
// mount(canvas) -> { onResize, destroy, setThinking, setSpeaking, setListening,
//                     bump, pulse, flare } — the contract every scene exposes;
// scene-mount.js forwards window.RiftBrain's six calls (plus setListening)
// straight through to whichever of these is returned.
//
// THE FOUR SEAMS, and nothing else here is original: (1) the mount/resize/
// contract wrapper itself — the real file is a single page-load script, this
// runs once per theme switch and has to be able to unmount; (2) the canvas
// sizes off canvas.getBoundingClientRect() rather than window.innerWidth/
// innerHeight, because this board shares the screen with the app's own
// chrome instead of owning it; (3) the board's "populated half" of
// decorative resistor/IC filler is replaced with THIS app's real connectors,
// via window.NameOSConnectorLEDs() — placed with the app's own rail/panel
// clearances, drawn with the real board's own IC-package renderer, plus one
// added status dot so a connector's proven/unproven state (the same four
// colours the Apps sheet's own rows use) can never be invisible; (4) the die
// wordmark reads window.NameOSAssistantName() instead of the real board's
// own hardcoded "J.A.R.V.I.S" — same scramble, same fit-to-width, same
// dot-tracking, a different string in.
//
// WHAT DID NOT COME ACROSS, SAID PLAINLY RATHER THAN SILENTLY DROPPED:
//   - The presentation flythrough camera (FLY / computeCam / camMatrix /
//     dealRun / the shot pool / the caption bar). It is welded to the real
//     page's own "1" key, its cinema-mode DOM toggle and a caption element
//     this app does not have. Every function below runs the real board's own
//     CAM === null path — which, by the real file's own comment on the trace
//     bevel, is what the board itself runs 99% of its life. Nothing here
//     approximates the camera; it simply is not present.
//   - The HUD clock / "who's working" strip / stall banner / TP1 scope. All
//     four read /state and /status from the voice-line server. This app has
//     no such endpoints and draws its own chrome (state pill, activity
//     strip) around this canvas instead — that chrome is index.html's job,
//     not this file's.
//   - Click-to-spawn-a-pulse (spawnAt) and click-on-the-emblem-to-toggle-
//     privacy-mode. Both are wired to page-level pointerdown handlers
//     (gateOpen, dashPanelOpen, setPrivacy) this app doesn't share. Hover IS
//     ported below, unwired to any click — it costs nothing and the emblem
//     answering a pointer is part of how the real one behaves.
//   - etchMark(). Defined in the real file and never called by it any more —
//     the wordmark moved into drawEmblem's own text pass. Porting a function
//     the real board itself does not run would not be a match; it would be
//     new dead code with the real board's name on it.
export function mount(canvas) {
  const ctx = canvas.getContext('2d', { alpha: false });
  /* CANVAS UNAVAILABLE. No equivalent of WebGL's own failure mode exists for
     2D; if getContext returns null, fall back to the theme's --bg (already
     what .brain's own CSS background is) rather than a broken half-scene. */
  if (!ctx) {
    return {
      onResize() {}, destroy() {}, setThinking() {}, setSpeaking() {},
      setListening() {}, bump() {}, pulse() {}, flare() {},
    };
  }

  /* --- reduced motion, live-updating same as the real board -------------- */
  const calmMQ = window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)');
  let reduceMotion = calmMQ ? calmMQ.matches : false;
  const onCalmChange = (e) => { reduceMotion = e.matches; };
  if (calmMQ && calmMQ.addEventListener) calmMQ.addEventListener('change', onCalmChange);
  else if (calmMQ && calmMQ.addListener) calmMQ.addListener(onCalmChange);

  /* ===========================================================================
     Voice Line — living circuit board (ported)
     ---------------------------------------------------------------------------
     Signals race the copper toward a chip at the centre. Everything is drawn
     procedurally. Structure:
       - the board is generated once, seeded, and BAKED to offscreen canvases
       - only packets, die, waves and indicators are drawn per frame
       - ONE eased pair of energies (motion, glow) drives every visual
     =========================================================================== */

  /* THE SCENE'S PALETTE — the site's own tokens, verbatim, not approximations.
     substrate/mask/silk/silkBright are the site's --bg/--bg-2/--ink-dim/--ink.
     Traces are NOT copper: copper sat inside the amber band that means ACTIVE
     on this board, so the trace is now the same lightness as copper (46%) at a
     third of the chroma, in the site's hue, and only cyan/gold/red are allowed
     to be saturated — inbound / outbound / alert. traceLit is the bloom layer
     and is deliberately light rather than coloured: it is energy, not
     direction. Do not rotate the three signal colours to make a screenshot
     look tidier — that deletes the information and keeps the decoration. */
  const PAL = {
    substrate: '#000000',
    weave: 'rgba(255,255,255,0.020)',
    mask: '#061420',
    trace: '#2f7fa8',      // structure, never signal
    traceLit: '#8fe6ff',   // the bloom — energy, not direction
    traceLitRGB: [143, 230, 255],
    silk: '#9aa0a8',
    silkRGB: [154, 160, 168],
    silkBright: '#e7edf0',
    pkgTop: '#061420',
    pkgEdge: '#28536b',
    inbound: [79, 214, 224],    // cyan — arriving.  MEANING. do not rotate.
    outbound: [255, 195, 107],  // gold — leaving.   MEANING. do not rotate.
    alert: [255, 59, 48],       // red  — alert.     MEANING. do not rotate.
    // Ambient neons carry NO meaning on purpose — a minority of packets,
    // direction-agnostic, the board idling rather than reporting.
    ambient: [[255, 92, 214], [163, 255, 96]],
  };

  // Everything printed ON the board uses the site's own --mono stack, character
  // for character, so the canvas and the chrome resolve to the same face.
  const SILK_FACE = 'monospace,ui-monospace,"SF Mono",Menlo,Consolas,' +
                    '"DejaVu Sans Mono",monospace';

  // motion and glow are deliberately separate axes: "alive" and "fast" are not
  // the same thing. Speaking cruises with full glow; thinking runs flat out.
  const STATES = {
    idle:      { motion: 0.20, glow: 0.15, flow: +1, rate: 0.45 },
    listening: { motion: 0.45, glow: 0.40, flow: -1, rate: 1.60 },
    thinking:  { motion: 1.00, glow: 0.55, flow: -1, rate: 2.60 },
    speaking:  { motion: 0.55, glow: 1.00, flow: +1, rate: 1.30 },
  };

  const CFG = {
    levelTau: 0.09,     // 90ms smoothing on level
    attackTau: 0.5,     // energy rises fast
    releaseTau: 1.0,    // and falls slower
    dprCap: 1.25,
    bootMs: 2000,
    pinsPerSide: 16,     // matches the density of the reference PCB photo
    maxWaves: 3,
    maxPackets: 260,
    seed: 0x5eed1e,
  };

  /* --- deterministic rng --------------------------------------------------- */
  function mulberry32(a) {
    return function () {
      a |= 0; a = (a + 0x6D2B79F5) | 0;
      let t = Math.imul(a ^ (a >>> 15), 1 | a);
      t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
  }
  let rnd = mulberry32(CFG.seed);
  const rr = (lo, hi) => lo + rnd() * (hi - lo);
  const ri = (lo, hi) => Math.floor(rr(lo, hi + 1));

  const TAU = Math.PI * 2;
  function rgba(c, a) { return `rgba(${c[0]},${c[1]},${c[2]},${a})`; }

  // The frozen status colours read from CSS, not re-typed here, so a connector
  // can never disagree with a future change to :root's --good/--warn/--bad/
  // --idle. SEAM: this function and the four-colour convention it reads are
  // this app's own (the real board carries no connector state at all); it is
  // the same lookup the Apps sheet's own dots use.
  const DOTCOL = { ok: '#22c55e', warn: '#f59e0b', idle: '#64748b', bad: '#ff7a7a' };
  function statusColor(state) {
    try {
      const cs = getComputedStyle(document.documentElement);
      const v = cs.getPropertyValue('--' + (
        state === 'ok' ? 'good' : state === 'bad' ? 'bad' : state === 'warn' ? 'warn' : 'idle'
      )).trim();
      if (v) return v;
    } catch (e) {}
    return DOTCOL[state] || DOTCOL.idle;
  }

  /* --- canvas --------------------------------------------------------------- */
  let W = 0, H = 0, DPR = 1;
  let baseC = document.createElement('canvas');
  let glowC = document.createElement('canvas');
  let scratchC = document.createElement('canvas');
  let SW = 0, SH = 0;

  /* --- board model ---------------------------------------------------------- */
  let chip = { x: 0, y: 0, r: 0 };
  let nets = [];
  let vias = [];
  let comps = [];
  let pinGlow = [];
  let viaGlow = [];
  let compGlow = [];
  let railW = 82;
  // SEAM — the real board owns the whole viewport and never drops its chip;
  // this canvas shares the screen with a chat panel that can cover the
  // centre below a usable size (spec §3.3). Gates drawChipPackage() in
  // paintBoardVector and the die/emblem/indicators in render().
  let hasChip = true;

  // One inset, shared by the edge-fan targets and the landing strip they
  // terminate on, so the two stay in agreement.
  function edgeMargin() { return Math.max(W, H) * 0.04; }

  /* Chamfer every interior corner of a polyline into a 45-degree mitre — the
     detail that makes routing read as copper rather than a wire diagram. */
  function mitre(raw, amount) {
    const clean = [raw[0]];
    for (const p of raw.slice(1)) {
      const q = clean[clean.length - 1];
      if (Math.hypot(p.x - q.x, p.y - q.y) > 0.5) clean.push(p);
    }
    if (clean.length < 3) return clean;
    const out = [clean[0]];
    for (let i = 1; i < clean.length - 1; i++) {
      const a = clean[i - 1], b = clean[i], c = clean[i + 1];
      const l1 = Math.hypot(b.x - a.x, b.y - a.y);
      const l2 = Math.hypot(c.x - b.x, c.y - b.y);
      const m = Math.min(amount, l1 * 0.45, l2 * 0.45);
      if (m < 1) { out.push(b); continue; }
      out.push({ x: b.x + (a.x - b.x) / l1 * m, y: b.y + (a.y - b.y) / l1 * m });
      out.push({ x: b.x + (c.x - b.x) / l2 * m, y: b.y + (c.y - b.y) / l2 * m });
    }
    out.push(clean[clean.length - 1]);
    return out;
  }

  function buildBoard() {
    rnd = mulberry32(CFG.seed);          // rebuild identically every time
    try {
      railW = parseFloat(getComputedStyle(document.documentElement)
        .getPropertyValue('--rail-w')) || 82;
    } catch (e) { railW = 82; }

    const cx = W / 2, cy = H / 2;
    const size = Math.min(W, H) * 0.19;
    chip = { x: cx, y: cy, r: size / 2 };
    nets = []; vias = []; comps = []; pinGlow = []; viaGlow = []; compGlow = [];

    /* SEAM — the app shares its screen with a chat panel; the real board owns
       the whole viewport and never needs this. Below the size a chat panel
       actually needs, the chip is dropped rather than half-covered — the
       state it carries still shows in the app's own top pill. */
    hasChip = (W >= 900 && H >= 620);
    if (!hasChip) { comps = []; vias = []; nets = []; return; }

    const margin = edgeMargin();
    const sides = [
      { nx: 0, ny: -1, dir: 6 },   // top
      { nx: 1, ny: 0,  dir: 0 },   // right
      { nx: 0, ny: 1,  dir: 2 },   // bottom
      { nx: -1, ny: 0, dir: 4 },   // left
    ];

    /* PINS SIT ON A CIRCLE, NOT A SQUARE. Each side owns a quadrant of the
       circle and its pins are spaced along that arc — the escape direction is
       radial rather than axis-aligned, so traces leave the emblem like spokes
       and the empty region around it is round, not the square a straight-edge
       layout would leave behind the round chip. */
    sides.forEach((side) => {
      for (let i = 0; i < CFG.pinsPerSide; i++) {
        const f = (i + 0.5) / CFG.pinsPerSide;
        const base = Math.atan2(side.ny, side.nx);
        const ang = base + (f - 0.5) * (Math.PI / 2) * 0.92;
        const ex = Math.cos(ang), ey = Math.sin(ang);
        const px = chip.x + ex * chip.r;
        const py = chip.y + ey * chip.r;

        // Route like an actual PCB: escape the package perpendicular, then run
        // in orthogonal channels toward a point on the board edge, doglegging
        // as it goes. Corners are mitred to 45 degrees afterwards.
        const esc = chip.r * rr(0.18, 0.55) + 10;
        let cur = { x: px + ex * esc, y: py + ey * esc };
        const raw = [{ x: px, y: py }, { x: cur.x, y: cur.y }];

        // Target ON the board — pulled back onto the landing strip at
        // `margin` from each edge, with a real pad at the end of it, so a
        // trace never trails off into blank space.
        const spread = rr(-0.85, 0.85);
        let tx, ty;
        if (side.ny !== 0) {
          tx = Math.min(W - margin, Math.max(margin, W * (0.5 + spread * 0.75)));
          ty = side.ny < 0 ? margin : H - margin;
        } else {
          tx = side.nx < 0 ? margin : W - margin;
          ty = Math.min(H - margin, Math.max(margin, H * (0.5 + spread * 0.75)));
        }
        const target = { x: tx, y: ty };

        // Alternate horizontal and vertical runs toward the target.
        let horizFirst = Math.abs(target.x - cur.x) > Math.abs(target.y - cur.y);
        const legs = ri(2, 3);
        for (let L = 0; L < legs; L++) {
          const f2 = (L + 1) / (legs + 1);
          const gx = cur.x + (target.x - cur.x) * f2 * rr(0.8, 1.35);
          const gy = cur.y + (target.y - cur.y) * f2 * rr(0.8, 1.35);
          if (horizFirst) {
            cur = { x: gx, y: cur.y }; raw.push({ x: cur.x, y: cur.y });
            cur = { x: cur.x, y: gy }; raw.push({ x: cur.x, y: cur.y });
          } else {
            cur = { x: cur.x, y: gy }; raw.push({ x: cur.x, y: cur.y });
            cur = { x: gx, y: cur.y }; raw.push({ x: cur.x, y: cur.y });
          }
          horizFirst = !horizFirst;
          if (rnd() < 0.45) vias.push({ x: cur.x, y: cur.y, r: rr(2.2, 3.6) });
        }
        // final approach: square onto the edge
        if (side.ny !== 0) raw.push({ x: cur.x, y: target.y });
        else raw.push({ x: target.x, y: cur.y });

        const pts = mitre(raw, Math.min(W, H) * 0.022);

        const cum = [0];
        let len = 0;
        for (let k = 1; k < pts.length; k++) {
          len += Math.hypot(pts[k].x - pts[k - 1].x, pts[k].y - pts[k - 1].y);
          cum.push(len);
        }
        if (len < 1) return;
        let bx0 = Infinity, by0 = Infinity, bx1 = -Infinity, by1 = -Infinity;
        for (const p of pts) {
          if (p.x < bx0) bx0 = p.x; if (p.x > bx1) bx1 = p.x;
          if (p.y < by0) by0 = p.y; if (p.y > by1) by1 = p.y;
        }
        nets.push({ pts, cum, len, pin: { x: px, y: py },
                    bbox: { x0: bx0, y0: by0, x1: bx1, y1: by1 } });
        pinGlow.push(0);

        // The landing pad — bigger than an ordinary via so it reads as a
        // terminus rather than one more dogleg.
        vias.push({ x: target.x, y: target.y, r: rr(2.6, 4.0) });
      }
    });

    /* THE LANDING RING — a real fan-out has a ring of vias/pads where the
       escape traces drop to another layer; two staggered bands, offset by
       half a step from each other and sat between the spokes. */
    const spokes = CFG.pinsPerSide * 4;
    for (let band = 0; band < 2; band++) {
      const rad = chip.r * (1.10 + band * 0.13);
      const count = Math.round(spokes * (band ? 0.7 : 1.0));
      for (let i = 0; i < count; i++) {
        const a = ((i + (band ? 0.5 : 0)) / count) * TAU + (band ? 0.04 : 0);
        vias.push({ x: chip.x + Math.cos(a) * rad,
                    y: chip.y + Math.sin(a) * rad,
                    r: rr(1.8, 3.0) });
      }
    }

    /* SEAM — THE COMPONENTS ARE THE PERSON'S ACTUAL CONNECTORS, not the real
       board's own decorative resistor/pad-bank filler and not its synthetic
       IC packages either. Same source and the same three-state rule the Apps
       sheet's own dots use (window.NameOSConnectorLEDs). With none connected
       the field is traces and vias and no packages — the honest empty state,
       not a decorative one. Placed with THIS app's own rail/panel clearances
       (the real board owns the whole viewport and has no such concern);
       drawn below by the real board's own drawComps() ic-package renderer,
       unmodified, plus one added status dot (see drawComps). */
    const leds = (window.NameOSConnectorLEDs ? window.NameOSConnectorLEDs() : []) || [];
    const panelW = Math.min(440, W - railW - 40);
    const openR = railW + 24, openL = W - panelW - 40, openT = 56 + 70, openB = H - 30;
    const n = Math.min(leds.length, W < 1000 ? 3 : 5);
    const slots = [];
    for (let i = 0; i < n; i++) {
      const conn = leds[i];
      const w = 74 + Math.max(0, conn.name.length - 6) * 7, h = 38;
      let placed = null;
      for (let tries = 0; tries < 120 && !placed; tries++) {
        const a = (i / n) * TAU + 0.6 + tries * 0.37;
        const rad = Math.min(W, H) * (0.26 + 0.055 * (tries % 6));
        const bx = cx + Math.cos(a) * rad - w / 2, by = cy + Math.sin(a) * rad * 0.85 - h / 2;
        if (bx < openR || bx + w > openL || by < openT || by + h > openB) continue;
        if (Math.hypot(bx + w / 2 - cx, by + h / 2 - cy) < chip.r * 1.45) continue;
        if (slots.some((s) => bx < s.x + s.w + 16 && bx + w > s.x - 16 && by < s.y + s.h + 16 && by + h > s.y - 16)) continue;
        placed = { x: bx + w / 2, y: by + h / 2, w, h };
      }
      if (placed) {
        const cp = {
          x: placed.x, y: placed.y, w, h, rot: 0, ic: true,
          pins: Math.max(3, Math.round(h / 7)),
          label: conn.name,
          part: conn.name.toUpperCase(),
          lot: (conn.state || 'idle').toUpperCase(),
          c: conn,
        };
        slots.push({ x: placed.x - w / 2, y: placed.y - h / 2, w, h });
        comps.push(cp);
      }
    }

    wireChips();
    buildMarks();
  }

  /* ---- CHIP-TO-CHIP WIRING --------------------------------------------------
     Nodes wired TO EACH OTHER around a hub, rather than a hub radiating into
     space — every package has a real run back to the centre, and neighbouring
     packages are linked to each other, which is what turns a starburst into a
     network. Every net built here goes through the SAME mitre and arc-length
     machinery as the edge fans, which is the whole reason packets and the
     arrival glow work on them with no further change. */

  function chipEdgeToward(t) {
    const dx = t.x - chip.x, dy = t.y - chip.y;
    const out = chip.r + 8;
    return Math.abs(dx) > Math.abs(dy)
      ? { x: chip.x + Math.sign(dx) * out, y: chip.y + dy * 0.22 }
      : { x: chip.x + dx * 0.22, y: chip.y + Math.sign(dy) * out };
  }
  function compEdgeToward(cp, t) {
    const dx = t.x - cp.x, dy = t.y - cp.y;
    return Math.abs(dx) * cp.h > Math.abs(dy) * cp.w
      ? { x: cp.x + Math.sign(dx) * (cp.w / 2 + 6), y: cp.y }
      : { x: cp.x, y: cp.y + Math.sign(dy) * (cp.h / 2 + 6) };
  }
  function linkNet(from, to) {
    const horizFirst = Math.abs(to.x - from.x) > Math.abs(to.y - from.y);
    const mid = horizFirst ? { x: to.x, y: from.y } : { x: from.x, y: to.y };
    const pts = mitre([from, mid, to], Math.min(W, H) * 0.022);
    if (pts.length < 2) return;
    const cum = [0];
    let len = 0;
    for (let k = 1; k < pts.length; k++) {
      len += Math.hypot(pts[k].x - pts[k - 1].x, pts[k].y - pts[k - 1].y);
      cum.push(len);
    }
    if (len < 24) return;
    let bx0 = Infinity, by0 = Infinity, bx1 = -Infinity, by1 = -Infinity;
    for (const q of pts) {
      if (q.x < bx0) bx0 = q.x; if (q.x > bx1) bx1 = q.x;
      if (q.y < by0) by0 = q.y; if (q.y > by1) by1 = q.y;
    }
    nets.push({ pts, cum, len, pin: { x: from.x, y: from.y },
                bbox: { x0: bx0, y0: by0, x1: bx1, y1: by1 } });
    pinGlow.push(0);
  }
  function wireChips() {
    const ics = comps.filter((c) => c.ic);
    if (!ics.length) return;
    for (const cp of ics) linkNet(chipEdgeToward(cp), compEdgeToward(cp, chip));
    const done = new Set();
    for (let i = 0; i < ics.length; i++) {
      let best = -1, bd = Infinity;
      for (let j = 0; j < ics.length; j++) {
        if (i === j) continue;
        const d = Math.hypot(ics[i].x - ics[j].x, ics[i].y - ics[j].y);
        if (d < bd) { bd = d; best = j; }
      }
      if (best < 0) continue;
      const key = i < best ? i + ':' + best : best + ':' + i;
      if (done.has(key)) continue;
      done.add(key);
      linkNet(compEdgeToward(ics[i], ics[best]), compEdgeToward(ics[best], ics[i]));
    }
  }

  /* WHICH NODES EACH NET PASSES, AND WHERE ALONG IT — built once so an
     arrival at runtime is a comparison against a distance already known,
     never a search. */
  const VIA_REACH = 7, COMP_REACH = 26;
  function buildMarks() {
    viaGlow = vias.map(() => 0);
    compGlow = comps.map(() => 0);
    const ics = [];
    for (let i = 0; i < comps.length; i++) if (comps[i].ic) ics.push(i);
    for (const net of nets) {
      const marks = [];
      for (let k = 0; k < net.pts.length; k++) {
        const pt = net.pts[k], sAt = net.cum[k];
        for (let vi = 0; vi < vias.length; vi++) {
          const v = vias[vi];
          if (Math.abs(v.x - pt.x) > VIA_REACH || Math.abs(v.y - pt.y) > VIA_REACH) continue;
          if (!marks.some((m) => m.kind === 0 && m.i === vi)) marks.push({ kind: 0, i: vi, s: sAt });
        }
        for (const ci of ics) {
          const cp = comps[ci];
          if (Math.abs(cp.x - pt.x) > cp.w / 2 + COMP_REACH) continue;
          if (Math.abs(cp.y - pt.y) > cp.h / 2 + COMP_REACH) continue;
          if (!marks.some((m) => m.kind === 1 && m.i === ci)) marks.push({ kind: 1, i: ci, s: sAt });
        }
      }
      marks.sort((a, b) => a.s - b.s);
      net.marks = marks;
    }
  }

  function pointAt(net, s) {
    const c = net.cum;
    let lo = 0, hi = c.length - 1;
    if (s <= 0) return net.pts[0];
    if (s >= net.len) return net.pts[net.pts.length - 1];
    while (lo < hi - 1) {
      const mid = (lo + hi) >> 1;
      if (c[mid] <= s) lo = mid; else hi = mid;
    }
    const seg = c[hi] - c[lo] || 1;
    const t = (s - c[lo]) / seg;
    const a = net.pts[lo], b = net.pts[hi];
    return { x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t };
  }

  /* --- baking --------------------------------------------------------------- */
  function bboxHit(bb, vis) {
    return !(bb.x1 < vis.x0 || bb.x0 > vis.x1 || bb.y1 < vis.y0 || bb.y0 > vis.y1);
  }
  function drawWeave(c, vis) {
    c.save();
    c.strokeStyle = PAL.weave; c.lineWidth = 1;
    const xa = vis ? Math.max(0, Math.floor(vis.x0 / 9) * 9) : 0;
    const xb = vis ? Math.min(W, vis.x1) : W;
    const ya = vis ? Math.max(0, Math.floor(vis.y0 / 9) * 9) : 0;
    const yb = vis ? Math.min(H, vis.y1) : H;
    const yTop = vis ? Math.max(0, vis.y0) : 0, yBot = vis ? Math.min(H, vis.y1) : H;
    const xLft = vis ? Math.max(0, vis.x0) : 0, xRgt = vis ? Math.min(W, vis.x1) : W;
    for (let x = xa; x < xb; x += 9) { c.beginPath(); c.moveTo(x, yTop); c.lineTo(x, yBot); c.stroke(); }
    for (let y = ya; y < yb; y += 9) { c.beginPath(); c.moveTo(xLft, y); c.lineTo(xRgt, y); c.stroke(); }
    c.restore();
  }
  // ONE PATH, ONE STROKE — every net batched into a single Path2D so it can be
  // stroked more than once (the bevel) without re-walking every point twice.
  function buildTracePath(vis) {
    const path = new Path2D();
    for (const n of nets) {
      if (vis && !bboxHit(n.bbox, vis)) continue;
      path.moveTo(n.pts[0].x, n.pts[0].y);
      for (let k = 1; k < n.pts.length; k++) path.lineTo(n.pts[k].x, n.pts[k].y);
    }
    return path;
  }
  // THE COPPER BEVEL — the same path stroked three times, offset toward and
  // away from one constant light source (upper-left), so it scales with zoom
  // automatically with no per-shot tuning.
  function strokeTracesPath(c, path, width, style, alpha, bevel) {
    c.save();
    c.lineJoin = 'round'; c.lineCap = 'round';
    if (bevel) {
      c.save(); c.translate(0.9, 0.9); c.globalAlpha = alpha * 0.5;
      c.strokeStyle = 'rgba(0,0,0,0.85)'; c.lineWidth = width; c.stroke(path);
      c.restore();
    }
    c.globalAlpha = alpha; c.strokeStyle = style; c.lineWidth = width; c.stroke(path);
    if (bevel) {
      c.save(); c.translate(-0.75, -0.75); c.globalAlpha = alpha * 0.5;
      c.strokeStyle = 'rgba(255,255,255,0.55)'; c.lineWidth = width * 0.4; c.stroke(path);
      c.restore();
    }
    c.restore();
  }
  // THE LANDING STRIP — a thin inset line at edgeMargin(), the ground-pour
  // boundary every real PCB has just inside its physical edge.
  function drawEdgeStrip(c) {
    const m = edgeMargin();
    c.save();
    c.strokeStyle = PAL.trace; c.globalAlpha = 0.22; c.lineWidth = 1;
    c.strokeRect(m, m, W - m * 2, H - m * 2);
    c.restore();
  }
  function drawVias(c, vis) {
    c.save();
    for (const v of vias) {
      if (vis && (v.x < vis.x0 || v.x > vis.x1 || v.y < vis.y0 || v.y > vis.y1)) continue;
      c.beginPath(); c.arc(v.x, v.y, v.r, 0, TAU);
      c.fillStyle = PAL.mask; c.fill();
      c.lineWidth = 1.3; c.strokeStyle = PAL.trace; c.globalAlpha = 0.6; c.stroke();
      c.globalAlpha = 1;
    }
    c.restore();
  }
  function drawComps(c, vis) {
    const wide = vis && (vis.x1 - vis.x0) > W * 0.5;
    c.save();
    c.font = `8px ${SILK_FACE}`;
    c.textAlign = 'center'; c.textBaseline = 'middle';
    for (const cp of comps) {
      if (vis && (cp.x < vis.x0 || cp.x > vis.x1 || cp.y < vis.y0 || cp.y > vis.y1)) continue;
      c.save();
      c.translate(cp.x, cp.y); c.rotate(cp.rot);

      if (cp.ic) {
        // SIDE PINS FIRST, so the body is drawn over the roots and the legs
        // read as going UNDER the package.
        const gap = cp.h / (cp.pins + 1);
        const legW = 5, legH = Math.max(2, gap * 0.34);
        c.fillStyle = PAL.pkgEdge; c.globalAlpha = 0.85;
        for (let i = 1; i <= cp.pins; i++) {
          const py = -cp.h / 2 + gap * i - legH / 2;
          c.fillRect(-cp.w / 2 - legW, py, legW, legH);
          c.fillRect(cp.w / 2, py, legW, legH);
        }
        c.globalAlpha = 1;
        if (!wide) {
          c.save();
          c.shadowColor = 'rgba(0,0,0,0.62)';
          c.shadowBlur = Math.max(3, cp.h * 0.30);
          c.shadowOffsetX = Math.max(1.5, cp.w * 0.05);
          c.shadowOffsetY = Math.max(1.5, cp.h * 0.14);
          c.fillStyle = PAL.pkgTop;
          c.fillRect(-cp.w / 2, -cp.h / 2, cp.w, cp.h);
          c.restore();
        }
        c.fillStyle = PAL.pkgTop;
        c.fillRect(-cp.w / 2, -cp.h / 2, cp.w, cp.h);
        c.strokeStyle = PAL.pkgEdge; c.globalAlpha = 0.75; c.lineWidth = 1;
        c.strokeRect(-cp.w / 2, -cp.h / 2, cp.w, cp.h);

        // The moulded lid.
        c.globalAlpha = 0.5;
        c.strokeStyle = PAL.pkgEdge; c.lineWidth = 0.6;
        c.strokeRect(-cp.w / 2 + 2.5, -cp.h / 2 + 2.5, cp.w - 5, cp.h - 5);

        // Landing pads under each leg.
        const gap2 = cp.h / (cp.pins + 1);
        c.globalAlpha = 0.30; c.fillStyle = PAL.traceLit;
        for (let i = 1; i <= cp.pins; i++) {
          const py2 = -cp.h / 2 + gap2 * i - Math.max(1, gap2 * 0.16);
          c.fillRect(-cp.w / 2 - 6.5, py2, 2.2, Math.max(1.4, gap2 * 0.32));
          c.fillRect(cp.w / 2 + 4.3, py2, 2.2, Math.max(1.4, gap2 * 0.32));
        }

        // pin-1 notch, bottom left.
        c.globalAlpha = 0.5; c.fillStyle = PAL.silk;
        c.beginPath();
        c.arc(-cp.w / 2 + 4.5, cp.h / 2 - 4.5, 1.7, 0, TAU);
        c.fill();

        if (!wide) {
          c.textAlign = 'center'; c.textBaseline = 'middle';
          c.globalAlpha = 0.34; c.fillStyle = PAL.silk;
          c.font = `${Math.max(2.6, cp.h * 0.20)}px ${SILK_FACE}`;
          c.fillText(cp.part, 0, -cp.h * 0.10);
          c.globalAlpha = 0.22;
          c.font = `${Math.max(2.2, cp.h * 0.15)}px ${SILK_FACE}`;
          c.fillText(cp.lot, 0, cp.h * 0.16);

          c.globalAlpha = 0.42;
          c.font = `${Math.max(3, cp.h * 0.26)}px ${SILK_FACE}`;
          c.fillText(cp.label, 0, -cp.h / 2 - 5);
        }

        /* SEAM — the real board carries no per-component status; this app's
           connectors do, and it must never be able to look proven when it
           is not (idle/warn/bad all read as visibly NOT ok). Same three-fact
           dot the Apps sheet's own connector rows use, read live so a theme
           switch or a fresh connection is never a stale colour baked in. */
        if (cp.c) {
          c.globalAlpha = 1;
          c.fillStyle = statusColor(cp.c.state);
          c.beginPath();
          c.arc(cp.w / 2 - 7, -cp.h / 2 + 6, 2.4, 0, TAU);
          c.fill();
        }

        c.restore();
        continue;
      }
      c.restore();
    }
    c.restore();
  }
  // The crisp static layer, in world coordinates.
  function paintBoardVector(c, vis) {
    const wide = vis && (vis.x1 - vis.x0) > W * 0.72;
    if (!wide) drawWeave(c, vis);
    drawEdgeStrip(c);
    const tracePath = buildTracePath(vis);
    strokeTracesPath(c, tracePath, 6.5, PAL.mask, 1, false);
    // Bevel is baked-only: full 3D bevel when vis===null (bake()'s own call),
    // flat trace on any live/culled call.
    strokeTracesPath(c, tracePath, 2.4, PAL.trace, 0.55, vis === null);
    drawVias(c, vis);
    drawComps(c, vis);
    if (hasChip) drawChipPackage(c);
  }
  // THE SUBSTRATE GRID — deliberately at the edge of visible: one pixel,
  // ~3% alpha. Baked under everything, before the traces.
  const GRID_STEP = 28;
  const GRID_INK = 'rgba(96, 168, 214, 0.030)';
  function paintGrid(b) {
    b.save();
    b.strokeStyle = GRID_INK; b.lineWidth = 1;
    b.beginPath();
    for (let x = 0.5; x <= W; x += GRID_STEP) { b.moveTo(x, 0); b.lineTo(x, H); }
    for (let y = 0.5; y <= H; y += GRID_STEP) { b.moveTo(0, y); b.lineTo(W, y); }
    b.stroke();
    b.restore();
  }

  function bake() {
    for (const c of [baseC, glowC]) { c.width = canvas.width; c.height = canvas.height; }
    SW = Math.max(2, Math.floor(canvas.width * 0.5));
    SH = Math.max(2, Math.floor(canvas.height * 0.5));
    scratchC.width = SW; scratchC.height = SH;

    const b = baseC.getContext('2d');
    b.setTransform(DPR, 0, 0, DPR, 0, 0);
    b.fillStyle = PAL.substrate;
    b.fillRect(0, 0, W, H);
    paintGrid(b);
    paintBoardVector(b, null);

    // VIGNETTE — bright around the chip, near-black in the corners, so the
    // centre reads as the subject.
    const vg = b.createRadialGradient(
      chip.x, chip.y, Math.min(W, H) * 0.10,
      chip.x, chip.y, Math.max(W, H) * 0.78);
    vg.addColorStop(0.00, 'rgba(1,3,7,0)');
    vg.addColorStop(0.55, 'rgba(1,3,7,0.38)');
    vg.addColorStop(1.00, 'rgba(1,3,7,0.94)');
    b.fillStyle = vg;
    b.fillRect(0, 0, W, H);

    // glow layer: the traces again, bright and blurred.
    const g = glowC.getContext('2d');
    g.setTransform(DPR, 0, 0, DPR, 0, 0);
    g.clearRect(0, 0, W, H);
    g.filter = 'blur(7px)';
    strokeTracesPath(g, buildTracePath(null), 3.0, PAL.traceLit, 0.85, false);
    g.filter = 'none';

    /* PUNCH THE COMPONENTS AND THE CHIP OUT OF THE GLOW — lines go UNDER
       components and under the chip, never over. destination-out erases
       alpha rather than painting, so this costs nothing per frame; baked
       once, with everything else. */
    g.save();
    g.globalCompositeOperation = 'destination-out';
    g.fillStyle = '#000';
    for (const cp of comps) {
      g.save();
      g.translate(cp.x, cp.y);
      g.rotate(cp.rot);
      g.fillRect(-cp.w / 2, -cp.h / 2, cp.w, cp.h);
      g.restore();
    }
    g.beginPath();
    g.arc(chip.x, chip.y, chip.r, 0, TAU);
    g.fill();
    g.restore();
  }

  function drawChipPackage(c) {
    const { x, y, r } = chip;
    /* THE SEAT THE EMBLEM SITS IN. Only the dark seat is baked here —
       everything that reads as the emblem is drawn live in drawEmblem(),
       because it turns and it answers his voice, and a baked layer can do
       neither. */
    c.save();
    const seat = c.createRadialGradient(x, y, r * 0.1, x, y, r);
    seat.addColorStop(0, '#04101a');
    seat.addColorStop(1, '#020a12');
    c.save();
    c.shadowColor = 'rgba(0,0,0,0.75)';
    c.shadowBlur = r * 0.42;
    c.shadowOffsetX = r * 0.06;
    c.shadowOffsetY = r * 0.10;
    c.fillStyle = '#000';
    c.beginPath(); c.arc(x, y, r * 0.98, 0, TAU); c.fill();
    c.restore();

    c.fillStyle = seat;
    c.beginPath(); c.arc(x, y, r, 0, TAU); c.fill();
    c.strokeStyle = PAL.pkgEdge; c.globalAlpha = 0.55; c.lineWidth = 1.2;
    c.beginPath(); c.arc(x, y, r, 0, TAU); c.stroke();
    c.restore();
  }

  /* --- runtime state -------------------------------------------------------- */
  const S = {
    target: 'idle',
    motion: 0.2, glow: 0.15, flow: 1, rate: 0.45,
    level: 0, levelRaw: 0,
    alert: 0, alertTarget: 0, alertHold: 0,
    packets: [],
    waves: [],
    t: 0,
    prevLevel: 0,
    lastWave: -1,
    chase: 0,
    scan: 0,
    boot: CFG.bootMs / 1000,
  };

  function ease(cur, tgt, dt, tauUp, tauDown) {
    const tau = tgt > cur ? tauUp : tauDown;
    const k = 1 - Math.exp(-dt / tau);
    return cur + (tgt - cur) * k;
  }

  // ABOUT ONE PACKET IN FOUR IS AMBIENT — too many and the board stops being
  // able to say "arriving"/"leaving" at a glance.
  const AMBIENT_SHARE = 0.25;
  function spawnPacket(flow) {
    if (S.packets.length >= CFG.maxPackets || !nets.length) return;
    const n = ri(0, nets.length - 1);
    const net = nets[n];
    const ambient = rnd() < AMBIENT_SHARE;
    S.packets.push({
      n,
      s: flow > 0 ? 0 : net.len,
      v: rr(0.55, 1.15),
      flow,
      life: 1,
      hue: ambient ? PAL.ambient[ri(0, PAL.ambient.length - 1)]
                   : (flow > 0 ? PAL.outbound : PAL.inbound),
    });
  }

  function step(dt) {
    S.t += dt;
    const def = STATES[S.target] || STATES.idle;
    S.motion = ease(S.motion, def.motion, dt, CFG.attackTau, CFG.releaseTau);
    S.glow = ease(S.glow, def.glow, dt, CFG.attackTau, CFG.releaseTau);
    // SEAM — the real board holds S.alertTarget from a continuous server
    // poll; flare(false) below is a discrete event, so it holds alertTarget
    // at 1 for a short window (alertHold) rather than forever, then lets the
    // real ease() curve (fast attack, slower release) bring it back down.
    if (S.alertHold > 0) { S.alertHold -= dt; if (S.alertHold <= 0) S.alertTarget = 0; }
    S.alert = ease(S.alert, S.alertTarget, dt, 0.25, 0.7);
    S.flow = def.flow;
    S.level = ease(S.level, S.levelRaw, dt, CFG.levelTau, CFG.levelTau);

    // Pressure waves fire on level PEAKS, not on level itself.
    if (S.target === 'speaking') {
      const riseRate = (S.level - S.prevLevel) / Math.max(dt, 1e-4);
      if (!reduceMotion && riseRate > 0.55 && S.level > 0.22 &&
          S.t - S.lastWave > 0.16 && S.waves.length < CFG.maxWaves) {
        S.waves.push({ r: chip.r * 0.9, a: 1 });
        S.lastWave = S.t;
      }
    }
    S.prevLevel = S.level;

    const maxR = Math.hypot(W, H) * 0.62;
    for (const w of S.waves) { w.r += (maxR * 0.9) * dt; w.a -= dt * 1.15; }
    S.waves = S.waves.filter((w) => w.a > 0 && w.r < maxR);

    // spawn
    const rate = def.rate * (14 + 26 * S.motion) * (S.target === 'speaking' ? (0.42 + 0.72 * S.level) : 1)
               * (reduceMotion ? 0.3 : 1);
    let spawn = rate * dt;
    while (spawn > 0) {
      if (spawn >= 1 || rnd() < spawn) spawnPacket(S.flow);
      spawn -= 1;
    }

    // advance. Speaking runs at 0.62x: the highest-motion state already gets
    // a second multiplier from live level, and two multipliers pulling the
    // same way is how something ends up frantic rather than lively.
    const SPEAK_EASE = 0.62;
    const slow = S.target === 'speaking' ? SPEAK_EASE : 1;
    const speed = (0.10 + 0.55 * S.motion) * Math.min(W, H) * (reduceMotion ? 0.3 : 1) * slow;
    for (const p of S.packets) {
      const was = p.s;
      p.s += p.flow * p.v * speed * dt;
      const net = nets[p.n];
      if (net.marks) {
        const lo = Math.min(was, p.s), hi = Math.max(was, p.s);
        for (const m of net.marks) {
          if (m.s < lo) continue;
          if (m.s > hi) break;
          if (m.kind === 0) viaGlow[m.i] = 1; else compGlow[m.i] = 1;
        }
      }
      if (p.flow < 0 && p.s <= 0) { pinGlow[p.n] = 1; p.life = 0; }
      else if (p.flow > 0 && p.s >= net.len) { p.life = 0; }
    }
    S.packets = S.packets.filter((p) => p.life > 0);

    for (let i = 0; i < pinGlow.length; i++) pinGlow[i] = Math.max(0, pinGlow[i] - dt * 2.2);
    for (let i = 0; i < viaGlow.length; i++) viaGlow[i] = Math.max(0, viaGlow[i] - dt * 3.2);
    for (let i = 0; i < compGlow.length; i++) compGlow[i] = Math.max(0, compGlow[i] - dt * 2.0);

    // The chase drives the indicator sweep AND the emblem's rotation, so it
    // eases with the traffic.
    S.chase += dt * (2 + 10 * S.motion) * (reduceMotion ? 0.3 : 1) * slow;
    S.scan += dt * (0.7 + 1.5 * S.motion) * (reduceMotion ? 0.3 : 1) * slow;
    if (S.boot > 0) S.boot = Math.max(0, S.boot - dt);
  }

  /* ---- render ---------------------------------------------------------------
     Every branch below is the real board's own CAM === null path — the
     presentation flythrough camera did not come across (see the file header);
     this is what the real board itself runs 99% of its life. */
  function render() {
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.fillStyle = PAL.substrate;
    ctx.fillRect(0, 0, canvas.width, canvas.height);

    // boot: reveal the board as power propagates from a corner
    if (S.boot > 0 && !reduceMotion) {
      const p = 1 - S.boot / (CFG.bootMs / 1000);
      const maxR = Math.hypot(W, H) * DPR * 1.05;
      const r = maxR * (p * p * (3 - 2 * p));
      ctx.save();
      ctx.beginPath(); ctx.arc(0, 0, r, 0, TAU); ctx.clip();
      ctx.drawImage(baseC, 0, 0);
      ctx.globalCompositeOperation = 'lighter';
      ctx.globalAlpha = 0.5;
      ctx.drawImage(glowC, 0, 0);
      ctx.restore();
      ctx.save();
      ctx.globalCompositeOperation = 'lighter';
      ctx.strokeStyle = rgba(PAL.outbound, 0.5 * (1 - p));
      ctx.lineWidth = 22 * DPR; ctx.beginPath(); ctx.arc(0, 0, r, 0, TAU); ctx.stroke();
      ctx.restore();
      return;
    }

    ctx.drawImage(baseC, 0, 0);

    // Trace bloom.
    ctx.save();
    ctx.globalCompositeOperation = 'lighter';
    ctx.globalAlpha = 0.10 + 0.38 * S.glow;
    ctx.drawImage(glowC, 0, 0);
    ctx.restore();

    // pressure waves: the glow layer masked to a ring, so ONLY traces light up.
    if (S.waves.length) {
      const sc = scratchC.getContext('2d');
      for (const w of S.waves) {
        sc.setTransform(1, 0, 0, 1, 0, 0);
        sc.clearRect(0, 0, SW, SH);
        sc.drawImage(glowC, 0, 0, SW, SH);
        sc.globalCompositeOperation = 'destination-in';
        const cxs = chip.x * DPR * 0.5, cys = chip.y * DPR * 0.5;
        const rs = w.r * DPR * 0.5, band = Math.max(9, rs * 0.15);
        const g = sc.createRadialGradient(cxs, cys, Math.max(0, rs - band), cxs, cys, rs + band * 0.5);
        g.addColorStop(0, 'rgba(0,0,0,0)');
        g.addColorStop(0.5, `rgba(0,0,0,${w.a})`);
        g.addColorStop(1, 'rgba(0,0,0,0)');
        sc.fillStyle = g; sc.fillRect(0, 0, SW, SH);
        sc.globalCompositeOperation = 'source-over';
        ctx.save();
        ctx.globalCompositeOperation = 'lighter';
        ctx.globalAlpha = 0.85 * w.a;
        ctx.setTransform(1, 0, 0, 1, 0, 0);
        ctx.drawImage(scratchC, 0, 0, canvas.width, canvas.height);
        ctx.restore();
      }
    }

    ctx.save();
    ctx.setTransform(DPR, 0, 0, DPR, 0, 0);   // setWorldTf(), CAM===null path
    ctx.globalCompositeOperation = 'lighter';

    // packets
    ctx.lineCap = 'round';
    for (const p of S.packets) {
      const net = nets[p.n];
      const a = pointAt(net, p.s);
      const tailS = p.s - p.flow * Math.min(26, net.len * 0.08);
      const bpt = pointAt(net, Math.max(0, Math.min(net.len, tailS)));
      const col = S.alert > 0.5 ? PAL.alert : p.hue;
      const bright = 0.35 + 0.65 * S.glow;
      ctx.strokeStyle = rgba(col, 0.55 * bright);
      ctx.lineWidth = 2.2;
      ctx.beginPath(); ctx.moveTo(bpt.x, bpt.y); ctx.lineTo(a.x, a.y); ctx.stroke();
      ctx.fillStyle = rgba(col, 0.9 * bright);
      ctx.beginPath(); ctx.arc(a.x, a.y, 2.1, 0, TAU); ctx.fill();
    }

    // via and package arrivals
    for (let i = 0; i < viaGlow.length; i++) {
      const g = viaGlow[i];
      if (g <= 0.02) continue;
      const v = vias[i];
      const r = v.r + 7;
      const rg = ctx.createRadialGradient(v.x, v.y, 0, v.x, v.y, r);
      rg.addColorStop(0, rgba(PAL.traceLitRGB, 0.62 * g));
      rg.addColorStop(1, rgba(PAL.traceLitRGB, 0));
      ctx.fillStyle = rg;
      ctx.beginPath(); ctx.arc(v.x, v.y, r, 0, TAU); ctx.fill();
    }
    for (let i = 0; i < compGlow.length; i++) {
      const g = compGlow[i];
      if (g <= 0.02) continue;
      const cp = comps[i];
      const r = Math.max(cp.w, cp.h) * 0.9 + 12;
      const rg = ctx.createRadialGradient(cp.x, cp.y, 0, cp.x, cp.y, r);
      rg.addColorStop(0, rgba(PAL.traceLitRGB, 0.42 * g));
      rg.addColorStop(1, rgba(PAL.traceLitRGB, 0));
      ctx.fillStyle = rg;
      ctx.beginPath(); ctx.arc(cp.x, cp.y, r, 0, TAU); ctx.fill();
      ctx.strokeStyle = rgba(PAL.traceLitRGB, 0.80 * g);
      ctx.lineWidth = 1.4;
      ctx.strokeRect(cp.x - cp.w / 2, cp.y - cp.h / 2, cp.w, cp.h);
    }

    // pin arrivals
    for (let i = 0; i < nets.length; i++) {
      const g = pinGlow[i];
      if (g <= 0.01) continue;
      const p = nets[i].pin;
      const col = S.alert > 0.5 ? PAL.alert : PAL.inbound;
      const rg = ctx.createRadialGradient(p.x, p.y, 0, p.x, p.y, 16);
      rg.addColorStop(0, rgba(col, 0.75 * g));
      rg.addColorStop(1, rgba(col, 0));
      ctx.fillStyle = rg;
      ctx.beginPath(); ctx.arc(p.x, p.y, 16, 0, TAU); ctx.fill();
    }

    if (hasChip) { drawDie(); drawEmblem(); drawIndicators(); }

    ctx.restore();
    drawAlert();
  }

  function ring(cx, cy, rad, from, to, width, col, alpha) {
    ctx.beginPath();
    ctx.arc(cx, cy, rad, from, to);
    ctx.strokeStyle = rgba(col, alpha);
    ctx.lineWidth = width;
    ctx.stroke();
  }

  // The glyphs the name scrambles through while thinking.
  const SCRAMBLE = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789#%&@$*+=<>/';

  /* SEAM — wordmarkFor() replaces the real board's own hardcoded "J.A.R.V.I.S"
     with THIS assistant's real name. Same convention the real string already
     demonstrates (dot-separated letters) for short names; every other length
     renders IN FULL now — drawEmblem()'s own fit-to-width pass below is what
     keeps a long name from overrunning the ring, by shrinking the font, not
     by cutting the word down. THIS USED TO CUT AT 12 CHARACTERS regardless of
     whether the ring could actually hold the word shrunk smaller — a single-
     word name over that length collapsed to ONE bare letter (Mark, 2026-09-24,
     filming on VM 106: the core showed "G" instead of the theme's actual
     name). Bare initials are still the true last resort — see initialsFor()
     and the fallback inside drawEmblem — but now only when even the
     legibility floor can't fit the word, decided by measuring it, never by
     guessing from a character count. */
  function wordmarkFor(name) {
    if (!name) return '';
    const upper = name.toUpperCase();
    return upper.length <= 7 ? upper.split('').join('.') : upper;
  }

  // The bare-initials fallback, applied only when drawEmblem's own fit pass
  // finds even the smallest legible size still spills the word past the
  // ring. Same shape wordmarkFor used to hand back on a length guess; now
  // reached only by an actual measurement.
  function initialsFor(upper) {
    return upper.split(/[\s-]+/).filter(Boolean).map((w) => w[0]).join('') || upper.slice(0, 3);
  }

  function drawEmblem() {
    const { x, y, r } = chip;
    const lvl = S.target === 'speaking' ? S.level : 0;
    const heat = 0.34 + 0.40 * S.glow + 0.50 * lvl + (emblemHover ? 0.42 : 0);
    const spin = S.chase * (0.05 + 0.09 * S.motion);
    const ink = S.alert > 0.5 ? PAL.alert : PAL.inbound;

    ctx.save();
    ctx.lineCap = 'butt';
    const W1 = Math.max(1, r * 0.010), W2 = Math.max(1, r * 0.016);

    ring(x, y, r * 0.985, spin * 0.20 + 0.10, spin * 0.20 + Math.PI - 0.06, W1, ink, 0.34 + 0.26 * heat);
    ring(x, y, r * 0.985, spin * 0.20 + Math.PI + 0.10, spin * 0.20 + TAU - 0.06, W1, ink, 0.34 + 0.26 * heat);
    ring(x, y, r * 0.905, spin * -0.30 - 0.55, spin * -0.30 + 0.55, W2, ink, 0.55 + 0.35 * heat);
    ring(x, y, r * 0.905, spin * -0.30 + Math.PI - 0.40, spin * -0.30 + Math.PI + 0.40, W2, ink, 0.45 + 0.35 * heat);

    ctx.lineWidth = Math.max(1, r * 0.007);
    for (let i = 0; i < 34; i++) {
      const a = spin * 0.45 + Math.PI * 0.72 + (i / 34) * Math.PI * 0.56;
      ctx.strokeStyle = rgba(ink, 0.30 + 0.34 * heat);
      ctx.beginPath();
      ctx.moveTo(x + Math.cos(a) * r * 0.80, y + Math.sin(a) * r * 0.80);
      ctx.lineTo(x + Math.cos(a) * r * 0.865, y + Math.sin(a) * r * 0.865);
      ctx.stroke();
    }
    for (let i = 0; i < 26; i++) {
      const a = spin * -0.6 - Math.PI * 0.30 + (i / 26) * Math.PI * 1.15;
      const rr2 = r * 0.735;
      const sz = r * (i % 4 === 0 ? 0.030 : 0.019);
      ctx.save();
      ctx.translate(x + Math.cos(a) * rr2, y + Math.sin(a) * rr2);
      ctx.rotate(a);
      ctx.fillStyle = rgba(ink, 0.32 + 0.40 * heat);
      ctx.fillRect(-sz / 2, -sz / 2, sz, sz);
      ctx.restore();
    }
    for (let i = 0; i < 44; i++) {
      const a = spin * 0.75 + (i / 44) * TAU;
      const rr2 = r * 0.625;
      ctx.fillStyle = rgba(ink, 0.22 + 0.34 * heat);
      ctx.beginPath();
      ctx.arc(x + Math.cos(a) * rr2, y + Math.sin(a) * rr2, Math.max(0.8, r * 0.008), 0, TAU);
      ctx.fill();
    }
    for (let i = 0; i < 3; i++) {
      const a0 = spin * -0.22 + i * (TAU / 3) + 0.16;
      ring(x, y, r * 0.545, a0, a0 + TAU / 3 - 0.32, W1, ink, 0.30 + 0.28 * heat);
    }

    // THE AMBER ARCS — gold is the board's existing outbound colour; no new
    // hue introduced. They travel, at different radii and opposite rates.
    const gold = PAL.outbound;
    const ga = spin * 0.9;
    ring(x, y, r * 0.815, ga, ga + Math.PI * 0.52, Math.max(1, r * 0.017), gold, 0.50 + 0.35 * heat);
    ring(x, y, r * 0.815, ga + Math.PI * 0.72, ga + Math.PI * 0.86, Math.max(1, r * 0.017), gold, 0.34 + 0.30 * heat);
    const gb = spin * -0.65 + Math.PI * 0.4;
    ring(x, y, r * 0.685, gb, gb + Math.PI * 0.34, Math.max(1, r * 0.013), gold, 0.42 + 0.34 * heat);
    for (let i = 0; i < 3; i++) {
      const a = ga + Math.PI * 0.10 + i * Math.PI * 0.16;
      ctx.fillStyle = rgba(gold, 0.60 + 0.40 * heat);
      ctx.beginPath();
      ctx.arc(x + Math.cos(a) * r * 0.815, y + Math.sin(a) * r * 0.815, Math.max(1.2, r * 0.014), 0, TAU);
      ctx.fill();
    }

    // The hover ring.
    if (emblemHover) {
      ring(x, y, r * 0.955, 0, TAU, Math.max(1.2, r * 0.016), ink, 0.75);
    }

    // THE BRIGHT INNER CIRCLE — the one crisp, fully closed line.
    ring(x, y, r * 0.455, 0, TAU, Math.max(1, r * 0.013), ink, 0.72 + 0.28 * heat);

    // The face and the name.
    ctx.fillStyle = 'rgba(3,9,16,' + (0.36 + 0.34 * heat).toFixed(3) + ')';
    ctx.beginPath(); ctx.arc(x, y, r * 0.448, 0, TAU); ctx.fill();

    // THE NAME SCRAMBLES WHILE HE IS WAITING. The dots do not move; only the
    // letters are replaced, so the shape stays legible as "the name being
    // worked on" rather than as noise. Steps at 12/sec, not 60.
    const rawName = window.NameOSAssistantName ? window.NameOSAssistantName() : '';
    const NAME = wordmarkFor(rawName);
    let label = NAME;
    if (NAME && S.target === 'thinking') {
      const step2 = Math.floor(S.t * 12);
      let out = '';
      for (let i = 0; i < NAME.length; i++) {
        const ch = NAME[i];
        if (ch === '.') { out += '.'; continue; }
        const n = (step2 * 31 + i * 17) % SCRAMBLE.length;
        out += SCRAMBLE[n];
      }
      label = out;
    }

    if (label) {
      // Measured and fitted, not sized by a fraction and hoped for — never
      // grows, only shrinks to fit, so the scramble's widest glyph run
      // cannot spill past the ring.
      const fitW = r * 0.74;
      // THE LEGIBILITY FLOOR, not an arbitrary clamp — scales with the ring
      // like every other size in this scene, so a small window's tiny chip
      // and a large window's big one both stop shrinking at a proportionally
      // readable size rather than a fixed pixel count that meant nothing at
      // either extreme.
      const minFS = Math.max(6, r * 0.045);
      let fs = Math.max(minFS, r * 0.135);
      ctx.font = `500 ${fs}px ${SILK_FACE}`;
      let measured = ctx.measureText(label).width;
      if (measured > fitW) {
        fs = Math.max(minFS, fs * (fitW / measured));
        ctx.font = `500 ${fs}px ${SILK_FACE}`;
        measured = ctx.measureText(label).width;
      }
      // LAST RESORT — a word so long even the floor above still spills past
      // the ring falls back to bare initials rather than clipping or
      // overflowing. Off the raw name, not the (possibly scrambled) label,
      // so a mid-scramble frame can never flicker in and out of this.
      if (measured > fitW && fs <= minFS + 0.01) {
        label = initialsFor(rawName.toUpperCase());
        ctx.font = `500 ${fs}px ${SILK_FACE}`;
        measured = ctx.measureText(label).width;
        if (measured > fitW) fs = Math.max(4, fs * (fitW / measured));
        ctx.font = `500 ${fs}px ${SILK_FACE}`;
      }
      ctx.textAlign = 'center';
      ctx.textBaseline = 'middle';
      ctx.fillStyle = rgba([226, 244, 255], 0.78 + 0.22 * heat);
      ctx.fillText(label, x, y);
    }

    // TALKING PUTS A TIGHT RING/LIFT INSIDE THE FACE, riding the live level.
    if (S.target === 'speaking') {
      const pulse = 0.35 + 0.65 * lvl;
      ctx.save();
      ctx.beginPath(); ctx.arc(x, y, r * 0.448, 0, TAU); ctx.clip();
      const lift = ctx.createRadialGradient(x, y, 0, x, y, r * 0.448);
      lift.addColorStop(0, rgba(PAL.outbound, 0.28 * pulse));
      lift.addColorStop(1, rgba(PAL.outbound, 0));
      ctx.fillStyle = lift;
      ctx.beginPath(); ctx.arc(x, y, r * 0.448, 0, TAU); ctx.fill();
      ctx.restore();
      ctx.strokeStyle = rgba(PAL.outbound, 0.30 + 0.55 * pulse);
      ctx.lineWidth = Math.max(1.2, r * 0.014 * (0.6 + pulse));
      ctx.beginPath(); ctx.arc(x, y, r * 0.425, 0, TAU); ctx.stroke();
    }

    ctx.restore();
  }

  function drawDie() {
    const { x, y, r } = chip;
    const dieR = r * 0.44;
    const lvl = S.target === 'speaking' ? S.level : 0;
    // Idle must read as banked, not lit.
    const bloom = 0.04 + 0.20 * S.glow + 0.95 * lvl;
    const col = S.alert > 0.5 ? PAL.alert : PAL.outbound;

    // The bloom is held inside the inner circle — its reach is fixed at
    // 0.44r, just inside the bright ring at 0.455r; only its brightness rides
    // the level.
    const CORE = r * 0.44;
    const g = ctx.createRadialGradient(x, y, 0, x, y, CORE);
    g.addColorStop(0, rgba(col, Math.min(1, bloom)));
    g.addColorStop(0.55, rgba(col, Math.min(1, bloom * 0.42)));
    g.addColorStop(1, rgba(col, 0));
    ctx.fillStyle = g;
    ctx.beginPath(); ctx.arc(x, y, CORE, 0, TAU); ctx.fill();

    ctx.save();
    ctx.beginPath(); ctx.arc(x, y, dieR, 0, TAU); ctx.clip();
    ctx.fillStyle = rgba(col, 0.03 + 0.60 * lvl + 0.09 * S.glow);
    ctx.fillRect(x - dieR, y - dieR, dieR * 2, dieR * 2);
    ctx.restore();

    // idle heartbeat: a slow breath so it never looks switched off.
    if (S.target === 'idle' && S.alert < 0.3) {
      const beat = 0.5 + 0.5 * Math.sin(S.t * TAU / 6);
      ctx.fillStyle = rgba(PAL.outbound, 0.012 + 0.030 * beat);
      ctx.beginPath(); ctx.arc(x, y, dieR, 0, TAU); ctx.fill();
    }
  }

  function drawIndicators() {
    const { x, y, r } = chip;
    const n = 12, rad = r * 1.32;
    for (let i = 0; i < n; i++) {
      const a = (i / n) * TAU - Math.PI / 2;
      const px = x + Math.cos(a) * rad, py = y + Math.sin(a) * rad;
      let lit = 0.10 + 0.10 * S.glow;
      if (S.motion > 0.6) {
        const head = S.chase % n;
        const d = Math.min(Math.abs(i - head), n - Math.abs(i - head));
        lit = Math.max(lit, 0.95 * Math.exp(-d * 0.9) * Math.min(1, S.motion));
      }
      const col = S.alert > 0.5 ? PAL.alert : (S.flow < 0 ? PAL.inbound : PAL.outbound);
      ctx.fillStyle = rgba(col, lit);
      ctx.beginPath(); ctx.arc(px, py, 2.6, 0, TAU); ctx.fill();
    }
  }

  function drawAlert() {
    if (S.alert <= 0.01) return;
    ctx.save();
    ctx.setTransform(DPR, 0, 0, DPR, 0, 0);
    const flick = 0.78 + 0.22 * Math.sin(S.t * 19);
    const a = S.alert * flick;
    // Crush green and blue so the traces themselves turn red.
    ctx.globalCompositeOperation = 'multiply';
    ctx.fillStyle = `rgb(255,${Math.round(255 * (1 - 0.82 * a))},${Math.round(255 * (1 - 0.88 * a))})`;
    ctx.fillRect(0, 0, W, H);
    ctx.globalCompositeOperation = 'lighter';
    const g = ctx.createRadialGradient(W / 2, H / 2, Math.min(W, H) * 0.14,
                                       W / 2, H / 2, Math.hypot(W, H) * 0.60);
    g.addColorStop(0, rgba(PAL.alert, 0));
    g.addColorStop(0.55, rgba(PAL.alert, 0.10 * a));
    g.addColorStop(1, rgba(PAL.alert, 0.60 * a));
    ctx.fillStyle = g; ctx.fillRect(0, 0, W, H);
    ctx.fillStyle = rgba(PAL.alert, 0.10 * a);
    ctx.fillRect(0, 0, W, H);
    ctx.restore();
  }

  /* HOVER ON THE EMBLEM — the cursor changing as well as the light is the
     convention people already know for "this does something". Ported as
     hover-only: the real board's click handler toggles privacy mode, which
     this app has no equivalent of (see file header). */
  let emblemHover = false;
  function onPointerMove(e) {
    const over = Math.hypot(e.clientX - chip.x - canvas.getBoundingClientRect().left,
                             e.clientY - chip.y - canvas.getBoundingClientRect().top) <= chip.r;
    if (over === emblemHover) return;
    emblemHover = over;
    try { canvas.style.cursor = over ? 'pointer' : ''; } catch (e2) {}
  }
  function onPointerLeave() {
    emblemHover = false;
    try { canvas.style.cursor = ''; } catch (e2) {}
  }
  window.addEventListener('pointermove', onPointerMove);
  window.addEventListener('pointerleave', onPointerLeave);

  /* --- resize ----------------------------------------------------------------
     SEAM: window.innerWidth/innerHeight -> canvas.getBoundingClientRect().
     The real board owns the whole viewport; this canvas shares the screen
     with the app's own chrome, same as every other scene here. */
  function resize() {
    DPR = Math.min(window.devicePixelRatio || 1, CFG.dprCap);
    const rect = canvas.getBoundingClientRect();
    W = Math.max(1, Math.round(rect.width));
    H = Math.max(1, Math.round(rect.height));
    canvas.width = Math.floor(W * DPR);
    canvas.height = Math.floor(H * DPR);
    buildBoard();
    // Packets refer to the previous layout; a compact layout has no nets.
    S.packets = [];
    S.waves = [];
    bake();
    render();
  }

  let raf = null;
  let running = true;
  let last = performance.now();
  function loop(now) {
    if (!running) return;
    const dt = Math.min(0.05, (now - last) / 1000); last = now;
    step(dt); render();
    raf = requestAnimationFrame(loop);
  }
  /* PAUSE WHEN HIDDEN, NOT WHEN UNFOCUSED — reported 2026-08-28: "it freezes
     if you click off the screen. It should not do this as a person may have
     multiple screens and run this in another screen."
     This listened for `blur`, which fires the moment the window stops being the
     ACTIVE one — including when it is sitting fully visible on a second monitor
     while you work on the first. So the board froze exactly where somebody would
     most want to watch it, which is the whole reason to put it on that monitor.
     `document.hidden` is the honest test: it is true when the window is
     minimised or its tab is genuinely not being shown, and false when the window
     is simply not focused. Blur is about attention; hidden is about visibility,
     and only visibility should stop us drawing.
     The cost control this was is still here and still real — a minimised window
     stops animating. It just no longer confuses "you are looking elsewhere" with
     "you cannot see this". */
  /* AND PAUSE OFF HOME, NOT JUST WHEN HIDDEN -- switching to Settings or Chat
     never sets document.hidden (the window stays visible and focused) but
     index.html's own CSS hides `.brain` entirely off Home (`body[data-view]:not(
     [data-view="home"]) .brain { display: none !important; }`). This board
     kept stepping and rendering into that hidden canvas the whole time
     anyone sat on another screen. `document.body.dataset.view` is the same
     public attribute that CSS rule reads, so a MutationObserver on it drives
     the identical onVis() path below with no new hook needed from anywhere. */
  function homeActive() {
    const v = document.body && document.body.dataset && document.body.dataset.view;
    return !v || v === 'home';
  }
  function onVis() {
    if (document.hidden || !homeActive()) { running = false; return; }
    if (!running && !reduceMotion) {
      running = true;
      // Reset the clock or dt is the whole hidden duration and the first frame
      // after returning lurches. Clamped in loop() too, but not by this much.
      last = performance.now();
      raf = requestAnimationFrame(loop);
    }
  }
  document.addEventListener('visibilitychange', onVis);
  let viewObserver = null;
  if (typeof MutationObserver !== 'undefined' && document.body) {
    viewObserver = new MutationObserver(onVis);
    viewObserver.observe(document.body, { attributes: true, attributeFilter: ['data-view'] });
  }

  resize();
  if (!reduceMotion) raf = requestAnimationFrame(loop);
  // A mount can happen while Home is already NOT the active view (a theme
  // switch made from Settings, say) -- onVis() otherwise only runs on the
  // NEXT visibilitychange or data-view mutation, so without this call a
  // scene mounted off-Home would run unpaused until something changes again,
  // possibly never. Runs after the line above on purpose: it can only ever
  // narrow `running` back to false, never race the initial schedule.
  onVis();
  /* prefers-reduced-motion: drawn once, static. No packets, no scramble, no
     boot sweep, no pressure waves — state changes below still call render()
     directly so the board is never stale, only ever still. */

  function setTarget(name) {
    const next = STATES[name] ? name : 'idle';
    if (next !== S.target) S.target = next;
    if (reduceMotion) render();
  }

  return {
    onResize() { resize(); },
    destroy() {
      running = false;
      if (raf) cancelAnimationFrame(raf);
      document.removeEventListener('visibilitychange', onVis);
      if (viewObserver) viewObserver.disconnect();
      window.removeEventListener('pointermove', onPointerMove);
      window.removeEventListener('pointerleave', onPointerLeave);
      if (calmMQ && calmMQ.removeEventListener) calmMQ.removeEventListener('change', onCalmChange);
      else if (calmMQ && calmMQ.removeListener) calmMQ.removeListener(onCalmChange);
    },
    setThinking(on) { if (S.target !== 'speaking') setTarget(on ? 'thinking' : 'idle'); },
    setSpeaking(on) { setTarget(on ? 'speaking' : 'idle'); if (!on) S.levelRaw = 0; },
    /* THE ONE ADDED METHOD, same contract every scene exposes. Never
       overrides speaking/thinking — the mic can be "armed" while a reply is
       still being spoken and that must not visually interrupt it. */
    setListening(on) {
      if (on) { if (S.target !== 'speaking' && S.target !== 'thinking') setTarget('listening'); }
      else if (S.target === 'listening') setTarget('idle');
    },
    bump(v) { S.levelRaw = Math.min(1, Math.max(S.levelRaw * 0.55, v)); if (reduceMotion) render(); },
    pulse() { S.levelRaw = Math.min(1, S.levelRaw + 0.3); if (reduceMotion) render(); },
    flare(ok) {
      if (ok === false) { S.alertTarget = 1; S.alertHold = 1.2; }
      else S.levelRaw = Math.min(1, S.levelRaw + 0.4);
      if (reduceMotion) render();
    },
  };
}
