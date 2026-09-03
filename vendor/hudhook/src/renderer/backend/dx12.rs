// This file was modified in a fork of jaskang/wukong-minimap.
//
// Upstream: https://github.com/jaskang/wukong-minimap (Apache-2.0)
// Fork:     https://github.com/Ouye/wukong-minimap
//
// Vendored from veeenu/hudhook 0.8.2 (MIT), then modified.
//
// Changes: track resource state so re-uploads are legal; keep textures in
// COMMON for cross-queue ownership transfer; use GetCopyableFootprints;
// upload through a CPU-writable CUSTOM heap with WriteToSubresource; clamp
// alpha 255 -> 254 to work around opaque-texture corruption on this game.
//
// See CHANGES.md for the full record of what was changed and why.

// NOTE: see this for ManuallyDrop instances https://github.com/microsoft/windows-rs/issues/2386

use std::ffi::c_void;
use std::mem::{offset_of, ManuallyDrop};
use std::{mem, ptr, slice};

use imgui::internal::RawWrapper;
use imgui::{BackendFlags, Context, DrawCmd, DrawData, DrawIdx, DrawVert, TextureId};
use tracing::{debug, error};
use windows::core::{s, w, Error, Interface, Result, HRESULT};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct3D::Fxc::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::*;

use crate::renderer::RenderEngine;
use crate::util::{self, Fence};
use crate::RenderContext;

pub struct D3D12RenderEngine {
    device: ID3D12Device,

    command_queue: ID3D12CommandQueue,
    command_allocator: ID3D12CommandAllocator,
    command_list: ID3D12GraphicsCommandList,

    #[allow(unused)]
    rtv_heap: ID3D12DescriptorHeap,
    rtv_heap_start: D3D12_CPU_DESCRIPTOR_HANDLE,
    texture_heap: TextureHeap,

    root_signature: ID3D12RootSignature,
    pipeline_state: ID3D12PipelineState,

    vertex_buffer: Buffer<DrawVert>,
    index_buffer: Buffer<u16>,
    projection_buffer: [[f32; 4]; 4],

    fence: Fence,
}

impl D3D12RenderEngine {
    pub fn new(command_queue: &ID3D12CommandQueue, ctx: &mut Context) -> Result<Self> {
        let (device, command_queue, command_allocator, command_list) =
            unsafe { create_command_objects(command_queue) }?;

        let (rtv_heap, texture_heap) = unsafe { create_heaps(&device) }?;
        let rtv_heap_start = unsafe { rtv_heap.GetCPUDescriptorHandleForHeapStart() };

        let (root_signature, pipeline_state) = unsafe { create_shader_program(&device) }?;

        let vertex_buffer = Buffer::new(&device, 5000)?;
        let index_buffer = Buffer::new(&device, 10000)?;

        let fence = Fence::new(&device)?;

        ctx.set_ini_filename(None);
        ctx.io_mut().backend_flags |= BackendFlags::RENDERER_HAS_VTX_OFFSET;
        ctx.set_renderer_name(String::from(concat!("hudhook-dx12@", env!("CARGO_PKG_VERSION"))));

        Ok(Self {
            device,
            command_queue,
            command_allocator,
            command_list,
            rtv_heap,
            rtv_heap_start,
            texture_heap,
            root_signature,
            pipeline_state,
            vertex_buffer,
            index_buffer,
            projection_buffer: Default::default(),
            fence,
        })
    }
}

impl RenderContext for D3D12RenderEngine {
    fn load_texture(&mut self, data: &[u8], width: u32, height: u32) -> Result<TextureId> {
        unsafe {
            let texture_id = self.texture_heap.create_texture(width, height)?;
            self.texture_heap.upload_texture(texture_id, data, width, height)?;
            Ok(texture_id)
        }
    }

    fn replace_texture(
        &mut self,
        texture_id: TextureId,
        data: &[u8],
        width: u32,
        height: u32,
    ) -> Result<()> {
        unsafe { self.texture_heap.upload_texture(texture_id, data, width, height) }
    }
}

impl RenderEngine for D3D12RenderEngine {
    type RenderTarget = ID3D12Resource;

    fn render(&mut self, draw_data: &DrawData, render_target: Self::RenderTarget) -> Result<()> {
        unsafe {
            self.device.CreateRenderTargetView(&render_target, None, self.rtv_heap_start);

            self.command_allocator.Reset()?;
            self.command_list.Reset(&self.command_allocator, None)?;

            let present_to_rtv_barriers = [util::create_barrier(
                &render_target,
                D3D12_RESOURCE_STATE_PRESENT,
                D3D12_RESOURCE_STATE_RENDER_TARGET,
            )];

            let rtv_to_present_barriers = [util::create_barrier(
                &render_target,
                D3D12_RESOURCE_STATE_RENDER_TARGET,
                D3D12_RESOURCE_STATE_COMMON,
            )];

            self.command_list.ResourceBarrier(&present_to_rtv_barriers);
            self.command_list.OMSetRenderTargets(1, Some(&self.rtv_heap_start), false, None);
            self.command_list.SetDescriptorHeaps(&[Some(self.texture_heap.srv_heap.clone())]);

            self.render_draw_data(draw_data)?;

            self.command_list.ResourceBarrier(&rtv_to_present_barriers);
            self.command_list.Close()?;
            self.command_queue.ExecuteCommandLists(&[Some(self.command_list.cast()?)]);
            self.command_queue.Signal(self.fence.fence(), self.fence.value())?;
            self.fence.wait()?;
            self.fence.incr();

            present_to_rtv_barriers.into_iter().for_each(util::drop_barrier);
            rtv_to_present_barriers.into_iter().for_each(util::drop_barrier);
        }

        Ok(())
    }

    fn setup_fonts(&mut self, ctx: &mut Context) -> Result<()> {
        let fonts = ctx.fonts();
        let fonts_texture = fonts.build_rgba32_texture();
        fonts.tex_id =
            self.load_texture(fonts_texture.data, fonts_texture.width, fonts_texture.height)?;
        Ok(())
    }
}

impl D3D12RenderEngine {
    unsafe fn render_draw_data(&mut self, draw_data: &DrawData) -> Result<()> {
        self.vertex_buffer.clear();
        self.index_buffer.clear();

        draw_data
            .draw_lists()
            .map(|draw_list| {
                (draw_list.vtx_buffer().iter().copied(), draw_list.idx_buffer().iter().copied())
            })
            .for_each(|(vertices, indices)| {
                self.vertex_buffer.extend(vertices);
                self.index_buffer.extend(indices);
            });

        self.vertex_buffer.upload(&self.device)?;
        self.index_buffer.upload(&self.device)?;

        self.projection_buffer = {
            let [l, t, r, b] = [
                draw_data.display_pos[0],
                draw_data.display_pos[1],
                draw_data.display_pos[0] + draw_data.display_size[0],
                draw_data.display_pos[1] + draw_data.display_size[1],
            ];

            [[2. / (r - l), 0., 0., 0.], [0., 2. / (t - b), 0., 0.], [0., 0., 0.5, 0.], [
                (r + l) / (l - r),
                (t + b) / (b - t),
                0.5,
                1.0,
            ]]
        };

        self.setup_render_state(draw_data);

        let mut vtx_offset = 0usize;
        let mut idx_offset = 0usize;

        for cl in draw_data.draw_lists() {
            for cmd in cl.commands() {
                match cmd {
                    DrawCmd::Elements { count, cmd_params } => {
                        let [cx, cy, cw, ch] = cmd_params.clip_rect;
                        let [x, y] = draw_data.display_pos;
                        let r = RECT {
                            left: (cx - x) as i32,
                            top: (cy - y) as i32,
                            right: (cw - x) as i32,
                            bottom: (ch - y) as i32,
                        };

                        if r.right > r.left && r.bottom > r.top {
                            let tex_handle =
                                self.texture_heap.textures[cmd_params.texture_id.id()].gpu_desc;
                            self.command_list.SetGraphicsRootDescriptorTable(1, tex_handle);
                            self.command_list.RSSetScissorRects(&[r]);
                            self.command_list.DrawIndexedInstanced(
                                count as _,
                                1,
                                (cmd_params.idx_offset + idx_offset) as _,
                                (cmd_params.vtx_offset + vtx_offset) as _,
                                0,
                            );
                        }
                    },
                    DrawCmd::ResetRenderState => {
                        // Q: looking at the commands recorded in here, it
                        // doesn't seem like this should have any effect
                        // whatsoever. What am I doing wrong?
                        self.setup_render_state(draw_data);
                    },
                    DrawCmd::RawCallback { callback, raw_cmd } => callback(cl.raw(), raw_cmd),
                }
            }
            idx_offset += cl.idx_buffer().len();
            vtx_offset += cl.vtx_buffer().len();
        }

        Ok(())
    }

    unsafe fn setup_render_state(&self, draw_data: &DrawData) {
        self.command_list.RSSetViewports(&[D3D12_VIEWPORT {
            TopLeftX: 0f32,
            TopLeftY: 0f32,
            Width: draw_data.display_size[0],
            Height: draw_data.display_size[1],
            MinDepth: 0f32,
            MaxDepth: 1f32,
        }]);

        self.command_list.IASetVertexBuffers(
            0,
            Some(&[D3D12_VERTEX_BUFFER_VIEW {
                BufferLocation: self.vertex_buffer.resource.GetGPUVirtualAddress(),
                SizeInBytes: (self.vertex_buffer.data.len() * mem::size_of::<DrawVert>()) as _,
                StrideInBytes: mem::size_of::<DrawVert>() as _,
            }]),
        );

        self.command_list.IASetIndexBuffer(Some(&D3D12_INDEX_BUFFER_VIEW {
            BufferLocation: self.index_buffer.resource.GetGPUVirtualAddress(),
            SizeInBytes: (self.index_buffer.data.len() * mem::size_of::<DrawIdx>()) as _,
            Format: if mem::size_of::<DrawIdx>() == 2 {
                DXGI_FORMAT_R16_UINT
            } else {
                DXGI_FORMAT_R32_UINT
            },
        }));
        self.command_list.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        self.command_list.SetPipelineState(&self.pipeline_state);
        self.command_list.SetGraphicsRootSignature(&self.root_signature);
        self.command_list.SetGraphicsRoot32BitConstants(
            0,
            16,
            self.projection_buffer.as_ptr() as *const c_void,
            0,
        );
        self.command_list.OMSetBlendFactor(Some(&[0f32; 4]));
    }
}

unsafe fn create_command_objects(
    command_queue: &ID3D12CommandQueue,
) -> Result<(ID3D12Device, ID3D12CommandQueue, ID3D12CommandAllocator, ID3D12GraphicsCommandList)> {
    let device: ID3D12Device = util::try_out_ptr(|v| unsafe { command_queue.GetDevice(v) })?;
    let command_queue = command_queue.clone();

    let command_allocator: ID3D12CommandAllocator =
        device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT)?;

    let command_list: ID3D12GraphicsCommandList =
        device.CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &command_allocator, None)?;
    command_list.Close()?;

    command_allocator.SetName(w!("hudhook Render Engine Command Allocator"))?;
    command_list.SetName(w!("hudhook Render Engine Command List"))?;

    Ok((device, command_queue, command_allocator, command_list))
}

unsafe fn create_heaps(device: &ID3D12Device) -> Result<(ID3D12DescriptorHeap, TextureHeap)> {
    let rtv_heap: ID3D12DescriptorHeap =
        device.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
            Type: D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
            NumDescriptors: 1,
            Flags: D3D12_DESCRIPTOR_HEAP_FLAG_NONE,
            NodeMask: 1,
        })?;

    let srv_heap: ID3D12DescriptorHeap =
        device.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
            Type: D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
            NumDescriptors: 8,
            Flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
            NodeMask: 0,
        })?;

    let texture_heap = TextureHeap::new(device, srv_heap)?;

    Ok((rtv_heap, texture_heap))
}

unsafe fn create_shader_program(
    device: &ID3D12Device,
) -> Result<(ID3D12RootSignature, ID3D12PipelineState)> {
    let parameters = [
        D3D12_ROOT_PARAMETER {
            ParameterType: D3D12_ROOT_PARAMETER_TYPE_32BIT_CONSTANTS,
            Anonymous: D3D12_ROOT_PARAMETER_0 {
                Constants: D3D12_ROOT_CONSTANTS {
                    ShaderRegister: 0,
                    RegisterSpace: 0,
                    Num32BitValues: 16,
                },
            },
            ShaderVisibility: D3D12_SHADER_VISIBILITY_VERTEX,
        },
        D3D12_ROOT_PARAMETER {
            ParameterType: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
            Anonymous: D3D12_ROOT_PARAMETER_0 {
                DescriptorTable: D3D12_ROOT_DESCRIPTOR_TABLE {
                    NumDescriptorRanges: 1,
                    pDescriptorRanges: &D3D12_DESCRIPTOR_RANGE {
                        RangeType: D3D12_DESCRIPTOR_RANGE_TYPE_SRV,
                        NumDescriptors: 1,
                        BaseShaderRegister: 0,
                        RegisterSpace: 0,
                        OffsetInDescriptorsFromTableStart: 0,
                    },
                },
            },
            ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
        },
    ];

    let root_signature_desc = D3D12_ROOT_SIGNATURE_DESC {
        NumParameters: 2,
        pParameters: parameters.as_ptr(),
        NumStaticSamplers: 1,
        pStaticSamplers: &D3D12_STATIC_SAMPLER_DESC {
            Filter: D3D12_FILTER_MIN_MAG_MIP_LINEAR,
            AddressU: D3D12_TEXTURE_ADDRESS_MODE_WRAP,
            AddressV: D3D12_TEXTURE_ADDRESS_MODE_WRAP,
            AddressW: D3D12_TEXTURE_ADDRESS_MODE_WRAP,
            MipLODBias: 0f32,
            MaxAnisotropy: 0,
            ComparisonFunc: D3D12_COMPARISON_FUNC_ALWAYS,
            BorderColor: D3D12_STATIC_BORDER_COLOR_TRANSPARENT_BLACK,
            MinLOD: 0f32,
            MaxLOD: 0f32,
            ShaderRegister: 0,
            RegisterSpace: 0,
            ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
        },
        Flags: D3D12_ROOT_SIGNATURE_FLAG_ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT
            | D3D12_ROOT_SIGNATURE_FLAG_DENY_HULL_SHADER_ROOT_ACCESS
            | D3D12_ROOT_SIGNATURE_FLAG_DENY_DOMAIN_SHADER_ROOT_ACCESS
            | D3D12_ROOT_SIGNATURE_FLAG_DENY_GEOMETRY_SHADER_ROOT_ACCESS,
    };

    let blob: ID3DBlob = util::try_out_err_blob(|v, err_blob| {
        D3D12SerializeRootSignature(
            &root_signature_desc,
            D3D_ROOT_SIGNATURE_VERSION_1_0,
            v,
            Some(err_blob),
        )
    })
    .map_err(util::print_error_blob("Serializing root signature"))
    .expect("D3D12SerializeRootSignature");

    let root_signature: ID3D12RootSignature = device.CreateRootSignature(
        0,
        slice::from_raw_parts(blob.GetBufferPointer() as *const u8, blob.GetBufferSize()),
    )?;

    const VS: &str = r#"
    cbuffer vertexBuffer : register(b0) {
      float4x4 ProjectionMatrix;
    };

    struct VS_INPUT {
      float2 pos: POSITION;
      float4 col: COLOR0;
      float2 uv: TEXCOORD0;
    };

    struct PS_INPUT {
      float4 pos: SV_POSITION;
      float4 col: COLOR0;
      float2 uv: TEXCOORD0;
    };

    PS_INPUT main(VS_INPUT input) {
      PS_INPUT output;
      output.pos = mul( ProjectionMatrix, float4(input.pos.xy, 0.f, 1.f));
      output.col = input.col;
      output.uv = input.uv;
      return output;
    }"#;

    const PS: &str = r#"
    struct PS_INPUT {
      float4 pos: SV_POSITION;
      float4 col: COLOR0;
      float2 uv: TEXCOORD0;
    };

    SamplerState sampler0: register(s0);
    Texture2D texture0: register(t0);

    float4 main(PS_INPUT input): SV_Target {
      float4 out_col = input.col * texture0.Sample(sampler0, input.uv);
      return out_col;
    }"#;

    let vtx_shader: ID3DBlob = util::try_out_err_blob(|v, err_blob| unsafe {
        D3DCompile(
            VS.as_ptr() as _,
            VS.len(),
            None,
            None,
            None::<&ID3DInclude>,
            s!("main\0"),
            s!("vs_5_0\0"),
            0,
            0,
            v,
            Some(err_blob),
        )
    })
    .map_err(util::print_error_blob("Compiling vertex shader"))
    .expect("D3DCompile");

    let pix_shader = util::try_out_err_blob(|v, err_blob| unsafe {
        D3DCompile(
            PS.as_ptr() as _,
            PS.len(),
            None,
            None,
            None::<&ID3DInclude>,
            s!("main\0"),
            s!("ps_5_0\0"),
            0,
            0,
            v,
            Some(err_blob),
        )
    })
    .map_err(util::print_error_blob("Compiling pixel shader"))
    .expect("D3DCompile");

    let input_elements = [
        D3D12_INPUT_ELEMENT_DESC {
            SemanticName: s!("POSITION"),
            SemanticIndex: 0,
            Format: DXGI_FORMAT_R32G32_FLOAT,
            InputSlot: 0,
            AlignedByteOffset: offset_of!(DrawVert, pos) as u32,
            InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
            InstanceDataStepRate: 0,
        },
        D3D12_INPUT_ELEMENT_DESC {
            SemanticName: s!("TEXCOORD"),
            SemanticIndex: 0,
            Format: DXGI_FORMAT_R32G32_FLOAT,
            InputSlot: 0,
            AlignedByteOffset: offset_of!(DrawVert, uv) as u32,
            InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
            InstanceDataStepRate: 0,
        },
        D3D12_INPUT_ELEMENT_DESC {
            SemanticName: s!("COLOR"),
            SemanticIndex: 0,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            InputSlot: 0,
            AlignedByteOffset: offset_of!(DrawVert, col) as u32,
            InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
            InstanceDataStepRate: 0,
        },
    ];

    let pso_desc = D3D12_GRAPHICS_PIPELINE_STATE_DESC {
        pRootSignature: ManuallyDrop::new(Some(root_signature.clone())),
        NodeMask: 1,
        PrimitiveTopologyType: D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE,
        SampleMask: u32::MAX,
        NumRenderTargets: 1,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Flags: D3D12_PIPELINE_STATE_FLAG_NONE,
        RTVFormats: [
            DXGI_FORMAT_B8G8R8A8_UNORM,
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
        ],
        DSVFormat: DXGI_FORMAT_D32_FLOAT,
        VS: D3D12_SHADER_BYTECODE {
            pShaderBytecode: unsafe { vtx_shader.GetBufferPointer() },
            BytecodeLength: unsafe { vtx_shader.GetBufferSize() },
        },
        PS: D3D12_SHADER_BYTECODE {
            pShaderBytecode: unsafe { pix_shader.GetBufferPointer() },
            BytecodeLength: unsafe { pix_shader.GetBufferSize() },
        },
        InputLayout: D3D12_INPUT_LAYOUT_DESC {
            pInputElementDescs: input_elements.as_ptr(),
            NumElements: 3,
        },
        BlendState: D3D12_BLEND_DESC {
            AlphaToCoverageEnable: false.into(),
            IndependentBlendEnable: false.into(),
            RenderTarget: [
                D3D12_RENDER_TARGET_BLEND_DESC {
                    BlendEnable: true.into(),
                    LogicOpEnable: false.into(),
                    SrcBlend: D3D12_BLEND_SRC_ALPHA,
                    DestBlend: D3D12_BLEND_INV_SRC_ALPHA,
                    BlendOp: D3D12_BLEND_OP_ADD,
                    SrcBlendAlpha: D3D12_BLEND_ONE,
                    DestBlendAlpha: D3D12_BLEND_INV_SRC_ALPHA,
                    BlendOpAlpha: D3D12_BLEND_OP_ADD,
                    LogicOp: Default::default(),
                    RenderTargetWriteMask: D3D12_COLOR_WRITE_ENABLE_ALL.0 as _,
                },
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
            ],
        },
        RasterizerState: D3D12_RASTERIZER_DESC {
            FillMode: D3D12_FILL_MODE_SOLID,
            CullMode: D3D12_CULL_MODE_NONE,
            FrontCounterClockwise: false.into(),
            DepthBias: D3D12_DEFAULT_DEPTH_BIAS,
            DepthBiasClamp: D3D12_DEFAULT_DEPTH_BIAS_CLAMP,
            SlopeScaledDepthBias: D3D12_DEFAULT_SLOPE_SCALED_DEPTH_BIAS,
            DepthClipEnable: true.into(),
            MultisampleEnable: false.into(),
            AntialiasedLineEnable: false.into(),
            ForcedSampleCount: 0,
            ConservativeRaster: D3D12_CONSERVATIVE_RASTERIZATION_MODE_OFF,
        },
        ..Default::default()
    };

    let pipeline_state = unsafe { device.CreateGraphicsPipelineState(&pso_desc)? };
    let _ = ManuallyDrop::into_inner(pso_desc.pRootSignature);

    Ok((root_signature, pipeline_state))
}

struct Buffer<T: Sized> {
    resource: ID3D12Resource,
    resource_capacity: usize,
    data: Vec<T>,
}

impl<T> Buffer<T> {
    fn new(device: &ID3D12Device, resource_capacity: usize) -> Result<Self> {
        let resource = Self::create_resource(device, resource_capacity)?;
        let data = Vec::with_capacity(resource_capacity);

        Ok(Self { resource, resource_capacity, data })
    }

    fn create_resource(device: &ID3D12Device, resource_capacity: usize) -> Result<ID3D12Resource> {
        util::try_out_ptr(|v| unsafe {
            device.CreateCommittedResource(
                &D3D12_HEAP_PROPERTIES {
                    Type: D3D12_HEAP_TYPE_UPLOAD,
                    CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
                    MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
                    CreationNodeMask: 0,
                    VisibleNodeMask: 0,
                },
                D3D12_HEAP_FLAG_NONE,
                &D3D12_RESOURCE_DESC {
                    Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
                    Alignment: 0,
                    Width: (resource_capacity * mem::size_of::<T>()) as u64,
                    Height: 1,
                    DepthOrArraySize: 1,
                    MipLevels: 1,
                    Format: DXGI_FORMAT_UNKNOWN,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
                    Flags: D3D12_RESOURCE_FLAG_NONE,
                },
                D3D12_RESOURCE_STATE_GENERIC_READ,
                None,
                v,
            )
        })
    }

    fn clear(&mut self) {
        self.data.clear();
    }

    fn extend<I: IntoIterator<Item = T>>(&mut self, it: I) {
        self.data.extend(it)
    }

    fn upload(&mut self, device: &ID3D12Device) -> Result<()> {
        let capacity = self.data.capacity();
        if capacity > self.resource_capacity {
            drop(mem::replace(&mut self.resource, Self::create_resource(device, capacity)?));
            self.resource_capacity = capacity;
        }

        unsafe {
            let mut resource_ptr = ptr::null_mut();
            self.resource.Map(0, None, Some(&mut resource_ptr))?;
            ptr::copy_nonoverlapping(self.data.as_ptr(), resource_ptr as *mut T, self.data.len());
            self.resource.Unmap(0, None);
        }

        Ok(())
    }
}

#[derive(Debug)]
#[allow(unused)]
struct Texture {
    resource: ID3D12Resource,
    gpu_desc: D3D12_GPU_DESCRIPTOR_HANDLE,
    /// Logical size the caller asked for.
    width: u32,
    height: u32,
    /// Size actually allocated. A square power-of-two texture samples as
    /// garbage on this game's D3D12 driver -- those are exactly the
    /// dimensions for which the driver picks its optimal swizzled layout
    /// (D3D12_TEXTURE_LAYOUT_UNKNOWN lets it choose), and that layout does not
    /// survive the CopyTextureRegion path here. 2000x2000 renders fine while
    /// 2048x2048 does not, at identical byte sizes; likewise 4000 vs 4096.
    /// Padding any power-of-two dimension by one texel keeps the driver on a
    /// layout that works. The padding column/row duplicates its neighbour, so
    /// sampling uv 0..1 looks unchanged.
    alloc_width: u32,
    alloc_height: u32,
    /// True when the resource lives on a CPU-writable CUSTOM heap and is
    /// filled with WriteToSubresource instead of CopyTextureRegion.
    cpu_writable: bool,
    /// The state the resource is currently left in.
    ///
    /// Without this, `upload_texture` is only correct the *first* time it runs:
    /// it unconditionally copies into the resource and then emits a
    /// COPY_DEST -> PIXEL_SHADER_RESOURCE barrier. On any later call (i.e. via
    /// `replace_texture`) the resource is already in PIXEL_SHADER_RESOURCE, so
    /// the copy targets a resource that is not in COPY_DEST and the barrier
    /// declares a StateBefore that does not match reality. Both are undefined
    /// behaviour; in practice the upload silently does not land and the texture
    /// samples as garbage.
    state: D3D12_RESOURCE_STATES,
}

struct TextureHeap {
    device: ID3D12Device,
    srv_heap: ID3D12DescriptorHeap,
    srv_staging_heap: ID3D12DescriptorHeap,
    textures: Vec<Texture>,
    command_queue: ID3D12CommandQueue,
    command_allocator: ID3D12CommandAllocator,
    command_list: ID3D12GraphicsCommandList,
    fence: Fence,
}

impl TextureHeap {
    fn new(device: &ID3D12Device, srv_heap: ID3D12DescriptorHeap) -> Result<Self> {
        let command_queue = unsafe {
            device.CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC {
                Type: D3D12_COMMAND_LIST_TYPE_DIRECT,
                Priority: 0,
                Flags: D3D12_COMMAND_QUEUE_FLAG_NONE,
                NodeMask: 0,
            })
        }?;

        let command_allocator: ID3D12CommandAllocator =
            unsafe { device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT) }?;

        let command_list: ID3D12GraphicsCommandList = unsafe {
            device.CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &command_allocator, None)
        }?;

        unsafe {
            command_list.Close()?;
            command_allocator.SetName(w!("hudhook Render Engine Command Allocator"))?;
            command_list.SetName(w!("hudhook Render Engine Command List"))?;
        }

        let srv_staging_heap: ID3D12DescriptorHeap = unsafe {
            device.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                Type: D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
                NumDescriptors: 8,
                Flags: D3D12_DESCRIPTOR_HEAP_FLAG_NONE,
                NodeMask: 0,
            })
        }?;

        let fence = Fence::new(device)?;

        Ok(Self {
            device: device.clone(),
            srv_heap,
            srv_staging_heap,
            textures: Vec::new(),
            command_queue,
            command_allocator,
            command_list,
            fence,
        })
    }

    unsafe fn resize_heap(&mut self) -> Result<()> {
        let mut desc = self.srv_heap.GetDesc();
        let mut desc_staging = self.srv_staging_heap.GetDesc();
        let old_num_descriptors = desc.NumDescriptors;

        if old_num_descriptors <= self.textures.len() as _ {
            desc.NumDescriptors *= 2;
            desc_staging.NumDescriptors = desc.NumDescriptors;

            let srv_heap: ID3D12DescriptorHeap = self.device.CreateDescriptorHeap(&desc)?;
            let srv_staging_heap: ID3D12DescriptorHeap =
                self.device.CreateDescriptorHeap(&desc_staging)?;
            self.device.CopyDescriptorsSimple(
                old_num_descriptors,
                srv_staging_heap.GetCPUDescriptorHandleForHeapStart(),
                self.srv_staging_heap.GetCPUDescriptorHandleForHeapStart(),
                D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
            );
            self.device.CopyDescriptorsSimple(
                old_num_descriptors,
                srv_heap.GetCPUDescriptorHandleForHeapStart(),
                srv_staging_heap.GetCPUDescriptorHandleForHeapStart(),
                D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
            );
            self.srv_heap = srv_heap;
            self.srv_staging_heap = srv_staging_heap;

            // Adjust texture GPU pointers.
            let gpu_heap_start = self.srv_heap.GetGPUDescriptorHandleForHeapStart();
            let heap_inc_size = self
                .device
                .GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV);

            self.textures.iter_mut().enumerate().for_each(|(texture_index, texture)| {
                texture.gpu_desc = D3D12_GPU_DESCRIPTOR_HANDLE {
                    ptr: gpu_heap_start.ptr + (texture_index as u32 * heap_inc_size) as u64,
                };
            })
        }

        Ok(())
    }

    unsafe fn create_texture(&mut self, width: u32, height: u32) -> Result<TextureId> {
        self.resize_heap()?;

        // (An earlier attempt padded power-of-two dimensions by one texel on the
        // theory that the driver's swizzled layout for those sizes was the
        // problem. It made no difference -- 4096 allocated as 4097x4097 was
        // still corrupt -- so the plumbing stays but the padding is off.)
        let alloc_width = width;
        let alloc_height = height;

        let cpu_heap_stg_start = self.srv_staging_heap.GetCPUDescriptorHandleForHeapStart();
        let cpu_heap_start = self.srv_heap.GetCPUDescriptorHandleForHeapStart();
        let gpu_heap_start = self.srv_heap.GetGPUDescriptorHandleForHeapStart();
        let heap_inc_size =
            self.device.GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV);

        let texture_index = self.textures.len() as u32;

        let cpu_desc_stg = D3D12_CPU_DESCRIPTOR_HANDLE {
            ptr: cpu_heap_stg_start.ptr + (texture_index * heap_inc_size) as usize,
        };

        let cpu_desc = D3D12_CPU_DESCRIPTOR_HANDLE {
            ptr: cpu_heap_start.ptr + (texture_index * heap_inc_size) as usize,
        };

        let gpu_desc = D3D12_GPU_DESCRIPTOR_HANDLE {
            ptr: gpu_heap_start.ptr + (texture_index * heap_inc_size) as u64,
        };

        // Preferred path: a CPU-writable CUSTOM heap, filled with
        // WriteToSubresource.
        //
        // The usual DEFAULT-heap + upload-buffer + CopyTextureRegion path was
        // verified to hand the GPU byte-perfect data (the upload buffer is read
        // back and measured just before the copy), and the result still comes
        // out with the image's structure intact but its texels shuffled within
        // small blocks -- the signature of the driver's tiled/swizzled texture
        // layout not being applied by the copy. WriteToSubresource performs
        // that same linear->tiled conversion on the CPU inside the driver, so
        // it sidesteps CopyTextureRegion, the upload buffer, the second command
        // queue, the fence and the resource barriers entirely.
        let desc = D3D12_RESOURCE_DESC {
            Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
            Alignment: 0,
            Width: alloc_width as _,
            Height: alloc_height as _,
            DepthOrArraySize: 1,
            MipLevels: 1,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
            Flags: D3D12_RESOURCE_FLAG_NONE,
        };

        let custom: Option<ID3D12Resource> = util::try_out_ptr(|v| unsafe {
            self.device.CreateCommittedResource(
                &D3D12_HEAP_PROPERTIES {
                    Type: D3D12_HEAP_TYPE_CUSTOM,
                    CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_WRITE_COMBINE,
                    MemoryPoolPreference: D3D12_MEMORY_POOL_L0,
                    CreationNodeMask: 1,
                    VisibleNodeMask: 1,
                },
                D3D12_HEAP_FLAG_NONE,
                &desc,
                D3D12_RESOURCE_STATE_GENERIC_READ,
                None,
                v,
            )
        })
        .ok();

        let cpu_writable = custom.is_some();
        if cpu_writable {
            debug!("create_texture {alloc_width}x{alloc_height}: CPU-writable CUSTOM heap");
        } else {
            debug!("create_texture {alloc_width}x{alloc_height}: CUSTOM heap unavailable, using upload+copy");
        }

        let texture: ID3D12Resource = match custom {
            Some(t) => t,
            None => util::try_out_ptr(|v| unsafe {
            self.device.CreateCommittedResource(
                &D3D12_HEAP_PROPERTIES {
                    Type: D3D12_HEAP_TYPE_DEFAULT,
                    CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
                    MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
                    CreationNodeMask: Default::default(),
                    VisibleNodeMask: Default::default(),
                },
                D3D12_HEAP_FLAG_NONE,
                &D3D12_RESOURCE_DESC {
                    Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
                    Alignment: 0,
                    Width: alloc_width as _,
                    Height: alloc_height as _,
                    DepthOrArraySize: 1,
                    MipLevels: 1,
                    Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
                    Flags: D3D12_RESOURCE_FLAG_NONE,
                },
                // COMMON, not COPY_DEST: this resource is touched by two
                // different queues, and COMMON is the only state from which
                // D3D12 will hand ownership between them. The copy below
                // promotes it to COPY_DEST implicitly.
                D3D12_RESOURCE_STATE_COMMON,
                None,
                v,
            )
            })?,
        };

        self.device.CreateShaderResourceView(
            &texture,
            Some(&D3D12_SHADER_RESOURCE_VIEW_DESC {
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                ViewDimension: D3D12_SRV_DIMENSION_TEXTURE2D,
                Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
                Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
                    Texture2D: D3D12_TEX2D_SRV {
                        MostDetailedMip: 0,
                        MipLevels: 1,
                        PlaneSlice: Default::default(),
                        ResourceMinLODClamp: Default::default(),
                    },
                },
            }),
            cpu_desc_stg,
        );

        self.device.CopyDescriptorsSimple(
            1,
            cpu_desc,
            cpu_desc_stg,
            D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
        );

        let id = TextureId::from(self.textures.len());
        self.textures.push(Texture {
            resource: texture.clone(),
            gpu_desc,
            width,
            height,
            alloc_width,
            alloc_height,
            cpu_writable,
            state: if cpu_writable {
                D3D12_RESOURCE_STATE_GENERIC_READ
            } else {
                D3D12_RESOURCE_STATE_COMMON
            },
        });

        Ok(id)
    }

    unsafe fn upload_texture(
        &mut self,
        texture_id: TextureId,
        data: &[u8],
        width: u32,
        height: u32,
    ) -> Result<()> {
        let (tex_resource, tex_state, alloc_width, alloc_height, cpu_writable) = {
            let texture = &self.textures[texture_id.id()];
            if texture.width != width || texture.height != height {
                error!(
                    "image size {width}x{height} do not match expected {}x{}",
                    texture.width, texture.height
                );
                return Err(Error::from_hresult(HRESULT(-1)));
            }
            (
                texture.resource.clone(),
                texture.state,
                texture.alloc_width,
                texture.alloc_height,
                texture.cpu_writable,
            )
        };

        // WORKAROUND: clamp alpha 255 -> 254.
        //
        // On this game + driver, any texel uploaded with alpha == 255 comes
        // back from the sampler scrambled, while the identical pixels uploaded
        // with alpha == 200 sample perfectly. Verified in both directions: the
        // map (opaque, always corrupt) renders correctly at alpha 200, and
        // mapwraper (mostly transparent, always fine) breaks when its alpha is
        // forced to 255. Nothing about the upload path matters -- the same
        // corruption survives CopyTextureRegion, WriteToSubresource, every
        // pitch, size, heap type, resource state and queue arrangement tried.
        //
        // The strong suspicion is the game's frame-generation UI handling
        // (1.0.20 integrated FSR4 frame generation) treating fully-opaque
        // overlay pixels as something to reproject. 254 vs 255 is a 0.4%
        // opacity difference -- invisible -- and it sidesteps the whole thing.
        let mut clamped = data.to_vec();
        for px in clamped.chunks_exact_mut(4) {
            if px[3] == 255 {
                px[3] = 254;
            }
        }
        let data: &[u8] = &clamped;

        // CPU-writable path: hand the rows straight to the driver and let it do
        // the linear -> tiled conversion. No upload buffer, no command list, no
        // queue, no fence, no barriers.
        if cpu_writable {
            let src_row = width * 4;
            tex_resource.WriteToSubresource(
                0,
                None,
                data.as_ptr() as *const _,
                src_row,
                src_row * height,
            )?;
            debug!("upload_texture {width}x{height}: WriteToSubresource");
            return Ok(());
        }

        let src_row_size = width * 4;
        let upload_row_size = alloc_width * 4;
        let align = D3D12_TEXTURE_DATA_PITCH_ALIGNMENT;

        // Ask the device for the authoritative copy layout instead of guessing
        // it. hand-computing `align(width * 4, 256)` happens to agree with the
        // driver most of the time, but it is not the contract -- the driver is
        // the only thing that knows how it wants a buffer laid out for a
        // buffer->texture copy.
        let dst_desc = tex_resource.GetDesc();
        let mut layout = D3D12_PLACED_SUBRESOURCE_FOOTPRINT::default();
        let mut num_rows: u32 = 0;
        let mut row_size_bytes: u64 = 0;
        let mut total_bytes: u64 = 0;
        self.device.GetCopyableFootprints(
            &dst_desc,
            0,
            1,
            0,
            Some(&mut layout),
            Some(&mut num_rows),
            Some(&mut row_size_bytes),
            Some(&mut total_bytes),
        );

        let guessed_pitch = upload_row_size.div_ceil(align) * align;
        let upload_pitch = layout.Footprint.RowPitch.max(guessed_pitch);

        let upload_size = alloc_height * upload_pitch;


        let upload_buffer: ID3D12Resource = util::try_out_ptr(|v| unsafe {
            self.device.CreateCommittedResource(
                &D3D12_HEAP_PROPERTIES {
                    Type: D3D12_HEAP_TYPE_UPLOAD,
                    CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
                    MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
                    CreationNodeMask: Default::default(),
                    VisibleNodeMask: Default::default(),
                },
                D3D12_HEAP_FLAG_NONE,
                &D3D12_RESOURCE_DESC {
                    Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
                    Alignment: 0,
                    Width: upload_size as _,
                    Height: 1,
                    DepthOrArraySize: 1,
                    MipLevels: 1,
                    Format: DXGI_FORMAT_UNKNOWN,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
                    Flags: D3D12_RESOURCE_FLAG_NONE,
                },
                D3D12_RESOURCE_STATE_GENERIC_READ,
                None,
                v,
            )
        })?;

        let mut upload_buffer_ptr = ptr::null_mut();
        upload_buffer.Map(0, None, Some(&mut upload_buffer_ptr))?;
        // Copy row by row into the (possibly padded) destination. Padding
        // columns and rows duplicate their neighbour so that sampling across
        // the full uv 0..1 range looks identical to the unpadded image.
        //
        // Historical note: this used to take a bulk-memcpy shortcut whenever
        // the row size happened to be 256-aligned. That shortcut is equivalent
        // when the pitches match, and removing it changed nothing -- keeping
        // the row loop only because it is the general case.
        //
        // The original code took a single bulk memcpy shortcut when the row
        // size happened to be 256-byte aligned (upload_row_size == upload_pitch).
        // Empirically, every texture that took that shortcut came out of the
        // GPU as noise, and every texture that went row by row was fine --
        // across 22 textures with no exception:
        //
        //   per-row : 286x286 icons, 2000x2000, 4000x4000, 729x4000  -> correct
        //   bulk    : 4096x4096, 2048x2048, 1024x1024, 256x256       -> garbage
        //
        // The two paths write the same bytes when the pitches are equal, so
        // this alone should not matter; going row-by-row unconditionally
        // removes the variable while costing nothing measurable.
        let last_texel = ((width - 1) * 4) as usize;
        for y in 0..alloc_height {
            let src_y = if y < height { y } else { height - 1 };
            let src = data.as_ptr().add((src_y * src_row_size) as usize);
            let dst = (upload_buffer_ptr as *mut u8).add((y * upload_pitch) as usize);
            ptr::copy_nonoverlapping(src, dst, src_row_size as usize);
            for x in width..alloc_width {
                ptr::copy_nonoverlapping(src.add(last_texel), dst.add((x * 4) as usize), 4);
            }
        }
        upload_buffer.Unmap(0, None);

        self.command_allocator.Reset()?;
        self.command_list.Reset(&self.command_allocator, None)?;

        // This texture is written here, on the TextureHeap's OWN command queue,
        // and later sampled on the render engine's queue. Two different queues.
        //
        // D3D12 requires a resource shared between queues to pass through
        // D3D12_RESOURCE_STATE_COMMON to transfer ownership: that is the
        // transition that flushes and resolves any driver-side compression
        // metadata (DCC and friends). Going straight from COPY_DEST to
        // PIXEL_SHADER_RESOURCE on the upload queue skips that, and the render
        // queue then samples the texture with metadata it does not understand
        // -- which reads back as a deterministic scramble of the real pixels,
        // exactly the corruption seen here. Whether a given texture uses
        // compression at all is a driver decision based on size and usage,
        // which is why two 4000x4000 textures could behave differently.
        //
        // So: leave the resource in COMMON, and let implicit state promotion
        // handle both the copy (COMMON -> COPY_DEST) and the sampling
        // (COMMON -> PIXEL_SHADER_RESOURCE) on whichever queue needs it.
        let pre_barriers = if tex_state == D3D12_RESOURCE_STATE_COMMON
            || tex_state == D3D12_RESOURCE_STATE_COPY_DEST
        {
            Vec::new()
        } else {
            vec![util::create_barrier(&tex_resource, tex_state, D3D12_RESOURCE_STATE_COMMON)]
        };
        if !pre_barriers.is_empty() {
            self.command_list.ResourceBarrier(&pre_barriers);
        }

        let dst_location = D3D12_TEXTURE_COPY_LOCATION {
            pResource: ManuallyDrop::new(Some(tex_resource.clone())),
            Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
            Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 { SubresourceIndex: 0 },
        };

        let src_location = D3D12_TEXTURE_COPY_LOCATION {
            pResource: ManuallyDrop::new(Some(upload_buffer.clone())),
            Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
            Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                PlacedFootprint: D3D12_PLACED_SUBRESOURCE_FOOTPRINT {
                    Offset: 0,
                    Footprint: D3D12_SUBRESOURCE_FOOTPRINT {
                        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                        Width: alloc_width,
                        Height: alloc_height,
                        Depth: 1,
                        RowPitch: upload_pitch,
                    },
                },
            },
        };

        self.command_list.CopyTextureRegion(&dst_location, 0, 0, 0, &src_location, None);
        // Back to COMMON, not to PIXEL_SHADER_RESOURCE -- see above.
        let barriers = [util::create_barrier(
            &tex_resource,
            D3D12_RESOURCE_STATE_COPY_DEST,
            D3D12_RESOURCE_STATE_COMMON,
        )];

        self.command_list.ResourceBarrier(&barriers);
        self.command_list.Close()?;
        self.command_queue.ExecuteCommandLists(&[Some(self.command_list.cast()?)]);
        self.command_queue.Signal(self.fence.fence(), self.fence.value())?;
        self.fence.wait()?;
        self.fence.incr();

        barriers.into_iter().for_each(util::drop_barrier);
        pre_barriers.into_iter().for_each(util::drop_barrier);

        // Remember where we left the resource, so the next upload knows to
        // transition it back to COPY_DEST first.
        self.textures[texture_id.id()].state = D3D12_RESOURCE_STATE_COMMON;

        // Apparently, leaking the upload buffer into the location is necessary.
        // Uncommenting the following line consistently leads to a crash, which
        // points to a double-free, but I don't know why: upload_buffer should
        // stay alive with a positive refcount until the end of this block.
        // let _ = ManuallyDrop::into_inner(src_location.pResource);
        let _ = ManuallyDrop::into_inner(dst_location.pResource);

        Ok(())
    }
}
