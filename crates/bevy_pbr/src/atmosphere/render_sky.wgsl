#import bevy_pbr::atmosphere::{
    types::{Atmosphere, AtmosphereSettings},
    bindings::{atmosphere, view, settings, atmosphere_transforms},
    functions::{
        sample_transmittance_lut, sample_sky_view_lut, 
        direction_world_to_atmosphere, uv_to_ray_direction, uv_to_ndc,
        sample_aerial_view_lut, view_radius,
        sample_sun_illuminance, max_atmosphere_distance,
        raymarch_atmosphere,
    },
};
#import bevy_render::view::View;

#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

#ifdef MULTISAMPLED
@group(0) @binding(13) var depth_texture: texture_depth_multisampled_2d;
#else
@group(0) @binding(13) var depth_texture: texture_depth_2d;
#endif

@fragment
fn main(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let depth = textureLoad(depth_texture, vec2<i32>(in.position.xy), 0);

    // TODO: this should be user controllable, whether to use per-pixel 
    // raymarching to get volumetric light shafts or the LUT
    // set this to 0.5 to compare with the raymarched reference
    const raymarch_split = 0.5;
    if depth == 0.0 {
        let ray_dir_ws = uv_to_ray_direction(in.uv);
        let ray_dir_as = direction_world_to_atmosphere(ray_dir_ws.xyz);

        let r = view_radius();
        let mu = ray_dir_ws.y;

        let transmittance = sample_transmittance_lut(r, mu);
        
        var inscattering = vec3(0.0);
        if (in.uv.x < raymarch_split) {
            let r = view_radius();
            let t_max = max_atmosphere_distance(r, ray_dir_ws.y);
            let sample_count = mix(1.0, f32(settings.sky_view_lut_samples), clamp(t_max * 0.01, 0.0, 1.0));
            
            inscattering = raymarch_atmosphere(r, ray_dir_ws.xyz, t_max, sample_count).inscattering;
        } else {
            inscattering = sample_sky_view_lut(r, ray_dir_as);
        }

        let sun_illuminance = sample_sun_illuminance(ray_dir_ws.xyz, transmittance);
        return vec4(inscattering + sun_illuminance, (transmittance.r + transmittance.g + transmittance.b) / 3.0);
    } else {
        if (in.uv.x < raymarch_split) {
            let ray_dir_ws = uv_to_ray_direction(in.uv);
            let view_pos = view.view_from_clip * vec4(uv_to_ndc(in.uv), depth, 1.0);
            let t_max = length(view_pos.xyz / view_pos.w) * settings.scene_units_to_m;
            let r = view_radius();
            let sample_count = mix(1.0, f32(settings.sky_view_lut_samples), clamp(t_max * 0.01, 0.0, 1.0));
            
            return vec4(raymarch_atmosphere(r, ray_dir_ws.xyz, t_max, 40.0).inscattering, 1.0);
        } else {
            return sample_aerial_view_lut(in.uv, depth);
        }
    }
}
