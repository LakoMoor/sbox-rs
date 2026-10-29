use crate::mesh::{MeshData, Vertex};
use anyhow::{anyhow, Result};
use bytemuck::{Pod, Zeroable};
use sbox_math::{sbox_to_view, Mat4, RotationExt, Transform, Vec3};
use sbox_resource::vtex::{TexFormat, Texture};
use sbox_scene::{CameraComponent, DirectionalLight, MaterialId, MeshId, ModelRenderer, PointLight, Scene};
use wgpu::util::DeviceExt;

const MAX_POINT_LIGHTS: usize = 8;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    camera_pos: [f32; 4],
    light_dir: [f32; 4],
    light_color: [f32; 4],
    ambient: [f32; 4],
    point_pos: [[f32; 4]; MAX_POINT_LIGHTS],
    point_color: [[f32; 4]; MAX_POINT_LIGHTS],
    counts: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ObjectUniform {
    model: [[f32; 4]; 4],
    normal_m: [[f32; 4]; 4],
    tint: [f32; 4],
    _pad: [f32; 12], // total 192 bytes; per-object stride is aligned to the device limit
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MaterialUniform {
    tint: [f32; 4],
    uv_scale: [f32; 4],
}

struct GpuMaterial {
    bind_group: wgpu::BindGroup,
}

/// Opaque handle to a GPU texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextureId(pub u32);

struct GpuMesh {
    vb: wgpu::Buffer,
    ib: wgpu::Buffer,
    index_count: u32,
}

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    globals_buf: wgpu::Buffer,
    globals_bg: wgpu::BindGroup,
    object_layout: wgpu::BindGroupLayout,
    object_buf: wgpu::Buffer,
    object_bg: wgpu::BindGroup,
    object_capacity: usize,
    object_stride: u64,
    depth_view: wgpu::TextureView,
    meshes: Vec<GpuMesh>,
    material_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    textures: Vec<wgpu::TextureView>,
    materials: Vec<GpuMaterial>,
    bc_supported: bool,
}

impl Renderer {
    pub fn new<W: Into<wgpu::SurfaceTarget<'static>>>(target: W, size: (u32, u32)) -> Result<Self> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let surface = instance.create_surface(target)?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .ok_or_else(|| anyhow!("no suitable GPU adapter"))?;
        log::info!("adapter: {:?}", adapter.get_info());
        let bc_supported = adapter.features().contains(wgpu::Features::TEXTURE_COMPRESSION_BC);
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                required_features: if bc_supported { wgpu::Features::TEXTURE_COMPRESSION_BC } else { wgpu::Features::empty() },
                ..Default::default()
            },
            None,
        ))?;

        let (w, h) = (size.0.max(1), size.1.max(1));
        let mut config = surface.get_default_config(&adapter, w, h).ok_or_else(|| anyhow!("surface unsupported"))?;
        let caps = surface.get_capabilities(&adapter);
        if let Some(f) = caps.formats.iter().copied().find(|f| f.is_srgb()) {
            config.format = f;
        }
        surface.configure(&device, &config);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scene"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let uniform_entry = |has_dynamic_offset| wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset,
                min_binding_size: None,
            },
            count: None,
        };
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals"),
            entries: &[uniform_entry(false)],
        });
        let object_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("object"),
            entries: &[uniform_entry(true)],
        });

        let globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: globals_buf.as_entire_binding() }],
        });

        let align = device.limits().min_uniform_buffer_offset_alignment as u64;
        let obj_size = std::mem::size_of::<ObjectUniform>() as u64;
        let object_stride = obj_size.div_ceil(align) * align;
        let object_capacity = 256;
        let (object_buf, object_bg) = Self::make_object_buffer(&device, &object_layout, object_capacity, object_stride);

        let material_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("material"),
            entries: &[
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
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
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
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            anisotropy_clamp: 8,
            ..Default::default()
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene"),
            bind_group_layouts: &[&globals_layout, &object_layout, &material_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("scene"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });

        let depth_view = Self::make_depth(&device, w, h);
        let mut this = Self {
            surface,
            device,
            queue,
            config,
            pipeline,
            globals_buf,
            globals_bg,
            object_layout,
            object_buf,
            object_bg,
            object_capacity,
            object_stride,
            depth_view,
            meshes: vec![],
            material_layout,
            sampler,
            textures: vec![],
            materials: vec![],
            bc_supported,
        };
        // texture 0 = white, material 0 = default white
        let white = this.upload_rgba8(&[255, 255, 255, 255], 1, 1);
        this.create_material(Some(white), [1.0; 4], [1.0, 1.0]);
        Ok(this)
    }

    fn upload_rgba8(&mut self, rgba: &[u8], w: u32, h: u32) -> TextureId {
        let tex = self.device.create_texture_with_data(
            &self.queue,
            &wgpu::TextureDescriptor {
                label: Some("rgba8"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            rgba,
        );
        self.textures.push(tex.create_view(&Default::default()));
        TextureId(self.textures.len() as u32 - 1)
    }

    /// Upload a Source 2 texture (sRGB colour interpretation). Returns `None` for
    /// formats/dimensions that cannot be represented (cubemaps, HDR, non-multiple-of-4 BC, ...).
    pub fn upload_texture(&mut self, t: &Texture) -> Option<TextureId> {
        use wgpu::TextureFormat as F;
        if t.is_cube() || t.depth > 1 {
            return None;
        }
        if t.format.is_compressed() && (!self.bc_supported || t.width % 4 != 0 || t.height % 4 != 0) {
            return None;
        }
        let format = match t.format {
            TexFormat::Dxt1 => F::Bc1RgbaUnormSrgb,
            TexFormat::Dxt5 => F::Bc3RgbaUnormSrgb,
            TexFormat::Bc7 => F::Bc7RgbaUnormSrgb,
            TexFormat::Ati1n => F::Bc4RUnorm,
            TexFormat::Ati2n => F::Bc5RgUnorm,
            TexFormat::Rgba8888 => F::Rgba8UnormSrgb,
            TexFormat::Bgra8888 => F::Bgra8UnormSrgb,
            _ => return None,
        };
        let data: Vec<u8> = t.mips.iter().flatten().copied().collect();
        let tex = self.device.create_texture_with_data(
            &self.queue,
            &wgpu::TextureDescriptor {
                label: Some("vtex"),
                size: wgpu::Extent3d { width: t.width, height: t.height, depth_or_array_layers: 1 },
                mip_level_count: t.mips.len() as u32,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &data,
        );
        self.textures.push(tex.create_view(&Default::default()));
        Some(TextureId(self.textures.len() as u32 - 1))
    }

    pub fn create_material(&mut self, color: Option<TextureId>, tint: [f32; 4], uv_scale: [f32; 2]) -> MaterialId {
        let view = &self.textures[color.map_or(0, |t| t.0 as usize)];
        let ub = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("material"),
            contents: bytemuck::bytes_of(&MaterialUniform { tint, uv_scale: [uv_scale[0], uv_scale[1], 0.0, 0.0] }),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("material"),
            layout: &self.material_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: ub.as_entire_binding() },
            ],
        });
        self.materials.push(GpuMaterial { bind_group });
        MaterialId(self.materials.len() as u32 - 1)
    }

    fn make_object_buffer(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        capacity: usize,
        stride: u64,
    ) -> (wgpu::Buffer, wgpu::BindGroup) {
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("objects"),
            size: stride * capacity as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("objects"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buf,
                    offset: 0,
                    size: wgpu::BufferSize::new(std::mem::size_of::<ObjectUniform>() as u64),
                }),
            }],
        });
        (buf, bg)
    }

    fn make_depth(device: &wgpu::Device, w: u32, h: u32) -> wgpu::TextureView {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("depth"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default())
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        if w == 0 || h == 0 {
            return;
        }
        self.config.width = w;
        self.config.height = h;
        self.surface.configure(&self.device, &self.config);
        self.depth_view = Self::make_depth(&self.device, w, h);
    }

    pub fn aspect(&self) -> f32 {
        self.config.width as f32 / self.config.height as f32
    }

    pub fn upload_mesh(&mut self, data: &MeshData) -> MeshId {
        let vb = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("vb"),
            contents: bytemuck::cast_slice(&data.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let ib = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ib"),
            contents: bytemuck::cast_slice(&data.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        self.meshes.push(GpuMesh { vb, ib, index_count: data.indices.len() as u32 });
        MeshId(self.meshes.len() as u32 - 1)
    }

    /// Render the scene from the highest-priority active camera.
    pub fn render(&mut self, scene: &Scene) -> Result<()> {
        let frame = match self.surface.get_current_texture() {
            Ok(f) => f,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                self.surface.configure(&self.device, &self.config);
                return Ok(());
            }
            Err(e) => return Err(e.into()),
        };
        let view = frame.texture.create_view(&Default::default());
        let depth = self.depth_view.clone();
        let aspect = self.aspect();
        self.draw(scene, &view, &depth, aspect)?;
        frame.present();
        Ok(())
    }

    /// Render one frame off-screen and return tightly packed RGBA8 pixels (sRGB).
    pub fn render_offscreen(&mut self, scene: &Scene, w: u32, h: u32) -> Result<Vec<u8>> {
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.config.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = tex.create_view(&Default::default());
        let depth = Self::make_depth(&self.device, w, h);
        self.draw(scene, &view, &depth, w as f32 / h as f32)?;

        let row = (w * 4).div_ceil(256) * 256;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            tex.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.queue.submit(Some(enc.finish()));
        let slice = buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
        self.device.poll(wgpu::Maintain::Wait);
        rx.recv()??;
        let data = slice.get_mapped_range();
        let bgra = matches!(self.config.format, wgpu::TextureFormat::Bgra8UnormSrgb | wgpu::TextureFormat::Bgra8Unorm);
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h as usize {
            for px in data[y * row as usize..y * row as usize + w as usize * 4].chunks(4) {
                if bgra {
                    out.extend([px[2], px[1], px[0], 255]);
                } else {
                    out.extend([px[0], px[1], px[2], 255]);
                }
            }
        }
        Ok(out)
    }

    fn draw(&mut self, scene: &Scene, view: &wgpu::TextureView, depth: &wgpu::TextureView, aspect: f32) -> Result<()> {
        let cam = scene.iter_components::<CameraComponent>().max_by_key(|(_, c)| c.priority);
        let (cam_xf, clear, view_proj) = match cam {
            Some((go, c)) => {
                let xf = scene.world_transform(go);
                let rigid = Transform::new(xf.position, xf.rotation, Vec3::ONE);
                let proj = Mat4::perspective_rh(c.fov_y(aspect), aspect, c.z_near, c.z_far);
                (xf, c.background, proj * sbox_to_view() * rigid.to_matrix().inverse())
            }
            None => (Transform::ZERO, [0.0, 0.0, 0.0, 1.0], Mat4::IDENTITY),
        };

        let mut globals = Globals::zeroed();
        globals.view_proj = view_proj.to_cols_array_2d();
        globals.camera_pos = [cam_xf.position.x, cam_xf.position.y, cam_xf.position.z, 1.0];
        globals.light_dir = [0.0, 0.0, -1.0, 0.0];
        globals.ambient = [0.1, 0.1, 0.1, 0.0];
        if let Some((go, l)) = scene.iter_components::<DirectionalLight>().next() {
            let d = scene.world_transform(go).rotation.forward();
            globals.light_dir = [d.x, d.y, d.z, 0.0];
            globals.light_color = [l.color[0], l.color[1], l.color[2], 1.0];
            globals.ambient = [l.ambient[0], l.ambient[1], l.ambient[2], 0.0];
        }
        let mut np = 0;
        for (go, l) in scene.iter_components::<PointLight>().take(MAX_POINT_LIGHTS) {
            let p = scene.world_transform(go).position;
            globals.point_pos[np] = [p.x, p.y, p.z, l.radius];
            globals.point_color[np] = [l.color[0] * l.intensity, l.color[1] * l.intensity, l.color[2] * l.intensity, 1.0];
            np += 1;
        }
        globals.counts[0] = np as u32;
        self.queue.write_buffer(&self.globals_buf, 0, bytemuck::bytes_of(&globals));

        // Gather draws.
        let draws: Vec<(MeshId, MaterialId, ObjectUniform)> = scene
            .iter_components::<ModelRenderer>()
            .filter(|(_, m)| (m.mesh.0 as usize) < self.meshes.len())
            .map(|(go, m)| {
                let model = scene.world_transform(go).to_matrix();
                let normal_m = model.inverse().transpose();
                (
                    m.mesh,
                    m.material,
                    ObjectUniform {
                        model: model.to_cols_array_2d(),
                        normal_m: normal_m.to_cols_array_2d(),
                        tint: m.tint,
                        _pad: [0.0; 12],
                    },
                )
            })
            .collect();

        if draws.len() > self.object_capacity {
            self.object_capacity = draws.len().next_power_of_two();
            let (b, bg) = Self::make_object_buffer(&self.device, &self.object_layout, self.object_capacity, self.object_stride);
            self.object_buf = b;
            self.object_bg = bg;
        }
        let stride = self.object_stride as usize;
        let mut staging = vec![0u8; stride * draws.len().max(1)];
        for (i, (_, _, u)) in draws.iter().enumerate() {
            staging[i * stride..i * stride + std::mem::size_of::<ObjectUniform>()].copy_from_slice(bytemuck::bytes_of(u));
        }
        self.queue.write_buffer(&self.object_buf, 0, &staging);

        let mut enc = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: srgb_to_linear(clear[0]) as f64,
                            g: srgb_to_linear(clear[1]) as f64,
                            b: srgb_to_linear(clear[2]) as f64,
                            a: clear[3] as f64,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.globals_bg, &[]);
            for (i, (mesh, mat, _)) in draws.iter().enumerate() {
                let m = &self.meshes[mesh.0 as usize];
                pass.set_bind_group(1, &self.object_bg, &[(i * stride) as u32]);
                let mi = if (mat.0 as usize) < self.materials.len() { mat.0 as usize } else { 0 };
                pass.set_bind_group(2, &self.materials[mi].bind_group, &[]);
                pass.set_vertex_buffer(0, m.vb.slice(..));
                pass.set_index_buffer(m.ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..m.index_count, 0, 0..1);
            }
        }
        self.queue.submit(Some(enc.finish()));
        Ok(())
    }
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}
