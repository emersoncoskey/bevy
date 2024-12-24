use crate::renderer::WgpuWrapper;
use crate::{
    define_atomic_id,
    render_asset::RenderAssets,
    render_resource::{BindGroupLayout, Buffer, Sampler, TextureView},
    renderer::RenderDevice,
    texture::GpuImage,
};
use alloc::sync::Arc;
use bevy_derive::{Deref, DerefMut};
use bevy_ecs::system::{SystemParam, SystemParamItem};
pub use bevy_render_macros::AsBindGroup;
use core::ops::Deref;
use encase::ShaderType;
use thiserror::Error;
use wgpu::{BindGroupEntry, BindGroupLayoutEntry, BindingResource, TextureViewDimension};

define_atomic_id!(BindGroupId);

/// Bind groups are responsible for binding render resources (e.g. buffers, textures, samplers)
/// to a [`TrackedRenderPass`](crate::render_phase::TrackedRenderPass).
/// This makes them accessible in the pipeline (shaders) as uniforms.
///
/// May be converted from and dereferences to a wgpu [`BindGroup`](wgpu::BindGroup).
/// Can be created via [`RenderDevice::create_bind_group`](RenderDevice::create_bind_group).
#[derive(Clone, Debug)]
pub struct BindGroup {
    id: BindGroupId,
    value: Arc<WgpuWrapper<wgpu::BindGroup>>,
}

impl BindGroup {
    /// Returns the [`BindGroupId`].
    #[inline]
    pub fn id(&self) -> BindGroupId {
        self.id
    }
}

impl From<wgpu::BindGroup> for BindGroup {
    fn from(value: wgpu::BindGroup) -> Self {
        BindGroup {
            id: BindGroupId::new(),
            value: Arc::new(WgpuWrapper::new(value)),
        }
    }
}

impl<'a> From<&'a BindGroup> for Option<&'a wgpu::BindGroup> {
    fn from(value: &'a BindGroup) -> Self {
        Some(value.deref())
    }
}

impl<'a> From<&'a mut BindGroup> for Option<&'a wgpu::BindGroup> {
    fn from(value: &'a mut BindGroup) -> Self {
        Some(&*value)
    }
}

impl Deref for BindGroup {
    type Target = wgpu::BindGroup;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

/// Converts a value to a [`BindGroup`] with a given [`BindGroupLayout`], which can then be used in Bevy shaders.
/// This trait can be derived (and generally should be). Read on for details and examples.
///
/// This is an opinionated trait that is intended to make it easy to generically
/// convert a type into a [`BindGroup`]. It provides access to specific render resources,
/// such as [`RenderAssets<GpuImage>`] and [`crate::texture::FallbackImage`]. If a type has a [`Handle<Image>`](bevy_asset::Handle),
/// these can be used to retrieve the corresponding [`Texture`](crate::render_resource::Texture) resource.
///
/// [`AsBindGroup::as_bind_group`] is intended to be called once, then the result cached somewhere. It is generally
/// ok to do "expensive" work here, such as creating a [`Buffer`] for a uniform.
///
/// If for some reason a [`BindGroup`] cannot be created yet (for example, the [`Texture`](crate::render_resource::Texture)
/// for an [`Image`](bevy_image::Image) hasn't loaded yet), just return [`AsBindGroupError::RetryNextUpdate`], which signals that the caller
/// should retry again later.
///
/// # Deriving
///
/// This trait can be derived. Field attributes like `uniform` and `texture` are used to define which fields should be bindings,
/// what their binding type is, and what index they should be bound at:
///
/// ```
/// # use bevy_render::render_resource::*;
/// # use bevy_image::Image;
/// # use bevy_color::LinearRgba;
/// # use bevy_asset::Handle;
/// # use bevy_render::storage::ShaderStorageBuffer;
///
/// #[derive(AsBindGroup)]
/// struct CoolMaterial {
///     #[uniform(0)]
///     color: LinearRgba,
///     #[texture(1)]
///     #[sampler(2)]
///     color_texture: Handle<Image>,
///     #[storage(3, read_only)]
///     storage_buffer: Handle<ShaderStorageBuffer>,
///     #[storage(4, read_only, buffer)]
///     raw_buffer: Buffer,
///     #[storage_texture(5)]
///     storage_texture: Handle<Image>,
/// }
/// ```
///
/// In WGSL shaders, the binding would look like this:
///
/// ```wgsl
/// @group(2) @binding(0) var<uniform> color: vec4<f32>;
/// @group(2) @binding(1) var color_texture: texture_2d<f32>;
/// @group(2) @binding(2) var color_sampler: sampler;
/// @group(2) @binding(3) var<storage> storage_buffer: array<f32>;
/// @group(2) @binding(4) var<storage> raw_buffer: array<f32>;
/// @group(2) @binding(5) var storage_texture: texture_storage_2d<rgba8unorm, read_write>;
/// ```
/// Note that the "group" index is determined by the usage context. It is not defined in [`AsBindGroup`]. For example, in Bevy material bind groups
/// are generally bound to group 2.
///
/// The following field-level attributes are supported:
///
/// * `uniform(BINDING_INDEX)`
///     * The field will be converted to a shader-compatible type using the [`ShaderType`] trait, written to a [`Buffer`], and bound as a uniform.
///         [`ShaderType`] is implemented for most math types already, such as [`f32`], [`Vec4`](bevy_math::Vec4), and
///         [`LinearRgba`](bevy_color::LinearRgba). It can also be derived for custom structs.
///
/// * `texture(BINDING_INDEX, arguments)`
///     * This field's [`Handle<Image>`](bevy_asset::Handle) will be used to look up the matching [`Texture`](crate::render_resource::Texture)
///         GPU resource, which will be bound as a texture in shaders. The field will be assumed to implement [`Into<Option<Handle<Image>>>`]. In practice,
///         most fields should be a [`Handle<Image>`](bevy_asset::Handle) or [`Option<Handle<Image>>`]. If the value of an [`Option<Handle<Image>>`] is
///         [`None`], the [`crate::texture::FallbackImage`] resource will be used instead. This attribute can be used in conjunction with a `sampler` binding attribute
///         (with a different binding index) if a binding of the sampler for the [`Image`](bevy_image::Image) is also required.
///
/// | Arguments             | Values                                                                  | Default              |
/// |-----------------------|-------------------------------------------------------------------------|----------------------|
/// | `dimension` = "..."   | `"1d"`, `"2d"`, `"2d_array"`, `"3d"`, `"cube"`, `"cube_array"`          | `"2d"`               |
/// | `sample_type` = "..." | `"float"`, `"depth"`, `"s_int"` or `"u_int"`                            | `"float"`            |
/// | `filterable` = ...    | `true`, `false`                                                         | `true`               |
/// | `multisampled` = ...  | `true`, `false`                                                         | `false`              |
/// | `visibility(...)`     | `all`, `none`, or a list-combination of `vertex`, `fragment`, `compute` | `vertex`, `fragment` |
///
/// * `storage_texture(BINDING_INDEX, arguments)`
///     * This field's [`Handle<Image>`](bevy_asset::Handle) will be used to look up the matching [`Texture`](crate::render_resource::Texture)
///         GPU resource, which will be bound as a storage texture in shaders. The field will be assumed to implement [`Into<Option<Handle<Image>>>`]. In practice,
///         most fields should be a [`Handle<Image>`](bevy_asset::Handle) or [`Option<Handle<Image>>`]. If the value of an [`Option<Handle<Image>>`] is
///         [`None`], the [`crate::texture::FallbackImage`] resource will be used instead.
///
/// | Arguments              | Values                                                                                     | Default       |
/// |------------------------|--------------------------------------------------------------------------------------------|---------------|
/// | `dimension` = "..."    | `"1d"`, `"2d"`, `"2d_array"`, `"3d"`, `"cube"`, `"cube_array"`                             | `"2d"`        |
/// | `image_format` = ...   | any member of [`TextureFormat`](crate::render_resource::TextureFormat)                     | `Rgba8Unorm`  |
/// | `access` = ...         | any member of [`StorageTextureAccess`](crate::render_resource::StorageTextureAccess)       | `ReadWrite`   |
/// | `visibility(...)`      | `all`, `none`, or a list-combination of `vertex`, `fragment`, `compute`                    | `compute`     |
///
/// * `sampler(BINDING_INDEX, arguments)`
///     * This field's [`Handle<Image>`](bevy_asset::Handle) will be used to look up the matching [`Sampler`] GPU
///         resource, which will be bound as a sampler in shaders. The field will be assumed to implement [`Into<Option<Handle<Image>>>`]. In practice,
///         most fields should be a [`Handle<Image>`](bevy_asset::Handle) or [`Option<Handle<Image>>`]. If the value of an [`Option<Handle<Image>>`] is
///         [`None`], the [`crate::texture::FallbackImage`] resource will be used instead. This attribute can be used in conjunction with a `texture` binding attribute
///         (with a different binding index) if a binding of the texture for the [`Image`](bevy_image::Image) is also required.
///
/// | Arguments              | Values                                                                  | Default                |
/// |------------------------|-------------------------------------------------------------------------|------------------------|
/// | `sampler_type` = "..." | `"filtering"`, `"non_filtering"`, `"comparison"`.                       |  `"filtering"`         |
/// | `visibility(...)`      | `all`, `none`, or a list-combination of `vertex`, `fragment`, `compute` |   `vertex`, `fragment` |
/// * `storage(BINDING_INDEX, arguments)`
///     * The field's [`Handle<Storage>`](bevy_asset::Handle) will be used to look up the matching [`Buffer`] GPU resource, which
///       will be bound as a storage buffer in shaders. If the `storage` attribute is used, the field is expected a raw
///       buffer, and the buffer will be bound as a storage buffer in shaders.
///     * It supports an optional `read_only` parameter. Defaults to false if not present.
///
/// | Arguments              | Values                                                                  | Default              |
/// |------------------------|-------------------------------------------------------------------------|----------------------|
/// | `visibility(...)`      | `all`, `none`, or a list-combination of `vertex`, `fragment`, `compute` | `vertex`, `fragment` |
/// | `read_only`            | if present then value is true, otherwise false                          | `false`              |
/// | `buffer`               | if present then the field will be assumed to be a raw wgpu buffer       |                      |
///
/// Note that fields without field-level binding attributes will be ignored.
/// ```
/// # use bevy_render::{render_resource::AsBindGroup};
/// # use bevy_color::LinearRgba;
/// # use bevy_asset::Handle;
/// #[derive(AsBindGroup)]
/// struct CoolMaterial {
///     #[uniform(0)]
///     color: LinearRgba,
///     this_field_is_ignored: String,
/// }
/// ```
///
///  As mentioned above, [`Option<Handle<Image>>`] is also supported:
/// ```
/// # use bevy_asset::Handle;
/// # use bevy_color::LinearRgba;
/// # use bevy_image::Image;
/// # use bevy_render::render_resource::AsBindGroup;
/// #[derive(AsBindGroup)]
/// struct CoolMaterial {
///     #[uniform(0)]
///     color: LinearRgba,
///     #[texture(1)]
///     #[sampler(2)]
///     color_texture: Option<Handle<Image>>,
/// }
/// ```
/// This is useful if you want a texture to be optional. When the value is [`None`], the [`crate::texture::FallbackImage`] will be used for the binding instead, which defaults
/// to "pure white".
///
/// Field uniforms with the same index will be combined into a single binding:
/// ```
/// # use bevy_render::{render_resource::AsBindGroup};
/// # use bevy_color::LinearRgba;
/// #[derive(AsBindGroup)]
/// struct CoolMaterial {
///     #[uniform(0)]
///     color: LinearRgba,
///     #[uniform(0)]
///     roughness: f32,
/// }
/// ```
///
/// In WGSL shaders, the binding would look like this:
/// ```wgsl
/// struct CoolMaterial {
///     color: vec4<f32>,
///     roughness: f32,
/// };
///
/// @group(2) @binding(0) var<uniform> material: CoolMaterial;
/// ```
///
/// Some less common scenarios will require "struct-level" attributes. These are the currently supported struct-level attributes:
/// * `uniform(BINDING_INDEX, ConvertedShaderType)`
///     * This also creates a [`Buffer`] using [`ShaderType`] and binds it as a uniform, much
///         like the field-level `uniform` attribute. The difference is that the entire [`AsBindGroup`] value is converted to `ConvertedShaderType`,
///         which must implement [`ShaderType`], instead of a specific field implementing [`ShaderType`]. This is useful if more complicated conversion
///         logic is required. The conversion is done using the [`AsBindGroupShaderType<ConvertedShaderType>`] trait, which is automatically implemented
///         if `&Self` implements [`Into<ConvertedShaderType>`]. Only use [`AsBindGroupShaderType`] if access to resources like [`RenderAssets<GpuImage>`] is
///         required.
/// * `bind_group_data(DataType)`
///     * The [`AsBindGroup`] type will be converted to some `DataType` using [`Into<DataType>`] and stored
///         as [`AsBindGroup::Data`] as part of the [`AsBindGroup::as_bind_group`] call. This is useful if data needs to be stored alongside
///         the generated bind group, such as a unique identifier for a material's bind group. The most common use case for this attribute
///         is "shader pipeline specialization". See [`SpecializedRenderPipeline`](crate::render_resource::SpecializedRenderPipeline).
/// * `bindless(COUNT)`
///     * This switch enables *bindless resources*, which changes the way Bevy
///       supplies resources (uniforms, textures, and samplers) to the shader.
///       When bindless resources are enabled, and the current platform supports
///       them, instead of presenting a single instance of a resource to your
///       shader Bevy will instead present a *binding array* of `COUNT` elements.
///       In your shader, the index of the element of each binding array
///       corresponding to the mesh currently being drawn can be retrieved with
///       `mesh[in.instance_index].material_bind_group_slot`.
///     * Bindless uniforms don't exist, so in bindless mode all uniforms and
///       uniform buffers are automatically replaced with read-only storage
///       buffers.
///     * The purpose of bindless mode is to improve performance by reducing
///       state changes. By grouping resources together into binding arrays, Bevy
///       doesn't have to modify GPU state as often, decreasing API and driver
///       overhead.
///     * If bindless mode is enabled, the `BINDLESS` definition will be
///       available. Because not all platforms support bindless resources, you
///       should check for the presence of this definition via `#ifdef` and fall
///       back to standard bindings if it isn't present.
///     * See the `shaders/shader_material_bindless` example for an example of
///       how to use bindless mode.
///
/// The previous `CoolMaterial` example illustrating "combining multiple field-level uniform attributes with the same binding index" can
/// also be equivalently represented with a single struct-level uniform attribute:
/// ```
/// # use bevy_render::{render_resource::{AsBindGroup, ShaderType}};
/// # use bevy_color::LinearRgba;
/// #[derive(AsBindGroup)]
/// #[uniform(0, CoolMaterialUniform)]
/// struct CoolMaterial {
///     color: LinearRgba,
///     roughness: f32,
/// }
///
/// #[derive(ShaderType)]
/// struct CoolMaterialUniform {
///     color: LinearRgba,
///     roughness: f32,
/// }
///
/// impl From<&CoolMaterial> for CoolMaterialUniform {
///     fn from(material: &CoolMaterial) -> CoolMaterialUniform {
///         CoolMaterialUniform {
///             color: material.color,
///             roughness: material.roughness,
///         }
///     }
/// }
/// ```
///
/// Setting `bind_group_data` looks like this:
/// ```
/// # use bevy_render::{render_resource::AsBindGroup};
/// # use bevy_color::LinearRgba;
/// #[derive(AsBindGroup)]
/// #[bind_group_data(CoolMaterialKey)]
/// struct CoolMaterial {
///     #[uniform(0)]
///     color: LinearRgba,
///     is_shaded: bool,
/// }
///
/// #[derive(Copy, Clone, Hash, Eq, PartialEq)]
/// struct CoolMaterialKey {
///     is_shaded: bool,
/// }
///
/// impl From<&CoolMaterial> for CoolMaterialKey {
///     fn from(material: &CoolMaterial) -> CoolMaterialKey {
///         CoolMaterialKey {
///             is_shaded: material.is_shaded,
///         }
///     }
/// }
/// ```
pub trait AsBindGroup {
    /// Data that will be stored alongside the "prepared" bind group.
    type Data: Send + Sync;

    type Param: SystemParam + 'static;

    /// The number of slots per bind group, if bindless mode is enabled.
    ///
    /// If this bind group doesn't use bindless, then this will be `None`.
    ///
    /// Note that the *actual* slot count may be different from this value, due
    /// to platform limitations. For example, if bindless resources aren't
    /// supported on this platform, the actual slot count will be 1.
    const BINDLESS_SLOT_COUNT: Option<u32> = None;

    /// label
    fn label() -> Option<&'static str> {
        None
    }

    /// Creates a bind group for `self` matching the layout defined in [`AsBindGroup::bind_group_layout`].
    fn as_bind_group(
        &self,
        layout: &BindGroupLayout,
        render_device: &RenderDevice,
        param: &mut SystemParamItem<'_, '_, Self::Param>,
    ) -> Result<PreparedBindGroup<Self::Data>, AsBindGroupError> {
        let UnpreparedBindGroup { bindings, data } =
            Self::unprepared_bind_group(self, layout, render_device, param)?;

        let entries = bindings
            .iter()
            .map(|(index, binding)| BindGroupEntry {
                binding: *index,
                resource: binding.get_binding(),
            })
            .collect::<Vec<_>>();

        let bind_group = render_device.create_bind_group(Self::label(), layout, &entries);

        Ok(PreparedBindGroup {
            bindings,
            bind_group,
            data,
        })
    }

    /// Returns a vec of (binding index, `OwnedBindingResource`).
    /// In cases where `OwnedBindingResource` is not available (as for bindless texture arrays currently),
    /// an implementor may return `AsBindGroupError::CreateBindGroupDirectly`
    /// from this function and instead define `as_bind_group` directly. This may
    /// prevent certain features, such as bindless mode, from working correctly.
    fn unprepared_bind_group(
        &self,
        layout: &BindGroupLayout,
        render_device: &RenderDevice,
        param: &mut SystemParamItem<'_, '_, Self::Param>,
    ) -> Result<UnpreparedBindGroup<Self::Data>, AsBindGroupError>;

    /// Creates the bind group layout matching all bind groups returned by [`AsBindGroup::as_bind_group`]
    fn bind_group_layout(render_device: &RenderDevice) -> BindGroupLayout
    where
        Self: Sized,
    {
        render_device.create_bind_group_layout(
            Self::label(),
            &Self::bind_group_layout_entries(render_device),
        )
    }

    /// Returns a vec of bind group layout entries
    fn bind_group_layout_entries(render_device: &RenderDevice) -> Vec<BindGroupLayoutEntry>
    where
        Self: Sized;
}

/// An error that occurs during [`AsBindGroup::as_bind_group`] calls.
#[derive(Debug, Error)]
pub enum AsBindGroupError {
    /// The bind group could not be generated. Try again next frame.
    #[error("The bind group could not be generated")]
    RetryNextUpdate,
    #[error("Create the bind group via `as_bind_group()` instead")]
    CreateBindGroupDirectly,
    #[error("At binding index {0}, the provided image sampler `{1}` does not match the required sampler type(s) `{2}`.")]
    InvalidSamplerType(u32, String, String),
}

/// A prepared bind group returned as a result of [`AsBindGroup::as_bind_group`].
pub struct PreparedBindGroup<T> {
    pub bindings: BindingResources,
    pub bind_group: BindGroup,
    pub data: T,
}

/// a map containing `OwnedBindingResource`s, keyed by the target binding index
pub struct UnpreparedBindGroup<T> {
    pub bindings: BindingResources,
    pub data: T,
}

/// A pair of binding index and binding resource, used as part of
/// [`PreparedBindGroup`] and [`UnpreparedBindGroup`].
#[derive(Deref, DerefMut)]
pub struct BindingResources(pub Vec<(u32, OwnedBindingResource)>);

/// An owned binding resource of any type (ex: a [`Buffer`], [`TextureView`], etc).
/// This is used by types like [`PreparedBindGroup`] to hold a single list of all
/// render resources used by bindings.
#[derive(Debug)]
pub enum OwnedBindingResource {
    Buffer(Buffer),
    TextureView(TextureViewDimension, TextureView),
    Sampler(Sampler),
}

impl OwnedBindingResource {
    pub fn get_binding(&self) -> BindingResource {
        match self {
            OwnedBindingResource::Buffer(buffer) => buffer.as_entire_binding(),
            OwnedBindingResource::TextureView(_, view) => BindingResource::TextureView(view),
            OwnedBindingResource::Sampler(sampler) => BindingResource::Sampler(sampler),
        }
    }
}

/// Converts a value to a [`ShaderType`] for use in a bind group.
///
/// This is automatically implemented for references that implement [`Into`].
/// Generally normal [`Into`] / [`From`] impls should be preferred, but
/// sometimes additional runtime metadata is required.
/// This exists largely to make some [`AsBindGroup`] use cases easier.
pub trait AsBindGroupShaderType<T: ShaderType> {
    /// Return the `T` [`ShaderType`] for `self`. When used in [`AsBindGroup`]
    /// derives, it is safe to assume that all images in `self` exist.
    fn as_bind_group_shader_type(&self, images: &RenderAssets<GpuImage>) -> T;
}

impl<T, U: ShaderType> AsBindGroupShaderType<U> for T
where
    for<'a> &'a T: Into<U>,
{
    #[inline]
    fn as_bind_group_shader_type(&self, _images: &RenderAssets<GpuImage>) -> U {
        self.into()
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate as bevy_render;
    use bevy_asset::Handle;
    use bevy_image::Image;

    #[test]
    fn texture_visibility() {
        #[derive(AsBindGroup)]
        pub struct TextureVisibilityTest {
            #[texture(0, visibility(all))]
            pub all: Handle<Image>,
            #[texture(1, visibility(none))]
            pub none: Handle<Image>,
            #[texture(2, visibility(fragment))]
            pub fragment: Handle<Image>,
            #[texture(3, visibility(vertex))]
            pub vertex: Handle<Image>,
            #[texture(4, visibility(compute))]
            pub compute: Handle<Image>,
            #[texture(5, visibility(vertex, fragment))]
            pub vertex_fragment: Handle<Image>,
            #[texture(6, visibility(vertex, compute))]
            pub vertex_compute: Handle<Image>,
            #[texture(7, visibility(fragment, compute))]
            pub fragment_compute: Handle<Image>,
            #[texture(8, visibility(vertex, fragment, compute))]
            pub vertex_fragment_compute: Handle<Image>,
        }
    }
}
