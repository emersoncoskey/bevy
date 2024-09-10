#import bevy_pbr::{
    mesh_view_types::{Lights, DirectionalLight},
    atmosphere::{
        types::{Atmosphere, AtmosphereSettings},
        common::uv_to_r_mu,
    }
}

@group(0) @binding(0) var<uniform> atmosphere: Atmosphere;
@group(0) @binding(1) var<uniform> settings: AtmosphereSettings;
@group(0) @binding(2) var<uniform> lights: Lights;
@group(0) @binding(3) var tranmittance_lut: texture_2d<f32>;
@group(0) @binding(4) var multiscattering_lut: texture_storage_2d<rgba16float, write>;

fn s2_sequence(n: u32) -> vec2<f32> {
//    const phi_2 = vec2(1.3247179572447460259609088, 1.7548776662466927600495087);
//    fract(0.5 + phi_2 * n);
    return vec2(0.0, 0.0);
}

//Lambert equal-area projection. 
fn map_to_sphere(uv: vec2<f32>) -> vec3<f32> {
    return vec3(0.0, 0.0, 0.0); //TODO
}

@compute 
@workgroup_size(16, 16, 1)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let uv: vec2<f32> = (vec2<f32>(global_id.xy) + 0.5) / vec2<f32>(settings.multiscattering_lut_size);
    let r = mix(atmosphere.bottom_radius, atmosphere.top_radius, uv.y);
    for (let sphere_sample_index: u32 = 0u; sphere_sample_index < settings.multiscattering_lut_dirs; sphere_sample_index++) {
        let dir = map_to_sphere(s2_sequence(sphere_sample_index));
        let view_dir = -dir;
        let mu = dir.y; // cos(azimuth_angle) = dot(vec3::up, dir);

        for (let step_index: u32 = 0u; step_index < settings.multiscattering_lut_samples; step_index++) {
        }
    }
}
