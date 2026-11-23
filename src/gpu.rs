//! The overlay's graphics: a transparent DirectComposition swap chain, Direct3D for the instanced sprites,
//! and Direct2D on the same surface for ink, particles, props and speech bubbles.
use std::ffi::c_void;
use std::mem::{size_of, ManuallyDrop};

use windows::core::{s, w, Interface, Result, HSTRING, PCSTR};
use windows::Foundation::Numerics::Matrix3x2;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectComposition::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;

use crate::sprites;
use crate::wear::Wear;

#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq)]
pub struct Instance {
    pub x: f32, pub y: f32, pub unit: f32, pub sy: f32, pub angle: f32, pub frame: f32, pub flip: f32, pub eye: f32,
    pub tint: [f32; 3], pub shadow: f32, pub tint2: [f32; 3], pub pattern: f32,
}

pub type Color = [f32; 4];

fn d2c(c: Color) -> D2D1_COLOR_F { D2D1_COLOR_F { r: c[0], g: c[1], b: c[2], a: c[3] } }
pub fn pt(x: f32, y: f32) -> D2D_POINT_2F { D2D_POINT_2F { x, y } }

pub struct Gpu {
    pub width: u32,
    pub height: u32,
    pub present_ms: std::cell::Cell<f32>,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    swap: IDXGISwapChain1,
    _dcomp: IDCompositionDevice,
    _target: IDCompositionTarget,
    _visual: IDCompositionVisual,
    rtv: Option<ID3D11RenderTargetView>,
    sprite_vs: ID3D11VertexShader,
    sprite_ps: ID3D11PixelShader,
    shadow_vs: ID3D11VertexShader,
    shadow_ps: ID3D11PixelShader,
    glow_vs: ID3D11VertexShader,
    glow_ps: ID3D11PixelShader,
    glow_view: ID3D11ShaderResourceView,
    sampler: ID3D11SamplerState,
    instances: ID3D11Buffer,
    instance_view: ID3D11ShaderResourceView,
    atlas_view: ID3D11ShaderResourceView,
    constants: ID3D11Buffer,
    blend: ID3D11BlendState,
    raster: ID3D11RasterizerState,
    capacity: usize,
    factory: ID2D1Factory1,
    pub d2d: ID2D1DeviceContext,
    brush: ID2D1SolidColorBrush,
    round: ID2D1StrokeStyle,
    dwrite: IDWriteFactory,
    pub bubble_font: IDWriteTextFormat,
    pub glyph_font: IDWriteTextFormat,
    pub zzz_font: IDWriteTextFormat,
    pub boom_font: IDWriteTextFormat,
    pub menu_font: IDWriteTextFormat,
}

unsafe fn compile(source: &str, entry: PCSTR, target: PCSTR) -> Result<ID3DBlob> {
    let (mut blob, mut errors) = (None, None);
    let result = D3DCompile(source.as_ptr() as _, source.len(), PCSTR::null(), None, None, entry, target, 0, 0, &mut blob, Some(&mut errors));
    if let (Err(_), Some(e)) = (&result, &errors) {
        let text = std::slice::from_raw_parts(e.GetBufferPointer() as *const u8, e.GetBufferSize());
        eprintln!("{}", String::from_utf8_lossy(text));
    }
    result?;
    Ok(blob.unwrap())
}

unsafe fn blob_bytes(b: &ID3DBlob) -> &[u8] { std::slice::from_raw_parts(b.GetBufferPointer() as *const u8, b.GetBufferSize()) }

unsafe fn vs(d: &ID3D11Device, src: &str, entry: PCSTR) -> Result<ID3D11VertexShader> {
    let blob = compile(src, entry, s!("vs_5_0"))?;
    let mut out = None;
    d.CreateVertexShader(blob_bytes(&blob), None, Some(&mut out))?;
    Ok(out.unwrap())
}

unsafe fn ps(d: &ID3D11Device, src: &str, entry: PCSTR) -> Result<ID3D11PixelShader> {
    let blob = compile(src, entry, s!("ps_5_0"))?;
    let mut out = None;
    d.CreatePixelShader(blob_bytes(&blob), None, Some(&mut out))?;
    Ok(out.unwrap())
}

unsafe fn instance_buffer(d: &ID3D11Device, capacity: usize) -> Result<(ID3D11Buffer, ID3D11ShaderResourceView)> {
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: (capacity * size_of::<Instance>()) as u32, Usage: D3D11_USAGE_DYNAMIC,
        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32, CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
        MiscFlags: D3D11_RESOURCE_MISC_BUFFER_STRUCTURED.0 as u32, StructureByteStride: size_of::<Instance>() as u32,
    };
    let (mut buffer, mut view) = (None, None);
    d.CreateBuffer(&desc, None, Some(&mut buffer))?;
    let buffer = buffer.unwrap();
    d.CreateShaderResourceView(&buffer, None, Some(&mut view))?;
    Ok((buffer, view.unwrap()))
}

unsafe fn text_format(f: &IDWriteFactory, family: &str, weight: DWRITE_FONT_WEIGHT, size: f32) -> Result<IDWriteTextFormat> {
    f.CreateTextFormat(&HSTRING::from(family), None, weight, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size, w!("en-us"))
}

impl Gpu {
    pub fn new(hwnd: HWND, width: u32, height: u32, ui: f32, software: bool) -> Result<Self> {
        unsafe {
            let (mut device, mut context) = (None, None);
            let mut make = |driver| D3D11CreateDevice(None, driver, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                                                      Some(&[D3D_FEATURE_LEVEL_11_0]), D3D11_SDK_VERSION, Some(&mut device), None, Some(&mut context));
            // PCs and virtual machines without a usable GPU fall back to Windows' software renderer.
            if software || make(D3D_DRIVER_TYPE_HARDWARE).is_err() { make(D3D_DRIVER_TYPE_WARP)?; }
            let device: ID3D11Device = device.unwrap();
            let context: ID3D11DeviceContext = context.unwrap();
            let dxgi: IDXGIDevice = device.cast()?;
            let factory2: IDXGIFactory2 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0))?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: width, Height: height, Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT, BufferCount: 2,
                Scaling: DXGI_SCALING_STRETCH, SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED, ..Default::default()
            };
            let swap = factory2.CreateSwapChainForComposition(&device, &desc, None)?;
            let dcomp: IDCompositionDevice = DCompositionCreateDevice(&dxgi)?;
            let target = dcomp.CreateTargetForHwnd(hwnd, true)?;
            let visual = dcomp.CreateVisual()?;
            visual.SetContent(&swap)?;
            target.SetRoot(&visual)?;
            dcomp.Commit()?;

            let src = include_str!("shaders.hlsl");
            let sprite_vs = vs(&device, src, s!("SpriteVS"))?;
            let sprite_ps = ps(&device, src, s!("SpritePS"))?;
            let shadow_vs = vs(&device, src, s!("ShadowVS"))?;
            let shadow_ps = ps(&device, src, s!("ShadowPS"))?;
            let glow_vs = vs(&device, src, s!("GlowVS"))?;
            let glow_ps = ps(&device, src, s!("GlowPS"))?;
            let halo = sprites::glow_atlas();
            let halo_w = sprites::GLOW_W * sprites::FRAME_COUNT;
            let halo_desc = D3D11_TEXTURE2D_DESC {
                Width: halo_w as u32, Height: sprites::GLOW_H as u32, MipLevels: 1, ArraySize: 1, Format: DXGI_FORMAT_R8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 }, Usage: D3D11_USAGE_IMMUTABLE,
                BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32, ..Default::default()
            };
            let halo_init = D3D11_SUBRESOURCE_DATA { pSysMem: halo.as_ptr() as _, SysMemPitch: halo_w as u32, SysMemSlicePitch: 0 };
            let (mut halo_tex, mut glow_view, mut sampler) = (None, None, None);
            device.CreateTexture2D(&halo_desc, Some(&halo_init), Some(&mut halo_tex))?;
            device.CreateShaderResourceView(&halo_tex.unwrap(), None, Some(&mut glow_view))?;
            device.CreateSamplerState(&D3D11_SAMPLER_DESC {
                Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR, AddressU: D3D11_TEXTURE_ADDRESS_CLAMP, AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressW: D3D11_TEXTURE_ADDRESS_CLAMP, MaxLOD: f32::MAX, ..Default::default()
            }, Some(&mut sampler))?;

            let cells = sprites::atlas();
            let atlas_w = sprites::W * sprites::FRAME_COUNT;
            let atlas_desc = D3D11_TEXTURE2D_DESC {
                Width: atlas_w as u32, Height: sprites::H as u32, MipLevels: 1, ArraySize: 1, Format: DXGI_FORMAT_R8_UINT,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 }, Usage: D3D11_USAGE_IMMUTABLE,
                BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32, ..Default::default()
            };
            let init = D3D11_SUBRESOURCE_DATA { pSysMem: cells.as_ptr() as _, SysMemPitch: atlas_w as u32, SysMemSlicePitch: 0 };
            let (mut atlas, mut atlas_view) = (None, None);
            device.CreateTexture2D(&atlas_desc, Some(&init), Some(&mut atlas))?;
            device.CreateShaderResourceView(&atlas.unwrap(), None, Some(&mut atlas_view))?;

            let capacity = 256;
            let (instances, instance_view) = instance_buffer(&device, capacity)?;
            let const_desc = D3D11_BUFFER_DESC { ByteWidth: 16, Usage: D3D11_USAGE_DYNAMIC, BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
                                                 CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32, ..Default::default() };
            let mut constants = None;
            device.CreateBuffer(&const_desc, None, Some(&mut constants))?;

            let mut blend_desc = D3D11_BLEND_DESC::default();
            blend_desc.RenderTarget[0] = D3D11_RENDER_TARGET_BLEND_DESC {
                BlendEnable: TRUE, SrcBlend: D3D11_BLEND_ONE, DestBlend: D3D11_BLEND_INV_SRC_ALPHA, BlendOp: D3D11_BLEND_OP_ADD,
                SrcBlendAlpha: D3D11_BLEND_ONE, DestBlendAlpha: D3D11_BLEND_INV_SRC_ALPHA, BlendOpAlpha: D3D11_BLEND_OP_ADD,
                RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8,
            };
            let (mut blend, mut raster) = (None, None);
            device.CreateBlendState(&blend_desc, Some(&mut blend))?;
            let raster_desc = D3D11_RASTERIZER_DESC { FillMode: D3D11_FILL_SOLID, CullMode: D3D11_CULL_NONE, DepthClipEnable: TRUE, ..Default::default() };
            device.CreateRasterizerState(&raster_desc, Some(&mut raster))?;

            let factory: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let d2_device = factory.CreateDevice(&dxgi)?;
            let d2d = d2_device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            d2d.SetDpi(96.0, 96.0);
            d2d.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            let brush = d2d.CreateSolidColorBrush(&d2c([1.0; 4]), None)?;
            let round: ID2D1StrokeStyle = factory.CreateStrokeStyle(&D2D1_STROKE_STYLE_PROPERTIES1 {
                startCap: D2D1_CAP_STYLE_ROUND, endCap: D2D1_CAP_STYLE_ROUND, dashCap: D2D1_CAP_STYLE_ROUND,
                lineJoin: D2D1_LINE_JOIN_ROUND, miterLimit: 10.0, dashStyle: D2D1_DASH_STYLE_SOLID, dashOffset: 0.0,
                transformType: D2D1_STROKE_TRANSFORM_TYPE_NORMAL,
            }, None)?.into();
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let bubble_font = text_format(&dwrite, "Segoe UI", DWRITE_FONT_WEIGHT_NORMAL, 13.5 * ui)?;
            let glyph_font = text_format(&dwrite, "Segoe UI Symbol", DWRITE_FONT_WEIGHT_BOLD, 18.0 * ui)?;
            let zzz_font = text_format(&dwrite, "Segoe UI", DWRITE_FONT_WEIGHT_BOLD, 16.0 * ui)?;
            let boom_font = text_format(&dwrite, "Impact", DWRITE_FONT_WEIGHT_NORMAL, 48.0 * ui)?;
            let menu_font = text_format(&dwrite, "Segoe UI", DWRITE_FONT_WEIGHT_SEMI_BOLD, 11.0 * ui)?;
            for f in [&glyph_font, &zzz_font, &boom_font, &menu_font] {
                f.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
                f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            }

            let mut gpu = Gpu {
                width, height, present_ms: std::cell::Cell::new(0.0), device, context, swap, _dcomp: dcomp, _target: target, _visual: visual, rtv: None, sprite_vs, sprite_ps,
                shadow_vs, shadow_ps, glow_vs, glow_ps, glow_view: glow_view.unwrap(), sampler: sampler.unwrap(), instances, instance_view, atlas_view: atlas_view.unwrap(), constants: constants.unwrap(),
                blend: blend.unwrap(), raster: raster.unwrap(), capacity, factory, d2d, brush, round, dwrite,
                bubble_font, glyph_font, zzz_font, boom_font, menu_font,
            };
            gpu.bind_targets()?;
            // Windows loads a font the first time it's drawn, which can freeze a frame, so do that now.
            gpu.clear();
            gpu.begin();
            for font in [&gpu.bubble_font, &gpu.glyph_font, &gpu.zzz_font, &gpu.boom_font, &gpu.menu_font] {
                gpu.text("Az09!?♪♫✦↻★➤♥☾✎▬✕✸♨☂●❄☻▲▼◎⚙✉⌨‖◌✖‹", font, 200.0, 200.0, [0.0, 0.0, 0.0, 0.01]);
            }
            if let Ok(f) = gpu.handwriting(28.0) { gpu.text("Az09", &f, 200.0, 200.0, [0.0, 0.0, 0.0, 0.01]); }
            gpu.end()?;
            gpu.clear();
            Ok(gpu)
        }
    }

    unsafe fn bind_targets(&mut self) -> Result<()> {
        let back: ID3D11Texture2D = self.swap.GetBuffer(0)?;
        let mut rtv = None;
        self.device.CreateRenderTargetView(&back, None, Some(&mut rtv))?;
        self.rtv = rtv;
        let surface: IDXGISurface = back.cast()?;
        let props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
            dpiX: 96.0, dpiY: 96.0, bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
            colorContext: ManuallyDrop::new(None),
        };
        let bitmap = self.d2d.CreateBitmapFromDxgiSurface(&surface, Some(&props))?;
        self.d2d.SetTarget(&bitmap);
        Ok(())
    }

    /// The desktop changed size: a monitor was added, removed or rescaled.
    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        unsafe {
            self.d2d.SetTarget(None);
            self.rtv = None;
            self.context.OMSetRenderTargets(None, None);
            self.context.ClearState();
            self.context.Flush();
            self.swap.ResizeBuffers(2, width, height, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SWAP_CHAIN_FLAG(0))?;
            self.width = width;
            self.height = height;
            self.bind_targets()
        }
    }

    pub fn clear(&self) {
        unsafe { self.context.ClearRenderTargetView(self.rtv.as_ref().unwrap(), &[0.0; 4]); }
    }

    /// Draw every buddy in two instanced calls: shadows, then sprites.
    pub fn draw_sprites(&mut self, items: &[Instance], glow: f32) -> Result<()> {
        if items.is_empty() { return Ok(()); }
        unsafe {
            if items.len() > self.capacity {
                self.capacity = items.len().next_power_of_two();
                let (b, v) = instance_buffer(&self.device, self.capacity)?;
                self.instances = b;
                self.instance_view = v;
            }
            let c = &self.context;
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            c.Map(&self.instances, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped))?;
            std::ptr::copy_nonoverlapping(items.as_ptr(), mapped.pData as *mut Instance, items.len());
            c.Unmap(&self.instances, 0);
            c.Map(&self.constants, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped))?;
            std::ptr::copy_nonoverlapping([self.width as f32, self.height as f32, glow, 0.0].as_ptr(), mapped.pData as *mut f32, 4);
            c.Unmap(&self.constants, 0);
            // Direct2D shares this device and changes its state, so everything is set again each frame.
            c.IASetInputLayout(None);
            c.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
            c.RSSetState(&self.raster);
            c.RSSetViewports(Some(&[D3D11_VIEWPORT { Width: self.width as f32, Height: self.height as f32, MaxDepth: 1.0, ..Default::default() }]));
            c.OMSetBlendState(&self.blend, Some(&[0.0; 4]), u32::MAX);
            c.OMSetDepthStencilState(None, 0);
            c.OMSetRenderTargets(Some(&[self.rtv.clone()]), None);
            c.GSSetShader(None, None);
            c.VSSetConstantBuffers(0, Some(&[Some(self.constants.clone())]));
            c.VSSetShaderResources(0, Some(&[Some(self.instance_view.clone())]));
            c.PSSetShaderResources(1, Some(&[Some(self.atlas_view.clone())]));
            c.VSSetShader(&self.shadow_vs, None);
            c.PSSetShader(&self.shadow_ps, None);
            c.DrawInstanced(4, items.len() as u32, 0, 0);
            if glow > 0.0 {
                c.PSSetConstantBuffers(0, Some(&[Some(self.constants.clone())]));
                c.PSSetShaderResources(2, Some(&[Some(self.glow_view.clone())]));
                c.PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
                c.VSSetShader(&self.glow_vs, None);
                c.PSSetShader(&self.glow_ps, None);
                c.DrawInstanced(4, items.len() as u32, 0, 0);
            }
            c.VSSetShader(&self.sprite_vs, None);
            c.PSSetShader(&self.sprite_ps, None);
            c.DrawInstanced(4, items.len() as u32, 0, 0);
            c.OMSetRenderTargets(None, None);
            c.Flush();
        }
        Ok(())
    }

    pub fn present(&self) -> Result<()> {
        let began = std::time::Instant::now();
        let result = unsafe { self.swap.Present(1, DXGI_PRESENT(0)).ok() };
        self.present_ms.set(began.elapsed().as_secs_f32() * 1000.0);
        result
    }

    // ---- Direct2D helpers ---------------------------------------------------------

    pub fn image(&self, w: &Wear) -> Result<ID2D1Bitmap1> {
        let props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
            dpiX: 96.0, dpiY: 96.0, bitmapOptions: D2D1_BITMAP_OPTIONS_NONE, colorContext: ManuallyDrop::new(None),
        };
        unsafe { self.d2d.CreateBitmap(D2D_SIZE_U { width: w.w, height: w.h }, Some(w.bgra.as_ptr() as *const c_void), w.w * 4, &props) }
    }

    /// Small pictures are pixel art and stay crisp. Bigger ones are smoothed.
    pub fn picture(&self, b: &ID2D1Bitmap1, x: f32, y: f32, w: f32, h: f32, crisp: bool) {
        let mode = if crisp { D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR } else { D2D1_INTERPOLATION_MODE_LINEAR };
        unsafe { self.d2d.DrawBitmap(b, Some(&D2D_RECT_F { left: x, top: y, right: x + w, bottom: y + h }), 1.0, mode, None, None); }
    }

    /// Rotate what's drawn next around a point, until `unturn`.
    pub fn turn(&self, angle: f32, cx: f32, cy: f32) {
        let (c, s) = (angle.cos(), angle.sin());
        let m = Matrix3x2 { M11: c, M12: s, M21: -s, M22: c, M31: cx - cx * c + cy * s, M32: cy - cx * s - cy * c };
        unsafe { self.d2d.SetTransform(&m); }
    }

    pub fn unturn(&self) { unsafe { self.d2d.SetTransform(&Matrix3x2::identity()); } }

    pub fn begin(&self) {
        unsafe {
            self.d2d.BeginDraw();
            self.d2d.SetTransform(&Matrix3x2::identity());
        }
    }

    pub fn end(&self) -> Result<()> { unsafe { self.d2d.EndDraw(None, None) } }

    fn set(&self, c: Color) { unsafe { self.brush.SetColor(&d2c(c)); } }

    pub fn rect(&self, x: f32, y: f32, w: f32, h: f32, c: Color) {
        self.set(c);
        unsafe { self.d2d.FillRectangle(&D2D_RECT_F { left: x, top: y, right: x + w, bottom: y + h }, &self.brush); }
    }

    pub fn oval(&self, cx: f32, cy: f32, rx: f32, ry: f32, c: Color) {
        self.set(c);
        unsafe { self.d2d.FillEllipse(&D2D1_ELLIPSE { point: pt(cx, cy), radiusX: rx, radiusY: ry }, &self.brush); }
    }

    pub fn ring(&self, cx: f32, cy: f32, rx: f32, ry: f32, width: f32, c: Color) {
        self.set(c);
        unsafe { self.d2d.DrawEllipse(&D2D1_ELLIPSE { point: pt(cx, cy), radiusX: rx, radiusY: ry }, &self.brush, width, None); }
    }

    pub fn line(&self, a: (f32, f32), b: (f32, f32), width: f32, c: Color) {
        self.set(c);
        unsafe { self.d2d.DrawLine(pt(a.0, a.1), pt(b.0, b.1), &self.brush, width, &self.round); }
    }

    pub fn path(&self, pts: &[D2D_POINT_2F], closed: bool) -> Result<ID2D1PathGeometry> {
        unsafe {
            let geo: ID2D1PathGeometry = self.factory.CreatePathGeometry()?.cast()?;
            let sink = geo.Open()?;
            sink.BeginFigure(pts[0], if closed { D2D1_FIGURE_BEGIN_FILLED } else { D2D1_FIGURE_BEGIN_HOLLOW });
            sink.AddLines(&pts[1..]);
            sink.EndFigure(if closed { D2D1_FIGURE_END_CLOSED } else { D2D1_FIGURE_END_OPEN });
            sink.Close()?;
            Ok(geo)
        }
    }

    pub fn stroke(&self, geo: &ID2D1PathGeometry, width: f32, c: Color) {
        self.set(c);
        unsafe { self.d2d.DrawGeometry(geo, &self.brush, width, &self.round); }
    }

    pub fn fill(&self, geo: &ID2D1PathGeometry, c: Color) {
        self.set(c);
        unsafe { self.d2d.FillGeometry(geo, &self.brush, None); }
    }

    pub fn round_rect(&self, x: f32, y: f32, w: f32, h: f32, r: f32, fill: Color, outline: Option<(Color, f32)>) {
        let rr = D2D1_ROUNDED_RECT { rect: D2D_RECT_F { left: x, top: y, right: x + w, bottom: y + h }, radiusX: r, radiusY: r };
        self.set(fill);
        unsafe {
            self.d2d.FillRoundedRectangle(&rr, &self.brush);
            if let Some((c, width)) = outline {
                self.set(c);
                self.d2d.DrawRoundedRectangle(&rr, &self.brush, width, &self.round);
            }
        }
    }

    pub fn layout(&self, text: &str, font: &IDWriteTextFormat, max_w: f32) -> Result<IDWriteTextLayout> {
        let wide: Vec<u16> = text.encode_utf16().collect();
        unsafe { self.dwrite.CreateTextLayout(&wide, font, max_w, 2000.0) }
    }

    pub fn layout_size(&self, l: &IDWriteTextLayout) -> (f32, f32) {
        let mut m = DWRITE_TEXT_METRICS::default();
        unsafe { let _ = l.GetMetrics(&mut m); }
        (m.widthIncludingTrailingWhitespace, m.height)
    }

    pub fn draw_layout(&self, l: &IDWriteTextLayout, x: f32, y: f32, c: Color) {
        self.set(c);
        unsafe { self.d2d.DrawTextLayout(pt(x, y), l, &self.brush, D2D1_DRAW_TEXT_OPTIONS_NONE); }
    }

    /// Text centred on a point.
    pub fn text(&self, text: &str, font: &IDWriteTextFormat, x: f32, y: f32, c: Color) {
        let wide: Vec<u16> = text.encode_utf16().collect();
        self.set(c);
        unsafe {
            self.d2d.DrawText(&wide, font, &D2D_RECT_F { left: x - 120.0, top: y - 60.0, right: x + 120.0, bottom: y + 60.0 },
                              &self.brush, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
        }
    }

    pub fn handwriting(&self, size: f32) -> Result<IDWriteTextFormat> {
        unsafe { text_format(&self.dwrite, "Segoe Print", DWRITE_FONT_WEIGHT_BOLD, size) }
    }
}
