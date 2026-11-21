struct Camera {
    right: vec4<f32>,
    up: vec4<f32>,
    depth: vec4<f32>,
    flags: vec4<f32>,
    xy_depth: vec4<f32>,
    depth_scale: vec4<f32>,
}
@group(0) @binding(0) var<uniform> camera:Camera;
const CANVAS = vec3<f32>(0.949);
const RAPID_COLOR = vec3<f32>(251.0, 140.0, 0.0) / 255.0;
const RAPID_ALPHA: f32 = 166.0 / 255.0;
const TRAIL_COLOR = vec3<f32>(0.2784, 0.6314, 0.5176);
const TRAIL_ALPHA: f32 = 0.28;
const SWEEP_ALPHA: f32 = 0.48;
const AXIS_FIRST = 0xfffffcu;
const LIVE_SEGMENT = 0xffffffu;
const HALO_RADIUS = 5.75;

struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) @interpolate(flat) owner: u32,
    @location(2) @interpolate(flat) tag: u32,
}

fn to_clip(p: vec3<f32>) -> vec3<f32> {
    let point = vec4(p, 1.0);
    return vec3(dot(point, camera.right), dot(point, camera.up), clamp(dot(point, camera.depth), 0.0, 1.0));
}

fn is_rapid(color: vec4<f32>) -> bool {
    return color.a < 0.9 && color.r > 0.9 && color.g > 0.2 && color.b < 0.1;
}

fn project_path(p: vec3<f32>, color: vec4<f32>, owner: u32, detail: vec3<f32>) -> Out {
    let clip = to_clip(p);
    let rapid = is_rapid(color);
    // Prefer a cut when both strokes occupy the same depth sample.
    let z = min(clip.z + select(0.0, 0.00000012, rapid), 1.0);
    let axis = owner >= AXIS_FIRST && owner < LIVE_SEGMENT;
    let kind = select(select(2u, 1u, rapid), 0u, axis);
    let tag = kind * 64u + select(0u, 32u, abs(detail.z) < 0.000001)
        + select(0u, owner - AXIS_FIRST, axis);
    return Out(vec4(clip.xy, z, 1.0), color, owner, tag);
}

@vertex fn vs(
    @location(0) p: vec3<f32>, @location(1) color: vec4<f32>,
    @location(2) owner: u32, @location(3) detail: vec3<f32>,
) -> Out {
    return project_path(p, color, owner, detail);
}

fn encode_owner(owner: u32) -> vec3<f32> {
    return vec3<f32>(vec3<u32>(owner, owner >> 8u, owner >> 16u) & vec3(255u)) / 255.0;
}

fn decode_owner(packed: vec3<f32>) -> u32 {
    let bytes = vec3<u32>(round(packed * 255.0));
    return bytes.x | (bytes.y << 8u) | (bytes.z << 16u);
}

@fragment fn fs(input: Out) -> @location(0) vec4<f32> {
    let rapid = is_rapid(input.color);
    if rapid && camera.flags.x < 0.5 { discard; }
    if camera.flags.w >= 1.0 && input.owner != 0u {
        let data = select(input.color.rgb, encode_owner(input.owner), input.tag >= 64u);
        return vec4(data, f32(input.tag) / 255.0);
    }
    if rapid { return vec4(mix(CANVAS, input.color.rgb, input.color.a), 1.0); }
    return input.color;
}

fn fullscreen(index: u32) -> vec4<f32> {
    let triangle = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    return vec4(triangle[index], 0.0, 1.0);
}

@vertex fn vs_quad(@builtin(vertex_index) index: u32) -> Out {
    return Out(fullscreen(index), camera.flags, 0u, 0u);
}

@vertex fn vs_sweep(@location(0) p: vec3<f32>, @location(1) owner: u32) -> Out {
    return Out(vec4(to_clip(p), 1.0), vec4(0.0), owner, 0u);
}

@fragment fn fs_trail(input: Out) -> @location(0) vec4<f32> {
    return vec4(TRAIL_COLOR, TRAIL_ALPHA + input.color.w * 0.000001);
}

@fragment fn fs_sweep(input: Out) -> @location(0) vec4<f32> {
    return vec4(encode_owner(input.owner), f32(input.owner >> 24u));
}

@vertex fn vs_point(
    @location(0) p: vec3<f32>, @location(1) color: vec4<f32>,
    @location(2) owner: u32, @location(3) detail: vec3<f32>,
    @builtin(vertex_index) index: u32,
) -> Out {
    let corners = array<vec2<f32>, 6>(vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(-1.0, 1.0),
        vec2(-1.0, 1.0), vec2(1.0, -1.0), vec2(1.0, 1.0));
    var out = project_path(p, color, owner, detail);
    out.position = vec4(out.position.xy + corners[index] * camera.flags.yz * 1.5, out.position.zw);
    return out;
}

// Stored separately so a wire can survive its own sweep, even on a ramp.
@group(0) @binding(1) var wires: texture_multisampled_2d<f32>;
@group(0) @binding(2) var wire_depth: texture_depth_multisampled_2d;
@group(0) @binding(3) var sweeps: texture_multisampled_2d<f32>;
@group(0) @binding(4) var sweep_depth: texture_depth_multisampled_2d;
@group(0) @binding(5) var halos: texture_multisampled_2d<f32>;

// Matches the vertex buffer layout. The resolve uses endpoints to test the
// crossing itself, rather than comparing unrelated depths at nearby pixels.
struct PathVertex {
    point: vec3<f32>,
    color: u32,
    owner: u32,
    diameter: f32,
    pad: f32,
    dz: f32,
}
@group(1) @binding(0) var<storage, read> path: array<PathVertex>;
@group(1) @binding(1) var<storage, read> axes: array<PathVertex>;
@group(1) @binding(2) var<storage, read> live_path: array<PathVertex>;

@vertex fn vs_resolve(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    return fullscreen(index);
}

struct Wire {
    color: vec3<f32>,
    owner: u32,
    tag: u32,
    depth: f32,
}

struct Sweep {
    owner: u32,
    planar: bool,
    depth: f32,
}

fn read_wire(pixel: vec2<i32>, index: i32) -> Wire {
    let raw = textureLoad(wires, pixel, index);
    let tag = u32(round(raw.a * 255.0));
    let kind = tag >> 6u;
    var color = raw.rgb;
    var owner = 0u;
    var depth = 1.0;
    if kind == 1u || kind == 2u {
        owner = decode_owner(raw.rgb);
        color = select(vec3(0.0, 0.0, 1.0), mix(CANVAS, RAPID_COLOR, RAPID_ALPHA), kind == 1u);
    }
    if kind == 0u { owner = AXIS_FIRST + (tag & 3u); }
    // Untagged samples are the reference grid, canvas or dark tool marker.
    if kind != 3u || raw.r < 0.45 {
        depth = textureLoad(wire_depth, pixel, index);
    }
    return Wire(color, owner, tag, depth);
}

fn read_sweep(pixel: vec2<i32>, index: i32, ignore: u32) -> Sweep {
    let raw = textureLoad(sweeps, pixel, index);
    let owner = decode_owner(raw.rgb);
    if owner == 0u || owner == ignore { return Sweep(0u, false, 1.0); }
    return Sweep(owner, raw.a > 0.5, textureLoad(sweep_depth, pixel, index));
}

fn is_line(wire: Wire) -> bool { return wire.tag < 192u; }
fn planar(wire: Wire) -> bool { return (wire.tag & 32u) != 0u; }

fn depth_gap(near: f32, far: f32, offset: vec2<f32>, same_plane_basis: bool) -> f32 {
    let gradient = camera.xy_depth.xy * camera.xy_depth.z;
    // Translate the neighboring depth to the receiver's pixel on a horizontal plane.
    let correction = select(0.0, dot(gradient, offset * vec2(1.0, -1.0)), same_plane_basis);
    return far - near + correction - max(0.000001, 0.8 * length(gradient));
}

fn path_vertex(owner: u32, end: u32) -> PathVertex {
    if owner == LIVE_SEGMENT { return live_path[end]; }
    if owner >= AXIS_FIRST { return axes[(owner - AXIS_FIRST) * 2u + end]; }
    return path[(owner - 1u) * 2u + end];
}

fn screen_point(point: vec3<f32>) -> vec3<f32> {
    let clip = to_clip(point);
    return vec3(clip.xy / camera.flags.yz, clip.z);
}

fn cross2(a: vec2<f32>, b: vec2<f32>) -> f32 { return a.x * b.y - a.y * b.x; }

struct ScreenLine {
    start: vec3<f32>,
    end: vec3<f32>,
    direction: vec2<f32>,
    span: f32,
}

fn screen_line(a: vec3<f32>, b: vec3<f32>) -> ScreenLine {
    let start = screen_point(a);
    let end = screen_point(b);
    let delta = end.xy - start.xy;
    let span = length(delta);
    return ScreenLine(start, end, delta / max(span, 0.0001), span);
}

fn wire_halo(near: ScreenLine, far: ScreenLine, along: f32) -> f32 {
    if min(near.span, far.span) < 0.0001 { return 0.0; }
    let angle = cross2(near.direction, far.direction);
    // Nearly parallel strokes should not erase a band along the run.
    if abs(angle) < 0.24 { return 0.0; }
    let delta = far.start.xy - near.start.xy;
    let t_far = cross2(delta, near.direction) / angle;
    let distance = abs(along - t_far);
    if distance >= HALO_RADIUS || t_far < 0.0 || t_far > far.span { return 0.0; }
    let t_near = cross2(delta, far.direction) / angle;
    if t_near < 0.0 || t_near > near.span { return 0.0; }
    let near_z = mix(near.start.z, near.end.z, t_near / near.span);
    let far_z = mix(far.start.z, far.end.z, t_far / far.span);
    if far_z - near_z <= max(0.000001, 0.5 * camera.depth_scale.x) { return 0.0; }
    // Keep the crossing crisp and taper the Gaussian tail in screen points.
    let tail = max(distance - 2.25, 0.0);
    return exp2(-0.72 * tail * tail) * (1.0 - smoothstep(4.8, HALO_RADIUS, distance));
}

fn wire_owner(pixel: vec2<i32>, sample: i32) -> u32 {
    let raw = textureLoad(wires, pixel, sample);
    let tag = u32(round(raw.a * 255.0));
    if tag >= 192u { return 0u; }
    if tag < 64u { return AXIS_FIRST + (tag & 3u); }
    return decode_owner(raw.rgb);
}

struct HaloVertex {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) start: vec3<f32>,
    @location(1) @interpolate(flat) end: vec3<f32>,
    @location(2) @interpolate(flat) direction: vec2<f32>,
    @location(3) @interpolate(flat) span: f32,
    @location(4) @interpolate(flat) owner: u32,
}

// Rasterize the caster's screen-space bounds. The stencil restricts the
// fragment work to the visible wire samples that could receive its halo.
@vertex fn vs_halo(@builtin(vertex_index) index: u32, @builtin(instance_index) owner: u32) -> HaloVertex {
    let a = path_vertex(owner, 0u);
    let b = path_vertex(owner, 1u);
    let line = screen_line(a.point, b.point);
    let rapid = (owner < AXIS_FIRST || owner == LIVE_SEGMENT) && a.color >> 24u < 230u;
    var result = HaloVertex(vec4(2.0, 2.0, 0.0, 1.0), line.start, line.end, line.direction, line.span, owner);
    if (rapid && camera.flags.x < 0.5) || line.span < 0.0001 { return result; }
    let corners = array<vec2<f32>, 6>(vec2(0.0, -1.0), vec2(1.0, -1.0), vec2(0.0, 1.0),
        vec2(0.0, 1.0), vec2(1.0, -1.0), vec2(1.0, 1.0));
    let corner = corners[index];
    let normal = vec2(-line.direction.y, line.direction.x);
    let radius = HALO_RADIUS + camera.xy_depth.z;
    let tangent = mix(-radius, line.span + radius, corner.x);
    let point = line.start.xy + tangent * line.direction + corner.y * radius * normal;
    result.position = vec4(point * camera.flags.yz, 0.0, 1.0);
    return result;
}

@fragment fn fs_halo(input: HaloVertex, @builtin(sample_index) sample: u32) -> @location(0) f32 {
    let pixel = vec2<i32>(input.position.xy);
    let owner = wire_owner(pixel, i32(sample));
    if owner == 0u || owner == input.owner { return 0.0; }
    let far = screen_line(path_vertex(owner, 0u).point, path_vertex(owner, 1u).point);
    let near = ScreenLine(input.start, input.end, input.direction, input.span);
    let center = vec2<f32>(pixel) + vec2(0.5);
    let at = (center * camera.xy_depth.z - 1.0 / camera.flags.yz) * vec2(1.0, -1.0);
    return wire_halo(near, far, dot(at - far.start.xy, far.direction));
}

fn sweep_separation(surface: Sweep, far: f32, offset: vec2<f32>, horizontal: bool) -> f32 {
    var diameter = camera.depth_scale.y;
    if surface.owner < AXIS_FIRST {
        let programmed = path[(surface.owner - 1u) * 2u].diameter;
        if programmed > 0.0 { diameter = programmed * camera.depth_scale.z; }
    }
    let gap = depth_gap(surface.depth, far, offset, surface.planar && horizontal);
    // Nearby parts of the same-size footprint should stay readable. A visibly
    // separated upper pass can dim a lower path regardless of how it was programmed.
    return smoothstep(diameter * 1.08, max(diameter * 1.22, 0.000001), gap);
}

const NEIGHBORS = array<vec2<i32>, 16>(
    vec2(1, 0), vec2(-1, 0), vec2(0, 1), vec2(0, -1),
    vec2(1, 1), vec2(-1, -1), vec2(1, -1), vec2(-1, 1),
    vec2(2, 0), vec2(-2, 0), vec2(0, 2), vec2(0, -2),
    vec2(2, 2), vec2(-2, -2), vec2(2, -2), vec2(-2, 2),
);

@fragment fn fs_resolve(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(position.xy);
    let show_sweep = camera.flags.w >= 2.0;
    let no_sweep = Sweep(0u, false, 1.0);
    var wire: array<Wire, 4>;
    var sweep: array<Sweep, 4>;
    var front_sweep = no_sweep;
    var backdrop = CANVAS;
    var has_backdrop = false;
    for (var i = 0; i < 4; i++) {
        wire[i] = read_wire(pixel, i);
        if wire[i].depth == 1.0 && !has_backdrop {
            backdrop = wire[i].color;
            has_backdrop = true;
        }
        sweep[i] = no_sweep;
        if show_sweep {
            sweep[i] = read_sweep(pixel, i, 0u);
            if sweep[i].depth < front_sweep.depth { front_sweep = sweep[i]; }
        }
    }

    var sweep_edge = 0.0;
    if front_sweep.owner != 0u {
        let limit = vec2<i32>(textureDimensions(wires)) - vec2(1);
        let stride = max(1, i32(round(1.0 / camera.xy_depth.z)));
        for (var n = 0; n < 16; n++) {
            let neighbor = clamp(pixel + NEIGHBORS[n] * stride, vec2(0), limit);
            let offset = vec2<f32>(neighbor - pixel);
            let distance = length(offset) * camera.xy_depth.z;
            if distance <= 2.5 {
                for (var i = 0; i < 4; i++) {
                    let edge = read_sweep(neighbor, i, front_sweep.owner);
                    if edge.owner == 0u { continue; }
                    let strength = 1.0 - smoothstep(0.85, 2.5, distance);
                    let separation = sweep_separation(edge, front_sweep.depth, offset, front_sweep.planar);
                    sweep_edge = max(sweep_edge, strength * 0.75 * separation);
                }
            }
        }
    }

    var color = vec3(0.0);
    let pigment = mix(CANVAS, TRAIL_COLOR, TRAIL_ALPHA / SWEEP_ALPHA);
    for (var i = 0; i < 4; i++) {
        let line = wire[i];
        let surface = sweep[i];
        var background = select(backdrop, line.color, line.depth == 1.0);
        if surface.owner != 0u {
            background = mix(background, TRAIL_COLOR, TRAIL_ALPHA * (1.0 - sweep_edge));
        }
        if line.depth == 1.0 {
            color += background;
            continue;
        }
        var ink = line.color;
        if surface.owner != 0u && surface.owner != line.owner {
            let separation = sweep_separation(surface, line.depth, vec2(0.0), planar(line));
            ink = mix(ink, pigment, SWEEP_ALPHA * separation);
        }
        let gap = select(0.0, textureLoad(halos, pixel, i).r, is_line(line));
        color += mix(ink, background, gap);
    }
    return vec4(color * 0.25, 1.0);
}

struct ToolVertex {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) @interpolate(flat) depths: vec2<f32>,
    @location(2) @interpolate(flat) length: f32,
}

@vertex fn vs_tool(
    @location(0) start: vec3<f32>,
    @location(1) end: vec3<f32>,
    @builtin(vertex_index) index: u32,
) -> ToolVertex {
    let corners = array<vec2<f32>, 6>(
        vec2(0.0, -1.0), vec2(1.0, -1.0), vec2(0.0, 1.0),
        vec2(0.0, 1.0), vec2(1.0, -1.0), vec2(1.0, 1.0),
    );
    let a = to_clip(start);
    let b = to_clip(end);
    let span = (b.xy - a.xy) / camera.flags.yz;
    let size = length(span);
    var tangent = vec2(1.0, 0.0);
    if size > 0.0001 { tangent = span / size; }
    let normal = vec2(-tangent.y, tangent.x);
    let corner = corners[index];
    let radius = 1.8;
    let local = vec2(mix(-radius, size + radius, corner.x), corner.y * radius);
    let offset = (local.x * tangent + local.y * normal) * camera.flags.yz;
    return ToolVertex(vec4(a.xy + offset, 0.5, 1.0), local, vec2(a.z, b.z), size);
}

struct ToolFragment {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
}

@fragment fn fs_tool(input: ToolVertex) -> ToolFragment {
    let along = clamp(input.local.x, 0.0, input.length);
    if length(vec2(input.local.x - along, input.local.y)) > 1.8 { discard; }
    var depth = min(input.depths.x, input.depths.y);
    if input.length > 0.0001 {
        depth = mix(input.depths.x, input.depths.y, along / input.length);
    }
    let gray = select(65.0, 90.0, input.local.x >= input.length - 0.4) / 255.0;
    return ToolFragment(vec4(vec3(gray), 1.0), depth);
}
