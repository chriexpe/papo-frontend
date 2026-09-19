//! Vidro fosco: desfoque do que a própria aplicação já desenhou.
//!
//! O compositor só consegue desfocar o que está *atrás da janela*. Para que
//! uma barra embace as mensagens que passam por baixo dela — o comportamento
//! das HIG — o desfoque precisa acontecer no nosso renderizador: copiamos o
//! pedaço do framebuffer, borramos em resolução reduzida e desenhamos de
//! volta no lugar.

use std::sync::{Arc, Mutex};

use eframe::glow::{self, HasContext};

/// Redução de resolução antes do borrão: o raio efetivo sai multiplicado por
/// este fator, a um quarto do custo por eixo.
const DOWNSCALE: i32 = 4;
/// Repetições do par horizontal/vertical.
const PASSES: usize = 3;
/// Realce de saturação, para que as cores por trás do vidro não morram.
const SATURATION: f32 = 1.35;

pub type SharedGlass = Arc<Mutex<GlassRenderer>>;

pub struct GlassRenderer {
    blur: glow::Program,
    composite: glow::Program,
    vao: glow::VertexArray,
    captured: glow::Texture,
    captured_size: (i32, i32),
    ping: [(glow::Framebuffer, glow::Texture); 2],
    ping_size: (i32, i32),
}

impl GlassRenderer {
    pub fn new(gl: &glow::Context) -> Option<SharedGlass> {
        unsafe {
            let header = if gl.version().is_embedded {
                "#version 300 es\nprecision highp float;\n"
            } else {
                "#version 330 core\n"
            };

            let blur = program(gl, header, VERTEX, BLUR_FRAGMENT)?;
            let composite = program(gl, header, VERTEX, COMPOSITE_FRAGMENT)?;
            let vao = gl.create_vertex_array().ok()?;
            let captured = new_texture(gl)?;
            let ping = [framebuffer(gl)?, framebuffer(gl)?];

            Some(Arc::new(Mutex::new(Self {
                blur,
                composite,
                vao,
                captured,
                captured_size: (0, 0),
                ping,
                ping_size: (0, 0),
            })))
        }
    }

    /// Desenha o fundo embaçado de um retângulo.
    ///
    /// `rect_px` é (x, y, largura, altura) em pixels físicos, com origem no
    /// canto superior esquerdo; `screen_h` é a altura do framebuffer.
    pub fn render(&mut self, gl: &glow::Context, rect_px: (i32, i32, i32, i32), screen_h: i32, radius_px: f32) {
        let (x, top, w, h) = rect_px;
        if w <= 1 || h <= 1 {
            return;
        }
        let y = screen_h - (top + h); // coordenadas do OpenGL crescem para cima
        let (dw, dh) = ((w / DOWNSCALE).max(1), (h / DOWNSCALE).max(1));

        unsafe {
            let previous_fbo = current_framebuffer(gl);
            let blend_was_on = gl.is_enabled(glow::BLEND);
            let scissor_was_on = gl.is_enabled(glow::SCISSOR_TEST);

            self.resize(gl, (w, h), (dw, dh));

            // 1. Copia o pedaço do framebuffer que está sob o vidro. A cópia
            // lê do READ_FRAMEBUFFER, que não é necessariamente o mesmo que o
            // egui tem ligado para desenhar.
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, previous_fbo);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.captured));
            gl.copy_tex_sub_image_2d(glow::TEXTURE_2D, 0, 0, 0, x, y, w, h);

            // 2. Borra em resolução reduzida, alternando horizontal/vertical.
            // O egui deixa o scissor recortado no retângulo da callback; nas
            // passagens fora da tela isso zeraria tudo.
            gl.disable(glow::SCISSOR_TEST);
            gl.disable(glow::BLEND);
            gl.bind_vertex_array(Some(self.vao));
            gl.use_program(Some(self.blur));
            gl.viewport(0, 0, dw, dh);
            let tex_loc = gl.get_uniform_location(self.blur, "u_tex");
            gl.uniform_1_i32(tex_loc.as_ref(), 0);
            gl.active_texture(glow::TEXTURE0);

            let dir_loc = gl.get_uniform_location(self.blur, "u_dir");
            let mut source = self.captured;
            for pass in 0..PASSES * 2 {
                let target = self.ping[pass % 2];
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(target.0));
                gl.bind_texture(glow::TEXTURE_2D, Some(source));
                let step = if pass % 2 == 0 {
                    [1.0 / dw as f32, 0.0]
                } else {
                    [0.0, 1.0 / dh as f32]
                };
                gl.uniform_2_f32(dir_loc.as_ref(), step[0], step[1]);
                gl.draw_arrays(glow::TRIANGLES, 0, 3);
                source = target.1;
            }

            // 3. Devolve o resultado ao lugar, com cantos arredondados.
            gl.bind_framebuffer(glow::FRAMEBUFFER, previous_fbo);
            if scissor_was_on {
                gl.enable(glow::SCISSOR_TEST);
            }
            gl.viewport(x, y, w, h);
            gl.use_program(Some(self.composite));
            gl.bind_texture(glow::TEXTURE_2D, Some(source));
            let loc = |name: &str| gl.get_uniform_location(self.composite, name);
            gl.uniform_1_i32(loc("u_tex").as_ref(), 0);
            gl.uniform_2_f32(loc("u_size").as_ref(), w as f32, h as f32);
            gl.uniform_1_f32(loc("u_radius").as_ref(), radius_px);
            gl.uniform_1_f32(loc("u_saturation").as_ref(), SATURATION);
            gl.enable(glow::BLEND);
            gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
            gl.draw_arrays(glow::TRIANGLES, 0, 3);

            // 4. Devolve o estado que o egui espera encontrar.
            gl.bind_vertex_array(None);
            gl.use_program(None);
            if !blend_was_on {
                gl.disable(glow::BLEND);
            }
            if !scissor_was_on {
                gl.disable(glow::SCISSOR_TEST);
            }
        }
    }

    unsafe fn resize(&mut self, gl: &glow::Context, captured: (i32, i32), ping: (i32, i32)) {
        unsafe {
            if self.captured_size != captured {
                gl.bind_texture(glow::TEXTURE_2D, Some(self.captured));
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA8 as i32,
                    captured.0,
                    captured.1,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(None),
                );
                self.captured_size = captured;
            }
            if self.ping_size != ping {
                for (fbo, tex) in self.ping {
                    gl.bind_texture(glow::TEXTURE_2D, Some(tex));
                    gl.tex_image_2d(
                        glow::TEXTURE_2D,
                        0,
                        glow::RGBA8 as i32,
                        ping.0,
                        ping.1,
                        0,
                        glow::RGBA,
                        glow::UNSIGNED_BYTE,
                        glow::PixelUnpackData::Slice(None),
                    );
                    gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
                    gl.framebuffer_texture_2d(
                        glow::FRAMEBUFFER,
                        glow::COLOR_ATTACHMENT0,
                        glow::TEXTURE_2D,
                        Some(tex),
                        0,
                    );
                }
                self.ping_size = ping;
            }
        }
    }

    pub fn destroy(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_program(self.blur);
            gl.delete_program(self.composite);
            gl.delete_vertex_array(self.vao);
            gl.delete_texture(self.captured);
            for (fbo, tex) in self.ping {
                gl.delete_framebuffer(fbo);
                gl.delete_texture(tex);
            }
        }
    }
}

unsafe fn current_framebuffer(gl: &glow::Context) -> Option<glow::Framebuffer> {
    let raw = unsafe { gl.get_parameter_i32(glow::DRAW_FRAMEBUFFER_BINDING) };
    std::num::NonZeroU32::new(raw as u32).map(glow::NativeFramebuffer)
}

unsafe fn new_texture(gl: &glow::Context) -> Option<glow::Texture> {
    unsafe {
        let tex = gl.create_texture().ok()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        for (name, value) in [
            (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
            (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
            (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
            (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
        ] {
            gl.tex_parameter_i32(glow::TEXTURE_2D, name, value as i32);
        }
        Some(tex)
    }
}

unsafe fn framebuffer(gl: &glow::Context) -> Option<(glow::Framebuffer, glow::Texture)> {
    unsafe {
        let tex = new_texture(gl)?;
        let fbo = gl.create_framebuffer().ok()?;
        Some((fbo, tex))
    }
}

unsafe fn program(
    gl: &glow::Context,
    header: &str,
    vertex: &str,
    fragment: &str,
) -> Option<glow::Program> {
    unsafe {
        let program = gl.create_program().ok()?;
        let mut shaders = Vec::new();
        for (kind, source) in [(glow::VERTEX_SHADER, vertex), (glow::FRAGMENT_SHADER, fragment)] {
            let shader = gl.create_shader(kind).ok()?;
            gl.shader_source(shader, &format!("{header}{source}"));
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                log::error!("shader do vidro falhou: {}", gl.get_shader_info_log(shader));
                return None;
            }
            gl.attach_shader(program, shader);
            shaders.push(shader);
        }
        gl.link_program(program);
        if !gl.get_program_link_status(program) {
            log::error!("link do vidro falhou: {}", gl.get_program_info_log(program));
            return None;
        }
        for shader in shaders {
            gl.detach_shader(program, shader);
            gl.delete_shader(shader);
        }
        Some(program)
    }
}

/// Triângulo que cobre a viewport inteira, sem buffers.
const VERTEX: &str = r#"
out vec2 v_uv;
void main() {
    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    v_uv = p;
    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}
"#;

const BLUR_FRAGMENT: &str = r#"
uniform sampler2D u_tex;
uniform vec2 u_dir;
in vec2 v_uv;
out vec4 frag;
void main() {
    float w0 = 0.2270270270;
    float w1 = 0.1945945946;
    float w2 = 0.1216216216;
    float w3 = 0.0540540541;
    float w4 = 0.0162162162;
    vec4 c = texture(u_tex, v_uv) * w0;
    c += (texture(u_tex, v_uv + u_dir) + texture(u_tex, v_uv - u_dir)) * w1;
    c += (texture(u_tex, v_uv + u_dir * 2.0) + texture(u_tex, v_uv - u_dir * 2.0)) * w2;
    c += (texture(u_tex, v_uv + u_dir * 3.0) + texture(u_tex, v_uv - u_dir * 3.0)) * w3;
    c += (texture(u_tex, v_uv + u_dir * 4.0) + texture(u_tex, v_uv - u_dir * 4.0)) * w4;
    frag = c;
}
"#;

const COMPOSITE_FRAGMENT: &str = r#"
uniform sampler2D u_tex;
uniform vec2 u_size;
uniform float u_radius;
uniform float u_saturation;
in vec2 v_uv;
out vec4 frag;

float rounded_box(vec2 p, vec2 half_size, float radius) {
    vec2 q = abs(p) - half_size + radius;
    return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - radius;
}

void main() {
    vec2 px = v_uv * u_size;
    float d = rounded_box(px - u_size * 0.5, u_size * 0.5, u_radius);
    float alpha = clamp(0.5 - d, 0.0, 1.0);
    if (alpha <= 0.0) {
        discard;
    }
    vec3 c = texture(u_tex, v_uv).rgb;
    float luma = dot(c, vec3(0.2126, 0.7152, 0.0722));
    c = mix(vec3(luma), c, u_saturation);
    frag = vec4(c, alpha);
}
"#;
