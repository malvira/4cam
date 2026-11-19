// The viewer's colours: defaults, overrides kept in this browser (View > Colours), and the
// views told when one changes. Every colour the 3D view and the playback draw with comes
// from here.

export const COLOR_DEFAULTS = {
  cut: "#2563eb",
  rapid: "#dc2626",
  part: "#c8ccd2",
  stock: "#7fb2e5",
  stockEdge: "#a5d0ff",
  uncut: "#94b3db",
  tool: "#e5b94e",
  draft: "#f59e0b",
  background: "",
};

export const COLOR_LABELS = {
  cut: "Toolpath cuts",
  rapid: "Rapids",
  part: "Part",
  stock: "Stock (bar or block)",
  stockEdge: "Stock outline",
  uncut: "Uncut stock (simulations)",
  tool: "Tool",
  draft: "New operation's preview",
  background: "3D view background",
};

const KEY = "marcam.colors";
let overrides = {};
try {
  overrides = JSON.parse(localStorage.getItem(KEY) || "{}") || {};
} catch {
  overrides = {};
}
const listeners = [];

/** A colour as "#rrggbb" ("" for the background: the page's own). */
export function color(key) {
  const v = overrides[key];
  return typeof v === "string" && /^#[0-9a-f]{6}$/i.test(v) ? v : COLOR_DEFAULTS[key];
}
/** As a number (three.js). */
export const colorNum = (key) => parseInt((color(key) || "#000000").slice(1), 16);
/** As [r, g, b] in 0..1. */
export const colorRgb = (key) => {
  const h = color(key) || "#000000";
  return [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16) / 255);
};

function save() {
  try {
    localStorage.setItem(KEY, JSON.stringify(overrides));
  } catch {
    // storage blocked: this page only
  }
  for (const f of listeners) f();
}

export function setColor(key, hex) {
  if (!(key in COLOR_DEFAULTS)) return;
  if (!hex || hex === COLOR_DEFAULTS[key]) delete overrides[key];
  else overrides[key] = hex;
  save();
}

/** Back to the defaults (every colour, or one). */
export function resetColors(key) {
  if (key) delete overrides[key];
  else overrides = {};
  save();
}

export function onColorsChange(f) {
  listeners.push(f);
}
