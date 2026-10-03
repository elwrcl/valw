//! OpenGL ES 3 through EGL on a Wayland surface: just enough to show the
//! frozen frame zoomed in.

use anyhow::{Context, Result, anyhow};
use glow::HasContext;
use khronos_egl as egl;
use wayland_client::{Connection, Proxy, protocol::wl_surface::WlSurface};

use super::view::View;
use crate::frame::Bgrx;

pub(crate) type Egl = egl::DynamicInstance<egl::EGL1_5>;

/// `EGL_PLATFORM_WAYLAND_KHR`.
const PLATFORM_WAYLAND: egl::Enum = 0x31D8;

const VERTEX: &str = "#version 300 es
// One triangle that covers the screen.
void main() {
    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}
";

const FRAGMENT: &str = "#version 300 es
precision highp float;
uniform sampler2D frame;
uniform float scale;
// The frame pixel at the screen's top-left.
uniform vec2 origin;
// Centre and radius in screen px; radius 0 means off.
uniform vec3 flashlight;
out vec4 color;
void main() {
    ivec2 size = textureSize(frame, 0);
    // gl_FragCoord starts at the bottom-left; the frame at the top-left.
    vec2 screen = vec2(gl_FragCoord.x, float(size.y) - gl_FragCoord.y);
    ivec2 texel = clamp(ivec2(floor(origin + screen / scale)), ivec2(0), size - 1);
    // The texture holds the frame's B, G, R, X bytes.
    vec3 rgb = texelFetch(frame, texel, 0).bgr;
    if (flashlight.z > 0.0) {
        float d = distance(screen, flashlight.xy);
        rgb *= mix(1.0, 0.25, smoothstep(flashlight.z - 1.5, flashlight.z + 1.5, d));
    }
    color = vec4(rgb, 1.0);
}
";

/// Loads libEGL and opens the compositor's EGL display.
pub(crate) fn open(conn: &Connection) -> Result<(Egl, egl::Display)> {
    let lib =
        unsafe { libloading::Library::new("libEGL.so.1") }.context("could not load libEGL.so.1")?;
    let egl = unsafe { Egl::load_required_from(lib) }
        .map_err(|e| anyhow!("libEGL.so.1 has no EGL 1.5: {e}"))?;
    let display = unsafe {
        egl.get_platform_display(
            PLATFORM_WAYLAND,
            conn.backend().display_ptr().cast(),
            &[egl::ATTRIB_NONE],
        )
    }
    .context("no EGL display for the Wayland connection")?;
    egl.initialize(display)
        .context("could not initialise EGL")?;
    Ok((egl, display))
}

pub(crate) fn config(egl: &Egl, display: egl::Display) -> Result<egl::Config> {
    #[rustfmt::skip]
    let attributes = [
        egl::SURFACE_TYPE, egl::WINDOW_BIT,
        egl::RENDERABLE_TYPE, egl::OPENGL_ES3_BIT,
        egl::RED_SIZE, 8, egl::GREEN_SIZE, 8, egl::BLUE_SIZE, 8,
        egl::NONE,
    ];
    egl.choose_first_config(display, &attributes)
        .context("eglChooseConfig failed")?
        .context("no EGL config for OpenGL ES 3 windows")
}

/// The EGL vendor, if a window config for OpenGL ES 3 exists.
pub fn probe(conn: &Connection) -> Result<String> {
    let (egl, display) = open(conn)?;
    let result = config(&egl, display).map(|_| {
        egl.query_string(Some(display), egl::VENDOR)
            .map(|v| v.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "unknown vendor".into())
    });
    let _ = egl.terminate(display);
    result
}

/// Draws the frozen frame on one surface.
pub struct Renderer {
    gl: glow::Context,
    scale: glow::UniformLocation,
    origin: glow::UniformLocation,
    flashlight: glow::UniformLocation,
    egl: Egl,
    display: egl::Display,
    context: egl::Context,
    surface: egl::Surface,
    // Destroyed after the EGL surface that draws into it.
    _window: wayland_egl::WlEglSurface,
}

impl Renderer {
    /// A GL surface of `size` buffer px on `surface`, showing `frame`.
    pub fn new(
        conn: &Connection,
        surface: &WlSurface,
        size: (u32, u32),
        frame: &Bgrx,
    ) -> Result<Renderer> {
        let (egl, display) = open(conn)?;
        egl.bind_api(egl::OPENGL_ES_API)
            .context("no OpenGL ES in EGL")?;
        let config = config(&egl, display)?;
        let context = egl
            .create_context(
                display,
                config,
                None,
                &[egl::CONTEXT_MAJOR_VERSION, 3, egl::NONE],
            )
            .context("could not create an OpenGL ES 3 context")?;
        let window = wayland_egl::WlEglSurface::new(surface.id(), size.0 as i32, size.1 as i32)
            .context("could not create a wl_egl_window")?;
        let egl_surface = unsafe {
            egl.create_window_surface(display, config, window.ptr() as egl::NativeWindowType, None)
        }
        .context("could not create the EGL window surface")?;
        egl.make_current(display, Some(egl_surface), Some(egl_surface), Some(context))
            .context("eglMakeCurrent failed")?;
        // valw paces drawing with frame callbacks; swaps must not block.
        egl.swap_interval(display, 0)
            .context("eglSwapInterval failed")?;

        let gl = unsafe {
            glow::Context::from_loader_function(|name| {
                egl.get_proc_address(name)
                    .map_or(std::ptr::null(), |f| f as *const _)
            })
        };
        let (scale, origin, flashlight) = unsafe { setup(&gl, size, frame)? };
        Ok(Renderer {
            gl,
            scale,
            origin,
            flashlight,
            egl,
            display,
            context,
            surface: egl_surface,
            _window: window,
        })
    }

    /// Draws `view` and swaps (which commits the surface).
    pub fn draw(&self, view: &View, flashlight: Option<(f64, f64, f64)>) -> Result<()> {
        let (x, y, r) = flashlight.unwrap_or_default();
        unsafe {
            self.gl.uniform_1_f32(Some(&self.scale), view.scale as f32);
            self.gl.uniform_2_f32(
                Some(&self.origin),
                view.origin.0 as f32,
                view.origin.1 as f32,
            );
            self.gl
                .uniform_3_f32(Some(&self.flashlight), x as f32, y as f32, r as f32);
            self.gl.draw_arrays(glow::TRIANGLES, 0, 3);
        }
        self.egl
            .swap_buffers(self.display, self.surface)
            .context("eglSwapBuffers failed")
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        let _ = self.egl.make_current(self.display, None, None, None);
        let _ = self.egl.destroy_surface(self.display, self.surface);
        let _ = self.egl.destroy_context(self.display, self.context);
        let _ = self.egl.terminate(self.display);
    }
}

/// Compiles the program, uploads the frame and returns the uniforms.
unsafe fn setup(
    gl: &glow::Context,
    size: (u32, u32),
    frame: &Bgrx,
) -> Result<(
    glow::UniformLocation,
    glow::UniformLocation,
    glow::UniformLocation,
)> {
    unsafe {
        tracing::info!("zoom renderer: {}", gl.get_parameter_string(glow::RENDERER));
        let program = gl.create_program().map_err(|e| anyhow!(e))?;
        for (kind, source) in [
            (glow::VERTEX_SHADER, VERTEX),
            (glow::FRAGMENT_SHADER, FRAGMENT),
        ] {
            let shader = gl.create_shader(kind).map_err(|e| anyhow!(e))?;
            gl.shader_source(shader, source);
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                anyhow::bail!("shader: {}", gl.get_shader_info_log(shader));
            }
            gl.attach_shader(program, shader);
        }
        gl.link_program(program);
        if !gl.get_program_link_status(program) {
            anyhow::bail!("program: {}", gl.get_program_info_log(program));
        }
        gl.use_program(Some(program));
        // GLES 3 needs a bound vertex array even without attributes.
        let vao = gl.create_vertex_array().map_err(|e| anyhow!(e))?;
        gl.bind_vertex_array(Some(vao));

        let texture = gl.create_texture().map_err(|e| anyhow!(e))?;
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MIN_FILTER,
            glow::NEAREST as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MAG_FILTER,
            glow::NEAREST as i32,
        );
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA8 as i32,
            frame.width() as i32,
            frame.height() as i32,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(Some(frame.as_raw())),
        );
        gl.viewport(0, 0, size.0 as i32, size.1 as i32);

        let uniform = |name: &str| {
            gl.get_uniform_location(program, name)
                .with_context(|| format!("no uniform {name}"))
        };
        gl.uniform_1_i32(uniform("frame").ok().as_ref(), 0);
        Ok((
            uniform("scale")?,
            uniform("origin")?,
            uniform("flashlight")?,
        ))
    }
}
