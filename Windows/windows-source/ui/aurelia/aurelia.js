/* AURELIA — the presence on a signed-in person's page.
 *
 * Mark, 2026-08-21, with a full Three.js spec: "instead of having Jarvis on
 * user's login.. can we give them something like this ... that motions when
 * spoken too and name it a different name."
 *
 * THE NAME. Aurelia — from aurum, gold, and the same root as the aurora this
 * scene actually renders in its background pass. It is not "Jarvis": that name
 * is Mark's own assistant and, separately, is already shipped as a product by
 * somebody else, so it cannot be the customer-facing one. Aurelia is what a
 * user meets; Jarvis is what Mark works with.
 *
 * THE SCENE IS HIS SPEC, BUILT AS SPECIFIED. Same geometry (SphereGeometry
 * 4.2, 200, 600 as Points), the same two shaders verbatim, the same constants,
 * the same composer order — background pass, then sphere with clear:false and
 * clearDepth:true, then UnrealBloom at (1.64, 1.14, 0.04). Nothing is
 * "improved": a spec this exact is a spec somebody chose.
 *
 * WHAT IS ADDED, AND ONLY THIS: it reacts to being spoken to. His words —
 * "that motions when spoken too". The scene already has a flare built for a
 * cursor; a voice is just another thing that can drive it. So speaking lights
 * the sphere from the point facing the viewer, thinking makes it churn, and
 * idle leaves it breathing. No new uniforms were invented to do it — the ones
 * the flare already uses are driven from state instead of from a pointer.
 *
 * VENDORED, NOT FETCHED FROM A CDN. Three r143 and the four postprocessing
 * files live under ./vendor. The spec asked for an unpkg importmap; on a page
 * somebody signs in to, that makes a third party's uptime part of ours, and
 * puts a script we do not control on a page behind a credential. Same library,
 * same version, served from here.
 */

import * as THREE from "./vendor/three.module.js";
import { EffectComposer } from "./vendor/postprocessing/EffectComposer.js";
import { RenderPass } from "./vendor/postprocessing/RenderPass.js";
import { UnrealBloomPass } from "./vendor/postprocessing/UnrealBloomPass.js";

const CONFIG = {
  colorWarm: "#ff4c33",
  colorCool: "#3366ff",

  /* SHARPER, AND THE RING IS LIT RATHER THAN LIT UP. Mark, 2026-08-21:
   * "shopen it up so that the light is not off of the ring itself but the ring
   * interacts."
   *
   * At 1.64 / 1.14 / 0.04 the BLOOM PASS WAS DOING THE LIGHTING. A threshold of
   * 0.04 means very nearly every point qualifies as a highlight, so the whole
   * cloud smeared into its own halo and the top of the ring blew out to flat
   * white. That is the "light off of the ring itself" he is describing: the ring
   * was the lamp. Pulled down hard and the threshold raised, so bloom is now a
   * highlight on the genuinely brightest points and the point cloud stays
   * legible as a cloud of points. */
  bloomStrength: 0.58,
  bloomRadius: 0.40,
  bloomThreshold: 0.26,

  /* A QUARTER SPEED. Mark: "can you slow this down a bit.. about 25% of where
   * you have it." 1.41 x 0.25. ONE KNOB DOES ALL OF IT — the sphere noise, the
   * aurora behind it and the per-point flicker all read off this same clock, so
   * they stay in step with each other instead of drifting apart. */
  noiseSpeed: 0.3525,

  introSeconds: 2.4,
  cameraDistance: 10.8,
  cursorRadius: 2.0,
  cursorFlare: 1.4,
  cursorHeat: 1.0,

  /* THE LIGHT IS NOW A THING IN THE SCENE, WHICH IS THE OTHER HALF OF HIS NOTE.
   * Before this there was no light at all: every point emitted its own colour
   * and the flare drove that colour to near-white, so there was nothing for the
   * ring to interact WITH. Now there is a key light up and slightly toward the
   * viewer — the ring has a lit side and a dark side, and the flare is a
   * highlight falling on it rather than the ring switching itself on. */
  lightDir: [0.16, 0.86, 0.48],
  lightColor: "#ffd2a0",
};

function hexToVec3(hex) {
  const n = parseInt(hex.slice(1), 16);
  return new THREE.Vector3(((n >> 16) & 255) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255);
}

const BG_VERT = `
    varying vec2 vUv;
    void main() {
        vUv = uv;
        gl_Position = vec4(position, 1.0);
    }
`;

const BG_FRAG = `
    uniform float uTime;
    uniform float uScroll;
    uniform vec2 uResolution;
    uniform vec3 color1;
    uniform vec3 color2;
    varying vec2 vUv;

    // Morgan McGuire noise
    float hash(float n) { return fract(sin(n) * 1e4); }
    float hash(vec2 p) { return fract(1e4 * sin(17.0 * p.x + p.y * 0.1) * (0.1 + abs(sin(p.y * 13.0 + p.x)))); }
    float noise(vec2 x) {
        vec2 i = floor(x);
        vec2 f = fract(x);
        float a = hash(i);
        float b = hash(i + vec2(1.0, 0.0));
        float c = hash(i + vec2(0.0, 1.0));
        float d = hash(i + vec2(1.0, 1.0));
        vec2 u = f * f * (3.0 - 2.0 * f);
        return mix(a, b, u.x) + (c - a) * u.y * (1.0 - u.x) + (d - b) * u.x * u.y;
    }
    #define OCTAVES 4
    float fbm(vec2 x) {
        float v = 0.0;
        float a = 0.5;
        vec2 shift = vec2(100);
        mat2 rot = mat2(cos(0.5), sin(0.5), -sin(0.5), cos(0.50));
        for (int i = 0; i < OCTAVES; ++i) {
            v += a * noise(x);
            x = rot * x * 2.0 + shift;
            a *= 0.5;
        }
        return v;
    }

    void main() {
        vec2 st = vUv;
        st.x *= uResolution.x / uResolution.y;

        float t = uTime * 6.0 + uScroll * 150.0;

        vec2 st1 = st * 1.2;
        vec2 st2 = st * 1.6;

        float f1 = fbm(st1 + vec2(t * 0.15, t * 0.1));
        float mask1 = pow(fbm(st1 + f1 * 2.5 - vec2(t * 0.25, 0.0)), 2.0) * 3.5;

        float f2 = fbm(st2 - vec2(t * 0.1, t * 0.2));
        float mask2 = pow(fbm(st2 + f2 * 2.0 + vec2(0.0, t * 0.2)), 2.5) * 4.0;

        vec3 finalColor = color2 * mask1 + color1 * mask2;

        vec2 aspectUv = vUv - 0.5;
        aspectUv.x *= uResolution.x / uResolution.y;
        float centerDist = length(aspectUv);

        float angle = atan(aspectUv.y, aspectUv.x) + uScroll * 15.0;
        float maskOffset = sin(angle * 3.0 + t * 0.4) * 0.05
                         + sin(angle * 5.0 - t * 0.6) * 0.03;
        float dynamicDist = centerDist + maskOffset;

        float cornerFade = smoothstep(0.7, 1.15, dynamicDist);
        cornerFade = pow(cornerFade, 1.6);

        finalColor *= cornerFade * 0.9;

        vec3 baseBg = vec3(0.012, 0.012, 0.02);

        gl_FragColor = vec4(baseBg + finalColor, 1.0);
    }
`;

const P_VERT = `
    uniform float uTime;
    uniform float uScroll;
    uniform float uIntro;
    uniform vec3 uColorTop;
    uniform vec3 uColorBottom;
    uniform vec3 uCursor;
    uniform float uCursorStrength;
    uniform float uCursorRadius;
    uniform float uCursorFlare;
    uniform float uCursorHeat;
    uniform vec3 uLightDir;
    varying float vEdgeFade;
    varying float vHeat;
    varying float vLit;
    varying vec3 vColor;

    vec4 permute(vec4 x){return mod(((x*34.0)+1.0)*x, 289.0);}
    vec4 taylorInvSqrt(vec4 r){return 1.79284291400159 - 0.85373472095314 * r;}
    float snoise(vec3 v){
      const vec2 C = vec2(1.0/6.0, 1.0/3.0); const vec4 D = vec4(0.0, 0.5, 1.0, 2.0);
      vec3 i = floor(v + dot(v, C.yyy)); vec3 x0 = v - i + dot(i, C.xxx);
      vec3 g = step(x0.yzx, x0.xyz); vec3 l = 1.0 - g;
      vec3 i1 = min(g.xyz, l.zxy); vec3 i2 = max(g.xyz, l.zxy);
      vec3 x1 = x0 - i1 + 1.0 * C.xxx; vec3 x2 = x0 - i2 + 2.0 * C.xxx; vec3 x3 = x0 - 1.0 + 3.0 * C.xxx;
      i = mod(i, 289.0);
      vec4 p = permute(permute(permute(i.z + vec4(0.0, i1.z, i2.z, 1.0)) + i.y + vec4(0.0, i1.y, i2.y, 1.0)) + i.x + vec4(0.0, i1.x, i2.x, 1.0));
      float n_ = 1.0/7.0; vec3 ns = n_ * D.wyz - D.xzx;
      vec4 j = p - 49.0 * floor(p * ns.z *ns.z);
      vec4 x_ = floor(j * ns.z); vec4 y_ = floor(j - 7.0 * x_);
      vec4 x = x_ *ns.x + ns.yyyy; vec4 y = y_ *ns.x + ns.yyyy; vec4 h = 1.0 - abs(x) - abs(y);
      vec4 b0 = vec4(x.xy, y.xy); vec4 b1 = vec4(x.zw, y.zw);
      vec4 s0 = floor(b0)*2.0 + 1.0; vec4 s1 = floor(b1)*2.0 + 1.0; vec4 sh = -step(h, vec4(0.0));
      vec4 a0 = b0.xzyw + s0.xzyw*sh.xxyy; vec4 a1 = b1.xzyw + s1.xzyw*sh.zzww;
      vec3 p0 = vec3(a0.xy,h.x); vec3 p1 = vec3(a0.zw,h.y); vec3 p2 = vec3(a1.xy,h.z); vec3 p3 = vec3(a1.zw,h.w);
      vec4 norm = taylorInvSqrt(vec4(dot(p0,p0), dot(p1,p1), dot(p2, p2), dot(p3,p3)));
      p0 *= norm.x; p1 *= norm.y; p2 *= norm.z; p3 *= norm.w;
      vec4 m = max(0.5 - vec4(dot(x0,x0), dot(x1,x1), dot(x2,x2), dot(x3,x3)), 0.0); m = m * m;
      return 42.0 * dot(m*m, vec4(dot(p0,x0), dot(p1,x1), dot(p2,x2), dot(p3,x3)));
    }

    void main() {
        vec3 normalVec = normalize(position);

        float noiseVal = snoise(position * 0.5 + uTime * 0.8);
        noiseVal += 0.5 * snoise(position * 1.5 - uTime * 1.2);
        vec3 spherePos = position + normalVec * (noiseVal * 0.5);

        float cursorDist = distance(spherePos, uCursor);
        float flareFall = 1.0 - smoothstep(0.0, uCursorRadius, cursorDist);
        flareFall = pow(flareFall, 1.5);
        float flicker = 0.65 + 0.35 * snoise(position * 3.0 + uTime * 5.0);
        float flare = flareFall * uCursorStrength * flicker;
        spherePos += normalVec * (flare * uCursorFlare);
        vHeat = clamp(flare * uCursorHeat, 0.0, 1.0);

        vec3 finalPos = spherePos;

        vec4 mvPosition = modelViewMatrix * vec4(finalPos, 1.0);

        vec3 viewDir = normalize(-mvPosition.xyz);
        vec3 worldNormal = normalize(normalMatrix * normalVec);
        float rim = 1.0 - abs(dot(viewDir, worldNormal));

        float edgeFadeSphere = smoothstep(0.4, 0.9, rim);

        /* HOW MUCH OF THE KEY LIGHT THIS POINT ACTUALLY CATCHES.
         *
         * WRAPPED, NOT A STRAIGHT LAMBERT, and the difference matters here more
         * than it would on a solid surface. A hard terminator across a cloud of
         * loose points does not read as a shadow — it reads as half the ring
         * having been deleted. Wrapping keeps the dark side present and merely
         * unlit, which is what makes it look like an object in a room.
         *
         * uLightDir is in view space (normalMatrix took the normal there), which
         * is the right space for it: the camera does not orbit, so the light
         * stays put relative to the person looking at it. */
        float lambert = dot(worldNormal, normalize(uLightDir));
        vLit = pow(clamp((lambert + 0.42) / 1.42, 0.0, 1.0), 1.4);

        float opacityMultiplier = 0.8;
        float sizeMultiplier = 1.5;

        float introSphere = mix(1.0, edgeFadeSphere, uIntro);
        vEdgeFade = introSphere * opacityMultiplier;
        vEdgeFade *= smoothstep(0.0, 0.2, uIntro);

        /* The flare still lifts opacity, but half as hard as it did. At 0.7 the
         * lit region went fully opaque and stopped being a cloud. */
        vEdgeFade += vHeat * 0.38 * smoothstep(0.0, 0.2, uIntro);

        float baseColorMix = smoothstep(-3.0, 3.0, position.y + position.x * 0.5);
        vColor = mix(uColorBottom, uColorTop, clamp(baseColorMix, 0.0, 1.0));

        /* Points swell less under the flare too. Growing them 2.6x was the other
         * half of the blowout: big soft sprites overlapping is a smear whatever
         * the bloom pass is set to. */
        gl_PointSize = sizeMultiplier * (10.0 / -mvPosition.z) * (1.0 + vHeat * 0.85);
        gl_PointSize = max(gl_PointSize, 1.5);

        gl_Position = projectionMatrix * mvPosition;
    }
`;

const P_FRAG = `
    uniform vec3 uLightColor;
    varying float vEdgeFade;
    varying float vHeat;
    varying float vLit;
    varying vec3 vColor;

    void main() {
        vec2 xy = gl_PointCoord.xy - vec2(0.5);
        float ll = length(xy);
        if(ll > 0.5) discard;

        /* A SHARPER SPRITE. smoothstep(0.5, 0.1, ll) ramps across almost the
         * whole disc, so every point was a soft smudge and forty thousand of
         * them overlapping read as fog. A small solid core with a short shoulder
         * is what makes it look like a point cloud again. */
        float pointAlpha = smoothstep(0.5, 0.30, ll);

        /* THE RING'S OWN COLOUR, LIT — NOT EMITTING. This is the line Mark's
         * note is actually about. The old version mixed straight to
         * vec3(1.0, 0.96, 0.84) as the flare rose, so the bright part of the
         * ring went white and became the light source. Now the point keeps its
         * own colour and the light decides how much of it you see; the flare
         * adds a warm highlight ON it, capped well below white so it can be
         * bright without going blank. */
        vec3 lit = vColor * (0.30 + 0.70 * vLit);
        vec3 col = lit + uLightColor * vHeat * 0.52;

        gl_FragColor = vec4(col, vEdgeFade * pointAlpha * 0.9);
    }
`;

export function mountAurelia(canvas, state, opts) {
  const renderer = new THREE.WebGLRenderer({ canvas, antialias: true, alpha: true });
  const size = () => ({ w: canvas.clientWidth || 1, h: canvas.clientHeight || 1 });
  let { w, h } = size();
  renderer.setSize(w, h, false);
  renderer.setPixelRatio(Math.min(devicePixelRatio, 2));
  renderer.autoClear = false;

  const scene = new THREE.Scene();
  scene.background = new THREE.Color(0x000000);
  const camera = new THREE.PerspectiveCamera(60, w / h, 0.1, 1000);
  camera.position.z = CONFIG.cameraDistance;

  const bgScene = new THREE.Scene();
  const bgCamera = new THREE.OrthographicCamera(-1, 1, 1, -1, -1, 1);
  const bgMaterial = new THREE.ShaderMaterial({
    uniforms: {
      uTime: { value: 0 },
      uScroll: { value: 0.0 },
      uResolution: { value: new THREE.Vector2(w, h) },
      color1: { value: new THREE.Color(CONFIG.colorWarm) },
      color2: { value: new THREE.Color(CONFIG.colorCool) },
    },
    vertexShader: BG_VERT,
    fragmentShader: BG_FRAG,
    depthWrite: false,
  });
  bgScene.add(new THREE.Mesh(new THREE.PlaneGeometry(2, 2), bgMaterial));

  const pMaterial = new THREE.ShaderMaterial({
    uniforms: {
      uTime: { value: 0 },
      uScroll: { value: 0.0 },
      uIntro: { value: 0.0 },
      uColorTop: { value: hexToVec3(CONFIG.colorWarm) },
      uColorBottom: { value: hexToVec3(CONFIG.colorCool) },
      uCursor: { value: new THREE.Vector3(0, 0, 4.2) },
      uCursorStrength: { value: 0 },
      uCursorRadius: { value: CONFIG.cursorRadius },
      uCursorFlare: { value: CONFIG.cursorFlare },
      uCursorHeat: { value: CONFIG.cursorHeat },
      uLightDir: { value: new THREE.Vector3(...CONFIG.lightDir).normalize() },
      uLightColor: { value: hexToVec3(CONFIG.lightColor) },
    },
    vertexShader: P_VERT,
    fragmentShader: P_FRAG,
    transparent: true,
    blending: THREE.AdditiveBlending,
    depthWrite: false,
  });
  const particles = new THREE.Points(new THREE.SphereGeometry(4.2, 200, 600), pMaterial);
  particles.frustumCulled = false;
  scene.add(particles);

  const pickSphere = new THREE.Mesh(new THREE.SphereGeometry(4.2, 48, 48));

  const composer = new EffectComposer(renderer);
  composer.addPass(new RenderPass(bgScene, bgCamera));
  const spherePass = new RenderPass(scene, camera);
  spherePass.clear = false;
  spherePass.clearDepth = true;
  composer.addPass(spherePass);
  composer.addPass(new UnrealBloomPass(
    new THREE.Vector2(w, h),
    CONFIG.bloomStrength, CONFIG.bloomRadius, CONFIG.bloomThreshold));
  composer.setSize(w, h);

  const raycaster = new THREE.Raycaster();
  const pointer = new THREE.Vector2();
  const cursorTarget = new THREE.Vector3(0, 0, 4.2);
  let pointerInside = false;

  canvas.addEventListener("pointermove", (e) => {
    const r = canvas.getBoundingClientRect();
    pointer.x = ((e.clientX - r.left) / r.width) * 2 - 1;
    pointer.y = -(((e.clientY - r.top) / r.height) * 2) + 1;
    pointerInside = true;
  });
  canvas.addEventListener("pointerdown", (e) => {
    const r = canvas.getBoundingClientRect();
    pointer.x = ((e.clientX - r.left) / r.width) * 2 - 1;
    pointer.y = -(((e.clientY - r.top) / r.height) * 2) + 1;
    pointerInside = true;
  });
  canvas.addEventListener("pointerleave", () => { pointerInside = false; });

  let time = 0;
  let introStart = 0;

  /* PREFERS-REDUCED-MOTION — Nadia, 2026-08-28: "the ring ignores
   * prefers-reduced-motion completely... for some people that is a health
   * matter, not a preference." scenes/board.js (Iris's Jarvis theme) already
   * checks this; this scene, wrapped by scenes/aurelia.js, was the one
   * left out when scene-mount.js was built.
   * Scoped narrowly: only the CONTINUOUS ambient noise clock (`time`, which
   * drives the sphere's surface perturbation every frame forever, on its
   * own, independent of anything happening) stops advancing below. The
   * intro settle (uIntro) still completes once and holds, and the
   * voice/thinking response (uCursor/uCursorStrength) still lerps to a new
   * pose on a real state change and then stops — bounded motion tied to an
   * actual event, which every established reduced-motion pattern treats
   * differently from unbounded ambient looping. composer.render() keeps
   * running every frame either way (see frame() below): once uTime and the
   * intro have both settled, nothing new is drawn, so the screen is static
   * even though the loop itself is not. */
  const REDUCE_MOTION = window.matchMedia
    && window.matchMedia("(prefers-reduced-motion: reduce)").matches;

  /* SPEAKING DRIVES THE SAME FLARE THE CURSOR DOES. Mark: "that motions when
   * spoken too." No new uniform was invented for it — the prominence already
   * exists and already looks right; a voice is simply another thing that can
   * push it. Speaking lights the face turned toward the viewer, so the sphere
   * appears to address the person rather than flaring off to one side. */
  const VOICE_POINT = new THREE.Vector3(0, 0, 4.2);

  /* WHERE THE RING SHOULD ACTUALLY SIT. Mark, 2026-08-21: "it is also
   * significantly off center."
   *
   * IT WAS, AND NOT BECAUSE OF ANYTHING IN THIS FILE. The canvas is full-bleed
   * on purpose — the aurora pass is written for a whole window and looked
   * cropped at anything less. But the conversation does NOT live in the whole
   * window: it sits in a stage box inset by the left rail and the top bar. So
   * the ring was centred on the WINDOW while the prompt box, the replies and the
   * readability scrim were all centred on the STAGE, about 135px apart at his
   * width. Two centres on one screen, and the eye picks that up immediately.
   *
   * THE CANVAS STAYS FULL-BLEED AND THE CAMERA MOVES INSTEAD. Insetting the
   * canvas would have fixed the ring and taken the aurora off three edges with
   * it. Shifting the camera laterally moves only the sphere.
   *
   * MEASURED FROM THE DOM, NEVER HARDCODED. The rail is min(23%, 270px), so the
   * offset is one number below 1174px wide and a different one above it — a
   * constant here would be correct on his monitor and wrong on his phone. */
  let camShiftX = 0;
  let camShiftY = 0;

  function recentre() {
    camShiftX = 0;
    camShiftY = 0;
    const sel = opts && opts.centerOn;
    if (!sel) return;
    const target = document.querySelector(sel);
    if (!target) return;
    const t = target.getBoundingClientRect();
    const c = canvas.getBoundingClientRect();
    if (!c.width || !c.height || !t.width || !t.height) return;
    // World units per CSS pixel at the sphere's own depth.
    const visibleH = 2 * CONFIG.cameraDistance
      * Math.tan((camera.fov * Math.PI / 180) / 2);
    const upp = visibleH / c.height;
    // The camera moves opposite to the way the sphere should appear to go:
    // screen-right is camera-left, screen-down is camera-up.
    camShiftX = -((t.left + t.width / 2) - (c.left + c.width / 2)) * upp;
    camShiftY = ((t.top + t.height / 2) - (c.top + c.height / 2)) * upp;
  }
  recentre();

  function frame(now) {
    requestAnimationFrame(frame);
    /* SKIP THE GPU WORK WHILE GENUINELY HIDDEN — added 2026-09-03. This loop
       had no pause of its own; scenes/aurelia.js's wrapper only ever paused
       its OWN separate level-easing tick (see that file's comment), and this
       one — the actual WebGL render() call — kept running underneath it
       regardless, spiking GPU on a minimised window. Gated the same way as
       every other scene now: `document.hidden`, never focus/blur, so a
       window merely sat on a second monitor still animates normally. The
       requestAnimationFrame(frame) call above stays unconditional on
       purpose — it is nearly free by itself, and re-arming the chain from
       outside this closure would mean exposing a resume path this vendored
       file was never given (see the header comment: "no dispose()"). Time
       simply does not advance and composer.render() is not called while
       hidden, so the one real cost — the render — stops.

       AND WHILE HOME ISN'T THE ACTIVE VIEW — added 2026-09-05. Same gap as
       document.hidden had before this file was opened for that fix: leaving
       Home for Settings or Chat never touches document.hidden, but
       index.html hides `.brain` (this canvas's container) entirely off Home
       via CSS. This is the single most expensive of this app's scenes —
       a full composer.render() through three passes and a five-mip bloom,
       every frame, on the GPU — so it is the one place this check earns the
       most back. `document.body.dataset.view` is the same public attribute
       index.html's own CSS rule reads; nothing new had to be exposed to
       reach it from here. */
    if (typeof document !== "undefined" && document.hidden) return;
    if (typeof document !== "undefined" && document.body) {
      const v = document.body.dataset && document.body.dataset.view;
      if (v && v !== "home") return;
    }
    if (!introStart) introStart = now;

    if (!REDUCE_MOTION) time += 0.005 * CONFIG.noiseSpeed;

    // Under reduced motion the intro jumps straight to its settled value
    // instead of easing in over CONFIG.introSeconds -- a bounded few-second
    // transition is not the ambient loop this guard exists to stop, but
    // scene-mount.js already disables the (also bounded) crossfade the same
    // way, and there is no reason for this scene to be the one place that
    // rule does not reach.
    const introRaw = Math.min((now - introStart) / (CONFIG.introSeconds * 1000), 1);
    const introEased = REDUCE_MOTION ? 1 : 1 - Math.pow(1 - introRaw, 3);
    pMaterial.uniforms.uIntro.value = introEased;

    const mode = (state && state.mode) || "idle";
    const level = (state && state.level) || 0;

    /* Thinking churns the noise faster; speaking pushes it harder still. The
     * scene's own clock is what carries that, so it reads as the same object
     * getting busier rather than as a second animation layered on. */
    const churn = mode === "thinking" ? 2.2 : mode === "talking" ? 1.5 : 1.0;
    if (!REDUCE_MOTION) time += 0.005 * CONFIG.noiseSpeed * (churn - 1);

    pMaterial.uniforms.uTime.value = time;
    bgMaterial.uniforms.uTime.value = time;

    const introZoom = (1 - introEased) * -3.0;
    camera.position.set(camShiftX, camShiftY,
                        CONFIG.cameraDistance + introZoom);
    camera.lookAt(camShiftX, camShiftY, CONFIG.cameraDistance - 100.0);

    let overSphere = false;
    if (pointerInside) {
      raycaster.setFromCamera(pointer, camera);
      const hits = raycaster.intersectObject(pickSphere);
      if (hits.length) { cursorTarget.copy(hits[0].point); overSphere = true; }
    }

    /* Voice outranks the pointer. Somebody being spoken to should see the
     * sphere respond to that, not to where their mouse happens to rest. */
    let want = overSphere ? 1 : 0;
    if (mode === "talking") {
      cursorTarget.copy(VOICE_POINT);
      want = Math.max(want, 0.45 + 0.55 * Math.min(1, level));
    } else if (mode === "thinking") {
      cursorTarget.copy(VOICE_POINT);
      want = Math.max(want, 0.22);
    }

    const u = pMaterial.uniforms;
    u.uCursorStrength.value += (want - u.uCursorStrength.value) * 0.09;
    u.uCursor.value.lerp(cursorTarget, 0.18);

    composer.render();
  }
  requestAnimationFrame(frame);

  function onResize() {
    const s = size();
    if (!s.w || !s.h) return;
    camera.aspect = s.w / s.h;
    camera.updateProjectionMatrix();
    renderer.setSize(s.w, s.h, false);
    composer.setSize(s.w, s.h);
    bgMaterial.uniforms.uResolution.value.set(s.w, s.h);
    // The rail is a percentage below 1174px wide, so the offset genuinely
    // changes as the window does. Re-measure rather than reuse.
    recentre();
  }
  window.addEventListener("resize", onResize);
  return { onResize };
}
