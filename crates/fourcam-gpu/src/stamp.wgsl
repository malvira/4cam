// Rotary stock simulation on the GPU: one invocation per (tool position, cell under the
// tool). Each finds where the cell's ray from the axis enters the tool (the same fixed-point
// iteration as the CPU's sim::stamp, in f32) and keeps the lowest with atomicMin on a key:
// height in micrometres << 8 | operation tag. The CPU gives each position its cell window.

struct Grid {
    x0: f32,
    dx: f32,
    nx: u32,
    na: u32,
    // positions in this dispatch, and the window size every invocation row covers
    count: u32,
    win_i: u32,
    win_j: u32,
    _pad: u32,
};

struct Pos {
    x: f32,
    r: f32,
    phi_t: f32,   // part angle under the tool (radians)
    radius: f32,  // tool radius
    kind: u32,    // 0 flat, 1 ball, 2 cone
    inv_tan: f32, // cone: 1 / tan(half angle)
    tag: u32,
    i0: u32,
    i1: u32,
    j0: i32,      // angular window, may run past 0 or na (wrapped)
    j1: i32,
    _pad: u32,
};

@group(0) @binding(0) var<uniform> grid: Grid;
@group(0) @binding(1) var<storage, read> pos: array<Pos>;
@group(0) @binding(2) var<storage, read_write> keys: array<atomic<u32>>;

const TAU: f32 = 6.283185307179586;

fn height(p: Pos, d: f32) -> f32 {
    if (p.kind == 1u) {
        return p.radius - sqrt(max(p.radius * p.radius - d * d, 0.0));
    }
    if (p.kind == 2u) {
        return d * p.inv_tan;
    }
    return 0.0;
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let n = gid.y;
    if (n >= grid.count) {
        return;
    }
    let cell = gid.x;
    if (cell >= grid.win_i * grid.win_j) {
        return;
    }
    let p = pos[n];
    let i = p.i0 + cell / grid.win_j;
    let jj = p.j0 + i32(cell % grid.win_j);
    if (i > p.i1 || jj > p.j1) {
        return;
    }
    let lim = p.radius * (1.0 - 1e-6);
    let u = grid.x0 + (f32(i) + 0.5) * grid.dx - p.x;
    if (abs(u) >= lim) {
        return;
    }
    let dphi = TAU / f32(grid.na);
    let delta = f32(jj) * dphi + 0.5 * dphi - p.phi_t;
    let cs = cos(delta);
    if (cs <= 1e-6) {
        return;
    }
    let s = abs(sin(delta));
    var rho = p.r / cs;
    var inside = false;
    for (var k = 0; k < 8; k++) {
        let d = sqrt(u * u + (rho * s) * (rho * s));
        if (d >= lim) {
            inside = false;
            break;
        }
        inside = true;
        let next = (p.r + height(p, d)) / cs;
        if (abs(next - rho) < 1e-6) {
            rho = next;
            break;
        }
        rho = next;
    }
    if (!inside) {
        return;
    }
    // wrap round the bar; kept positive first: `%` of a negative i32 is not portable
    // across backends (one driver here gave the unsigned remainder)
    let j = u32(jj + 2 * i32(grid.na)) % grid.na;
    let um = u32(max(rho, 0.0) * 1000.0);
    atomicMin(&keys[i * grid.na + j], (um << 8u) | (p.tag & 255u));
}
