//! The paint shader on a Wayland surface: EGL + OpenGL ES 3, set up like
//! zoom's renderer. Optionally composites a premultiplied overlay (the
//! picker's cards) on top in the same pass.

use anyhow::{Context, Result, anyhow};
use glow::HasContext;
use khronos_egl as egl;
use wayland_client::{Connection, Proxy, protocol::wl_surface::WlSurface};

use super::Palette;
use crate::zoom::gl::{Egl, config, open};

const VERTEX: &str = "#version 300 es
// One triangle that covers the screen.
void main() {
    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}
";

const FRAGMENT: &str = include_str!("paint.frag");

/// What one frame shows.
#[derive(Debug, Clone, Copy)]
pub struct Frame {
    /// Seconds since the animation started.
    pub time: f32,
    /// 0 = calm flow, 1 = swirl around the centre.
    pub motion: f32,
    pub speed: f32,
    /// Blob size; 1 is the default.
    pub scale: f32,
    pub palette: Palette,
}

struct Uniforms {
    res: glow::UniformLocation,
    t: glow::UniformLocation,
    motion: glow::UniformLocation,
    speed: glow::UniformLocation,
    scale: glow::UniformLocation,
    c1: glow::UniformLocation,
    c2: glow::UniformLocation,
    c3: glow::UniformLocation,
    use_overlay: glow::UniformLocation,
}

pub struct Paint {
    gl: glow::Context,
    u: Uniforms,
    overlay: glow::Texture,
    size: (u32, u32),
    egl: Egl,
    display: egl::Display,
    context: egl::Context,
    surface: egl::Surface,
    // Destroyed after the EGL surface that draws into it.
    window: wayland_egl::WlEglSurface,
}

impl Paint {
    /// A GL surface of `size` buffer px on `surface`.
    pub fn new(conn: &Connection, surface: &WlSurface, size: (u32, u32)) -> Result<Paint> {
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
        // Paced by frame callbacks; swaps must not block.
        egl.swap_interval(display, 0)
            .context("eglSwapInterval failed")?;
        let gl = unsafe {
            glow::Context::from_loader_function(|name| {
                egl.get_proc_address(name)
                    .map_or(std::ptr::null(), |f| f as *const _)
            })
        };
        let (u, overlay) = unsafe { setup(&gl, size)? };
        Ok(Paint {
            gl,
            u,
            overlay,
            size,
            egl,
            display,
            context,
            surface: egl_surface,
            window,
        })
    }

    /// Several renderers (one per output) share the EGL display: each makes
    /// its own context current before touching GL.
    fn current(&self) -> Result<()> {
        self.egl
            .make_current(
                self.display,
                Some(self.surface),
                Some(self.surface),
                Some(self.context),
            )
            .context("eglMakeCurrent failed")
    }

    /// The surface's buffer size changed.
    pub fn resize(&mut self, size: (u32, u32)) {
        if size != self.size && self.current().is_ok() {
            self.window.resize(size.0 as i32, size.1 as i32, 0, 0);
            unsafe { self.gl.viewport(0, 0, size.0 as i32, size.1 as i32) };
            self.size = size;
        }
    }

    /// Shows `pixmap` (premultiplied RGBA, the surface's size) over the
    /// paint, or nothing.
    pub fn set_overlay(&mut self, pixmap: Option<&tiny_skia::Pixmap>) {
        if self.current().is_err() {
            return;
        }
        unsafe {
            self.gl
                .uniform_1_i32(Some(&self.u.use_overlay), pixmap.is_some() as i32);
            if let Some(p) = pixmap {
                self.gl.active_texture(glow::TEXTURE0);
                self.gl.bind_texture(glow::TEXTURE_2D, Some(self.overlay));
                self.gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA8 as i32,
                    p.width() as i32,
                    p.height() as i32,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(Some(p.data())),
                );
            }
        }
    }

    /// Draws `frame` and swaps (which commits the surface).
    pub fn draw(&self, frame: &Frame) -> Result<()> {
        self.current()?;
        let p = frame.palette;
        unsafe {
            let gl = &self.gl;
            gl.uniform_2_f32(Some(&self.u.res), self.size.0 as f32, self.size.1 as f32);
            gl.uniform_1_f32(Some(&self.u.t), frame.time);
            gl.uniform_1_f32(Some(&self.u.motion), frame.motion);
            gl.uniform_1_f32(Some(&self.u.speed), frame.speed);
            gl.uniform_1_f32(Some(&self.u.scale), frame.scale);
            gl.uniform_3_f32(Some(&self.u.c1), p.base[0], p.base[1], p.base[2]);
            gl.uniform_3_f32(Some(&self.u.c2), p.body[0], p.body[1], p.body[2]);
            gl.uniform_3_f32(
                Some(&self.u.c3),
                p.highlight[0],
                p.highlight[1],
                p.highlight[2],
            );
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
        }
        self.egl
            .swap_buffers(self.display, self.surface)
            .context("eglSwapBuffers failed")
    }
}

impl Drop for Paint {
    fn drop(&mut self) {
        let _ = self.egl.make_current(self.display, None, None, None);
        let _ = self.egl.destroy_surface(self.display, self.surface);
        let _ = self.egl.destroy_context(self.display, self.context);
        // No eglTerminate: the display is shared by every renderer on this
        // connection, and terminating it would break the others.
    }
}

/// Compiles the program and returns its uniforms and the overlay texture.
unsafe fn setup(gl: &glow::Context, size: (u32, u32)) -> Result<(Uniforms, glow::Texture)> {
    unsafe {
        let program = gl.create_program().map_err(|e| anyhow!(e))?;
        for (kind, source) in [
            (glow::VERTEX_SHADER, VERTEX),
            (glow::FRAGMENT_SHADER, FRAGMENT),
        ] {
            let shader = gl.create_shader(kind).map_err(|e| anyhow!(e))?;
            gl.shader_source(shader, source);
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                anyhow::bail!("paint shader: {}", gl.get_shader_info_log(shader));
            }
            gl.attach_shader(program, shader);
        }
        gl.link_program(program);
        if !gl.get_program_link_status(program) {
            anyhow::bail!("paint program: {}", gl.get_program_info_log(program));
        }
        gl.use_program(Some(program));
        let vao = gl.create_vertex_array().map_err(|e| anyhow!(e))?;
        gl.bind_vertex_array(Some(vao));

        let overlay = gl.create_texture().map_err(|e| anyhow!(e))?;
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, Some(overlay));
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
        gl.viewport(0, 0, size.0 as i32, size.1 as i32);

        let uniform = |name: &str| {
            gl.get_uniform_location(program, name)
                .with_context(|| format!("no uniform {name}"))
        };
        if let Ok(sampler) = uniform("overlay") {
            gl.uniform_1_i32(Some(&sampler), 0);
        }
        let u = Uniforms {
            res: uniform("res")?,
            t: uniform("t")?,
            motion: uniform("motion")?,
            speed: uniform("speed")?,
            scale: uniform("scale")?,
            c1: uniform("c1")?,
            c2: uniform("c2")?,
            c3: uniform("c3")?,
            use_overlay: uniform("use_overlay")?,
        };
        gl.uniform_1_i32(Some(&u.use_overlay), 0);
        Ok((u, overlay))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shader_declares_its_uniforms() {
        for name in [
            "res",
            "t",
            "motion",
            "speed",
            "scale",
            "c1",
            "c2",
            "c3",
            "overlay",
            "use_overlay",
        ] {
            assert!(
                FRAGMENT
                    .lines()
                    .any(|l| l.starts_with("uniform") && l.contains(&format!(" {name};"))),
                "{name} missing"
            );
        }
    }
}
