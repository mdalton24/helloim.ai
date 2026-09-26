// SCENE-MOUNT — the theme-aware successor to aurelia-mount.js. Mounts
// whichever scene the current theme names on #brainCanvas and re-registers
// the SAME window.RiftBrain surface every existing call site already uses:
//   resize()  setThinking(on)  setSpeaking(on)  bump(v)  pulse()  flare(ok)
// plus ONE new optional method, setListening(on) — see its own comment in
// index.html at anchorArm()/anchorTake(). Nothing else about the bridge
// changed; this file is Iris's spec, build order step 4-5:
// ~/Documents/Iris/2026-08-28-nameos-themes.md.
//
// WHY A NEW CANVAS ON EVERY SWITCH, NOT THE SAME ONE. Bella's scene is
// WebGL; Jarvis's is 2D. A canvas that has already returned a WebGL
// rendering context returns null for '2d' on the same element (and vice
// versa) — that is a hard browser rule, not a bug to work around. So a
// theme switch replaces the <canvas id="brainCanvas"> element itself
// rather than reusing it; every existing lookup does document.getElementById
// AT THE MOMENT IT NEEDS THE CANVAS, never holds a stale reference, so this
// is safe.
import * as aurelia from './scenes/aurelia.js';
import * as board from './scenes/board.js';
// Only the six supported characters are routable, including direct scene
// switches. Retired source assets stay on disk; importing them here would
// keep a second, hidden roster alive after their picker entries were removed.
import * as capcom from './scenes/capcom.js';
import * as koan from './scenes/koan.js';
import * as noir from './scenes/noir.js';
import * as nova from './scenes/nova.js';

const SCENES = { aurelia, board, capcom, koan, noir, nova };

const brain = document.querySelector('.brain');
let activeHandle = null;
let activeCanvas = document.getElementById('brainCanvas');

// THIS USED TO GUESS -- Wren, 2026-09-05, tracking down "Bella's circle
// doesn't show on Home." `id === 'jarvis' ? 'board' : 'aurelia'` was a
// second, hand-maintained copy of the exact mapping index.html's own THEMES
// array already owns (`scene:` on each entry), and it drifted the moment
// Vega's eight ported faces got picker entries: 'nova' (scene 'bella' AT
// THE TIME -- Nova got her own scene id, 'nova', on 2026-09-07, matching the
// site's rebuild; this paragraph is left describing the bug as it stood on
// the date above), 'blitz', 'capcom', 'cyclops', 'koan', 'noir', 'phreak'
// and 'retrowave' all have their OWN scene id, and every one of them would
// have been mounted as
// 'aurelia' at first paint regardless -- silently wrong for 8 of the 10
// themes the instant one of them was the stored default, and RIGHT by pure
// coincidence for 'bella' and 'jarvis' only, which is exactly why nobody
// caught it: the two themes anyone tests first happen to be the two this
// function still gets right.
//
// READS `THEMES` DIRECTLY, NO SECOND COPY OF THE MAP -- confirmed this is
// actually reachable, not assumed: index.html's own `<script type="module"
// src="./scene-mount.js">` tag sits after the classic `<script>` that
// declares `const THEMES = [...]`, and a module's top-level scope chains up
// through the same global environment a classic script's top-level
// let/const bindings live in -- verified with a throwaway two-file page
// (one classic script declaring a const, one module reading it back) rather
// than assumed from how module scoping is usually described. So this reads
// the one array index.html already maintains; a theme that changes its
// scene there needs no matching edit here, ever again.
function sceneForTheme() {
  const id = document.documentElement.dataset.theme || 'jarvis';
  if (typeof THEMES !== 'undefined') {
    const theme = THEMES.find((t) => t.id === id);
    if (theme) return theme.scene;
  }
  // THEMES not loaded yet (or the id is stale/unknown) -- same fallback
  // shape as before, kept only as a last resort.
  return id === 'jarvis' ? 'board' : 'aurelia';
}

function mountScene(name, canvas) {
  const mod = SCENES[name] || SCENES.aurelia;
  let handle = null;
  try { handle = mod.mount(canvas); }
  catch (e) {
    // THIS USED TO BE catch (e) {} — same audit that found aurelia.js's own
    // silent catch (see that file's comment). This is the OUTER one: if a
    // scene module's mount() throws synchronously — not just aurelia's, any
    // of the eleven — it landed here with no trace either, one level above
    // the per-scene catches most of them already have. Degrading to the dark
    // surface is still correct; doing it mute is not.
    console.error(`[scene-mount] ${name}.mount() threw — falling back to the dark surface:`, e);
  }
  // NO setPalette ON THE DARK-SURFACE FALLBACK, ON PURPOSE — same reasoning
  // as faceScene.js's own conditional attach (see that file's comment):
  // nothing is actually drawn here, so "does this scene respond to an
  // Appearance change" should honestly read false, not a harmless no-op
  // that happens to look identical to "yes, but nothing changed".
  return handle || {
    onResize() {}, destroy() {}, setThinking() {}, setSpeaking() {},
    setListening() {}, bump() {}, pulse() {}, flare() {},
  };
}

activeHandle = mountScene(sceneForTheme(), activeCanvas);

/* IT MOUNTS BEHIND THE SIGN-IN SCREEN, AND THAT USED TO KILL IT SILENTLY —
 * carried over verbatim from aurelia-mount.js. #app is `hidden` until
 * signIn() runs, so at mount time the canvas measures 0x0; a
 * ResizeObserver on the canvas itself is what tells either scene to look
 * again the moment it is actually revealed. Board.js's own resize() re-reads
 * getBoundingClientRect() the same way aurelia's does, so this fix covers
 * both scenes for free. */
let ro = null;
function watchCanvas(canvas) {
  if (!('ResizeObserver' in window)) return;
  let lastW = 0, lastH = 0;
  ro = new ResizeObserver(() => {
    const r = canvas.getBoundingClientRect();
    const w = Math.round(r.width), h = Math.round(r.height);
    if (!w || !h || (w === lastW && h === lastH)) return;
    lastW = w; lastH = h;
    activeHandle && activeHandle.onResize && activeHandle.onResize();
  });
  ro.observe(canvas);
}
watchCanvas(activeCanvas);

const REDUCE_MOTION = window.matchMedia
  && window.matchMedia('(prefers-reduced-motion: reduce)').matches;

/* THE CROSSFADE — Iris's spec §2.3: tokens swap instantly (that is CSS,
 * already done by the moment this runs), the hero canvas crossfades over
 * 180ms: old scene opacity -> 0, unmount, mount new, opacity -> 1. Instant
 * under prefers-reduced-motion. Called by applyTheme() in index.html;
 * guarded there with `window.NameOSSwitchScene &&` like every other
 * cross-file call in this app. */
window.NameOSSwitchScene = function (sceneName) {
  const oldCanvas = activeCanvas;
  const oldHandle = activeHandle;
  // Off the id immediately -- for the ~180ms crossfade both canvases are in
  // the DOM at once, and nothing outside this file ever looks the element
  // up by id (everything goes through window.RiftBrain), so this just
  // avoids a transient duplicate id rather than fixing a real lookup bug.
  oldCanvas.removeAttribute('id');

  // THE BUG THIS GUARDS AGAINST -- Wren, 2026-09-06, Mark: "the home screen
  // also doesn't show the theme icon in the theme when chosen." Reproduced
  // by driving the real click path (Settings -> Themes -> pick a card) and
  // reading canvas state back over CDP: choosing a theme from ANY .iaView
  // runs applyTheme() -> here while `.brain`'s ancestor is `display:none`
  // (it's Home-only, see index.html's own comment above that rule). A CSS
  // transition never STARTS on an element that isn't rendered, so it never
  // fires transitionend either -- that is a browser rule, not a timing
  // fluke, and confirmed by measurement, not assumption: both canvases came
  // back stuck at `opacity: 0` after returning to Home, one still holding
  // the OLD scene's pixels and never removed, the other appended but never
  // handed to mountScene() at all because `finish` (which does that) was
  // waiting on an event that could not fire. Same failure a plain page
  // reload would not show, which is exactly why nobody caught it by eye.
  // Skipping straight to the REDUCE_MOTION path once the container is
  // provably not rendered is correct, not a hack -- animating a crossfade
  // nobody can see was already pointless even before it broke.
  const invisible = getComputedStyle(brain).display === 'none';
  const skipAnim = REDUCE_MOTION || invisible;

  // Position/size/display all come from the existing ".brain canvas" CSS
  // rule (it targets the tag, not the id) — only the crossfade needs an
  // inline style here.
  const newCanvas = document.createElement('canvas');
  newCanvas.id = 'brainCanvas';
  newCanvas.style.opacity = skipAnim ? '1' : '0';
  newCanvas.style.transition = skipAnim ? 'none' : 'opacity 180ms ease';
  brain.appendChild(newCanvas);

  const finish = () => {
    const newHandle = mountScene(sceneName, newCanvas);
    activeCanvas = newCanvas;
    activeHandle = newHandle;
    if (ro) ro.disconnect();
    watchCanvas(newCanvas);
    requestAnimationFrame(() => { newCanvas.style.opacity = '1'; });
    // A FRESH MOUNT STARTS ON THE SCENE'S OWN DEFAULT COLOURS -- an
    // Appearance override chosen before this switch would otherwise be
    // silently lost on the new canvas until the next click of the
    // Appearance button. Re-reads the live attribute rather than caching
    // the id anywhere in this module, so it is never stale. The four hexes
    // here are the same ones index.html's Appearance CSS block and its
    // APPEARANCES array both carry -- restated a third time rather than
    // imported, because this module has no reach into that classic
    // script's top-level consts (different script, same page) and four
    // short hex literals are cheaper to keep in sync by eye than a new
    // cross-file plumbing path would be to build and verify tonight.
    const APPEARANCE_HEX = { mint: '#52ffa5', ice: '#8fe6ff', amber: '#ffbe5a', coral: '#ff5a3c' };
    const appearanceId = document.documentElement.dataset.appearance;
    if (appearanceId && APPEARANCE_HEX[appearanceId] && newHandle.setPalette) {
      newHandle.setPalette({ accent: APPEARANCE_HEX[appearanceId] });
    }

    const finishSwap = () => {
      oldHandle && oldHandle.destroy && oldHandle.destroy();
      oldCanvas.remove();
    };
    if (skipAnim) finishSwap();
    else newCanvas.addEventListener('transitionend', finishSwap, { once: true });
  };

  if (skipAnim) { oldCanvas.style.opacity = '0'; finish(); }
  else {
    oldCanvas.style.transition = 'opacity 180ms ease';
    oldCanvas.style.opacity = '0';
    oldCanvas.addEventListener('transitionend', finish, { once: true });
  }
};

// The same bridge shape every existing call site already relies on, plus
// setListening — forwarded to whichever scene is mounted right now.
window.RiftBrain = {
  resize() { activeHandle && activeHandle.onResize && activeHandle.onResize(); },
  setThinking(on) { activeHandle && activeHandle.setThinking && activeHandle.setThinking(on); },
  setSpeaking(on) { activeHandle && activeHandle.setSpeaking && activeHandle.setSpeaking(on); },
  setListening(on) { activeHandle && activeHandle.setListening && activeHandle.setListening(on); },
  bump(v) { activeHandle && activeHandle.bump && activeHandle.bump(v); },
  pulse() { activeHandle && activeHandle.pulse && activeHandle.pulse(); },
  flare(ok) { activeHandle && activeHandle.flare && activeHandle.flare(ok); },
  /* THE APPEARANCE HOOK — Mastermind round 95, added for the dock's one-
   * click Appearance control (index.html: applyAppearance()). `colors` is
   * either `{ accent }` or null (null means "follow the skin", i.e. put the
   * scene's own default back — see below). OPTIONAL ON EVERY SCENE'S OWN
   * CONTRACT, same as setListening was when it was added: a scene whose
   * handle has no setPalette (most of them, today — see each scene file's
   * own note) simply does not respond, which is an honest "the chrome
   * recoloured, the canvas did not" rather than a thrown error. Only
   * noir.js and koan.js implement this so far; board.js and capcom.js were
   * deliberately left out because their own source comments say their
   * colours carry real meaning (signal direction, telemetry lanes) that a
   * cosmetic override must not delete — see this app's handback report. */
  setPalette(colors) { activeHandle && activeHandle.setPalette && activeHandle.setPalette(colors); },
  /* WILL AN APPEARANCE CHANGE ACTUALLY BE VISIBLE — Vance, round 95 review:
   * the Appearance button looks like it does almost nothing on a skin whose
   * canvas ignores it (Giles, Capcom), and a person has no way to tell "did
   * that work?" from "is this broken?". index.html's paintDockAppearMenu()
   * uses this to add one honest line for exactly that case. READS THE REAL
   * HANDLE, NOT A SEPARATE MANIFEST — faceScene.js only attaches
   * `setPalette` to a scene's own handle when that scene's `def` actually
   * implements one (see that file's own comment on why the method used to
   * always exist and now does not), so this can never drift from what
   * RiftBrain.setPalette() above would really do; there is nothing here to
   * keep in sync by hand as scenes gain real support one at a time. */
  supportsPalette() { return !!(activeHandle && activeHandle.setPalette); },
};
