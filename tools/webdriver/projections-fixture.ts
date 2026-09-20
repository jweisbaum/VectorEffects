/** Real WebGL compilation, coordinate readback, and world-map coverage. */
import { PROJECTIONS, MAP_PROJECTIONS, worldHeightDeg } from "../../ui/src/map/projection";
import { cameraForProjection, panBy } from "../../ui/src/map/camera";
import { GEO_VERT } from "../../ui/src/map/shaders";
import { MapRenderer, type RenderState } from "../../ui/src/map/renderer";
import { parseBasemap } from "../../ui/src/map/format";
import type { TileCache } from "../../ui/src/map/tiles";

export async function projectionsFixture(basemapBase64: string) {
  const canvas = document.createElement("canvas");
  canvas.width = 720; canvas.height = 460;
  const gl = canvas.getContext("webgl2", { preserveDrawingBuffer: true });
  if (!gl) throw new Error("WebGL2 unavailable");
  function shader(kind: number, source: string) {
    const result = gl!.createShader(kind)!;
    gl!.shaderSource(result, source); gl!.compileShader(result);
    if (!gl!.getShaderParameter(result, gl!.COMPILE_STATUS)) throw new Error(gl!.getShaderInfoLog(result)!);
    return result;
  }
  // Execute the actual shared shader helpers and read their float32 results.
  const vertex = shader(gl.VERTEX_SHADER, GEO_VERT.slice(0, GEO_VERT.indexOf("void main()")) + `
    out vec2 coordinates;
    void main() { coordinates = vec2(latToY(aLonLat.x), yToLat(aLonLat.y)); gl_Position = vec4(0.0); }
  `);
  const fragment = shader(gl.FRAGMENT_SHADER, "#version 300 es\nprecision highp float; out vec4 color; void main() {color=vec4(1.0);}");
  const program = gl.createProgram()!;
  gl.attachShader(program, vertex); gl.attachShader(program, fragment);
  gl.transformFeedbackVaryings(program, ["coordinates"], gl.INTERLEAVED_ATTRIBS);
  gl.linkProgram(program);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(program)!);
  const vao = gl.createVertexArray(); gl.bindVertexArray(vao);
  const input = gl.createBuffer(); gl.bindBuffer(gl.ARRAY_BUFFER, input);
  const attribute = gl.getAttribLocation(program, "aLonLat");
  gl.enableVertexAttribArray(attribute); gl.vertexAttribPointer(attribute, 2, gl.FLOAT, false, 0, 0);
  const feedback = gl.createTransformFeedback(); gl.bindTransformFeedback(gl.TRANSFORM_FEEDBACK, feedback);
  const output = gl.createBuffer(); gl.bindBuffer(gl.TRANSFORM_FEEDBACK_BUFFER, output);
  const latitudes = [-75, -60, -30, 0, 30, 60, 75];
  gl.bufferData(gl.TRANSFORM_FEEDBACK_BUFFER, latitudes.length * 8, gl.STREAM_READ);
  gl.bindBufferBase(gl.TRANSFORM_FEEDBACK_BUFFER, 0, output);
  gl.useProgram(program); gl.enable(gl.RASTERIZER_DISCARD);
  const metrics = [];
  for (const projection of PROJECTIONS) {
    const values = new Float32Array(latitudes.flatMap(lat => [lat, projection.yOf(lat)]));
    gl.bindBuffer(gl.ARRAY_BUFFER, input); gl.bufferData(gl.ARRAY_BUFFER, values, gl.STREAM_DRAW);
    gl.uniform1i(gl.getUniformLocation(program, "uProjection"), projection.mode);
    gl.beginTransformFeedback(gl.POINTS); gl.drawArrays(gl.POINTS, 0, latitudes.length); gl.endTransformFeedback();
    const actual = new Float32Array(values.length);
    gl.getBufferSubData(gl.TRANSFORM_FEEDBACK_BUFFER, 0, actual);
    let error = 0;
    latitudes.forEach((lat, i) => {
      error = Math.max(error, Math.abs(actual[2 * i]! - projection.yOf(lat)), Math.abs(actual[2 * i + 1]! - lat));
    });
    if (!Number.isFinite(error) || error > 0.0002) throw new Error(`${projection.id}: shader error ${error} degrees`);
    metrics.push({ projection: projection.id, maxErrorDegrees: error });
  }
  gl.disable(gl.RASTERIZER_DISCARD); gl.bindTransformFeedback(gl.TRANSFORM_FEEDBACK, null);
  gl.bindVertexArray(null); gl.deleteVertexArray(vao); gl.deleteBuffer(input); gl.deleteBuffer(output);
  gl.deleteTransformFeedback(feedback); gl.deleteProgram(program); gl.deleteShader(vertex); gl.deleteShader(fragment);

  const texture = gl.createTexture()!; gl.bindTexture(gl.TEXTURE_2D, texture);
  // A fully covered eastward current; the main renderer draws both its raster and glyphs.
  const bytes = new Uint8Array(4); new DataView(bytes.buffer).setUint32(0, 2457 | (1024 << 14) | (31 << 26), true);
  gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, 1, 1, 0, gl.RGBA, gl.UNSIGNED_BYTE, bytes);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
  const coverage = new Uint8Array(8192).fill(255);
  const tiles = { get: () => texture, peek: () => texture, coverageOf: () => coverage,
    windCoverageOf: () => new Uint8Array(8192), unresolved: () => false,
    rangeOf: () => ({ wind: null, current: [2457, 2457] }) } as unknown as TileCache;
  const basemap = Uint8Array.from(atob(basemapBase64), c => c.charCodeAt(0));
  const parsed = parseBasemap(basemap.buffer);
  // The compact fixture includes one LOD. Reuse it for regional zoom checks.
  const renderer = new MapRenderer(gl, {...parsed, lods: [...parsed.lods, {...parsed.lods[0]!, marker: 50}]}, tiles);
  const selected = MAP_PROJECTIONS.filter(p => !p.general || p.general.movable || ["robinson","mollweide","winkel_tripel","equal_earth","sinusoidal","epsg_3413","epsg_3031","epsg_6931","epsg_27700","epsg_2056","epsg_5514","epsg_27200","epsg_2193","epsg_3338","epsg_26931","epsg_26961","epsg_32604"].includes(p.id));
  const montage = document.createElement("canvas"); montage.width = 1440; montage.height = Math.ceil(selected.length / 2) * 490;
  const ctx = montage.getContext("2d")!;
  ctx.fillStyle = "#131b25"; ctx.fillRect(0, 0, montage.width, montage.height);
  for (const [i, projection] of selected.entries()) {
    const state: RenderState = {
      camera: projection.general ? cameraForProjection({centerLon: 0, centerLat: 20, pxPerDeg: 2}, {width: canvas.width, height:canvas.height}, projection.id) : { centerLon: 0, centerLat: 0, pxPerDeg: Math.min(700 / 360, 440 / worldHeightDeg(projection)), projection: projection.id },
      view: { width: canvas.width, height: canvas.height }, frame: "fixture/0", heldFrame: null,
      ramps: { wind: { min: 0, max: 20 }, current: { min: 0, max: 20 } },
      gradients: { wind: [[0, 0, 0], [1, 1, 1]], current: [[0.03, 0.12, 0.15], [0.1, 0.3, 0.5]] },
      showGlyphs: true, showGraticule: true, pixelRatio: 1,
    };
    const start = performance.now(); renderer.render(state); gl.finish();
    const coldMs=performance.now()-start;
    const first=new Uint8Array(canvas.width*canvas.height*4);gl.readPixels(0,0,canvas.width,canvas.height,gl.RGBA,gl.UNSIGNED_BYTE,first);
    const warm=performance.now();renderer.render(state);gl.finish();
    const warmMs=performance.now()-warm;
    const second=new Uint8Array(first.length);gl.readPixels(0,0,canvas.width,canvas.height,gl.RGBA,gl.UNSIGNED_BYTE,second);
    if(first.some((value,index)=>value!==second[index]))throw new Error(`${projection.id}: cached redraw differs from first frame`);
    if (projection.general) metrics.push({projection: projection.id, renderMs: Math.round(coldMs), warmMs:Math.round(warmMs)});
    await new Promise(resolve => setTimeout(resolve, 0));
    const error = gl.getError(); if (error !== 0) throw new Error(`${projection.id}: GL error ${error}`);
    if (["orthographic", "mollweide", "epsg_27700"].includes(projection.id)) {
      // A georeferenced image must land at the same geographic centre as the
      // field, including after a cached redraw has changed the active VAO.
      const imageTexture=gl.createTexture()!;gl.activeTexture(gl.TEXTURE0);gl.bindTexture(gl.TEXTURE_2D,imageTexture);
      gl.texImage2D(gl.TEXTURE_2D,0,gl.RGBA8,1,1,0,gl.RGBA,gl.UNSIGNED_BYTE,new Uint8Array([0,255,0,255]));
      gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MIN_FILTER,gl.NEAREST);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MAG_FILTER,gl.NEAREST);
      const {centerLon,centerLat}=state.camera;
      renderer.render({...state,showGlyphs:false,showGraticule:false,images:[{layer:1,over:true,texture:imageTexture,opacity:1,
        placeLon:[10,0,centerLon-5],placeLat:[0,-10,centerLat+5]}]});
      const pixel=new Uint8Array(4);gl.readPixels(canvas.width/2,canvas.height/2,1,1,gl.RGBA,gl.UNSIGNED_BYTE,pixel);
      if(pixel[0]!>5||pixel[1]!<250||pixel[2]!>5)throw new Error(`${projection.id}: image is not aligned with the geographic centre: ${pixel}`);
      gl.deleteTexture(imageTexture);renderer.render(state);gl.finish();
    }
    const x = i % 2 * 720, y = Math.floor(i / 2) * 490;
    ctx.drawImage(canvas, x, y + 30); ctx.fillStyle = "white"; ctx.font = "18px sans-serif";
    ctx.fillText(projection.label, x + 12, y + 24);
  }
  const image = montage.toDataURL("image/png");

  // What turning costs: a globe at the size of a real window, its centre
  // moved every frame so nothing about the last one can be reused, with the
  // frame finished on the GPU before the clock is read. Glyphs are timed
  // apart because they are placed on the CPU under these projections.
  canvas.width = 2880; canvas.height = 1590;
  const large = { width: canvas.width, height: canvas.height };
  const turning = [];
  const moved = MAP_PROJECTIONS.filter(p => p.general?.movable || ["robinson", "mollweide", "epsg_3413"].includes(p.id));
  for (const projection of moved) {
    for (const showGlyphs of [false, true]) {
      const base = cameraForProjection({centerLon: 0, centerLat: 20, pxPerDeg: 2}, large, projection.id);
      const times: number[] = [];
      for (let frame = 0; frame < 16; frame++) {
        const state: RenderState = {
          // A globe is turned; a fixed map is dragged, a dozen pixels a frame.
          camera: projection.general?.movable ? {...base, centerLon: -40 + frame * 1.7, centerLat: 15 + frame * 0.6}
            : panBy(base, large, frame * 13, frame * 5), view: large,
          frame: "fixture/0", heldFrame: null,
          ramps: { wind: { min: 0, max: 20 }, current: { min: 0, max: 20 } },
          gradients: { wind: [[0, 0, 0], [1, 1, 1]], current: [[0.03, 0.12, 0.15], [0.1, 0.3, 0.5]] },
          showGlyphs, showGraticule: true, pixelRatio: 2,
        };
        const start = performance.now(); renderer.render(state); gl.finish();
        times.push(performance.now() - start);
        // Let the endpoint's poll in between frames: a script that finds the
        // webview busy for thirty seconds times out, and takes the endpoint
        // with it. A message, not a timer — an uncomposited webview's timers
        // are throttled to tens of seconds.
        await new Promise(resolve => { const channel = new MessageChannel();
          channel.port1.onmessage = () => resolve(null); channel.port2.postMessage(0); });
      }
      // The first frames render the base map's source tiles; turning is the rest.
      const steady = times.slice(4).sort((a, b) => a - b);
      turning.push({ projection: projection.id, glyphs: showGlyphs,
        medianMs: Math.round(steady[steady.length >> 1]!), worstMs: Math.round(steady[steady.length - 1]!) });
      const error = gl.getError(); if (error !== 0) throw new Error(`${projection.id}: GL error ${error} while turning`);
    }
  }

  renderer.dispose(); gl.deleteTexture(texture); gl.getExtension("WEBGL_lose_context")?.loseContext();
  return { metrics, turning, image, count: selected.length };
}
