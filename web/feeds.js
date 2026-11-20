// Feeds and speeds: the engine's calculator (marcam::feeds) for a tool's cutting-data rows
// and an operation's feeds. Chip load per tooth x flutes x rpm, the material's surface speed,
// radial chip thinning at small stepovers (adaptive clearing above all) and depth limits,
// within the machine's spindle range, power and feed cap.

/** The calculator's tool from a library or job tool shape ({kind, diameter, ...}). */
export function calcTool(shape) {
  const d = shape.diameter ?? shape.tip_diameter;
  switch (shape.kind) {
    case "ball":
      return { kind: "ball", diameter: d };
    case "tapered_ball":
      return { kind: "ball", diameter: shape.tip_diameter ?? d };
    case "vbit":
      return { kind: "vbit", diameter: d ?? 6, angle: shape.angle ?? 90 };
    case "drill":
      return { kind: "drill", diameter: d };
    default:
      return { kind: "flat", diameter: d };
  }
}

/** Feeds and speeds: opts {material, tool (a shape), flutes?, flute_length?, op?, adaptive?,
 *  stepover? (mm), stepdown?, rpm?, machine?}. Resolves to {rpm, feed, plunge, stepdown,
 *  stepover, ramp_angle, chip_load, chip_thickness, surface_speed, notes, preset}; rejects
 *  with the reason (no material, an unknown one, no diameter). */
export async function calcFeeds(engine, opts) {
  if (!opts.material) throw new Error("Pick a material first: the feeds depend on it");
  const { result } = await engine("feedsCalc", { ...opts, tool: calcTool(opts.tool) });
  if (!result || result.error) throw new Error(result?.error || "no result");
  return result;
}

/** One line saying what the values are and what limited them. */
export function feedsSummary(f) {
  const head = `${f.rpm} rpm, ${f.feed} mm/min (chip ${f.chip_thickness} mm, ${f.surface_speed} m/min)`;
  return f.notes?.length ? `${head}. ${f.notes.join("; ")}` : head;
}
