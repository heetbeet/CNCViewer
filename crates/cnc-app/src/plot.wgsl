struct Camera { right:vec4<f32>, up:vec4<f32>, depth:vec4<f32>, flags:vec4<f32> }
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
@vertex fn vs(@location(0) p:vec3<f32>,@location(1) color:vec4<f32>)->Out {
    var out:Out;
    let clip=to_clip(p);
    out.position=vec4(clip.xy,path_depth(clip.z,color),1.0);
    out.color=color;
    return out;
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
@fragment fn fs_trail(input:Out)->@location(0) vec4<f32> {
    return vec4(TRAIL_COLOR,TRAIL_ALPHA+input.color.w*0.000001);
}
@fragment fn fs_depth_trail()->@location(0) vec4<f32> {
    let alpha = 0.70;
    return vec4(mix(CANVAS, TRAIL_COLOR, TRAIL_ALPHA / alpha), alpha);
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

fn is_depth_edge(center: f32, near: f32, opposite: f32) -> bool {
    // Leave a smoothly sloped surface alone. Its depth gradient continues
    // across the pixel, unlike a foreground stroke crossing the surface.
    let gradient = select(abs(opposite - center), 0.0, opposite == 1.0);
    return center - near > max(0.000001, 1.5 * gradient);
}

@fragment fn fs_halo(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(position.xy);
    let color = textureLoad(scene, pixel, 0);
    if all(abs(color.rgb - CANVAS) < vec3(0.002)) {
        return color;
    }
    let limit = vec2<i32>(textureDimensions(scene)) - vec2(1);
    let center = nearest_depth(pixel);
    var halo = 0.0;
    for (var axis = 0; axis < 2; axis++) {
        let step = select(vec2(1, 0), vec2(0, 1), axis == 1);
        let before = nearest_depth(max(pixel - step, vec2(0)));
        let after = nearest_depth(min(pixel + step, limit));
        if is_depth_edge(center, before, after) || is_depth_edge(center, after, before) {
            halo = 0.55;
            break;
        }
        let before_outer = nearest_depth(max(pixel - 2 * step, vec2(0)));
        let after_outer = nearest_depth(min(pixel + 2 * step, limit));
        if is_depth_edge(center, before_outer, after_outer) ||
           is_depth_edge(center, after_outer, before_outer) {
            halo = 0.24;
        }
    }
    return vec4(mix(color.rgb, CANVAS, halo), color.a);
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
