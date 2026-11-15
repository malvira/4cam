// Draggable splitter bars between panes. Each bar resizes one target element (the pane on one
// side of it); the pane on the other side is flexible and takes the rest. Mouse, touch and pen
// (pointer events with capture), arrow keys when focused (Shift = bigger steps), double-click
// to reset, sizes saved in localStorage. Off on narrow (stacked) layouts.

const KEY = "4cam.split";
const WIDE = window.matchMedia("(min-width: 901px)");

function load() {
  try {
    return JSON.parse(localStorage.getItem(KEY) || "{}");
  } catch {
    return {};
  }
}
function save(all) {
  try {
    localStorage.setItem(KEY, JSON.stringify(all));
  } catch {}
}

/**
 * bar: the splitter element; target: the element whose size it sets; axis "x" (width) or "y"
 * (height); after: true if the target is after the bar (dragging toward it shrinks it);
 * min/max in px (max may be a function); id: storage key.
 */
export function splitter({ bar, target, axis, after = false, min, max, id }) {
  const prop = axis === "x" ? "width" : "height";
  const flexProp = "flexBasis";
  const maxPx = () => (typeof max === "function" ? max() : max);
  const clamp = (v) => Math.round(Math.max(min, Math.min(maxPx(), v)));
  bar.setAttribute("role", "separator");
  bar.setAttribute("aria-orientation", axis === "x" ? "vertical" : "horizontal");
  bar.tabIndex = 0;

  const current = () => target.getBoundingClientRect()[prop];
  function apply(px) {
    if (!WIDE.matches) {
      target.style[prop] = "";
      target.style[flexProp] = "";
      target.style.flexGrow = "";
      return;
    }
    if (px == null) {
      target.style[prop] = "";
      target.style[flexProp] = "";
      target.style.flexGrow = "";
    } else {
      const v = clamp(px);
      target.style[flexProp] = `${v}px`;
      target.style.flexGrow = "0";
      target.style.flexShrink = "0";
      target.style[prop] = `${v}px`;
    }
    bar.setAttribute("aria-valuenow", String(Math.round(current())));
    bar.setAttribute("aria-valuemin", String(min));
    bar.setAttribute("aria-valuemax", String(Math.round(maxPx())));
  }
  function set(px, persist = true) {
    apply(px);
    if (persist) {
      const all = load();
      if (px == null) delete all[id];
      else all[id] = clamp(px);
      save(all);
    }
  }

  let start = null;
  bar.addEventListener("pointerdown", (ev) => {
    if (!WIDE.matches || ev.button !== 0) return;
    ev.preventDefault();
    bar.setPointerCapture(ev.pointerId);
    start = { pos: axis === "x" ? ev.clientX : ev.clientY, size: current() };
    bar.classList.add("active");
    document.body.classList.add("resizing", axis === "x" ? "resizing-x" : "resizing-y");
  });
  bar.addEventListener("pointermove", (ev) => {
    if (!start) return;
    const d = (axis === "x" ? ev.clientX : ev.clientY) - start.pos;
    apply(start.size + (after ? -d : d));
  });
  const end = (ev) => {
    if (!start) return;
    start = null;
    bar.classList.remove("active");
    document.body.classList.remove("resizing", "resizing-x", "resizing-y");
    if (bar.hasPointerCapture?.(ev.pointerId)) bar.releasePointerCapture(ev.pointerId);
    set(current());
  };
  bar.addEventListener("pointerup", end);
  bar.addEventListener("pointercancel", end);
  bar.addEventListener("dblclick", () => set(null));
  bar.addEventListener("keydown", (ev) => {
    const grow = axis === "x" ? { ArrowRight: 1, ArrowLeft: -1 } : { ArrowDown: 1, ArrowUp: -1 };
    const dir = grow[ev.key];
    if (!dir) {
      if (ev.key === "Home" || ev.key === "Escape") set(null);
      return;
    }
    ev.preventDefault();
    const step = ev.shiftKey ? 50 : 10;
    set(current() + (after ? -dir : dir) * step);
  });

  const restore = () => apply(load()[id] ?? null);
  WIDE.addEventListener("change", restore);
  window.addEventListener("resize", () => {
    const saved = load()[id];
    if (saved != null) apply(saved);
  });
  restore();
  return { set, current };
}
