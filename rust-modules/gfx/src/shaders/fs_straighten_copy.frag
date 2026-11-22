// The STRAIGHTEN pass's COPY program (`gfx/straighten.rs`): `fs_straighten.frag`'s arithmetic, `rgb' = a > 0 ?
// rgb / a : 0` with alpha as it is, reading the pixel from a copy of the frame sampled 1:1 (`v_uv` is the
// pixel's place in the whole viewport, `vs_straighten.vert`). For a GPU with no framebuffer fetch: the
// simulator and the Tizen emulator. WHY the pass exists, why `a == 0` writes zero colour, and the
// precision argument are in `fs_straighten.frag`'s header and apply here unchanged.
//
// A FILE OF ITS OWN, with no preprocessor directive: the shared source's `#extension` lines made YaGL (the
// Tizen emulator's GLES2, over the host's Mesa) reject the program at its `precision` statement even in an
// inactive branch, so this program had none and a frame over a video plane stayed premultiplied. The
// three lines of arithmetic below are the fetch file's, character for character; a test holds them equal.
precision mediump float;
varying highp vec2 v_uv;
uniform sampler2D u_tex;
void main(){
  highp vec4 c = texture2D(u_tex, v_uv);
  highp float k = c.a > 0.0 ? 1.0 / c.a : 0.0;
  gl_FragColor = vec4(c.rgb * k, c.a);
}
