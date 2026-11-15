"""Fit-test set: 5 knobs with stepped rectangular holes, numbered on the back. Writes knob_fit_set.3mf."""
import numpy as np, zipfile
import knob

NOMINAL = (4.0, 5.0)
CLEARANCES = [0.2, 0.3, 0.4, 0.5, 0.6]   # total added to each dimension
PIX, EMB, SINK = 0.7, 0.5, 0.2           # label pixel size, emboss height, overlap into body
LABEL_Y = 7.0                            # label centre distance from axis (printed orientation)
SPACING = 25.0

FONT = {1: [".X.", "XX.", ".X.", ".X.", "XXX"],
        2: ["XXX", "..X", "XXX", "X..", "XXX"],
        3: ["XXX", "..X", "XXX", "..X", "XXX"],
        4: ["X.X", "X.X", "XXX", "..X", "..X"],
        5: ["XXX", "X..", "XXX", "..X", "XXX"]}

def label(d):
    """Voxel digit, built directly in PRINT orientation (back face up at z=BOSS top region)."""
    g = [[c == "X" for c in row] for row in reversed(FONT[d])]   # g[row][col], row 0 = bottom
    R, C = len(g), len(g[0])
    fill = lambda r, c: 0 <= r < R and 0 <= c < C and g[r][c]
    x0, y0 = -C * PIX / 2, -LABEL_Y - R * PIX / 2
    z0, z1 = knob.H - SINK, knob.H + EMB          # back face sits at z = H after flipping
    P = lambda c, r, z: (x0 + c * PIX, y0 + r * PIX, z)
    T = []
    quad = lambda a, b, c_, d_: T.extend([(a, b, c_), (a, c_, d_)])   # CCW seen from outside
    for r in range(R):
        for c in range(C):
            if not g[r][c]: continue
            quad(P(c, r, z1), P(c+1, r, z1), P(c+1, r+1, z1), P(c, r+1, z1))      # top
            quad(P(c, r, z0), P(c, r+1, z0), P(c+1, r+1, z0), P(c+1, r, z0))      # bottom
            if not fill(r-1, c): quad(P(c, r, z0), P(c+1, r, z0), P(c+1, r, z1), P(c, r, z1))
            if not fill(r+1, c): quad(P(c+1, r+1, z0), P(c, r+1, z0), P(c, r+1, z1), P(c+1, r+1, z1))
            if not fill(r, c-1): quad(P(c, r+1, z0), P(c, r, z0), P(c, r, z1), P(c, r+1, z1))
            if not fill(r, c+1): quad(P(c+1, r, z0), P(c+1, r+1, z0), P(c+1, r+1, z1), P(c+1, r, z1))
    return np.array(T, float)

def to_print(T):
    """Flip 180° about X so the front face sits on the bed; back face/boss point up."""
    T = T.copy(); T[..., 1] *= -1; T[..., 2] *= -1; T[..., 2] += knob.H
    return T

def mesh_xml(oid, T, name):
    v, idx = np.unique(T.reshape(-1, 3).round(5), axis=0, return_inverse=True)
    f = idx.reshape(-1, 3)
    vs = "".join(f'<vertex x="{a:.5f}" y="{b:.5f}" z="{c:.5f}"/>' for a, b, c in v)
    ts = "".join(f'<triangle v1="{a}" v2="{b}" v3="{c}"/>' for a, b, c in f)
    return f'<object id="{oid}" type="model" name="{name}"><mesh><vertices>{vs}</vertices><triangles>{ts}</triangles></mesh></object>', f

objs, items, checks = [], [], []
oid = 1
for i, cl in enumerate(CLEARANCES):
    n = i + 1
    w, l = NOMINAL[0] + cl, NOMINAL[1] + cl
    body = to_print(knob.build(w, l))
    lab = label(n)
    xb, fb = mesh_xml(oid, body, f"knob{n}_body"); xl, fl = mesh_xml(oid + 1, lab, f"knob{n}_label")
    objs += [xb, xl]
    objs.append(f'<object id="{oid+2}" type="model" name="#{n} hole {w:.1f}x{l:.1f}"><components>'
                f'<component objectid="{oid}"/><component objectid="{oid+1}"/></components></object>')
    x = (i - 2) * SPACING
    items.append(f'<item objectid="{oid+2}" transform="1 0 0 0 1 0 0 0 1 {x+128:.3f} 128 0"/>')
    checks.append((n, w, l, fb, fl, body, lab))
    oid += 3

model = ('<?xml version="1.0" encoding="UTF-8"?><model unit="millimeter" xml:lang="en-US" '
         'xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02"><resources>'
         + "".join(objs) + "</resources><build>" + "".join(items) + "</build></model>")
with zipfile.ZipFile("knob_fit_set.3mf", "w", zipfile.ZIP_DEFLATED) as z:
    z.writestr("[Content_Types].xml", '<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/></Types>')
    z.writestr("_rels/.rels", '<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Target="/3D/3dmodel.model" Id="rel0" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/></Relationships>')
    z.writestr("3D/3dmodel.model", model)

def check(f, T):
    e = np.sort(np.r_[f[:, [0, 1]], f[:, [1, 2]], f[:, [2, 0]]], axis=1)
    _, c = np.unique(e, axis=0, return_counts=True)
    vol = np.einsum('ij,ij->i', T[:, 0], np.cross(T[:, 1], T[:, 2])).sum() / 6
    return (c != 2).sum(), vol
for n, w, l, fb, fl, body, lab in checks:
    print(n, f"{w:.1f}x{l:.1f}", "body(nonmanifold,vol)", check(fb, body), "label", check(fl, lab),
          "label r-range", np.hypot(lab[..., 0], lab[..., 1]).min().round(2), np.hypot(lab[..., 0], lab[..., 1]).max().round(2))
