// BELLA'S SCENE — a thin wrapper over ../aurelia/aurelia.js, giving it the
// same 7-method contract every scene now exposes to scene-mount.js:
//   mount(canvas) -> { onResize, destroy, setThinking, setSpeaking, bump, pulse, flare }
//
// Everything below this line USED to live directly in aurelia-mount.js.
// Moving it here — rather than rewriting it — is Iris's spec, build order
// step 4: "scenes/aurelia.js (a wrapper over what exists) — prove a
// Bella→Bella remount is invisible." The state object, the FLOOR easing and
// the tick loop are unchanged; only their home moved, so a Bella→Bella
// remount (switching Jarvis back to Bella) reruns the exact code path that
// already ran at first launch.
import { mountAurelia } from '../aurelia/aurelia.js';

const FLOOR = { talking: 0.12, thinking: 0.22, idle: 0 };

export function mount(canvas) {
  const state = { mode: 'idle', level: 0 };
  let handle = null;
  try { handle = mountAurelia(canvas, state, {}); }
  catch (e) {
    // THIS USED TO BE catch (e) {} — Vega, 2026-09-05, tracking down "Bella's
    // circle/sphere doesn't appear on Home." Bella is the theme whose THEMES
    // entry names `scene: 'aurelia'` (index.html), so THIS file is Bella's
    // actual visual, and this catch is the one place a WebGL failure here
    // (no context available, a lost context, a bad shader compile — anything
    // mountAurelia() or `new THREE.WebGLRenderer()` inside it can throw) went
    // to a dark canvas with zero trace. Degrading gracefully is still right —
    // a thrown scene must not take the app's chrome down with it — but
    // degrading SILENTLY is not: nothing before this line ever told anyone
    // *why* Bella went black, so every earlier report of it looked identical
    // to a scene that simply hadn't been asked to mount yet. Logged, never
    // swallowed, from here down.
    console.error('[scene:aurelia] mountAurelia() threw — Bella\'s scene will show the dark surface instead:', e);
  }

  let raf = null;
  let running = true;
  function tick() {
    if (!running) return;
    const floor = FLOOR[state.mode] || 0;
    state.level += ((floor > state.level ? floor : 0) - state.level) * 0.08;
    if (state.level < floor) state.level = floor;
    raf = requestAnimationFrame(tick);
  }
  raf = requestAnimationFrame(tick);

  // COST CONTROL, applied here too — Iris's spec §3.4: "apply the same
  // pause to Bella's scene while you are in there." This ONE ONLY pauses
  // the level easing.
  //
  // THIS COMMENT USED TO SAY THE UNDERLYING WEBGL RENDER "IS VENDORED CODE
  // THIS BUILD DID NOT OTHERWISE OPEN, SO IT JUST KEEPS PAINTING REGARDLESS."
  // That stopped being true on 2026-09-03, when ../aurelia/aurelia.js's own
  // frame() loop was opened to add its own document.hidden check — see that
  // file's comment. It was opened again on 2026-09-05 for the Home-view
  // check below. Two independent pause mechanisms now cover this scene:
  // this file's tick() (level easing only) and aurelia.js's own frame()
  // (the actual composer.render() call, by far the more expensive of the
  // two) — kept separate because they were written at different times for
  // different costs, not because either one alone is sufficient.
  //
  // IT USED TO PAUSE ON WINDOW BLUR, WHICH WAS THE WRONG TRIGGER — corrected
  // 2026-09-03, same fault and same fix as scenes/board.js's own onVis()
  // (2026-08-28: "it freezes if you click off the screen... a person may
  // have multiple screens and run this in another screen") and
  // scenes/lib/faceScene.js's createScene(), which had the identical blur
  // wiring until the same pass. `blur` fires the instant the window stops
  // being the ACTIVE one, including while sitting fully visible on a second
  // monitor — exactly backwards from what "pause when nobody can see it"
  // means. `document.hidden` is the honest test (true on minimise / genuine
  // occlusion, false on a merely unfocused-but-visible window), and it is
  // what Chromium's own real rAF throttling is tied to in both the browser
  // and WebView2 — so this now matches the platform, not just the intent.
  //
  // AND OFF HOME, NOT JUST HIDDEN — added 2026-09-05, same reasoning as
  // aurelia.js's own frame() check and board.js's onVis(): leaving Home
  // never sets document.hidden, but index.html's CSS hides `.brain`
  // entirely off Home. `document.body.dataset.view` is that same public
  // attribute.
  function homeActive() {
    const v = document.body && document.body.dataset && document.body.dataset.view;
    return !v || v === 'home';
  }
  function onVis() {
    if (document.hidden || !homeActive()) { running = false; return; }
    if (!running) { running = true; raf = requestAnimationFrame(tick); }
  }
  document.addEventListener('visibilitychange', onVis);
  let viewObserver = null;
  if (typeof MutationObserver !== 'undefined' && document.body) {
    viewObserver = new MutationObserver(onVis);
    viewObserver.observe(document.body, { attributes: true, attributeFilter: ['data-view'] });
  }
  // A mount can happen while Home is already not the active view -- onVis()
  // otherwise only runs on the next visibilitychange/data-view mutation, so
  // without this a scene mounted off-Home would tick unpaused until
  // something changes again, possibly never. Safe after the initial
  // requestAnimationFrame(tick) above: it can only narrow `running` to
  // false, never double-schedule.
  onVis();

  return {
    onResize() { handle && handle.onResize && handle.onResize(); },
    destroy() {
      running = false;
      if (raf) cancelAnimationFrame(raf);
      document.removeEventListener('visibilitychange', onVis);
      if (viewObserver) viewObserver.disconnect();
      // aurelia.js exposes no dispose() (see the build report) — the WebGL
      // context on this canvas is abandoned, not freed, when the canvas
      // itself is discarded by scene-mount.js on the next switch.
    },
    setThinking(on) { if (state.mode !== 'talking') state.mode = on ? 'thinking' : 'idle'; },
    setSpeaking(on) { state.mode = on ? 'talking' : 'idle'; if (!on) state.level = 0; },
    /* max(level*0.55, v) — the board's exact shape. A new word never CUTS a
     * louder one still ringing out; it only ever lifts. */
    bump(v) { state.level = Math.min(1, Math.max(state.level * 0.55, v)); },
    pulse() { state.level = Math.min(1, state.level + 0.3); },
    flare() { state.level = 1; },
  };
}
