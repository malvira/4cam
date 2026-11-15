"""Tapered diamond-knurled knob with rectangular-post boss. Writes knob.stl (mm)."""
import numpy as np, struct, sys

# ---- parameters (mm) ----
H        = 15.0   # knob body height
R_BASE   = 10.0   # radius at back face (Ø20)
R_TOP    = 8.0    # radius at front face (Ø16)
KN_N     = 30     # knurl pitch count around
KN_DEPTH = 0.6    # knurl depth
KN_ANGLE = 30.0   # helix angle of knurl lines (deg)
CHAMFER  = 0.8    # front edge chamfer
BOSS_H   = 3.0    # boss height
BOSS_R   = 5.0    # boss radius (Ø10)
HOLE_W   = 4.1    # rectangular hole width (final size, no extra clearance)
HOLE_L   = 5.1    # rectangular hole length
HOLE_IN  = 7.0    # how far hole extends into knob body past back face

NA = KN_N * 16            # angular samples (multiple of 8 so square corners land on samples)
NZ = 150
th = np.arange(NA) * 2 * np.pi / NA

def tri(x):               # triangle wave, period 1, range 0..1
    return 1 - 2 * np.abs(x - np.floor(x) - 0.5)

def outer_ring(z):
    R = R_BASE + (R_TOP - R_BASE) * z / H
    k = np.tan(np.radians(KN_ANGLE)) * KN_N / (2 * np.pi * R)   # helix rate
    u = th * KN_N / (2 * np.pi)
    pyr = np.minimum(tri(u + k * z), tri(u - k * z))           # diamond pyramids
    fade = np.clip((H - z) / CHAMFER, 0, 1)                    # knurl fades into chamfer
    r = R - KN_DEPTH * (1 - pyr) * fade - CHAMFER * (1 - fade) ** 1
    return np.c_[r * np.cos(th), r * np.sin(th), np.full(NA, z)]

def circ(r, z):
    return np.c_[r * np.cos(th), r * np.sin(th), np.full(NA, z)]

def rect(z):
    r = 1 / np.maximum(np.abs(np.cos(th)) / (HOLE_W / 2), np.abs(np.sin(th)) / (HOLE_L / 2))
    return np.c_[r * np.cos(th), r * np.sin(th), np.full(NA, z)]

def build(hole_w=HOLE_W, hole_l=HOLE_L):
    global HOLE_W, HOLE_L
    HOLE_W, HOLE_L = hole_w, hole_l
    tris = []
    def band(a, b):           # stitch ring a -> ring b, outward normal for CCW a below b
        for i in range(NA):
            j = (i + 1) % NA
            tris.append((a[i], a[j], b[j])); tris.append((a[i], b[j], b[i]))
    def fan(ring, c, up):
        for i in range(NA):
            j = (i + 1) % NA
            tris.append((c, ring[i], ring[j]) if up else (c, ring[j], ring[i]))

    # outer knurled wall z=0..H
    zs = np.linspace(0, H, NZ + 1)
    rings = [outer_ring(z) for z in zs]
    for a, b in zip(rings, rings[1:]): band(a, b)
    fan(rings[-1], np.array([0, 0, H]), True)                 # front face
    band(circ(BOSS_R, 0), rings[0])                            # back face annulus (normal -z)
    band(circ(BOSS_R, -BOSS_H), circ(BOSS_R, 0))               # boss wall
    band(rect(-BOSS_H), circ(BOSS_R, -BOSS_H))            # boss end annulus
    band(rect(HOLE_IN), rect(-BOSS_H))               # hole wall (inward)
    fan(rect(HOLE_IN), np.array([0, 0, HOLE_IN]), False)  # hole ceiling

    return np.array(tris)

if __name__ == "__main__":
    T = build()
    # fix winding per-band orientation by signed volume check, then write
    vol = np.einsum('ij,ij->i', T[:, 0], np.cross(T[:, 1], T[:, 2])).sum() / 6
    with open(sys.argv[1] if len(sys.argv) > 1 else "knob.stl", "wb") as f:
        f.write(b"knurled knob".ljust(80, b" ")); f.write(struct.pack("<I", len(T)))
        for t in T:
            n = np.cross(t[1] - t[0], t[2] - t[0]); n = n / (np.linalg.norm(n) or 1)
            f.write(struct.pack("<12fH", *n, *t[0], *t[1], *t[2], 0))
    print("triangles", len(T), "volume mm3", round(vol, 1))
