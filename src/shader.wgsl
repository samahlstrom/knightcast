struct Uniform { mvp: mat4x4<f32> };
@group(0) @binding(0) var<uniform> u: Uniform;
@group(0) @binding(1) var tex: texture_2d<f32>;
@group(0) @binding(2) var smp: sampler;
struct Output { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32>, @location(1) color: vec4<f32> };
@vertex fn vs(@location(0) p:vec3<f32>, @location(1) uv:vec2<f32>, @location(2) color:vec4<f32>) -> Output {
 var o:Output; o.pos=u.mvp*vec4(p,1.);o.uv=uv;o.color=color;return o;
}
@fragment fn fs(i:Output)->@location(0) vec4<f32> {
 let c=textureSample(tex,smp,i.uv)*i.color;
 if c.a<0.01 {discard;} return c;
}
