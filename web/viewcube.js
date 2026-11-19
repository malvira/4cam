// A view cube in the 3D view's upper-right corner: its faces are the machine's directions
// (Top +Z, Bottom -Z, Front -Y, Back +Y, Right +X, Left -X; on a rotary setup X runs along
// the axis). Clicking a face turns the camera to look from that side, an edge or a corner to
// the views in between; the cube turns with the camera; the home button goes back to the
// usual iso view. View only: it moves the camera, nothing else.

const SIZE = 96; // CSS pixels
const MARGIN = 8;
// Box faces in three.js order: +X, -X, +Y, -Y, +Z, -Z
const FACES = [
  ["Right", [1, 0, 0]],
  ["Left", [-1, 0, 0]],
  ["Back", [0, 1, 0]],
  ["Front", [0, -1, 0]],
  ["Top", [0, 0, 1]],
  ["Bottom", [0, 0, -1]],
];
export const VIEW_DIRS = Object.fromEntries(FACES.map(([n, d]) => [n.toLowerCase(), d]));
// the usual iso view: from the front left, above
export const HOME_DIR = [-0.35, -1.0, 0.75];

function label(THREE, text) {
  const c = document.createElement("canvas");
  c.width = c.height = 128;
  const g = c.getContext("2d");
  g.fillStyle = "#2b3442";
  g.fillRect(0, 0, 128, 128);
  g.strokeStyle = "#7fb2e5";
  g.lineWidth = 6;
  g.strokeRect(3, 3, 122, 122);
  g.fillStyle = "#e5edf7";
  g.font = "bold 30px system-ui, sans-serif";
  g.textAlign = "center";
  g.textBaseline = "middle";
  g.fillText(text, 64, 66);
  const t = new THREE.CanvasTexture(c);
  t.colorSpace = THREE.SRGBColorSpace;
  return t;
}

/** Add the cube to the view `v` (from getThree); returns { render, goTo, screenOf, dispose }. */
export function addViewCube(v, host) {
  const { THREE, renderer, camera, controls } = v;
  const scene = new THREE.Scene();
  const cam = new THREE.OrthographicCamera(-1, 1, 1, -1, 0.1, 10);
  cam.up.set(0, 0, 1);
  const mats = FACES.map(([n]) => new THREE.MeshBasicMaterial({ map: label(THREE, n) }));
  const cube = new THREE.Mesh(new THREE.BoxGeometry(1, 1, 1), mats);
  scene.add(cube);
  scene.add(new THREE.LineSegments(new THREE.EdgesGeometry(cube.geometry), new THREE.LineBasicMaterial({ color: 0x7fb2e5 })));
  // highlight under the pointer: a small marker where it would go
  const hover = new THREE.Mesh(new THREE.SphereGeometry(0.09, 12, 8), new THREE.MeshBasicMaterial({ color: 0xf59e0b }));
  hover.visible = false;
  scene.add(hover);

  // the home button
  const home = document.createElement("button");
  home.type = "button";
  home.id = "viewHome";
  home.className = "view-home";
  home.title = "Back to the usual iso view";
  home.textContent = "⌂";
  host.appendChild(home);

  const rect = () => {
    const w = host.clientWidth;
    return { x: w - SIZE - MARGIN, y: MARGIN, w: SIZE, h: SIZE };
  };

  /** The direction under a point of the host (CSS px), or null: a face, edge or corner. */
  function dirAt(px, py) {
    const r = rect();
    if (px < r.x || px > r.x + r.w || py < r.y || py > r.y + r.h) return null;
    const ndc = new THREE.Vector2(((px - r.x) / r.w) * 2 - 1, -((py - r.y) / r.h) * 2 + 1);
    const ray = new THREE.Raycaster();
    ray.setFromCamera(ndc, cam);
    const hit = ray.intersectObject(cube)[0];
    if (!hit) return null;
    // near a border along an axis: that axis counts (edges: two, corners: three)
    const p = hit.point;
    const d = [p.x, p.y, p.z].map((c) => (Math.abs(c) > 0.33 ? Math.sign(c) : 0));
    return { dir: d, point: p };
  }

  let anim = null;
  /** Turn the camera to look from `dir` (machine directions), at the same distance. */
  function goTo(dir, ms = 350) {
    const t = controls.target.clone();
    const from = camera.position.clone().sub(t);
    const dist = from.length();
    let to = new THREE.Vector3(...dir).normalize();
    // straight down or up: a hair off so the orbit's up (+Z) stays defined, +Y up the screen
    if (Math.abs(to.z) > 0.9999) to = new THREE.Vector3(0, -1e-4 * Math.sign(to.z || 1), Math.sign(to.z)).normalize();
    const a = from.clone().normalize();
    const t0 = performance.now();
    anim = { a, b: to, dist, t, t0, ms };
  }
  function step() {
    if (!anim) return;
    const k = Math.min(1, (performance.now() - anim.t0) / anim.ms);
    const e = k * k * (3 - 2 * k);
    const d = anim.a.clone().lerp(anim.b, e);
    if (d.lengthSq() < 1e-8) d.copy(anim.b);
    camera.position.copy(anim.t).add(d.normalize().multiplyScalar(anim.dist));
    camera.lookAt(anim.t);
    controls.update();
    if (k >= 1) anim = null;
  }

  // clicks on the cube: caught before the orbit controls see them
  const down = (ev) => {
    const b = host.getBoundingClientRect();
    const h = dirAt(ev.clientX - b.left, ev.clientY - b.top);
    if (!h) return;
    ev.stopPropagation();
    ev.preventDefault();
    if (ev.type === "pointerdown") goTo(h.dir);
  };
  host.addEventListener("pointerdown", down, true);
  host.addEventListener("wheel", (ev) => {
    const b = host.getBoundingClientRect();
    if (dirAt(ev.clientX - b.left, ev.clientY - b.top)) ev.stopPropagation();
  }, true);
  host.addEventListener("pointermove", (ev) => {
    const b = host.getBoundingClientRect();
    const h = dirAt(ev.clientX - b.left, ev.clientY - b.top);
    hover.visible = !!h;
    if (h) hover.position.set(...h.dir.map((c) => c * 0.5));
    host.style.cursor = h ? "pointer" : "";
  });
  home.addEventListener("click", () => goTo(HOME_DIR));

  /** Draw the cube in its corner (after the main view), turned like the camera. */
  function render() {
    step();
    const r = rect();
    const H = host.clientHeight;
    const dir = camera.position.clone().sub(controls.target).normalize();
    cam.position.copy(dir.multiplyScalar(3));
    cam.up.copy(camera.up);
    cam.lookAt(0, 0, 0);
    const scale = 0.95;
    cam.left = cam.bottom = -scale;
    cam.right = cam.top = scale;
    cam.updateProjectionMatrix();
    renderer.setScissorTest(true);
    renderer.setScissor(r.x, H - r.y - r.h, r.w, r.h);
    renderer.setViewport(r.x, H - r.y - r.h, r.w, r.h);
    // over the view, not on a cleared square: depth only
    const auto = renderer.autoClear;
    renderer.autoClear = false;
    renderer.clearDepth();
    renderer.render(scene, cam);
    renderer.autoClear = auto;
    renderer.setScissorTest(false);
    renderer.setViewport(0, 0, host.clientWidth, H);
  }

  /** Where a named view's face is on the page (client px), for tests. */
  function screenOf(name) {
    const d = VIEW_DIRS[name];
    if (!d) return null;
    const p = new THREE.Vector3(...d).multiplyScalar(0.5).project(cam);
    const r = rect(), b = host.getBoundingClientRect();
    return [b.left + r.x + ((p.x + 1) / 2) * r.w, b.top + r.y + ((1 - p.y) / 2) * r.h];
  }

  return { render, goTo, screenOf, busy: () => !!anim, dirAt };
}
