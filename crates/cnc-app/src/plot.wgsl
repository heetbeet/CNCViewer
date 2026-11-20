struct Camera { right:vec4<f32>, up:vec4<f32>, flags:vec4<f32> }
@group(0) @binding(0) var<uniform> camera:Camera;
struct Out { @builtin(position) position:vec4<f32>, @location(0) color:vec4<f32> }
@vertex fn vs(@location(0) p:vec3<f32>,@location(1) color:vec4<f32>)->Out {
    var out:Out;
    out.position=vec4(dot(vec4(p,1.0),camera.right),dot(vec4(p,1.0),camera.up),0.0,1.0);
    out.color=color;
    return out;
}
@fragment fn fs(input:Out)->@location(0) vec4<f32> {
    if(camera.flags.x<0.5 && input.color.a<0.9 && input.color.r>0.9 && input.color.g>0.2 && input.color.b<0.1){discard;}
    return input.color;
}
@vertex fn vs_quad(@builtin(vertex_index) n:u32)->Out {
    var out:Out;
    let p=array<vec2<f32>,3>(vec2(-1.0,-1.0),vec2(3.0,-1.0),vec2(-1.0,3.0));
    out.position=vec4(p[n],0.0,1.0);
    out.color=camera.flags;
    return out;
}
@fragment fn fs_trail(input:Out)->@location(0) vec4<f32> {
    return vec4(0.2784,0.6314,0.5176,0.28+input.color.w*0.000001);
}

@vertex fn vs_point(@location(0) p:vec3<f32>,@location(1) color:vec4<f32>,@builtin(vertex_index) n:u32)->Out {
    var out:Out;
    let corners=array<vec2<f32>,6>(vec2(-1.0,-1.0),vec2(1.0,-1.0),vec2(-1.0,1.0),vec2(-1.0,1.0),vec2(1.0,-1.0),vec2(1.0,1.0));
    let center=vec2(dot(vec4(p,1.0),camera.right),dot(vec4(p,1.0),camera.up));
    out.position=vec4(center+corners[n]*camera.flags.yz*1.5,0.0,1.0);out.color=color;return out;
}
