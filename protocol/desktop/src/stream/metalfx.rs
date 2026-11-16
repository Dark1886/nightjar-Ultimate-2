//! Native spatial scaling of VideoToolbox surfaces. HDR stays on the existing renderer.
use crate::media::FrameInfo;
use sdl3_sys::everything::*;
use std::{ffi::c_void, ptr};

unsafe extern "C" {
    fn nj_metalfx_supported() -> bool;
    fn nj_metalfx_new(layer: *mut c_void, iw: i32, ih: i32, ow: i32, oh: i32) -> *mut c_void;
    fn nj_metalfx_draw(
        context: *mut c_void,
        input: *mut c_void,
        parameters: *const f32,
        gpu_ms: *mut f64,
    ) -> *mut c_void;
    fn nj_metalfx_free(context: *mut c_void);
    fn nj_metalfx_buffer_free(buffer: *mut c_void);
}

pub fn supported() -> bool {
    static SUPPORTED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SUPPORTED.get_or_init(|| unsafe { nj_metalfx_supported() })
}

pub struct Spatial {
    context: *mut c_void,
    texture: *mut SDL_Texture,
    buffer: *mut c_void,
    input: (i32, i32),
    output: (i32, i32),
    pub gpu_ms: f64,
}

impl Spatial {
    pub fn new(renderer: *mut SDL_Renderer, input: (i32, i32), output: (i32, i32)) -> Option<Self> {
        unsafe {
            let context = nj_metalfx_new(
                SDL_GetRenderMetalLayer(renderer),
                input.0,
                input.1,
                output.0,
                output.1,
            );
            (!context.is_null()).then_some(Self {
                context,
                texture: ptr::null_mut(),
                buffer: ptr::null_mut(),
                input,
                output,
                gpu_ms: 0.0,
            })
        }
    }
    pub fn matches(&self, input: (i32, i32), output: (i32, i32)) -> bool {
        self.input == input && self.output == output
    }

    /// The caller keeps the decoded picture alive until this synchronous GPU submission completes.
    pub unsafe fn draw(
        &mut self,
        renderer: *mut SDL_Renderer,
        buffer: *mut c_void,
        info: &FrameInfo,
    ) -> Option<*mut SDL_Texture> {
        let parameters = conversion(info);
        let output = nj_metalfx_draw(self.context, buffer, parameters.as_ptr(), &mut self.gpu_ms);
        if output.is_null() {
            return None;
        }
        let props = SDL_CreateProperties();
        SDL_SetNumberProperty(
            props,
            SDL_PROP_TEXTURE_CREATE_FORMAT_NUMBER,
            SDL_PIXELFORMAT_BGRA32.0.into(),
        );
        SDL_SetNumberProperty(
            props,
            SDL_PROP_TEXTURE_CREATE_WIDTH_NUMBER,
            self.output.0.into(),
        );
        SDL_SetNumberProperty(
            props,
            SDL_PROP_TEXTURE_CREATE_HEIGHT_NUMBER,
            self.output.1.into(),
        );
        SDL_SetNumberProperty(
            props,
            SDL_PROP_TEXTURE_CREATE_COLORSPACE_NUMBER,
            SDL_COLORSPACE_SRGB.0.into(),
        );
        SDL_SetPointerProperty(
            props,
            SDL_PROP_TEXTURE_CREATE_METAL_PIXELBUFFER_POINTER,
            output,
        );
        let texture = SDL_CreateTextureWithProperties(renderer, props);
        SDL_DestroyProperties(props);
        if texture.is_null() {
            nj_metalfx_buffer_free(output);
            return None;
        }
        SDL_SetTextureBlendMode(texture, SDL_BLENDMODE_NONE);
        SDL_DestroyTexture(self.texture);
        nj_metalfx_buffer_free(self.buffer);
        self.texture = texture;
        self.buffer = output;
        Some(texture)
    }
}

impl Drop for Spatial {
    fn drop(&mut self) {
        unsafe {
            if !self.texture.is_null() {
                SDL_DestroyTexture(self.texture);
            }
            nj_metalfx_buffer_free(self.buffer);
            nj_metalfx_free(self.context);
        }
    }
}

// YUV -> nonlinear RGB, matching the frame's matrix/range. P010 samples occupy
// the high ten bits of a normalized 16-bit texture (not a normalized ten-bit texture).
fn conversion(info: &FrameInfo) -> [f32; 8] {
    let (kr, kb) = match info.color_space {
        5 | 6 => (0.299, 0.114),
        9 | 10 => (0.2627, 0.0593),
        _ => (0.2126, 0.0722),
    };
    let kg = 1.0 - kr - kb;
    let unit = if info.ten_bit != 0 {
        64.0 / 65535.0
    } else {
        1.0 / 255.0
    };
    let scale = if info.ten_bit != 0 { 4.0 } else { 1.0 };
    let (offset, yscale, cscale) = if info.color_range == 2 {
        (
            0.0,
            1.0 / ((256.0 * scale - 1.0) * unit),
            1.0 / ((256.0 * scale - 1.0) * unit),
        )
    } else {
        (
            16.0 * scale * unit,
            1.0 / (219.0 * scale * unit),
            1.0 / (224.0 * scale * unit),
        )
    };
    [
        offset,
        yscale,
        128.0 * scale * unit,
        cscale,
        2.0 * (1.0 - kr),
        -2.0 * kb * (1.0 - kb) / kg,
        -2.0 * kr * (1.0 - kr) / kg,
        2.0 * (1.0 - kb),
    ]
}
