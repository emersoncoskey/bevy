//! Procedural Atmospheric Scattering.
//!
//! This plugin implements [Hillaire's 2020 paper](https://sebh.github.io/publications/egsr2020.pdf)
//! on real-time atmospheric scattering. While it *will* work simply as a
//! procedural skybox, it also does much more. It supports dynamic time-of-
//! -day, multiple directional lights, and since it's applied as a post-processing
//! effect *on top* of the existing skybox, a starry skybox would automatically
//! show based on the time of day. Scattering in front of terrain (similar
//! to distance fog, but more complex) is handled as well, and takes into
//! account the directional light color and direction.
//!
//! Adding the [`Atmosphere`] component to a 3d camera will enable the effect,
//! which by default is set to look similar to Earth's atmosphere. See the
//! documentation on the component itself for information regarding its fields.
//!
//! Performance-wise, the effect should be fairly cheap since the LUTs (Look
//! Up Tables) that encode most of the data are small, and take advantage of the
//! fact that the atmosphere is symmetric. Performance is also proportional to
//! the number of directional lights in the scene. In order to tune
//! performance more finely, the [`AtmosphereSettings`] camera component
//! manages the size of each LUT and the sample count for each ray.
//!
//! Given how similar it is to [`crate::volumetric_fog`], it might be expected
//! that these two modules would work together well. However for now using both
//! at once is untested, and might not be physically accurate. These may be
//! integrated into a single module in the future.
//!
//! [Shadertoy]: https://www.shadertoy.com/view/slSXRW
//!
//! [Unreal Engine Implementation]: https://github.com/sebh/UnrealEngineSkyAtmosphere

mod node;
pub mod resources;

use bevy_app::{App, Plugin};
use bevy_asset::load_internal_asset;
use bevy_core_pipeline::core_3d::graph::Node3d;
use bevy_ecs::{
    component::Component,
    query::{Changed, QueryItem, With},
    schedule::IntoSystemConfigs,
    system::{lifetimeless::Read, Query},
};
use bevy_math::{UVec2, UVec3, Vec3, Vec4};
use bevy_reflect::Reflect;
use bevy_render::{
    extract_component::UniformComponentPlugin,
    render_resource::{DownlevelFlags, ShaderType, SpecializedRenderPipelines},
};
use bevy_render::{
    extract_component::{ExtractComponent, ExtractComponentPlugin},
    render_graph::{RenderGraphApp, ViewNodeRunner},
    render_resource::{Shader, TextureFormat, TextureUsages},
    renderer::RenderAdapter,
    Render, RenderApp, RenderSet,
};

use bevy_core_pipeline::core_3d::{graph::Core3d, Camera3d};
use resources::{
    prepare_atmosphere_transforms, queue_render_sky_pipelines, AtmosphereTransforms,
    RenderSkyBindGroupLayouts,
};
use tracing::warn;

use self::{
    node::{AtmosphereLutsNode, AtmosphereNode, RenderSkyNode},
    resources::{
        prepare_atmosphere_bind_groups, prepare_atmosphere_textures, AtmosphereBindGroupLayouts,
        AtmosphereLutPipelines, AtmosphereSamplers,
    },
};

mod shaders {
    use bevy_asset::Handle;
    use bevy_render::render_resource::Shader;

    pub const TYPES: Handle<Shader> = Handle::weak_from_u128(0xB4CA686B10FA592B508580CCC2F9558C);
    pub const FUNCTIONS: Handle<Shader> =
        Handle::weak_from_u128(0xD5524FD88BDC153FBF256B7F2C21906F);
    pub const BRUNETON_FUNCTIONS: Handle<Shader> =
        Handle::weak_from_u128(0x7E896F48B707555DD11985F9C1594459);
    pub const BINDINGS: Handle<Shader> = Handle::weak_from_u128(0x140EFD89B5D4C8490AB895010DFC42FE);

    pub const TRANSMITTANCE_LUT: Handle<Shader> =
        Handle::weak_from_u128(0xEECBDEDFEED7F4EAFBD401BFAA5E0EFB);
    pub const MULTISCATTERING_LUT: Handle<Shader> =
        Handle::weak_from_u128(0x65915B32C44B6287C0CCE1E70AF2936A);
    pub const SKY_VIEW_LUT: Handle<Shader> =
        Handle::weak_from_u128(0x54136D7E6FFCD45BE38399A4E5ED7186);
    pub const AERIAL_VIEW_LUT: Handle<Shader> =
        Handle::weak_from_u128(0x6FDEC284AD356B78C3A4D8ED4CBA0BC5);
    pub const RENDER_SKY: Handle<Shader> =
        Handle::weak_from_u128(0x1951EB87C8A6129F0B541B1E4B3D4962);
}

#[doc(hidden)]
pub struct AtmospherePlugin;

impl Plugin for AtmospherePlugin {
    fn build(&self, app: &mut App) {
        load_internal_asset!(app, shaders::TYPES, "types.wgsl", Shader::from_wgsl);
        load_internal_asset!(app, shaders::FUNCTIONS, "functions.wgsl", Shader::from_wgsl);
        load_internal_asset!(
            app,
            shaders::BRUNETON_FUNCTIONS,
            "bruneton_functions.wgsl",
            Shader::from_wgsl
        );

        load_internal_asset!(app, shaders::BINDINGS, "bindings.wgsl", Shader::from_wgsl);

        load_internal_asset!(
            app,
            shaders::TRANSMITTANCE_LUT,
            "transmittance_lut.wgsl",
            Shader::from_wgsl
        );

        load_internal_asset!(
            app,
            shaders::MULTISCATTERING_LUT,
            "multiscattering_lut.wgsl",
            Shader::from_wgsl
        );

        load_internal_asset!(
            app,
            shaders::SKY_VIEW_LUT,
            "sky_view_lut.wgsl",
            Shader::from_wgsl
        );

        load_internal_asset!(
            app,
            shaders::AERIAL_VIEW_LUT,
            "aerial_view_lut.wgsl",
            Shader::from_wgsl
        );

        load_internal_asset!(
            app,
            shaders::RENDER_SKY,
            "render_sky.wgsl",
            Shader::from_wgsl
        );

        app.register_type::<Atmosphere>()
            .register_type::<AtmosphereSettings>()
            .add_plugins((
                ExtractComponentPlugin::<Atmosphere>::default(),
                ExtractComponentPlugin::<AtmosphereSettings>::default(),
                UniformComponentPlugin::<AtmosphereUniforms>::default(),
                UniformComponentPlugin::<AtmosphereSettings>::default(),
            ));
    }

    fn finish(&self, app: &mut App) {
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };

        let render_adapter = render_app.world().resource::<RenderAdapter>();

        if !render_adapter
            .get_downlevel_capabilities()
            .flags
            .contains(DownlevelFlags::COMPUTE_SHADERS)
        {
            warn!("AtmospherePlugin not loaded. GPU lacks support for compute shaders.");
            return;
        }

        if !render_adapter
            .get_texture_format_features(TextureFormat::Rgba16Float)
            .allowed_usages
            .contains(TextureUsages::STORAGE_BINDING)
        {
            warn!("AtmospherePlugin not loaded. GPU lacks support: TextureFormat::Rgba16Float does not support TextureUsages::STORAGE_BINDING.");
            return;
        }

        render_app
            .init_resource::<AtmosphereBindGroupLayouts>()
            .init_resource::<RenderSkyBindGroupLayouts>()
            .init_resource::<AtmosphereSamplers>()
            .init_resource::<AtmosphereLutPipelines>()
            .init_resource::<AtmosphereTransforms>()
            .init_resource::<SpecializedRenderPipelines<RenderSkyBindGroupLayouts>>()
            .add_systems(
                Render,
                (
                    configure_camera_depth_usages.in_set(RenderSet::ManageViews),
                    queue_render_sky_pipelines.in_set(RenderSet::Queue),
                    prepare_atmosphere_textures.in_set(RenderSet::PrepareResources),
                    prepare_atmosphere_transforms.in_set(RenderSet::PrepareResources),
                    prepare_atmosphere_bind_groups.in_set(RenderSet::PrepareBindGroups),
                ),
            )
            .add_render_graph_node::<ViewNodeRunner<AtmosphereLutsNode>>(
                Core3d,
                AtmosphereNode::RenderLuts,
            )
            .add_render_graph_edges(
                Core3d,
                (
                    // END_PRE_PASSES -> RENDER_LUTS -> MAIN_PASS
                    Node3d::EndPrepasses,
                    AtmosphereNode::RenderLuts,
                    Node3d::StartMainPass,
                ),
            )
            .add_render_graph_node::<ViewNodeRunner<RenderSkyNode>>(
                Core3d,
                AtmosphereNode::RenderSky,
            )
            .add_render_graph_edges(
                Core3d,
                (
                    Node3d::MainOpaquePass,
                    AtmosphereNode::RenderSky,
                    Node3d::MainTransparentPass,
                ),
            );
    }
}

/// The density profile describes how the density of a medium
/// changes with respect to altitude.
#[derive(Clone, Reflect)]
pub enum DensityProfile {
    /// Exponential density profile using a scale height.
    Exponential {
        /// The rate of falloff of particulate with respect to altitude:
        /// optical density = exp(-altitude / scale_height). This value
        /// must be positive.
        ///
        /// units: m
        scale_height: f32,
    },
    /// Tent-shaped density profile using absolute distance from the center altitude.
    Tent {
        /// The altitude at which the density profile reaches its maximum.
        ///
        /// units: m
        center_altitude: f32,

        /// The width of the density profile at the center altitude.
        ///
        /// units: m
        layer_width: f32,

        /// The exponent of the density profile used to shape the density
        /// profile.
        ///
        /// units: N/A
        exponent: f32,
    },
}

/// The phase function describes how light is scattered by a medium
/// given the angle between the incoming and outgoing light.
#[derive(Clone, Reflect)]
pub enum PhaseFunction {
    /// Rayleigh phase function for molecules
    Rayleigh,
    /// Henyey-Greenstein phase function for aerosols
    HenyeyGreenstein(f32),
    /// Cornette-Shanks phase function for aerosols improved over Henyey-Greenstein
    /// also known as Schlick phase function
    CornetteShanks(f32),
    /// Dual Lobe phase function used for simulating backscattering
    /// in water vapor in clouds or ice crystals.
    DualLobe(f32, f32, f32),
}

/// CPU representation of a medium or particulate that interacts with light
#[derive(Clone, Reflect)]
pub struct Medium {
    /// The scattering optical density of the particulate, or how much light
    /// it scatters per meter.
    ///
    /// units: m^-1
    pub scattering: Vec3,

    /// The absorbing optical density of the particulate, or how much light
    /// it absorbs per meter.
    ///
    /// units: m^-1
    pub absorption: Vec3,

    /// The density profile of the medium
    pub density_profile: DensityProfile,

    /// The phase function of the medium
    pub phase_function: PhaseFunction,
}

impl Medium {
    pub const EMPTY: Self = Self {
        scattering: Vec3::ZERO,
        absorption: Vec3::ZERO,
        density_profile: DensityProfile::Tent {
            center_altitude: 0.0,
            layer_width: 0.0,
            exponent: 1.0,
        },
        phase_function: PhaseFunction::Rayleigh,
    };
}

/// GPU representation of a medium
#[derive(Clone, ShaderType)]
pub struct GpuMedium {
    pub scattering: Vec3,
    pub absorption: Vec3,
    pub density_params: Vec4,
    pub phase_params: Vec4,
}

impl From<Medium> for GpuMedium {
    fn from(medium: Medium) -> Self {
        let density_params = match medium.density_profile {
            DensityProfile::Exponential { scale_height } => Vec4::new(scale_height, 0.0, 0.0, 0.0),
            DensityProfile::Tent {
                center_altitude,
                layer_width,
                exponent,
            } => Vec4::new(center_altitude, layer_width, exponent, 1.0),
        };

        let phase_params = match medium.phase_function {
            PhaseFunction::Rayleigh => Vec4::ZERO,
            PhaseFunction::HenyeyGreenstein(g) => Vec4::new(g, 0.0, 0.0, 1.0),
            PhaseFunction::CornetteShanks(g) => Vec4::new(g, 0.0, 0.0, 2.0),
            PhaseFunction::DualLobe(g1, g2, w) => Vec4::new(g1, g2, w, 3.0),
        };

        Self {
            scattering: medium.scattering,
            absorption: medium.absorption,
            density_params,
            phase_params,
        }
    }
}

/// This component describes the atmosphere of a planet, and when added to a camera
/// will enable atmospheric scattering for that camera. This is only compatible with
/// HDR cameras.
///
/// Most atmospheric particles scatter and absorb light in two main ways:
///
/// Rayleigh scattering occurs among very small particles, like individual gas
/// molecules. It's wavelength dependent, and causes colors to separate out as
/// light travels through the atmosphere. These particles *don't* absorb light.
///
/// Mie scattering occurs among slightly larger particles, like dust and sea spray.
/// These particles *do* absorb light, but Mie scattering and absorption is
/// *wavelength independent*.
///
/// Ozone acts differently from the other two, and is special-cased because
/// it's very important to the look of Earth's atmosphere. It's wavelength
/// dependent, but only *absorbs* light. Also, while the density of particles
/// participating in Rayleigh and Mie scattering falls off roughly exponentially
/// from the planet's surface, ozone only exists in a band centered at a fairly
/// high altitude.
#[derive(Clone, Component, Reflect)]
pub struct Atmosphere {
    /// Radius of the planet
    ///
    /// units: m
    pub bottom_radius: f32,

    /// Radius at which we consider the atmosphere to 'end' for our
    /// calculations (from center of planet)
    ///
    /// units: m
    pub top_radius: f32,

    /// An approximation of the average albedo (or color, roughly) of the
    /// planet's surface. This is used when calculating multiscattering.
    ///
    /// units: N/A
    pub ground_albedo: Vec3,

    /// An atmosphere has multiple layers, each composed of a medium that interacts with light
    pub layers: [Medium; 3],
}

#[derive(Clone, Component, ShaderType)]
pub struct AtmosphereUniforms {
    pub bottom_radius: f32,
    pub top_radius: f32,
    pub ground_albedo: Vec3,
    pub layers: [GpuMedium; 3],
}

impl From<Atmosphere> for AtmosphereUniforms {
    fn from(atmosphere: Atmosphere) -> Self {
        Self {
            bottom_radius: atmosphere.bottom_radius,
            top_radius: atmosphere.top_radius,
            ground_albedo: atmosphere.ground_albedo,
            layers: atmosphere.layers.map(GpuMedium::from),
        }
    }
}

impl Atmosphere {
    pub const EARTH: Self = Self {
        bottom_radius: 6_360_000.0,
        top_radius: 6_460_000.0,
        ground_albedo: Vec3::splat(0.0),
        layers: [
            // Rayleigh scattering (air molecules)
            Medium {
                scattering: Vec3::new(5.802e-6, 13.558e-6, 33.100e-6),
                absorption: Vec3::ZERO,
                density_profile: DensityProfile::Exponential {
                    scale_height: 8_000.0,
                },
                phase_function: PhaseFunction::Rayleigh,
            },
            // Mie scattering (aerosols)
            Medium {
                scattering: Vec3::splat(3.996e-6),
                absorption: Vec3::splat(0.444e-6),
                density_profile: DensityProfile::Exponential {
                    scale_height: 1_200.0,
                },
                phase_function: PhaseFunction::CornetteShanks(0.8),
            },
            // Ozone layer
            Medium {
                scattering: Vec3::ZERO,
                absorption: Vec3::new(0.650e-6, 1.881e-6, 0.085e-6),
                density_profile: DensityProfile::Tent {
                    center_altitude: 25_000.0,
                    layer_width: 30_000.0,
                    exponent: 1.0,
                },
                phase_function: PhaseFunction::Rayleigh,
            },
        ],
    };

    pub const MARS: Self = Self {
        bottom_radius: 3_389_500.0,
        top_radius: 3_509_500.0,
        ground_albedo: Vec3::splat(0.1),
        layers: [
            // CO2 (primary atmospheric component on Mars)
            Medium {
                scattering: Vec3::new(0.019918e-03, 0.01357e-03, 0.00575e-03),
                absorption: Vec3::ZERO,
                density_profile: DensityProfile::Exponential {
                    scale_height: 10_430.0,
                },
                phase_function: PhaseFunction::Rayleigh,
            },
            // Dust (Martian aerosols)
            Medium {
                scattering: Vec3::splat(5.361771e-05),
                absorption: Vec3::splat(5.530838e-07),
                density_profile: DensityProfile::Exponential {
                    scale_height: 3_095.0,
                },
                phase_function: PhaseFunction::CornetteShanks(0.85),
            },
            // Empty third layer (Mars has no ozone layer)
            Medium::EMPTY,
        ],
    };

    pub fn with_density_multiplier(mut self, mult: f32) -> Self {
        for layer in self.layers.iter_mut() {
            layer.scattering *= mult;
            layer.absorption *= mult;
        }
        self
    }
}

impl Default for Atmosphere {
    fn default() -> Self {
        Self::EARTH
    }
}

impl ExtractComponent for Atmosphere {
    type QueryData = Read<Atmosphere>;

    type QueryFilter = With<Camera3d>;

    type Out = AtmosphereUniforms;

    fn extract_component(item: QueryItem<'_, Self::QueryData>) -> Option<Self::Out> {
        Some(AtmosphereUniforms::from(item.clone()))
    }
}

/// This component controls the resolution of the atmosphere LUTs, and
/// how many samples are used when computing them.
///
/// The transmittance LUT stores the transmittance from a point in the
/// atmosphere to the outer edge of the atmosphere in any direction,
/// parametrized by the point's radius and the cosine of the zenith angle
/// of the ray.
///
/// The multiscattering LUT stores the factor representing luminance scattered
/// towards the camera with scattering order >2, parametrized by the point's radius
/// and the cosine of the zenith angle of the sun.
///
/// The sky-view lut is essentially the actual skybox, storing the light scattered
/// towards the camera in every direction with a cubemap.
///
/// The aerial-view lut is a 3d LUT fit to the view frustum, which stores the luminance
/// scattered towards the camera at each point (RGB channels), alongside the average
/// transmittance to that point (A channel).
#[derive(Clone, Component, Reflect, ShaderType)]
pub struct AtmosphereSettings {
    /// The size of the transmittance LUT
    pub transmittance_lut_size: UVec2,

    /// The size of the multiscattering LUT
    pub multiscattering_lut_size: UVec2,

    /// The size of the sky-view LUT.
    pub sky_view_lut_size: UVec2,

    /// The size of the aerial-view LUT.
    pub aerial_view_lut_size: UVec3,

    /// The number of points to sample along each ray when
    /// computing the transmittance LUT
    pub transmittance_lut_samples: u32,

    /// The number of rays to sample when computing each
    /// pixel of the multiscattering LUT
    pub multiscattering_lut_dirs: u32,

    /// The number of points to sample when integrating along each
    /// multiscattering ray
    pub multiscattering_lut_samples: u32,

    /// The number of points to sample along each ray when
    /// computing the sky-view LUT.
    pub sky_view_lut_samples: u32,

    /// The number of points to sample for each slice along the z-axis
    /// of the aerial-view LUT.
    pub aerial_view_lut_samples: u32,

    /// The maximum distance from the camera to evaluate the
    /// aerial view LUT. The slices along the z-axis of the
    /// texture will be distributed linearly from the camera
    /// to this value.
    ///
    /// units: m
    pub aerial_view_lut_max_distance: f32,

    /// A conversion factor between scene units and meters, used to
    /// ensure correctness at different length scales.
    pub scene_units_to_m: f32,
}

impl Default for AtmosphereSettings {
    fn default() -> Self {
        Self {
            transmittance_lut_size: UVec2::new(256, 128),
            transmittance_lut_samples: 40,
            multiscattering_lut_size: UVec2::new(32, 32),
            multiscattering_lut_dirs: 64,
            multiscattering_lut_samples: 20,
            sky_view_lut_size: UVec2::new(400, 200),
            sky_view_lut_samples: 16,
            aerial_view_lut_size: UVec3::new(32, 32, 32),
            aerial_view_lut_samples: 10,
            aerial_view_lut_max_distance: 3.2e4,
            scene_units_to_m: 1.0,
        }
    }
}

impl ExtractComponent for AtmosphereSettings {
    type QueryData = Read<AtmosphereSettings>;

    type QueryFilter = (With<Camera3d>, With<Atmosphere>);

    type Out = AtmosphereSettings;

    fn extract_component(item: QueryItem<'_, Self::QueryData>) -> Option<Self::Out> {
        Some(item.clone())
    }
}

fn configure_camera_depth_usages(
    mut cameras: Query<&mut Camera3d, (Changed<Camera3d>, With<AtmosphereUniforms>)>,
) {
    for mut camera in &mut cameras {
        camera.depth_texture_usages.0 |= TextureUsages::TEXTURE_BINDING.bits();
    }
}
