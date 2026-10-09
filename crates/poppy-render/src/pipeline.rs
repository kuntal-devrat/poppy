//! Video rendering and color conversion pipeline using `wgpu`.

use std::sync::mpsc::sync_channel;
use poppy_core::PixelFormat;
use poppy_hw::DecodedFrame;
use crate::RenderError;

/// Color space and transfer function specification for video playback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorSpace {
    /// ITU-R BT.709 with limited range (standard for HD video: Y in 16..235, UV in 16..240).
    #[default]
    Bt709Limited,
    /// ITU-R BT.709 with full dynamic range (0..255).
    Bt709Full,
    /// ITU-R BT.601 with limited range (standard for SD video).
    Bt601Limited,
    /// ITU-R BT.601 with full dynamic range.
    Bt601Full,
    /// ITU-R BT.2020 with limited range (standard for 4K / 8K HDR).
    Bt2020Limited,
    /// ITU-R BT.2020 with full dynamic range.
    Bt2020Full,
}

impl ColorSpace {
    pub fn matrix_index(&self) -> u32 {
        match self {
            Self::Bt709Limited | Self::Bt709Full => 0,
            Self::Bt601Limited | Self::Bt601Full => 1,
            Self::Bt2020Limited | Self::Bt2020Full => 2,
        }
    }

    pub fn is_full_range(&self) -> bool {
        matches!(
            self,
            Self::Bt709Full | Self::Bt601Full | Self::Bt2020Full
        )
    }
}

/// Uniform structure passed to WGSL shader.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct ColorParamsUniform {
    color_matrix: u32,
    color_range: u32,
    _padding0: u32,
    _padding1: u32,
}

/// Hardware render pipeline for YUV to sRGB conversion.
pub struct VideoRenderPipeline {
    pipeline_nv12: wgpu::RenderPipeline,
    pipeline_p010: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform_buffer: wgpu::Buffer,
}

impl VideoRenderPipeline {
    pub fn new(device: &wgpu::Device, output_format: wgpu::TextureFormat) -> Result<Self, RenderError> {
        let shader_src = include_str!("shader.wgsl");
        let shader_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("poppy_color_convert_shader"),
            source: wgpu::ShaderSource::Wgsl(shader_src.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("poppy_video_bind_group_layout"),
            entries: &[
                // Y plane texture
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // UV plane texture
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // Bilinear Sampler
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                // Uniform parameters
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("poppy_video_pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let create_pipeline = |label: &'static str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader_module,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader_module,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: output_format,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    strip_index_format: None,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: None,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    unclipped_depth: false,
                    conservative: false,
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            })
        };

        let pipeline_nv12 = create_pipeline("poppy_nv12_pipeline");
        let pipeline_p010 = create_pipeline("poppy_p010_pipeline");

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("poppy_video_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("poppy_color_params_uniform"),
            size: std::mem::size_of::<ColorParamsUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Ok(Self {
            pipeline_nv12,
            pipeline_p010,
            bind_group_layout,
            sampler,
            uniform_buffer,
        })
    }
}

/// Zero-copy GPU video renderer powered by `wgpu`.
pub struct GpuRenderer {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pipeline: VideoRenderPipeline,
    output_format: wgpu::TextureFormat,
}

impl GpuRenderer {
    /// Asynchronously initializes a new GPU renderer instance.
    pub async fn new() -> Result<Self, RenderError> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .ok_or_else(|| RenderError::AdapterNotFound("No compatible GPU adapter found".into()))?;

        let supported_features = adapter.features();
        let mut required_features = wgpu::Features::empty();
        if supported_features.contains(wgpu::Features::TEXTURE_FORMAT_16BIT_NORM) {
            required_features |= wgpu::Features::TEXTURE_FORMAT_16BIT_NORM;
        }

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("poppy_gpu_device"),
                    required_features,
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::Performance,
                },
                None,
            )
            .await
            .map_err(|e| RenderError::DeviceInitFailed(e.to_string()))?;

        let output_format = wgpu::TextureFormat::Rgba8Unorm;
        let pipeline = VideoRenderPipeline::new(&device, output_format)?;

        Ok(Self {
            instance,
            adapter,
            device,
            queue,
            pipeline,
            output_format,
        })
    }

    /// Synchronously initializes a new GPU renderer instance.
    pub fn new_sync() -> Result<Self, RenderError> {
        pollster::block_on(Self::new())
    }

    /// Update the shader uniform buffer with the given color space.
    fn update_uniform(&self, color_space: ColorSpace) {
        let params = ColorParamsUniform {
            color_matrix: color_space.matrix_index(),
            color_range: if color_space.is_full_range() { 1 } else { 0 },
            _padding0: 0,
            _padding1: 0,
        };
        self.queue.write_buffer(
            &self.pipeline.uniform_buffer,
            0,
            bytemuck::bytes_of(&params),
        );
    }

    /// Renders an NV12 frame directly into a target `wgpu::TextureView`.
    #[allow(clippy::too_many_arguments)]
    pub fn render_nv12_to_view(
        &self,
        y_plane: &[u8],
        uv_plane: &[u8],
        width: u32,
        height: u32,
        stride_y: u32,
        stride_uv: u32,
        color_space: ColorSpace,
        target_view: &wgpu::TextureView,
    ) -> Result<(), RenderError> {
        self.update_uniform(color_space);

        let uv_width = width.div_ceil(2);
        let uv_height = height.div_ceil(2);

        let y_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("poppy_nv12_y_texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let uv_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("poppy_nv12_uv_texture"),
            size: wgpu::Extent3d {
                width: uv_width,
                height: uv_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rg8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        // Upload Y plane
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &y_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            y_plane,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride_y),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        // Upload UV plane
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &uv_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            uv_plane,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride_uv),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: uv_width,
                height: uv_height,
                depth_or_array_layers: 1,
            },
        );

        let y_view = y_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let uv_view = uv_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("poppy_nv12_bind_group"),
            layout: &self.pipeline.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&y_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&uv_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.pipeline.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.pipeline.uniform_buffer.as_entire_binding(),
                },
            ],
        });

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("poppy_render_encoder"),
        });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("poppy_color_convert_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            render_pass.set_pipeline(&self.pipeline.pipeline_nv12);
            render_pass.set_bind_group(0, &bind_group, &[]);
            render_pass.draw(0..3, 0..1);
        }

        self.queue.submit(Some(encoder.finish()));
        Ok(())
    }

    /// Renders an NV12 frame to an offscreen texture and reads back sRGB RGBA8 bytes.
    #[allow(clippy::too_many_arguments)]
    pub fn render_nv12_to_rgba(
        &self,
        y_plane: &[u8],
        uv_plane: &[u8],
        width: u32,
        height: u32,
        stride_y: u32,
        stride_uv: u32,
        color_space: ColorSpace,
    ) -> Result<Vec<u8>, RenderError> {
        let target_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("poppy_target_rgba_texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.output_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        let target_view = target_texture.create_view(&wgpu::TextureViewDescriptor::default());

        self.render_nv12_to_view(
            y_plane,
            uv_plane,
            width,
            height,
            stride_y,
            stride_uv,
            color_space,
            &target_view,
        )?;

        self.read_texture_rgba(&target_texture, width, height)
    }

    /// Renders a P010 (10-bit HDR) frame directly into RGBA8 bytes.
    #[allow(clippy::too_many_arguments)]
    pub fn render_p010_to_rgba(
        &self,
        y_plane: &[u16],
        uv_plane: &[u16],
        width: u32,
        height: u32,
        stride_y: u32,
        stride_uv: u32,
        color_space: ColorSpace,
    ) -> Result<Vec<u8>, RenderError> {
        if !self.device.features().contains(wgpu::Features::TEXTURE_FORMAT_16BIT_NORM) {
            return Err(RenderError::UnsupportedFormat(
                "GPU device lacks TEXTURE_FORMAT_16BIT_NORM feature for P010 rendering".into(),
            ));
        }

        self.update_uniform(color_space);

        let uv_width = width.div_ceil(2);
        let uv_height = height.div_ceil(2);

        let y_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("poppy_p010_y_texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R16Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let uv_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("poppy_p010_uv_texture"),
            size: wgpu::Extent3d {
                width: uv_width,
                height: uv_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rg16Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let y_bytes: &[u8] = bytemuck::cast_slice(y_plane);
        let uv_bytes: &[u8] = bytemuck::cast_slice(uv_plane);

        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &y_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            y_bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride_y * 2),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &uv_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            uv_bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride_uv * 2),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: uv_width,
                height: uv_height,
                depth_or_array_layers: 1,
            },
        );

        let target_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("poppy_target_rgba_texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.output_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        let target_view = target_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let y_view = y_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let uv_view = uv_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("poppy_p010_bind_group"),
            layout: &self.pipeline.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&y_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&uv_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.pipeline.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.pipeline.uniform_buffer.as_entire_binding(),
                },
            ],
        });

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("poppy_p010_render_encoder"),
        });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("poppy_p010_render_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            render_pass.set_pipeline(&self.pipeline.pipeline_p010);
            render_pass.set_bind_group(0, &bind_group, &[]);
            render_pass.draw(0..3, 0..1);
        }

        self.queue.submit(Some(encoder.finish()));
        self.read_texture_rgba(&target_texture, width, height)
    }

    /// Converts a generic decoded frame into RGBA8 pixels using GPU acceleration.
    pub fn render_frame(&self, frame: &DecodedFrame) -> Result<Vec<u8>, RenderError> {
        let data = frame
            .data
            .as_ref()
            .ok_or_else(|| RenderError::UnsupportedFormat("DecodedFrame has no raw data buffer".into()))?;

        match frame.pixel_format {
            PixelFormat::Nv12 => {
                let y_size = (frame.width * frame.height) as usize;
                let uv_size = (frame.width * frame.height.div_ceil(2)) as usize;
                if data.len() < y_size + uv_size {
                    return Err(RenderError::UnsupportedFormat("Frame data buffer too small for NV12".into()));
                }
                let y_plane = &data[..y_size];
                let uv_plane = &data[y_size..y_size + uv_size];

                self.render_nv12_to_rgba(
                    y_plane,
                    uv_plane,
                    frame.width,
                    frame.height,
                    frame.width,
                    frame.width,
                    ColorSpace::Bt709Limited,
                )
            }
            PixelFormat::P010 => {
                let y_len = (frame.width * frame.height) as usize;
                let uv_len = (frame.width * frame.height.div_ceil(2)) as usize;
                let u16_slice: &[u16] = bytemuck::cast_slice(data);
                if u16_slice.len() < y_len + uv_len {
                    return Err(RenderError::UnsupportedFormat("Frame data buffer too small for P010".into()));
                }
                let y_plane = &u16_slice[..y_len];
                let uv_plane = &u16_slice[y_len..y_len + uv_len];

                self.render_p010_to_rgba(
                    y_plane,
                    uv_plane,
                    frame.width,
                    frame.height,
                    frame.width,
                    frame.width,
                    ColorSpace::Bt2020Limited,
                )
            }
            _ => Err(RenderError::UnsupportedFormat(format!("{:?}", frame.pixel_format))),
        }
    }

    /// Internal helper to read back texture pixels from GPU memory into host RAM.
    fn read_texture_rgba(
        &self,
        texture: &wgpu::Texture,
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>, RenderError> {
        let unaligned_bytes_per_row = width * 4;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_bytes_per_row = unaligned_bytes_per_row.div_ceil(align) * align;
        let buffer_size = (padded_bytes_per_row * height) as u64;

        let staging_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("poppy_staging_readback_buffer"),
            size: buffer_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("poppy_readback_encoder"),
        });

        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &staging_buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        self.queue.submit(Some(encoder.finish()));

        let buffer_slice = staging_buffer.slice(..);
        let (sender, receiver) = sync_channel(1);

        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });

        self.device.poll(wgpu::Maintain::Wait);

        receiver
            .recv()
            .map_err(|e| RenderError::BufferMapFailed(e.to_string()))?
            .map_err(|e| RenderError::BufferMapFailed(e.to_string()))?;

        let mapped_data = buffer_slice.get_mapped_range();
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);

        for row in 0..height {
            let start = (row * padded_bytes_per_row) as usize;
            let end = start + (width * 4) as usize;
            rgba.extend_from_slice(&mapped_data[start..end]);
        }

        drop(mapped_data);
        staging_buffer.unmap();

        Ok(rgba)
    }
}
