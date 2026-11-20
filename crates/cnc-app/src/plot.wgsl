struct Camera {
    right: vec4<f32>,
    up: vec4<f32>,
    depth: vec4<f32>,
    flags: vec4<f32>,
    xy_depth: vec4<f32>,
}
@group(0) @binding(0) var<uniform> camera:Camera;
const CANVAS = vec3<f32>(0.949);
const RAPID_COLOR = vec3<f32>(251.0, 140.0, 0.0) / 255.0;
const RAPID_ALPHA: f32 = 166.0 / 255.0;
const TRAIL_COLOR = vec3<f32>(0.2784, 0.6314, 0.5176);
const TRAIL_ALPHA: f32 = 0.28;
const SWEEP_ALPHA: f32 = 0.48;
const PI: f32 = 3.141592653589793;

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

fn project_path(p: vec3<f32>, color: vec4<f32>, owner: u32, direction: vec3<f32>) -> Out {
    let clip = to_clip(p);
    let rapid = is_rapid(color);
    // Prefer a cut when both strokes occupy the same depth sample.
    let z = min(clip.z + select(0.0, 0.00000012, rapid), 1.0);
    var tag = 0u;
    if camera.flags.w >= 1.0 && (owner != 0u || color.a >= 0.9) {
        let screen = vec2(dot(camera.right.xyz, direction), dot(camera.up.xyz, direction)) / camera.flags.yz;
        var angle = 0u;
        if dot(screen, screen) > 0.0 {
            angle = u32(fract(atan2(screen.y, screen.x) / PI) * 32.0 + 0.5) & 31u;
        }
        let kind = select(0u, select(2u, 1u, rapid), owner != 0u);
        tag = kind * 64u + select(0u, 32u, abs(direction.z) < 0.000001) + angle;
    }
    return Out(vec4(clip.xy, z, 1.0), color, owner, tag);
}

@vertex fn vs(
    @location(0) p: vec3<f32>, @location(1) color: vec4<f32>,
    @location(2) owner: u32, @location(3) direction: vec3<f32>,
) -> Out {
    return project_path(p, color, owner, direction);
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
    if camera.flags.w >= 1.0 && (input.owner != 0u || input.color.a >= 0.9) {
        let data = select(input.color.rgb, encode_owner(input.owner), input.owner != 0u);
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
    @location(2) owner: u32, @location(3) direction: vec3<f32>,
    @builtin(vertex_index) index: u32,
) -> Out {
    let corners = array<vec2<f32>, 6>(vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(-1.0, 1.0),
        vec2(-1.0, 1.0), vec2(1.0, -1.0), vec2(1.0, 1.0));
    var out = project_path(p, color, owner, direction);
    out.position = vec4(out.position.xy + corners[index] * camera.flags.yz * 1.5, out.position.zw);
    return out;
}

// Stored separately so a wire can survive its own sweep, even on a ramp.
@group(0) @binding(1) var wires: texture_multisampled_2d<f32>;
@group(0) @binding(2) var wire_depth: texture_depth_multisampled_2d;
@group(0) @binding(3) var sweeps: texture_multisampled_2d<f32>;
@group(0) @binding(4) var sweep_depth: texture_depth_multisampled_2d;

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
    // Untagged samples are the reference grid, canvas or dark tool marker.
    if kind != 3u || raw.r < 0.45 {
        depth = textureLoad(wire_depth, pixel, index);
    }
    return Wire(color, owner, tag, depth);
}

fn read_sweep(pixel: vec2<i32>, index: i32) -> Sweep {
    let raw = textureLoad(sweeps, pixel, index);
    let owner = decode_owner(raw.rgb);
    if owner == 0u { return Sweep(0u, false, 1.0); }
    return Sweep(owner, raw.a > 0.5, textureLoad(sweep_depth, pixel, index));
}

fn is_line(wire: Wire) -> bool { return wire.tag < 192u; }
fn planar(wire: Wire) -> bool { return (wire.tag & 32u) != 0u; }
fn same_wire(a: Wire, b: Wire) -> bool { return a.owner == b.owner && a.tag >> 6u == b.tag >> 6u; }

fn depth_gap(near: f32, far: f32, offset: vec2<f32>, same_plane_basis: bool) -> f32 {
    let gradient = camera.xy_depth.xy * camera.xy_depth.z;
    // Translate the neighboring depth to the receiver's pixel on a horizontal plane.
    let correction = select(0.0, dot(gradient, offset * vec2(1.0, -1.0)), same_plane_basis);
    return far - near + correction - max(0.000001, 0.8 * length(gradient));
}

fn wire_halo(near: Wire, far: Wire, offset: vec2<f32>) -> f32 {
    if !is_line(near) || !is_line(far) || same_wire(near, far) { return 0.0; }
    let gap = depth_gap(near.depth, far.depth, offset, planar(near) && planar(far));
    if gap <= 0.0 { return 0.0; }
    let distance = length(offset) * camera.xy_depth.z;
    let angle = f32(i32(near.tag & 31u) - i32(far.tag & 31u)) * PI / 32.0;
    let crossing = abs(sin(angle));
    // Keep a different-colored rear stroke out of the foreground's MSAA pixels.
    if distance == 0.0 {
        return select(0.0, 1.0, near.tag >> 6u != far.tag >> 6u || crossing > 0.5);
    }
    let width = mix(0.8, 3.1, crossing) * mix(0.55, 1.0, smoothstep(0.0, 0.0008, gap));
    return (1.0 - smoothstep(width * 0.68, width, distance)) * smoothstep(0.24, 0.68, crossing);
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
    var front = Wire(CANVAS, 0u, 255u, 1.0);
    var front_sweep = no_sweep;
    var backdrop = CANVAS;
    var has_backdrop = false;
    for (var i = 0; i < 4; i++) {
        wire[i] = read_wire(pixel, i);
        if wire[i].depth < front.depth { front = wire[i]; }
        if wire[i].depth == 1.0 && !has_backdrop {
            backdrop = wire[i].color;
            has_backdrop = true;
        }
        sweep[i] = no_sweep;
        if show_sweep {
            sweep[i] = read_sweep(pixel, i);
            if sweep[i].depth < front_sweep.depth { front_sweep = sweep[i]; }
        }
    }

    var line_gap = 0.0;
    var sweep_gap = 0.0;
    var sweep_edge = 0.0;
    if is_line(front) || front_sweep.owner != 0u {
        let limit = vec2<i32>(textureDimensions(wires)) - vec2(1);
        let stride = max(1, i32(round(1.0 / camera.xy_depth.z)));
        for (var n = 0; n < 16; n++) {
            let neighbor = clamp(pixel + NEIGHBORS[n] * stride, vec2(0), limit);
            let offset = vec2<f32>(neighbor - pixel);
            let distance = length(offset) * camera.xy_depth.z;
            if line_gap < 1.0 && is_line(front) {
                // Read the samples individually so a thin stroke cannot vanish in the resolve.
                for (var i = 0; i < 4; i++) {
                    line_gap = max(line_gap, wire_halo(read_wire(neighbor, i), front, offset));
                }
            }
            if show_sweep && distance <= 2.5 {
                for (var i = 0; i < 4; i++) {
                    let edge = read_sweep(neighbor, i);
                    if edge.owner == 0u || edge.owner == front_sweep.owner { continue; }
                    let strength = 1.0 - smoothstep(0.85, 2.5, distance);
                    if front_sweep.owner != 0u &&
                        depth_gap(edge.depth, front_sweep.depth, offset, edge.planar && front_sweep.planar) > 0.0 {
                        sweep_edge = max(sweep_edge, strength * 0.75);
                    }
                    if is_line(front) && edge.owner != front.owner &&
                        depth_gap(edge.depth, front.depth, offset, edge.planar && planar(front)) > 0.0 {
                        sweep_gap = max(sweep_gap, strength * 0.78);
                    }
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
        if surface.owner != 0u && surface.owner != line.owner &&
            depth_gap(surface.depth, line.depth, vec2(0.0), surface.planar && planar(line)) > 0.0 {
            ink = mix(ink, pigment, SWEEP_ALPHA);
        }
        var gap = 0.0;
        if is_line(line) {
            gap = max(wire_halo(front, line, vec2(0.0)),
                select(0.0, max(line_gap, sweep_gap), same_wire(line, front)));
        }
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
