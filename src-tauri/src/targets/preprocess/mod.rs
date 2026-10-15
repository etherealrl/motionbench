use windows::{
    Win32::Graphics::{
        Direct3D::{Fxc::D3DCompile, ID3DBlob},
        Direct3D11::*,
        Dxgi::Common::*,
    },
    core::s,
};

pub struct Preprocess {
    image: ID3D11Texture2D,
    view: ID3D11ShaderResourceView,
    output: ID3D11Buffer,
    staging: ID3D11Buffer,
    uav: ID3D11UnorderedAccessView,
    sampler: ID3D11SamplerState,
    shader: ID3D11ComputeShader,
    size: usize,
}

impl Preprocess {
    pub fn new(
        device: &ID3D11Device,
        width: u32,
        height: u32,
        size: usize,
    ) -> windows::core::Result<Self> {
        let scale = (size as f64 / width as f64).min(size as f64 / height as f64);
        let w = (width as f64 * scale).round() as usize;
        let h = (height as f64 * scale).round() as usize;
        let left = (size - w) / 2;
        let top = (size - h) / 2;
        let source = format!(
            r#"
Texture2D<float4> image : register(t0);
SamplerState linearClamp : register(s0);
RWStructuredBuffer<float> output : register(u0);
[numthreads(16,16,1)] void main(uint3 id : SV_DispatchThreadID) {{
    if (id.x >= {size} || id.y >= {size}) return;
    float3 pixel = float3(114.0/255.0,114.0/255.0,114.0/255.0);
    if (id.x >= {left} && id.x < {right} && id.y >= {top} && id.y < {bottom}) {{
        float2 uv = (float2(id.xy) - float2({left},{top}) + 0.5) / float2({w},{h});
        pixel = image.SampleLevel(linearClamp,uv,0).rgb;
    }}
    uint index = id.y * {size} + id.x;
    output[index] = pixel.r;
    output[index + {plane}] = pixel.g;
    output[index + {twoplane}] = pixel.b;
}}
"#,
            right = left + w,
            bottom = top + h,
            plane = size * size,
            twoplane = 2 * size * size
        );
        let mut blob: Option<ID3DBlob> = None;
        let mut errors = None;
        unsafe {
            D3DCompile(
                source.as_ptr().cast(),
                source.len(),
                None,
                None,
                None,
                s!("main"),
                s!("cs_5_0"),
                0,
                0,
                &mut blob,
                Some(&mut errors),
            )
        }?;
        let blob = blob.unwrap();
        let code = unsafe {
            std::slice::from_raw_parts(blob.GetBufferPointer() as *const u8, blob.GetBufferSize())
        };
        let mut shader = None;
        unsafe { device.CreateComputeShader(code, None, Some(&mut shader)) }?;
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            ..Default::default()
        };
        let mut image = None;
        unsafe { device.CreateTexture2D(&desc, None, Some(&mut image)) }?;
        let image = image.unwrap();
        let mut view = None;
        unsafe { device.CreateShaderResourceView(&image, None, Some(&mut view)) }?;
        let desc = D3D11_BUFFER_DESC {
            ByteWidth: (size * size * 3 * 4) as u32,
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_UNORDERED_ACCESS.0 as u32,
            MiscFlags: D3D11_RESOURCE_MISC_BUFFER_STRUCTURED.0 as u32,
            StructureByteStride: 4,
            ..Default::default()
        };
        let mut output = None;
        unsafe { device.CreateBuffer(&desc, None, Some(&mut output)) }?;
        let output = output.unwrap();
        let mut uav = None;
        unsafe { device.CreateUnorderedAccessView(&output, None, Some(&mut uav)) }?;
        let desc = D3D11_BUFFER_DESC {
            ByteWidth: (size * size * 3 * 4) as u32,
            Usage: D3D11_USAGE_STAGING,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            ..Default::default()
        };
        let mut staging = None;
        unsafe { device.CreateBuffer(&desc, None, Some(&mut staging)) }?;
        let desc = D3D11_SAMPLER_DESC {
            Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
            AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
            AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
            AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
            MaxLOD: f32::MAX,
            ..Default::default()
        };
        let mut sampler = None;
        unsafe { device.CreateSamplerState(&desc, Some(&mut sampler)) }?;
        Ok(Self {
            image,
            view: view.unwrap(),
            output,
            staging: staging.unwrap(),
            uav: uav.unwrap(),
            sampler: sampler.unwrap(),
            shader: shader.unwrap(),
            size,
        })
    }
    pub fn copy(&self, context: &ID3D11DeviceContext, source: &ID3D11Texture2D) {
        unsafe { context.CopyResource(&self.image, source) };
    }
    pub fn tensor(
        &self,
        context: &ID3D11DeviceContext,
        input: &mut [f32],
    ) -> windows::core::Result<()> {
        self.dispatch(context);
        unsafe {
            context.CopyResource(&self.staging, &self.output);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            context.Map(&self.staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
            input.copy_from_slice(std::slice::from_raw_parts(
                mapped.pData as *const f32,
                self.size * self.size * 3,
            ));
            context.Unmap(&self.staging, 0);
        }
        Ok(())
    }

    pub fn output(&self) -> &ID3D11Buffer {
        &self.output
    }

    pub fn dispatch(&self, context: &ID3D11DeviceContext) {
        unsafe {
            context.CSSetShader(&self.shader, None);
            context.CSSetShaderResources(0, Some(&[Some(self.view.clone())]));
            context.CSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            context.CSSetUnorderedAccessViews(0, 1, Some([Some(self.uav.clone())].as_ptr()), None);
            context.Dispatch(
                self.size.div_ceil(16) as u32,
                self.size.div_ceil(16) as u32,
                1,
            );
            context.CSSetUnorderedAccessViews(0, 1, Some([None].as_ptr()), None);
            context.CSSetShaderResources(0, Some(&[None]));
        }
    }
}

#[cfg(test)]
mod tests;
