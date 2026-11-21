// CelesVault adaptation 2026-11-21; see provenance.json and LICENSE.
import type { BackgroundId } from '@/platform/appearance-types';
import { BUILDER } from './builder';
import { fragment, isBuilder, PALETTES } from './shaders';
import type { BackgroundParameters } from '@/platform/appearance-types';

const VERT = 'attribute vec2 p;void main(){gl_Position=vec4(p,0.,1.);}';
const hex = (s: string) => [1, 3, 5].map((i) => parseInt(s.slice(i, i + 2), 16) / 255);

/** 一个编译好的背景着色器：原创款用 u_c1..u_c3，21st Builder 款用打包的 vec4 uniform */
export class ShaderProgram {
  readonly prog: WebGLProgram;
  private loc: Record<string, WebGLUniformLocation | null> = {};
  private position: number;
  private lastTheme = Number.NaN;
  /** 流光：WebGPU 画好的帧（上传成的纹理）；为 null 时着色器画自带的近似版 */
  tex: WebGLTexture | null = null;
  constructor(private gl: WebGLRenderingContext, readonly id: BackgroundId) {
    const sh = (type: number, src: string) => { const s = gl.createShader(type)!; gl.shaderSource(s, src); gl.compileShader(s); if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) { const info=gl.getShaderInfoLog(s); gl.deleteShader(s); throw new Error(`背景编译失败：${info}`); } return s; };
    const prog = gl.createProgram();
    if (!prog) throw new Error('无法创建背景渲染程序');
    let vertex: WebGLShader | null = null, pixel: WebGLShader | null = null;
    try {
      vertex=sh(gl.VERTEX_SHADER, VERT); pixel=sh(gl.FRAGMENT_SHADER, fragment(id));
      gl.attachShader(prog, vertex); gl.attachShader(prog, pixel); gl.linkProgram(prog);
      if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) throw new Error(`背景链接失败：${gl.getProgramInfoLog(prog)}`);
    } catch (error) { gl.deleteProgram(prog); throw error; }
    finally { if (vertex) gl.deleteShader(vertex); if (pixel) gl.deleteShader(pixel); }
    this.prog = prog;
    this.position = gl.getAttribLocation(prog, 'p');
    const names = isBuilder(id) ? ['u_colors', 'u_scene', 'u_shape', 'u_surface', 'u_finish', 'u_transform', 'u_space', 'u_cursor'] : ['u_res', 'u_t', 'u_c1', 'u_c2', 'u_c3', 'u_bg', 'u_tex', 'u_has'];
    for (const n of names) this.loc[n] = gl.getUniformLocation(prog, n);
  }

  bind(): void {
    const gl = this.gl;
    gl.useProgram(this.prog);
    const a = this.position;
    gl.enableVertexAttribArray(a);
    gl.vertexAttribPointer(a, 2, gl.FLOAT, false, 0, 0);
  }

  /** t 是秒 */
  /** k：0 = 亮色调色板，1 = 暗色；昼夜切换时在两者之间插值，和日月按钮同速 */
  draw(w: number, h: number, t: number, theme: 'light' | 'dark' | number, params: BackgroundParameters): void {
    const gl = this.gl, L = this.loc;
    const k = typeof theme === 'number' ? theme : theme === 'dark' ? 1 : 0;
    const mix3 = (a: number[], b: number[]) => a.map((v, i) => v + (b[i] - v) * k);
    const colorsChanged = this.lastTheme !== k;
    this.lastTheme = k;
    if (isBuilder(this.id)) {
      const pr = BUILDER[this.id], u = pr.u;
      if (colorsChanged) {
        const pad = (c: number[][]) => { const o = c.slice(); while (o.length < 8) o.push(o[o.length - 1]); return o; };
        const lc = pad(pr.light.map(hex)), dc = pad(pr.dark.map(hex));
        const cols = lc.map((c, i) => mix3(c, dc[i]));
        gl.uniform3fv(L.u_colors, new Float32Array(cols.flat()));
      }
      gl.uniform4f(L.u_scene, w, h, t * u.timeScale, (k < 0.5 ? pr.light : pr.dark).length);
      {
        gl.uniform4f(L.u_shape, u.scale * params.scale, u.intensity, u.paramA, u.warp);
        gl.uniform4f(L.u_surface, u.detail * params.detail, u.contrast, u.brightness, u.saturation);
        gl.uniform4f(L.u_finish, u.hue, u.vignette, u.blur, u.grain * params.grain);
        gl.uniform4f(L.u_transform, u.seed, u.rotate, u.drift, u.oklab);
        gl.uniform4f(L.u_space, u.offsetX, u.offsetY, 0, 0);
        gl.uniform4f(L.u_cursor, 0, 0, 0, 0);
      }
    } else {
      const a = PALETTES.light[this.id]!, b = PALETTES.dark[this.id]!;
      gl.uniform2f(L.u_res, w, h);
      gl.uniform1f(L.u_t, t);
      if (colorsChanged) {
        gl.uniform3fv(L.u_bg, mix3(Array.from(a.bg), Array.from(b.bg)));
        gl.uniform3fv(L.u_c1, mix3(Array.from(a.c[0]), Array.from(b.c[0])));
        gl.uniform3fv(L.u_c2, mix3(Array.from(a.c[1]), Array.from(b.c[1])));
        gl.uniform3fv(L.u_c3, mix3(Array.from(a.c[2]), Array.from(b.c[2])));
      }
      if (L.u_has) {
        gl.uniform1f(L.u_has, this.tex ? 1 : 0);
        // 没有 WebGPU 帧时也要解绑：0 号纹理单元上可能是透镜的离屏纹理，会形成反馈环
        gl.activeTexture(gl.TEXTURE0); gl.bindTexture(gl.TEXTURE_2D, this.tex); gl.uniform1i(L.u_tex, 0);
      }
    }
    gl.drawArrays(gl.TRIANGLES, 0, 3);
  }
}

export function setupQuad(gl: WebGLRenderingContext): WebGLBuffer | null {
  const buf = gl.createBuffer();
  gl.bindBuffer(gl.ARRAY_BUFFER, buf);
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW);
  return buf;
}

