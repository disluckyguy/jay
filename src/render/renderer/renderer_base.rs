use {
    crate::{
        fixed::Fixed,
        format::{Format},
        rect::Rect,
        render::{
            gl::{
                frame_buffer::{GlFrameBuffer},
                sys::{
                    glActiveTexture, glBindTexture, glDisableVertexAttribArray, glDrawArrays,
                    glEnableVertexAttribArray, glTexParameteri, glUniform1i, glUniform4f,
                    glUseProgram, glVertexAttribPointer, GL_FALSE, GL_FLOAT, GL_LINEAR,
                    GL_TEXTURE0, GL_TEXTURE_MIN_FILTER, GL_TRIANGLES, GL_TRIANGLE_STRIP,
                },
                texture::image_target,
            },
            renderer::context::RenderContext,
            sys::{glDisable, glEnable, GL_BLEND},
            Texture,
        },
        theme::Color,
        utils::rc_eq::rc_eq,
    },
    std::{
        rc::Rc,
    },
};
use crate::render::sys::{GL_COLOR_BUFFER_BIT, glClear, glClearColor};

pub struct RendererBase<'a> {
    pub(super) ctx: &'a Rc<RenderContext>,
    pub(super) fb: &'a GlFrameBuffer,
    pub(super) scaled: bool,
    pub(super) scale: Fixed,
    pub(super) scalef: f64,
}

impl RendererBase<'_> {
    pub fn physical_extents(&self) -> Rect {
        Rect::new_sized(0, 0, self.fb.width, self.fb.height).unwrap()
    }

    pub(super) fn scale_point(&self, mut x: i32, mut y: i32) -> (i32, i32) {
        if self.scaled {
            x = (x as f64 * self.scalef).round() as _;
            y = (y as f64 * self.scalef).round() as _;
        }
        (x, y)
    }

    pub fn scale_rect(&self, mut rect: Rect) -> Rect {
        if self.scaled {
            let x1 = (rect.x1() as f64 * self.scalef).round() as _;
            let y1 = (rect.y1() as f64 * self.scalef).round() as _;
            let x2 = (rect.x2() as f64 * self.scalef).round() as _;
            let y2 = (rect.y2() as f64 * self.scalef).round() as _;
            rect = Rect::new(x1, y1, x2, y2).unwrap();
        }
        rect
    }

    fn x_to_f(&self, x: i32) -> f32 {
        2.0 * (x as f32 / self.fb.width as f32) - 1.0
    }

    fn y_to_f(&self, y: i32) -> f32 {
        2.0 * (y as f32 / self.fb.height as f32) - 1.0
    }

    pub fn clear(&self, c: &Color) {
        unsafe {
            glClearColor(c.r, c.g, c.b, c.a);
            glClear(GL_COLOR_BUFFER_BIT);
        }
    }

    pub fn fill_boxes(&self, boxes: &[Rect], color: &Color) {
        self.fill_boxes2(boxes, color, 0, 0);
    }

    pub fn fill_boxes2(&self, boxes: &[Rect], color: &Color, dx: i32, dy: i32) {
        if boxes.is_empty() {
            return;
        }
        let (dx, dy) = self.scale_point(dx, dy);
        let mut pos = Vec::with_capacity(boxes.len() * 12);
        for bx in boxes {
            let bx = self.scale_rect(*bx);
            let x1 = self.x_to_f(bx.x1() + dx);
            let y1 = self.y_to_f(bx.y1() + dy);
            let x2 = self.x_to_f(bx.x2() + dx);
            let y2 = self.y_to_f(bx.y2() + dy);
            pos.extend_from_slice(&[
                // triangle 1
                x2, y1, // top right
                x1, y1, // top left
                x1, y2, // bottom left
                // triangle 2
                x2, y1, // top right
                x1, y2, // bottom left
                x2, y2, // bottom right
            ]);
        }
        unsafe {
            glUseProgram(self.ctx.fill_prog.prog);
            glUniform4f(self.ctx.fill_prog_color, color.r, color.g, color.b, color.a);
            glVertexAttribPointer(
                self.ctx.fill_prog_pos as _,
                2,
                GL_FLOAT,
                GL_FALSE,
                0,
                pos.as_ptr() as _,
            );
            glEnableVertexAttribArray(self.ctx.fill_prog_pos as _);
            glDrawArrays(GL_TRIANGLES, 0, (boxes.len() * 6) as _);
            glDisableVertexAttribArray(self.ctx.fill_prog_pos as _);
        }
    }

    pub fn render_texture(
        &mut self,
        texture: &Texture,
        x: i32,
        y: i32,
        format: &Format,
        tpoints: Option<&[f32; 8]>,
        tsize: Option<(i32, i32)>,
        tscale: Fixed,
    ) {
        assert!(rc_eq(&self.ctx.ctx, &texture.ctx.ctx));
        unsafe {
            glActiveTexture(GL_TEXTURE0);

            let target = image_target(texture.gl.external_only);

            glBindTexture(target, texture.gl.tex);
            glTexParameteri(target, GL_TEXTURE_MIN_FILTER, GL_LINEAR);

            let progs = match texture.gl.external_only {
                true => match &self.ctx.tex_external {
                    Some(p) => p,
                    _ => {
                        log::error!("Trying to render an external-only texture but context does not support the required extension");
                        return;
                    }
                },
                false => &self.ctx.tex_internal,
            };
            let prog = match format.has_alpha {
                true => {
                    glEnable(GL_BLEND);
                    &progs.alpha
                }
                false => {
                    glDisable(GL_BLEND);
                    &progs.solid
                }
            };

            glUseProgram(prog.prog.prog);

            glUniform1i(prog.tex, 0);

            static DEFAULT_TEXCOORD: [f32; 8] = [1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0];

            let texcoord: &[f32; 8] = match tpoints {
                None => &DEFAULT_TEXCOORD,
                Some(tp) => tp,
            };

            let f_width = self.fb.width as f32;
            let f_height = self.fb.height as f32;

            let (twidth, theight) = if let Some(size) = tsize {
                size
            } else {
                let (mut w, mut h) = (texture.gl.width, texture.gl.height);
                if tscale != self.scale {
                    let tscale = tscale.to_f64();
                    w = (w as f64 * self.scalef / tscale).round() as _;
                    h = (h as f64 * self.scalef / tscale).round() as _;
                }
                (w, h)
            };

            let x1 = 2.0 * (x as f32 / f_width) - 1.0;
            let y1 = 2.0 * (y as f32 / f_height) - 1.0;
            let x2 = 2.0 * ((x + twidth) as f32 / f_width) - 1.0;
            let y2 = 2.0 * ((y + theight) as f32 / f_height) - 1.0;

            let pos: [f32; 8] = [
                x2, y1, // top right
                x1, y1, // top left
                x2, y2, // bottom right
                x1, y2, // bottom left
            ];

            glVertexAttribPointer(
                prog.texcoord as _,
                2,
                GL_FLOAT,
                GL_FALSE,
                0,
                texcoord.as_ptr() as _,
            );
            glVertexAttribPointer(prog.pos as _, 2, GL_FLOAT, GL_FALSE, 0, pos.as_ptr() as _);

            glEnableVertexAttribArray(prog.texcoord as _);
            glEnableVertexAttribArray(prog.pos as _);

            glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);

            glDisableVertexAttribArray(prog.texcoord as _);
            glDisableVertexAttribArray(prog.pos as _);

            glBindTexture(target, 0);
        }
    }
}
