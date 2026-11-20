struct Camera {
    right: vec4<f32>,
    up: vec4<f32>,
    depth: vec4<f32>,
    flags: vec4<f32>,
    xy_depth: vec4<f32>,
}
@group(0) @binding(0) var<uniform> camera:Camera;
const CANVAS = vec3(0.949);
struct Out { @builtin(position) position:vec4<f32>, @location(0) color:vec4<f32> }
fn to_clip(p: vec3<f32>) -> vec3<f32> {
    let point=vec4(p,1.0);
    return vec3(dot(point,camera.right), dot(point,camera.up), clamp(dot(point,camera.depth),0.0,1.0));
}
fn is_rapid(color: vec4<f32>) -> bool {
    return color.a < 0.9 && color.r > 0.9 && color.g > 0.2 && color.b < 0.1;
}
fn path_depth(depth: f32, color: vec4<f32>) -> f32 {
    // Prefer a cut when both strokes occupy the same depth sample.
    return min(depth + select(0.0, 0.00000012, is_rapid(color)), 1.0);
}
fn project_path(p: vec3<f32>, color: vec4<f32>) -> Out {
    var out:Out;
    let clip=to_clip(p);
    out.position=vec4(clip.xy,path_depth(clip.z,color),1.0);
    out.color=color;
    return out;
}
@vertex fn vs(@location(0) p:vec3<f32>,@location(1) color:vec4<f32>)->Out {
    return project_path(p, color);
}
@fragment fn fs(input:Out)->@location(0) vec4<f32> {
    if is_rapid(input.color) {
        if camera.flags.x < 0.5 { discard; }
        // Keep the subdued rapid color, but give the line an opaque depth sample.
        return vec4(mix(CANVAS, input.color.rgb, input.color.a), 1.0);
    }
    return input.color;
}
@vertex fn vs_quad(@builtin(vertex_index) n:u32)->Out {
    var out:Out;
    let p=array<vec2<f32>,3>(vec2(-1.0,-1.0),vec2(3.0,-1.0),vec2(-1.0,3.0));
    out.position=vec4(p[n],0.0,1.0);
    out.color=camera.flags;
    return out;
}
const TRAIL_COLOR = vec3(0.2784, 0.6314, 0.5176);
const TRAIL_ALPHA = 0.28;
@vertex fn vs_sweep(@location(0) p: vec3<f32>, @location(1) color: vec4<f32>) -> Out {
    var out = project_path(p, color);
    // Samples covered by a thin centerline can be offset from the true stroke
    // center. Keep the blue stroke readable on its own swept surface.
    let sample_depth = length(camera.xy_depth.xy) * camera.xy_depth.z;
    out.position.z = min(out.position.z + 0.78 * sample_depth + 0.00000012, 1.0);
    return out;
}
@fragment fn fs_trail(input:Out)->@location(0) vec4<f32> {
    return vec4(TRAIL_COLOR,TRAIL_ALPHA+input.color.w*0.000001);
}
@fragment fn fs_depth_trail()->@location(0) vec4<f32> {
    let alpha = 0.70;
    return vec4(mix(CANVAS, TRAIL_COLOR, TRAIL_ALPHA / alpha), alpha);
}

struct WireHalo {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) @interpolate(flat) depths: vec2<f32>,
    @location(2) @interpolate(flat) length: f32,
    @location(3) @interpolate(flat) normal_depth: f32,
    @location(4) @interpolate(flat) margin: f32,
    @location(5) @interpolate(flat) taper: f32,
}

@vertex fn vs_wire_halo(
    @location(0) start: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) end: vec3<f32>,
    @builtin(vertex_index) index: u32,
) -> WireHalo {
    let corners = array<vec2<f32>, 6>(
        vec2(0.0, -1.0), vec2(1.0, -1.0), vec2(0.0, 1.0),
        vec2(0.0, 1.0), vec2(1.0, -1.0), vec2(1.0, 1.0),
    );
    let a = to_clip(start);
    let b = to_clip(end);
    let pixel = camera.flags.yz;
    let span = (b.xy - a.xy) / pixel;
    let span_length = length(span);
    let tangent = span / max(span_length, 0.0001);
    let normal = vec2(-tangent.y, tangent.x);
    let corner = corners[index];
    let radius = 4.0;
    let local = vec2(span_length * corner.x, corner.y * radius);
    let offset = (local.x * tangent + local.y * normal) * pixel;
    var position = vec4(a.xy + offset, 0.5, 1.0);
    let depths = vec2(path_depth(a.z, color), path_depth(b.z, color));
    let on_plane = camera.xy_depth.w > 0.5 && abs(start.z - end.z) < 0.000001;
    let gradient = select(vec2(0.0), camera.xy_depth.xy, on_plane);
    let normal_depth = dot(gradient, normal);
    let slope = abs(depths.y - depths.x) / max(span_length, 1.0);
    let margin = select(1.1 * slope, 1.4 * length(gradient), on_plane) * camera.xy_depth.z;
    if is_rapid(color) && camera.flags.x < 0.5 {
        position = vec4(2.0, 2.0, 0.5, 1.0);
    }
    return WireHalo(position, local, depths, span_length, normal_depth, margin, select(1.0, 0.0, on_plane));
}

struct HaloFragment {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
}

@fragment fn fs_wire_halo(wire: WireHalo) -> HaloFragment {
    let along = clamp(wire.local.x, 0.0, wire.length);
    let dist = abs(wire.local.y);
    if dist < 1.1 || dist >= 4.0 { discard; }
    let fraction = along / max(wire.length, 0.0001);
    // Project the halo onto the stroke's cutting plane where it has one. This
    // keeps a run's edge or its next segment from cutting a gap in that run.
    let depth = mix(wire.depths.x, wire.depths.y, fraction)
        + wire.normal_depth * wire.local.y + wire.margin + 0.000001;
    // End the strip where the stroke ends. Ease out at sloped joins so the
    // neighboring segment can meet it even when their depth gradients differ.
    let to_end = min(along, wire.length - along);
    let strength = (1.0 - smoothstep(2.4, 4.0, dist))
        * mix(1.0, smoothstep(0.0, 1.6, to_end), wire.taper);
    return HaloFragment(vec4(CANVAS, strength), clamp(depth, 0.0, 1.0));
}

@group(0) @binding(1) var scene: texture_2d<f32>;
@group(0) @binding(2) var scene_depth: texture_depth_multisampled_2d;

@vertex fn vs_halo(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let triangle = array<vec2<f32>, 3>(vec2(-1.0,-1.0), vec2(3.0,-1.0), vec2(-1.0,3.0));
    return vec4(triangle[index], 0.0, 1.0);
}

fn nearest_depth(pixel: vec2<i32>) -> f32 {
    return min(
        min(textureLoad(scene_depth, pixel, 0), textureLoad(scene_depth, pixel, 1)),
        min(textureLoad(scene_depth, pixel, 2), textureLoad(scene_depth, pixel, 3)),
    );
}

@fragment fn fs_halo(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(position.xy);
    let color = textureLoad(scene, pixel, 0);
    // The translucent sweep writes its coverage to alpha. Wire halos are drawn
    // from their own geometry, so neighboring wire pixels cannot blur a cut.
    if color.a < 0.999 || all(abs(color.rgb - CANVAS) < vec3(0.002)) {
        return vec4(color.rgb, 1.0);
    }
    let limit = vec2<i32>(textureDimensions(scene)) - vec2(1);
    let center = nearest_depth(pixel);
    let directions = array<vec2<i32>, 8>(
        vec2(1, 0), vec2(-1, 0), vec2(0, 1), vec2(0, -1),
        vec2(1, 1), vec2(-1, -1), vec2(1, -1), vec2(-1, 1),
    );
    var strength = 0.0;
    for (var i = 0; i < 8; i++) {
        let step = directions[i];
        let edge = clamp(pixel + step, vec2(0), limit);
        let inner = clamp(pixel + 2 * step, vec2(0), limit);
        if textureLoad(scene, inner, 0).a < 0.85 {
            let near = nearest_depth(inner);
            let slope = abs(near - nearest_depth(clamp(inner + step, vec2(0), limit)));
            if center - near > max(0.000001, 3.0 * slope) {
                if textureLoad(scene, edge, 0).a < 0.85 {
                    strength = 0.88;
                    break;
                }
                strength = 0.38;
            }
        }
    }
    return vec4(mix(color.rgb, CANVAS, strength), 1.0);
}

@vertex fn vs_point(@location(0) p:vec3<f32>,@location(1) color:vec4<f32>,@builtin(vertex_index) n:u32)->Out {
    var out:Out;
    let corners=array<vec2<f32>,6>(vec2(-1.0,-1.0),vec2(1.0,-1.0),vec2(-1.0,1.0),vec2(-1.0,1.0),vec2(1.0,-1.0),vec2(1.0,1.0));
    let center=to_clip(p);
    out.position=vec4(center.xy+corners[n]*camera.flags.yz*1.5,path_depth(center.z,color),1.0);out.color=color;return out;
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
