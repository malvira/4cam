"""4th-axis (rotary) radial toolpaths for the tapered diamond-knurl knob.

Writes two G-code files (one per tool, so no tool-change syntax is needed):
  knob_1_profile.nc  - T1 flat endmill: rough + finish the turned profile
                       (front chamfer, Ø20->Ø16 taper, Ø10x3 boss, optional neck)
  knob_2_knurl.nc    - T2 V-bit: diamond knurl (two families of helical grooves) + 45° front chamfer

Coordinate conventions (CHECK AGAINST YOUR Z1 SETUP):
  A  rotates about the X axis, degrees.
  X0 = front face of the knob (free end of the stock). Part extends toward -X (toward the chuck).
  Y0 = rotation axis centreline (tool must be exactly over the axis).
  Z0 = rotation axis centreline (so Z = radius at the tool tip).
  Units mm, absolute (G21 G90).

Feed rate: F on moves that mix X/Z with A is written so that the *surface* speed is correct
for controllers that treat degrees as length in the feed vector (GRBL/Smoothieware style).
Set FEED_MODE = "linear_only" if your controller ignores A when computing feed.
"""
import numpy as np

# ---------------- part geometry (from knob.py) ----------------
H        = 15.0    # body length
R_BACK   = 10.0    # body radius at back (Ø20)
R_FRONT  = 8.0     # body radius at front (Ø16)
CHAMFER  = 0.8     # 45° chamfer on front edge
BOSS_R   = 5.0     # boss radius (Ø10)
BOSS_H   = 3.0     # boss length
NECK     = True    # cut a neck behind the boss so the part can be sawn off
NECK_R   = 3.0     # neck radius left (Ø6) - saw through this, then face the boss end
NECK_W   = 3.7     # neck width (just over the endmill diameter)

KN_N     = 30      # grooves per family (= diamonds around)
KN_ANGLE = 30.0    # helix angle of the grooves (deg from the X axis)
KN_FILL  = 0.95    # 1.0 = full-point pyramids; <1 leaves small flats on the peaks

# ---------------- stock & tools ----------------
R_STOCK  = 25.0 / 2   # Ø25 x 100 round 6061
T1_D     = 3.175      # 1/8" single-flute flat endmill
T1_FLUTE = 12.0       # flute length
T2_ANGLE = 90.0       # V-bit included angle

# ---------------- cutting parameters (conservative for the Z1's 150 W spindle) ----------------
SPINDLE      = 12000
ROUGH_DOC    = 0.4     # radial depth per roughing level
ROUGH_STEP   = 1.0     # X advance per revolution, roughing
ROUGH_FEED   = 300.0   # surface feed mm/min (1 flute @ 12k rpm = 0.025 mm/tooth)
FINISH_ALLOW = 0.15    # radial stock left by roughing
FINISH_STEP  = 0.4     # X advance per revolution, finishing (knurl removes the taper surface anyway)
FINISH_FEED  = 250.0
KNURL_PASSES = (0.6, 1.0)  # fraction of groove depth per pass
KNURL_FEED   = 200.0
PLUNGE_FEED  = 60.0
SAFE_Z       = R_STOCK + 3.0
SEG_DEG      = 6.0     # angular segment length for rotary moves
FEED_MODE    = "vector_with_degrees"   # or "linear_only"

RT   = T1_D / 2
X_BOSS_END = -H - BOSS_H
X_NECK_END = X_BOSS_END - (NECK_W if NECK else 0.0)


def body_R(x):
    """Nominal (pre-knurl) body radius at axial position x (x<=0)."""
    return R_FRONT + (R_BACK - R_FRONT) * (-x) / H


def target_r(x):
    """Finished turned radius at x (vectorised). Stock radius where nothing is cut."""
    x = np.asarray(x, float)
    r = np.full_like(x, R_STOCK)
    body = (x >= -H) & (x <= 0)
    r = np.where(body, body_R(x), r)
    ch = (x > -CHAMFER) & (x <= 0)
    r = np.where(ch, body_R(x) - (x + CHAMFER), r)
    r = np.where(x > 0, R_FRONT - CHAMFER, r)                        # past the front face: leftover stock
    r = np.where((x < -H) & (x >= X_BOSS_END), BOSS_R, r)
    if NECK:
        r = np.where((x < X_BOSS_END) & (x >= X_NECK_END), NECK_R, r)
    return r


# Tool-centre Z for a flat endmill: highest target point under the flat bottom (gouge-free)
XG = np.round(np.arange(-28.0, 4.0 + 1e-9, 0.02), 4)
_fine = np.arange(-RT, RT + 1e-9, 0.01)
ZT = np.array([target_r(x + _fine).max() for x in XG])


def zt(x):
    return np.interp(x, XG, ZT)


class GCode:
    def __init__(self, title, tool):
        self.lines = [f"({title})", f"({tool})",
                      "(Zero: X0 front face, Y0/Z0 rotation axis, A rotates about X)",
                      "G21 G90 G94", "G17"]
        self.x = self.z = None
        self.a = 0.0

    def emit(self, s):
        self.lines.append(s)

    def rapid(self, x=None, z=None, a=None):
        w = "G0"
        if z is not None: w += f" Z{z:.3f}"; self.z = z
        if x is not None: w += f" X{x:.3f}"; self.x = x
        if a is not None: w += f" A{a:.3f}"; self.a = a
        self.emit(w)

    def cut(self, x, z, a, surf_feed):
        dx, dz, da = x - self.x, z - self.z, a - self.a
        r = max(0.5 * (z + self.z), 0.5)
        true_len = np.sqrt(dx * dx + dz * dz + (r * np.radians(da)) ** 2)
        if true_len < 1e-6:
            return
        dt = true_len / surf_feed
        if dz < 0:                       # never descend faster than the plunge feed
            dt = max(dt, -dz / PLUNGE_FEED)
        if FEED_MODE == "linear_only":
            lin = np.hypot(dx, dz)
            f = lin / dt if lin > 1e-6 else surf_feed
        else:
            f = np.sqrt(dx * dx + dz * dz + da * da) / dt
        self.emit(f"G1 X{x:.3f} Z{z:.3f} A{a:.3f} F{f:.0f}")
        self.x, self.z, self.a = x, z, a

    def save(self, path):
        self.emit("M5")
        self.emit(f"G0 Z{SAFE_Z:.3f}")
        self.emit("M30")
        open(path, "w").write("\n".join(self.lines) + "\n")


def helix(g, x0, x1, zfun, step, feed, z_start=None, direction=1):
    """Rotary helix from x0 to x1 with Z = zfun(x). A turns `direction`. Full rev at each end."""
    if z_start is not None:            # ramp down over one revolution at x0
        n = int(360 / SEG_DEG)
        for i in range(1, n + 1):
            f = i / n
            g.cut(x0, z_start + (zfun(x0) - z_start) * f, g.a + direction * SEG_DEG, feed)
    n_end = int(360 / SEG_DEG)
    for _ in range(n_end):             # dwell revolution at start
        g.cut(x0, zfun(x0), g.a + direction * SEG_DEG, feed)
    L = abs(x1 - x0)
    if L > 1e-9:
        n = max(1, int(np.ceil(L / step * 360 / SEG_DEG)))
        for i in range(1, n + 1):
            x = x0 + (x1 - x0) * i / n
            g.cut(x, zfun(x), g.a + direction * 360 * L / step / n, feed)
    for _ in range(n_end):             # clean-up revolution at end
        g.cut(x1, zfun(x1), g.a + direction * SEG_DEG, feed)


def intervals(mask, xs):
    out, start = [], None
    for i, m in enumerate(mask):
        if m and start is None: start = i
        if not m and start is not None: out.append((xs[start], xs[i - 1])); start = None
    if start is not None: out.append((xs[start], xs[-1]))
    return out


def profile_program():
    g = GCode("Knob radial profile - rough + finish", f"T1 {T1_D:.3f} mm single-flute flat endmill, {T1_FLUTE:.0f} mm flute, aluminium")
    g.emit("M6 T1 (tool change + tool length measurement)")
    g.emit(f"M3 S{SPINDLE}")
    g.emit("G4 P3")
    g.rapid(z=SAFE_Z)
    xs = XG[(XG >= X_NECK_END - RT - 1) & (XG <= RT + 1.0)]
    z_need = zt(xs) + FINISH_ALLOW
    levels = []
    L = R_STOCK - ROUGH_DOC
    while L > z_need.min() - ROUGH_DOC + 1e-6:
        levels.append(max(L, z_need.min()))
        L -= ROUGH_DOC
    prev = R_STOCK
    direction = 1
    g.emit("(--- roughing ---)")
    for L in levels:
        mask = z_need < L - 1e-6
        for xa, xb in intervals(mask, xs):
            # zt() already accounts for the tool radius, so this interval is exactly where the
            # tool centre can sit at level L: no wall drops inside a pass.
            zf = lambda x, L=L: max(L, float(zt(x)) + FINISH_ALLOW)
            # go front->back on even levels, back->front on odd so A stays bounded
            if direction < 0: xa, xb = xb, xa
            zs = max(prev + 0.5, zf(xa))   # never plunge below the profile at the entry point
            g.rapid(z=SAFE_Z); g.rapid(x=xa)
            g.emit(f"G1 Z{zs:.3f} F{PLUNGE_FEED:.0f}"); g.z = zs
            helix(g, xa, xb, zf, ROUGH_STEP, ROUGH_FEED, z_start=zs, direction=direction)
            g.rapid(z=SAFE_Z)
        prev = L
        direction = -direction
    g.emit("(--- finishing ---)")
    xa, xb = RT + 0.5, X_NECK_END + RT + 0.02
    g.rapid(z=SAFE_Z); g.rapid(x=xa)
    z0 = float(zt(xa))
    g.emit(f"G1 Z{z0 + 0.5:.3f} F{PLUNGE_FEED:.0f}"); g.z = z0 + 0.5
    helix(g, xa, xb, lambda x: float(zt(x)), FINISH_STEP, FINISH_FEED, z_start=z0 + 0.5, direction=1)
    g.rapid(z=SAFE_Z)
    return g


def knurl_depth(x):
    R = body_R(x)
    p_perp = 2 * np.pi * R / KN_N * np.cos(np.radians(KN_ANGLE))
    return KN_FILL * p_perp / (2 * np.tan(np.radians(T2_ANGLE / 2)))


def knurl_program():
    g = GCode("Knob diamond knurl", f"T2 {T2_ANGLE:.0f} deg V-bit, tip must be on the axis centreline (Y0)")
    g.emit("(Pattern is rotationally symmetric: A start position does not matter)")
    g.emit("G92 A0 (re-zero A here so the profile job's rotations don't unwind)")
    g.emit("M6 T2 (tool change + tool length measurement)")
    g.emit(f"M3 S{SPINDLE}")
    g.emit("G4 P3")
    g.rapid(z=SAFE_Z)
    x_back, x_front = -H - 0.6, 0.6        # run out past both ends of the body
    xs = np.linspace(x_back, x_front, 120)
    xc = np.clip(xs, -H, 0)
    # angle swept along the groove: dθ/dx = tan(α)/R(x)
    dth = np.tan(np.radians(KN_ANGLE)) / body_R(xc)
    theta = np.degrees(np.concatenate([[0], np.cumsum(0.5 * (dth[1:] + dth[:-1]) * np.diff(xs))]))
    zbot = body_R(xc) - knurl_depth(xc)
    for frac in KNURL_PASSES:
        z = body_R(xc) - (body_R(xc) - zbot) * frac
        for fam in (+1, -1):
            g.emit(f"(--- knurl family {'+' if fam > 0 else '-'} pass {frac:.0%} ---)")
            for k in range(KN_N):
                a0 = k * 360.0 / KN_N
                a = a0 + fam * theta
                g.rapid(z=SAFE_Z); g.rapid(x=xs[0], a=a[0])
                g.rapid(z=R_BACK + 0.5)
                g.emit(f"G1 Z{z[0]:.3f} F{PLUNGE_FEED:.0f}"); g.z = z[0]
                for xi, zi, ai in zip(xs[1:], z[1:], a[1:]):
                    g.cut(xi, zi, ai, KNURL_FEED)
                g.rapid(z=SAFE_Z)
    # Front chamfer with the V-bit flank (a flat endmill can only stair-step it).
    # 90° bit: flank surface r = Z_tip + (x_tip - x) / tan(half-angle); pass through (0, R_FRONT - CHAMFER).
    g.emit("(--- front chamfer with V-bit flank ---)")
    k = 1 / np.tan(np.radians(T2_ANGLE / 2))
    x_tip = CH_TIP_X
    for extra in (0.2, 0.0):
        z_tip = (R_FRONT - CHAMFER) - k * x_tip + extra
        g.rapid(z=SAFE_Z); g.rapid(x=x_tip)
        g.emit(f"G1 Z{z_tip:.3f} F{PLUNGE_FEED:.0f}"); g.z = z_tip
        for _ in range(int(2 * 360 / SEG_DEG)):
            g.cut(x_tip, z_tip, g.a + SEG_DEG, KNURL_FEED)
    g.rapid(z=SAFE_Z)
    return g


CH_TIP_X = 0.5   # V-bit tip sits this far past the front face when cutting the chamfer


if __name__ == "__main__":
    depth = R_STOCK - min(NECK_R if NECK else BOSS_R, BOSS_R)
    assert depth < T1_FLUTE - 1.0, f"radial depth {depth} mm exceeds usable flute length"
    p = profile_program(); p.save("knob_1_profile.nc")
    k = knurl_program();   k.save("knob_2_knurl.nc")
    for name, g in (("profile", p), ("knurl", k)):
        print(name, len(g.lines), "lines")
