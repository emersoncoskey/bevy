use std::ops::Deref;

use bevy_app::Plugin;
use bevy_asset::{embedded_asset, AssetServer, Handle};
use bevy_color::LinearRgba;
use bevy_render::render_resource::{
    BindGroup, BlendState, ColorTargetState, ColorWrites, FragmentState, LoadOp, Operations,
    RenderPassColorAttachment, RenderPassDescriptor, Shader, StoreOp, TextureView, VertexState,
};

use crate::core::{
    resource::{pipeline::RenderGraphRenderPipelineDescriptor, RenderDependencies, RenderHandle},
    RenderGraphBuilder,
};

pub struct FullscreenPlugin;

impl Plugin for FullscreenPlugin {
    fn build(&self, app: &mut bevy_app::App) {
        embedded_asset!(app, "fullscreen.wgsl");
        embedded_asset!(app, "blit.wgsl");
    }
}

/// uses the [`FULLSCREEN_SHADER_HANDLE`] to output a
/// ```wgsl
/// struct FullscreenVertexOutput {
///     [[builtin(position)]]
///     position: vec4<f32>;
///     [[location(0)]]
///     uv: vec2<f32>;
/// };
/// ```
/// from the vertex shader.
/// The draw call should render one triangle: `render_pass.draw(0..3, 0..1);`
pub fn fullscreen_shader_vertex_state(graph: &RenderGraphBuilder) -> VertexState {
    VertexState {
        shader: graph
            .world_resource::<AssetServer>()
            .load("embedded://bevy_render_graph/std/fullscreen.wgsl"),
        shader_defs: Vec::new(),
        entry_point: "fullscreen_vertex_shader".into(),
        buffers: Vec::new(),
    }
}

pub fn fullscreen_pass<'g>(
    graph: &mut RenderGraphBuilder<'_, 'g>,
    shader: Handle<Shader>,
    target: RenderHandle<'g, TextureView>,
    blend: Option<BlendState>,
    clear_color: Option<LinearRgba>,
    bind_groups: &[RenderHandle<'g, BindGroup>],
) {
    let format = graph
        .meta(target)
        .descriptor
        .format
        .unwrap_or_else(|| graph.meta(graph.meta(target).texture).format);
    let pipeline = graph.new_resource(RenderGraphRenderPipelineDescriptor {
        label: Some("fullscreen_pass_pipeline".into()),
        layout: bind_groups
            .iter()
            .map(|bind_group| graph.meta(*bind_group).descriptor.layout)
            .collect(),
        push_constant_ranges: Vec::new(),
        vertex: fullscreen_shader_vertex_state(graph),
        primitive: Default::default(),
        depth_stencil: Default::default(),
        multisample: Default::default(),
        fragment: Some(FragmentState {
            shader,
            shader_defs: Vec::new(),
            entry_point: "fullscreen_frag".into(),
            targets: vec![Some(ColorTargetState {
                format,
                blend,
                write_mask: ColorWrites::all(),
            })],
        }),
    });

    let should_clear = graph.is_fresh(target);
    let ops = Operations {
        load: if should_clear {
            if let Some(clear_color) = clear_color {
                LoadOp::Clear(clear_color.into())
            } else {
                LoadOp::Load
            }
        } else {
            LoadOp::Load
        },
        store: StoreOp::Store,
    };

    let mut dependencies = RenderDependencies::new();
    dependencies.write(target);
    for bind_group in bind_groups {
        dependencies.add_bind_group(graph, *bind_group);
    }

    graph.add_node(
        Some("fullscreen_pass".into()),
        dependencies,
        move |ctx, cmds, _| {
            let mut render_pass = cmds.begin_render_pass(&RenderPassDescriptor {
                label: Some("fullscreen_pass"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: ctx.get(target).deref(),
                    resolve_target: None,
                    ops,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            render_pass.set_pipeline(ctx.get(pipeline).deref());
            render_pass.draw(0..3, 0..1);
        },
    );
}

pub mod blit {
    use std::ops::Deref;

    use bevy_asset::{AssetServer, Handle};
    use bevy_color::LinearRgba;
    use bevy_render::render_resource::{
        BlendState, ColorTargetState, ColorWrites, FragmentState, LoadOp, Operations,
        RenderPassColorAttachment, RenderPassDescriptor, Sampler, SamplerDescriptor, Shader,
        ShaderStages, StoreOp, TextureView,
    };

    use crate::{
        core::{
            resource::{pipeline::RenderGraphRenderPipelineDescriptor, RenderHandle},
            RenderGraphBuilder,
        },
        deps,
        std::{BindGroupBuilder, SrcDst},
    };

    use super::fullscreen_shader_vertex_state;

    pub fn one<'g>(
        graph: &mut RenderGraphBuilder<'_, 'g>,
        src_dst: SrcDst<'g, TextureView>,
        sampler: Option<RenderHandle<'g, Sampler>>,
        blend: Option<BlendState>,
        clear_color: Option<LinearRgba>,
    ) {
        let shader = graph
            .world_resource::<AssetServer>()
            .load("embedded://bevy_render_graph/std/blit.wgsl");
        custom(graph, shader, src_dst, sampler, blend, clear_color);
    }

    pub fn custom<'g>(
        graph: &mut RenderGraphBuilder<'_, 'g>,
        shader: Handle<Shader>,
        src_dst: SrcDst<'g, TextureView>,
        sampler: Option<RenderHandle<'g, Sampler>>,
        blend: Option<BlendState>,
        clear_color: Option<LinearRgba>,
    ) {
        let sampler = sampler.unwrap_or_else(|| graph.new_resource(SamplerDescriptor::default()));
        let bind_group = BindGroupBuilder::new(
            graph,
            Some("blit_bind_group".into()),
            ShaderStages::FRAGMENT,
        )
        .texture(src_dst.src)
        .sampler(sampler)
        .build();

        let format = graph
            .meta(src_dst.dst)
            .descriptor
            .format
            .unwrap_or_else(|| graph.meta(graph.meta(src_dst.dst).texture).format);
        let pipeline = graph.new_resource(RenderGraphRenderPipelineDescriptor {
            label: Some("blit_pipeline".into()),
            layout: vec![graph.meta(bind_group).descriptor.layout],
            push_constant_ranges: Vec::new(),
            vertex: fullscreen_shader_vertex_state(graph),
            //want that as a dependency
            primitive: Default::default(),
            depth_stencil: Default::default(),
            multisample: Default::default(),
            fragment: Some(FragmentState {
                shader,
                shader_defs: Vec::new(),
                entry_point: "blit_frag".into(),
                targets: vec![Some(ColorTargetState {
                    format,
                    blend,
                    write_mask: ColorWrites::all(),
                })],
            }),
        });

        let should_clear = graph.is_fresh(src_dst.dst);
        let ops = if should_clear {
            if let Some(clear_color) = clear_color {
                Operations {
                    load: LoadOp::Clear(clear_color.into()),
                    store: StoreOp::Store,
                }
            } else {
                Operations {
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                }
            }
        } else {
            Operations {
                load: LoadOp::Load,
                store: StoreOp::Store,
            }
        };

        graph.add_node(
            Some("blit_node".into()),
            deps![src_dst],
            move |ctx, cmds, _| {
                let mut render_pass = cmds.begin_render_pass(&RenderPassDescriptor {
                    label: Some("blit_pass"),
                    color_attachments: &[Some(RenderPassColorAttachment {
                        view: ctx.get(src_dst.dst).deref(),
                        resolve_target: None,
                        ops,
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                render_pass.set_pipeline(ctx.get(pipeline).deref());
                render_pass.set_bind_group(0, ctx.get(bind_group).deref(), &[]);
                render_pass.draw(0..3, 0..1);
            },
        );
    }
}
