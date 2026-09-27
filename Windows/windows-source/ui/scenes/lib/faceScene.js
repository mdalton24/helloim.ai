// faceScene.js — VISUALIZER SCENE CONTRACT, ES-module port for this app.
//
// SECOND-HOP PORT. helloim-v4's src/lib/scene.ts (Vite/TS, hand-written
// Canvas 2D) was already ported once, value-for-value, into
// ~/helloim-app/renderer/lib/scene.js as an IIFE hanging its exports off
// `window.HelloimSceneKit` — that app's CSP is `script-src 'self'` with
// classic (non-module) <script> tags, so it had no `import`/`export` to
// use. THIS app (~/Documents/JARVIS/desktop) is ES modules throughout —
// scene-mount.js, scenes/board.js and scenes/aurelia.js all use
// import/export already — so this port undoes only the module-system
// wrapper: every function, constant and easing value below is unchanged
// from helloim-app's renderer/lib/scene.js, which is itself unchanged from
// helloim-v4's source. The IIFE's own double-execution hazard (a classic
// <script> observed running twice under a CDP debugger, corrupting a
// top-level `const`) does not apply to a `type="module"` script — the spec
// guarantees a module evaluates once per specifier — so that wrapper is
// dropped rather than carried forward as dead ceremony.
//
// A scene is a module exposing `mount(canvas) => SceneHandle` — the exact
// contract scene-mount.js already drives board.js and aurelia.js through:
// { onResize, destroy, setThinking, setSpeaking, setListening, bump, pulse,
// flare }. The host (index.html / scene-mount.js) sizes the canvas, forwards
// real assistant state through the handle — bump(v) already carries the
// app's real audio/TTS level, not a synthetic one — and calls destroy()
// before discarding the element. Nothing about the SIGNAL changes in this
// port: createScene() below is the same level state machine every face
// scene shares, fed by whatever the app's existing audio pipeline already
// calls on window.RiftBrain.
//
// Rules enforced here so every scene inherits them for free:
//   - mount() never throws. No 2D context -> a handle of no-ops.
//   - bump() LIFTS, never cuts:  level = min(1, max(level * 0.55, v))
//   - level rises toward the mode floor quickly, decays freely below it.
//   - prefers-reduced-motion holds a single frame, applied LIVE.
//   - document.hidden pauses the loop, visibility resumes it -- NOT window
//     blur/focus. See the PAUSE ON HIDDEN, NOT UNFOCUSED comment below;
//     this line used to read "window blur pauses the loop, focus resumes
//     it", which was the bug board.js was already fixed for on 2026-08-28
//     and which this file re-introduced for the nine faces ported through
//     it on 2026-09-02 -- nobody carried the earlier fix across the seam.
//
/**
 * Global playback speed for every visualizer. 1 = raw real time.
 * Lowered deliberately: the packs read as "alive" rather than "frantic".
 * Change this ONE number to retune the pace of all ten themes at once.
 */
const TIME_SCALE = 0.5;

/** Floors per mode. Thinking sits highest: sustained churn with no audio. */
const FLOORS = { idle: 0.0, listening: 0.06, talking: 0.12, thinking: 0.22 };

/** A handle whose every method is a no-op. Returned when rendering is dead. */
function nullHandle() {
  const noop = () => undefined;
  return {
    onResize: noop,
    destroy: noop,
    setThinking: noop,
    setSpeaking: noop,
    setListening: noop,
    bump: noop,
    pulse: noop,
    flare: noop,
    setPalette: noop,
  };
}

/**
 * Build a scene from a definition. Handles sizing, the RAF loop, the level
 * state machine, reduced motion and blur/focus pausing.
 *
 * `build(s)` is called once with the live SceneState and must return a
 * SceneDef: { layout?, draw, dispose?, onBump?, onFlare?, onModeChange? }.
 */
function createScene(canvas, build) {
  let ctx = null;
  try {
    ctx = canvas.getContext('2d', { alpha: false });
  } catch (e) {
    // THIS USED TO BE catch {} — Vega, 2026-09-05, auditing every scene for
    // the silent-catch pattern that hid why Bella's WebGL scene went black.
    // This is the shared 2D-context path for the OTHER ten ported faces
    // (bella.js, blitz, capcom, cyclops, jarvis, koan, noir, phreak,
    // retrowave, sizzle all mount through this one function) — a canvas that
    // already returned a different context type, or a browser genuinely
    // out of 2D contexts, threw here and every one of those themes went dark
    // with nothing in the console to say which.
    console.error('[faceScene] getContext(\'2d\') threw:', e);
    ctx = null;
  }
  if (!ctx) return nullHandle();

  const media =
    typeof window !== 'undefined' && window.matchMedia
      ? window.matchMedia('(prefers-reduced-motion: reduce)')
      : null;

  const s = {
    ctx,
    w: 1,
    h: 1,
    dpr: 1,
    t: 0,
    dt: 0,
    level: 0,
    mode: 'idle',
    listening: false,
    ping: 0,
    flareSign: 0,
    reduced: !!(media && media.matches),
    frame: 0,
  };

  const measure = () => {
    const rect = canvas.getBoundingClientRect();
    const dpr = Math.min(2, window.devicePixelRatio || 1);
    const cssW = Math.max(1, Math.round(rect.width || canvas.width || 1));
    const cssH = Math.max(1, Math.round(rect.height || canvas.height || 1));
    canvas.width = Math.round(cssW * dpr);
    canvas.height = Math.round(cssH * dpr);
    s.w = cssW;
    s.h = cssH;
    s.dpr = dpr;
  };

  measure();

  let def;
  try {
    def = build(s);
    if (def.layout) def.layout(s);
  } catch (e) {
    // THIS USED TO BE catch {} — the worst of the audit's finds, because
    // `build(s)` is the scene's OWN setup code (bella.js's layout(), or any
    // of the other nine faces') — a bug in a scene file itself, not an
    // environment failure, landed here and produced the exact same silent
    // black canvas as no WebGL being available at all. There was no way to
    // tell "this browser can't do it" from "this scene has a bug" before now.
    console.error('[faceScene] build()/layout() threw — scene will render nothing:', e);
    return nullHandle();
  }

  let raf = 0;
  let running = true;
  let last = performance.now();
  let heldFrame = false;
  let drawErrorLogged = false;

  const frameLoop = (now) => {
    raf = 0;
    if (!running) return;

    const raw = Math.min(0.05, Math.max(0.001, (now - last) / 1000));
    last = now;
    // Every scene reads time through this, so slowing the packs is a one-line change.
    const dt = raw * TIME_SCALE;

    // --- level state machine -------------------------------------------
    const floor = FLOORS[s.mode];
    s.level += ((floor > s.level ? floor : 0) - s.level) * 0.045;
    if (s.level < floor) s.level = floor;
    if (s.level < 0.0005) s.level = 0;
    s.ping *= 0.972;
    if (s.ping < 0.002) {
      s.ping = 0;
      s.flareSign = 0;
    }

    if (!s.reduced) {
      s.t += dt;
      s.dt = dt;
      s.frame += 1;
    } else {
      s.dt = 0;
    }

    if (!s.reduced || !heldFrame) {
      ctx.setTransform(s.dpr, 0, 0, s.dpr, 0, 0);
      try {
        def.draw(s);
      } catch (e) {
        // A scene that throws must not take the interface with it — that
        // part of the old comment was right and stays. What was missing is
        // any trace at all: this runs up to 60x/sec, so it logs ONCE (not
        // per-frame, which would flood the console and could itself cost a
        // frame) and then goes back to silently no-op'ing every frame after,
        // which is still better than a canvas that flickers between drawn
        // and blank while the interface keeps working.
        if (!drawErrorLogged) {
          drawErrorLogged = true;
          console.error('[faceScene] draw() threw — this scene will render nothing further:', e);
        }
      }
      heldFrame = true;
    }

    raf = requestAnimationFrame(frameLoop);
  };

  raf = requestAnimationFrame(frameLoop);

  const pause = () => {
    running = false;
    if (raf) cancelAnimationFrame(raf);
    raf = 0;
  };
  const resume = () => {
    if (running) return;
    running = true;
    last = performance.now();
    raf = requestAnimationFrame(frameLoop);
  };

  /* PAUSE ON HIDDEN, NOT UNFOCUSED -- same fault, same fix, as board.js's
     own onVis() (scenes/board.js, 2026-08-28: "it freezes if you click off
     the screen... a person may have multiple screens and run this in
     another screen"). This file used to also listen for `blur`, which
     fires the instant the WINDOW stops being the active one -- including
     while it sits fully visible on a second monitor, which is exactly the
     scenario board.js was corrected for. Chromium (and WebView2, which
     hosts this app on Windows) ties its real rAF throttling to
     `document.hidden` -- tab/window occlusion -- not to focus; a merely
     unfocused-but-visible window keeps painting at full rate on its own.
     So blur/focus were never buying a real cost saving here, only a wrong
     freeze. `document.hidden` is still checked, and still pauses -- a
     minimised window (or one WebView2 marks not-visible) correctly stops
     animating; only "you looked at another window" no longer does. */
  /* AND PAUSE WHEN HOME ITSELF ISN'T THE ACTIVE VIEW -- Vega, 2026-09-05,
     Mark's own report via Wren: leaving Home for another in-app screen
     (Settings, Chat...) never touches document.hidden -- the window stays
     visible and focused -- yet index.html's own CSS hides the whole scene:
     `body[data-view]:not([data-view="home"]) .brain { display: none
     !important; }`. Nothing above noticed the canvas underneath had gone
     display:none, so all ten faces this file drives kept running their
     full rAF loop into an element nobody could see. `document.body.dataset
     .view` is the SAME attribute that CSS rule reads -- public DOM state on
     `<body>`, not a private variable of index.html's own closures -- so
     this needs no new signal from that file at all; it is already reachable
     from here. Treated exactly like document.hidden: a MutationObserver on
     that one attribute drives the same onVis() this file already had. */
  const homeActive = () => {
    const v = document.body && document.body.dataset && document.body.dataset.view;
    return !v || v === 'home';
  };
  const onVis = () => (document.hidden || !homeActive() ? pause() : resume());
  const onMotion = (e) => {
    s.reduced = e.matches;
    heldFrame = false;
  };

  document.addEventListener('visibilitychange', onVis);
  if (media && media.addEventListener) media.addEventListener('change', onMotion);
  let viewObserver = null;
  if (typeof MutationObserver !== 'undefined' && document.body) {
    viewObserver = new MutationObserver(onVis);
    viewObserver.observe(document.body, { attributes: true, attributeFilter: ['data-view'] });
  }
  // Home may not be the active view at mount time either (a theme switch
  // made while parked on Settings, say) -- run it once up front rather than
  // waiting for the first attribute change or visibility flip.
  onVis();

  const setMode = (mode) => {
    if (s.mode === mode) return;
    s.mode = mode;
    heldFrame = false;
    if (def.onModeChange) def.onModeChange(s, mode);
  };

  let speaking = false;
  let thinking = false;

  const resolveMode = () => {
    if (speaking) setMode('talking');
    else if (thinking) setMode('thinking');
    else if (s.listening) setMode('listening');
    else setMode('idle');
  };

  const handle = {
    onResize() {
      measure();
      heldFrame = false;
      try {
        if (def.layout) def.layout(s);
      } catch (e) {
        // Was silent. A layout() that throws on resize left the scene
        // stuck at its old geometry with nothing said about why.
        console.error('[faceScene] layout() threw on resize:', e);
      }
    },
    destroy() {
      pause();
      document.removeEventListener('visibilitychange', onVis);
      if (viewObserver) viewObserver.disconnect();
      if (media && media.removeEventListener) media.removeEventListener('change', onMotion);
      try {
        if (def.dispose) def.dispose();
      } catch (e) {
        // Was silent. Non-fatal by the time we're tearing down, but a
        // scene's own cleanup failing silently is still worth a trace.
        console.error('[faceScene] dispose() threw:', e);
      }
    },
    setThinking(on) {
      thinking = !!on;
      resolveMode(); // must not override an active speaking state
    },
    setSpeaking(on) {
      speaking = !!on;
      if (!on) s.level = 0;
      resolveMode();
    },
    setListening(on) {
      s.listening = !!on;
      resolveMode();
    },
    bump(v) {
      const amt = Math.max(0, Math.min(1, v || 0));
      // LIFTS, never cuts.
      s.level = Math.min(1, Math.max(s.level * 0.55, amt));
      heldFrame = false;
      if (def.onBump) def.onBump(s, amt);
    },
    pulse() {
      s.ping = Math.max(s.ping, 0.55);
      s.flareSign = s.flareSign || 1;
      heldFrame = false;
    },
    flare(ok) {
      s.ping = 1;
      s.flareSign = ok === false ? -1 : 1;
      heldFrame = false;
      if (def.onFlare) def.onFlare(s, ok !== false);
    },
  };
  /* THE APPEARANCE HOOK, FORWARDED — Mastermind round 95 (index.html's
   * dock, applyAppearance()) — AND ONLY ATTACHED WHEN THE SCENE ITSELF
   * IMPLEMENTS ONE. Vance, round 95 follow-up: the dock's Appearance menu
   * needs to know, honestly, whether the ACTIVE scene will actually
   * respond before it lets the button imply "this changes the picture" —
   * see index.html's paintDockAppearMenu() and scene-mount.js's own
   * RiftBrain.supportsPalette(). The first version of this method always
   * existed on the handle (a no-op when `def.setPalette` was absent), which
   * is the right shape for bump()/pulse()/flare() — nothing outside this
   * file ever needs to ask "would flare() do anything?" — but it is the
   * WRONG shape here, because something outside this file now does need to
   * ask exactly that. Conditionally attaching the property is what makes
   * `'setPalette' in activeHandle` (or a bare truthiness check) an honest
   * answer instead of always true. Only noir.js and koan.js implement
   * `setPalette` on their own `def` so far — see each file's own comment
   * for why the rest were deliberately left alone. */
  if (def.setPalette) {
    handle.setPalette = function setPalette(colors) {
      try { def.setPalette(colors); } catch (e) {
        console.error('[faceScene] setPalette() threw:', e);
      }
    };
  }
  return handle;
}

/* ------------------------------------------------------------------ */
/* small shared drawing helpers used by more than one scene            */
/* ------------------------------------------------------------------ */

const rnd = (a, b) => a + Math.random() * (b - a);
const pick = (arr) => arr[(Math.random() * arr.length) | 0];
const clamp01 = (v) => (v < 0 ? 0 : v > 1 ? 1 : v);

/** Fill the whole frame with a flat colour. Scenes never leave it unpainted. */
function fillFrame(s, color) {
  s.ctx.fillStyle = color;
  s.ctx.fillRect(0, 0, s.w, s.h);
}

/** A soft radial glow — cheap stand-in for a bloom pass on 2D canvas. */
function glow(s, x, y, r, color, alpha) {
  if (alpha === undefined) alpha = 1;
  if (r <= 0) return;
  const g = s.ctx.createRadialGradient(x, y, 0, x, y, r);
  g.addColorStop(0, color);
  g.addColorStop(1, 'rgba(0,0,0,0)');
  s.ctx.save();
  s.ctx.globalAlpha = alpha;
  s.ctx.globalCompositeOperation = 'lighter';
  s.ctx.fillStyle = g;
  s.ctx.beginPath();
  s.ctx.arc(x, y, r, 0, Math.PI * 2);
  s.ctx.fill();
  s.ctx.restore();
}

export { TIME_SCALE, FLOORS, nullHandle, createScene, rnd, pick, clamp01, fillFrame, glow };
