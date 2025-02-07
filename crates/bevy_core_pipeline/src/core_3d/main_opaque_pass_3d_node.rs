use crate::skybox::{SkyboxBindGroup, SkyboxPipelineId};
use bevy_ecs::{prelude::World, query::QueryItem};
use bevy_render::{
    camera::ExtractedCamera,
    diagnostic::RecordDiagnostics,
    render_graph::{NodeRunError, RenderGraphContext, ViewNode},
    render_phase::TrackedRenderPass,
    render_resource::{CommandEncoderDescriptor, PipelineCache, RenderPassDescriptor, StoreOp},
    renderer::RenderContext,
    view::{ViewDepthTexture, ViewTarget, ViewUniformOffset},
};
use tracing::error;
#[cfg(feature = "trace")]
use tracing::info_span;

use super::MainPhasesReadOnly;

/// A [`bevy_render::render_graph::Node`] that runs the [`Opaque3d`] and [`AlphaMask3d`]
/// [`ViewBinnedRenderPhases`]s.
#[derive(Default)]
pub struct MainOpaquePass3dNode;
impl ViewNode for MainOpaquePass3dNode {
    type ViewQuery = (
        &'static ExtractedCamera,
        &'static ViewTarget,
        &'static ViewDepthTexture,
        Option<&'static SkyboxPipelineId>,
        Option<&'static SkyboxBindGroup>,
        &'static ViewUniformOffset,
        MainPhasesReadOnly<'static>,
    );

    fn run<'w>(
        &self,
        graph: &mut RenderGraphContext,
        render_context: &mut RenderContext<'w>,
        (
            camera,
            target,
            depth,
            skybox_pipeline,
            skybox_bind_group,
            view_uniform_offset,
            main_phases,
        ): QueryItem<'w, Self::ViewQuery>,
        world: &'w World,
    ) -> Result<(), NodeRunError> {
        let diagnostics = render_context.diagnostic_recorder();

        let color_attachments = [Some(target.get_color_attachment())];
        let depth_stencil_attachment = Some(depth.get_attachment(StoreOp::Store));

        let view_entity = graph.view_entity();
        render_context.add_command_buffer_generation_task(move |render_device| {
            #[cfg(feature = "trace")]
            let _main_opaque_pass_3d_span = info_span!("main_opaque_pass_3d").entered();

            // Command encoder setup
            let mut command_encoder =
                render_device.create_command_encoder(&CommandEncoderDescriptor {
                    label: Some("main_opaque_pass_3d_command_encoder"),
                });

            // Render pass setup
            let render_pass = command_encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("main_opaque_pass_3d"),
                color_attachments: &color_attachments,
                depth_stencil_attachment,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            let mut render_pass = TrackedRenderPass::new(&render_device, render_pass);
            let pass_span = diagnostics.pass_span(&mut render_pass, "main_opaque_pass_3d");

            if let Some(viewport) = camera.viewport.as_ref() {
                render_pass.set_camera_viewport(viewport);
            }

            // Opaque draws
            if !main_phases.opaque.is_empty() {
                #[cfg(feature = "trace")]
                let _opaque_main_pass_3d_span = info_span!("opaque_main_pass_3d").entered();
                if let Err(err) = main_phases
                    .opaque
                    .render(&mut render_pass, world, view_entity)
                {
                    error!("Error encountered while rendering the opaque phase {err:?}");
                }
            }

            // Alpha draws
            if !main_phases.alpha_mask.is_empty() {
                #[cfg(feature = "trace")]
                let _alpha_mask_main_pass_3d_span = info_span!("alpha_mask_main_pass_3d").entered();
                if let Err(err) =
                    main_phases
                        .alpha_mask
                        .render(&mut render_pass, world, view_entity)
                {
                    error!("Error encountered while rendering the alpha mask phase {err:?}");
                }
            }

            // Skybox draw using a fullscreen triangle
            if let (Some(skybox_pipeline), Some(SkyboxBindGroup(skybox_bind_group))) =
                (skybox_pipeline, skybox_bind_group)
            {
                let pipeline_cache = world.resource::<PipelineCache>();
                if let Some(pipeline) = pipeline_cache.get_render_pipeline(skybox_pipeline.0) {
                    render_pass.set_render_pipeline(pipeline);
                    render_pass.set_bind_group(
                        0,
                        &skybox_bind_group.0,
                        &[view_uniform_offset.offset, skybox_bind_group.1],
                    );
                    render_pass.draw(0..3, 0..1);
                }
            }

            pass_span.end(&mut render_pass);
            drop(render_pass);
            command_encoder.finish()
        });

        Ok(())
    }
}
