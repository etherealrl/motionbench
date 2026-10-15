use super::*;
use windows::Win32::{Foundation::HMODULE, Graphics::Direct3D::D3D_DRIVER_TYPE_WARP};
#[test]
fn gpu_letterbox_preserves_rgb_planes() {
    let mut device = None;
    let mut context = None;
    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_WARP,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
    }
    .unwrap();
    let (device, context) = (device.unwrap(), context.unwrap());
    let pixels = [0u8, 128, 255, 255].repeat(8);
    let desc = D3D11_TEXTURE2D_DESC {
        Width: 4,
        Height: 2,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        ..Default::default()
    };
    let data = D3D11_SUBRESOURCE_DATA {
        pSysMem: pixels.as_ptr().cast(),
        SysMemPitch: 16,
        SysMemSlicePitch: 32,
    };
    let mut texture = None;
    unsafe { device.CreateTexture2D(&desc, Some(&data), Some(&mut texture)) }.unwrap();
    let preprocess = Preprocess::new(&device, 4, 2, 8).unwrap();
    preprocess.copy(&context, &texture.unwrap());
    let mut input = vec![0f32; 3 * 8 * 8];
    preprocess.tensor(&context, &mut input).unwrap();
    for y in 0..8 {
        for x in 0..8 {
            for (c, expected) in [1.0f32, 128.0 / 255.0, 0.0].iter().enumerate() {
                let expected = if (2..6).contains(&y) {
                    *expected
                } else {
                    114.0 / 255.0
                };
                assert!((input[c * 64 + y * 8 + x] - expected).abs() < 1e-5);
            }
        }
    }
}
