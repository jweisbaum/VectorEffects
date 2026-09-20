/** GPU resources for the fixed general projections. Geographic source tiles
 * remain ordinary textures, drawn through a mesh made in the projection's own
 * plane (PlaneMesh in camera.ts). Nothing here is made per camera: the mesh,
 * an image's cut of it and the graticule all last until the mesh is rebuilt,
 * and a pan or a zoom only moves `uMesh`. */
import { meshPlacement, projectedMesh, tileBounds, type Camera, type PlaneMesh, type Viewport, type VisibleTile } from "../camera";
import type { ImageDraw } from "../renderer";
import { clipPolygon, type MeshVertex } from "./mesh";

const vertex=`#version 300 es
precision highp float;
layout(location=0) in vec2 aUV;
layout(location=1) in vec2 aScreen;
uniform vec2 uViewport;
uniform vec3 uMesh;
out vec2 vUV;
void main(){vUV=aUV;vec2 screen=aScreen*uMesh.z+uMesh.xy;gl_Position=vec4(screen.x/uViewport.x*2.0-1.0,1.0-screen.y/uViewport.y*2.0,0.0,1.0);}`;
const fragment=`#version 300 es
precision highp float;
uniform sampler2D uTexture;
in vec2 vUV;out vec4 color;
void main(){color=texture(uTexture,vUV);if(color.a>0.0)color.rgb/=color.a;}`;
function program(gl:WebGL2RenderingContext):WebGLProgram {
  const result=gl.createProgram()!;
  for(const [type,source] of [[gl.VERTEX_SHADER,vertex],[gl.FRAGMENT_SHADER,fragment]] as const){
    const shader=gl.createShader(type)!;gl.shaderSource(shader,source);gl.compileShader(shader);
    if(!gl.getShaderParameter(shader,gl.COMPILE_STATUS))throw new Error(gl.getShaderInfoLog(shader)??"Projection shader failed");
    gl.attachShader(result,shader);gl.deleteShader(shader);
  }
  gl.linkProgram(result);
  if(!gl.getProgramParameter(result,gl.LINK_STATUS))throw new Error(gl.getProgramInfoLog(result)??"Projection shader link failed");
  return result;
}
interface BufferMesh {vao:WebGLVertexArrayObject;buffer:WebGLBuffer;count:number;}
interface BaseTile {texture:WebGLTexture;}
const keyOf=(tile:VisibleTile)=>`${tile.z}/${tile.x}/${tile.y}/${tile.lonOffset}`;
export class ProjectedSurface {
  private readonly program:WebGLProgram;
  private readonly viewport:WebGLUniformLocation|null;
  private readonly sampler:WebGLUniformLocation|null;
  private readonly placement:WebGLUniformLocation|null;
  private readonly framebuffer:WebGLFramebuffer;
  private mesh:PlaneMesh|null=null;
  /** An image's cut of the mesh, by its placement: made once a mesh, not once a frame. */
  private readonly imageMeshes=new Map<string,BufferMesh>();
  private readonly meshes=new Map<string,BufferMesh>();
  private readonly baseMeshes=new Map<string,BufferMesh>();
  private readonly baseTiles=new Map<string,BaseTile>();
  private imageMesh:BufferMesh;
  private lines:BufferMesh;
  private lineKey="";
  constructor(private readonly gl:WebGL2RenderingContext){
    this.program=program(gl);this.viewport=gl.getUniformLocation(this.program,"uViewport");
    this.sampler=gl.getUniformLocation(this.program,"uTexture");this.placement=gl.getUniformLocation(this.program,"uMesh");
    this.framebuffer=gl.createFramebuffer()!;
    this.imageMesh=this.makeMesh(new Float32Array());this.lines=this.makeMesh(new Float32Array());
  }
  private makeMesh(data:Float32Array):BufferMesh {
    const gl=this.gl,vao=gl.createVertexArray()!,buffer=gl.createBuffer()!;
    gl.bindVertexArray(vao);gl.bindBuffer(gl.ARRAY_BUFFER,buffer);gl.bufferData(gl.ARRAY_BUFFER,data,gl.DYNAMIC_DRAW);
    gl.enableVertexAttribArray(0);gl.vertexAttribPointer(0,2,gl.FLOAT,false,16,0);
    gl.enableVertexAttribArray(1);gl.vertexAttribPointer(1,2,gl.FLOAT,false,16,8);
    return {vao,buffer,count:data.length/4};
  }
  private drop(mesh:BufferMesh):void{this.gl.deleteBuffer(mesh.buffer);this.gl.deleteVertexArray(mesh.vao);}
  private ensure(camera:Camera,view:Viewport):void {
    const mesh=projectedMesh(camera,view);if(this.mesh===mesh)return;
    for(const old of this.meshes.values())this.drop(old);this.meshes.clear();
    for(const old of this.baseMeshes.values())this.drop(old);this.baseMeshes.clear();
    for(const old of this.imageMeshes.values())this.drop(old);this.imageMeshes.clear();
    this.lineKey="";
    for(const tile of mesh.tiles){
      this.meshes.set(keyOf(tile),this.makeMesh(tile.vertices));
      const data=new Float32Array(tile.vertices);
      for(let i=0;i<data.length;i+=4){data[i]=(2+data[i]!*384)/388;data[i+1]=(386-data[i+1]!*384)/388;}
      this.baseMeshes.set(keyOf(tile),this.makeMesh(data));
    }
    this.mesh=mesh;
  }
  bindTile(camera:Camera,view:Viewport,tile:VisibleTile):number {
    this.ensure(camera,view);const mesh=this.meshes.get(keyOf(tile));
    if(!mesh)return 0;this.gl.bindVertexArray(mesh.vao);return mesh.count;
  }
  /** Render geographic land/coast data once per source tile, then reuse it on
   * every frame and in every projection. No world-size bitmap limits local zoom. */
  drawBase(camera:Camera,view:Viewport,tiles:readonly VisibleTile[],kind:"land"|"coast",lod:number,
    draw:(camera:Camera,view:Viewport,kind:"land"|"coast")=>void):void {
    const gl=this.gl;
    for(const tile of tiles){
      const texture=this.baseTexture(view,tile,kind,lod,draw);
      gl.useProgram(this.program);gl.uniform2f(this.viewport,view.width,view.height);gl.uniform1i(this.sampler,0);
      const place=meshPlacement(camera,view);gl.uniform3f(this.placement,place.x,place.y,place.scale);
      gl.activeTexture(gl.TEXTURE0);gl.bindTexture(gl.TEXTURE_2D,texture);
      // Reuse the camera's mesh with FBO y-up UVs and the texture bleed.
      this.ensure(camera,view);
      const mesh=this.baseMeshes.get(keyOf(tile));if(!mesh)continue;
      gl.bindVertexArray(mesh.vao);gl.drawArrays(gl.TRIANGLES,0,mesh.count);
    }
  }
  /** One source tile of land or coast, rendered on first use and kept. Shared
   * by the mesh path above and the renderer's own GPU-projected one. */
  baseTexture(view:Viewport,tile:VisibleTile,kind:"land"|"coast",lod:number,
    draw:(camera:Camera,view:Viewport,kind:"land"|"coast")=>void):WebGLTexture {
    const gl=this.gl;
    {
      const key=`${kind}/${lod}/${tile.z}/${tile.x}/${tile.y}`;
      let base=this.baseTiles.get(key);
      if(!base){
        const texture=gl.createTexture()!;gl.activeTexture(gl.TEXTURE0);gl.bindTexture(gl.TEXTURE_2D,texture);
        // Two texels of bleed prevent transparent sampling at tile boundaries.
        gl.texImage2D(gl.TEXTURE_2D,0,gl.RGBA8,388,388,0,gl.RGBA,gl.UNSIGNED_BYTE,null);
        gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MIN_FILTER,gl.LINEAR);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MAG_FILTER,gl.LINEAR);
        gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_WRAP_S,gl.CLAMP_TO_EDGE);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_WRAP_T,gl.CLAMP_TO_EDGE);
        gl.bindFramebuffer(gl.FRAMEBUFFER,this.framebuffer);gl.framebufferTexture2D(gl.FRAMEBUFFER,gl.COLOR_ATTACHMENT0,gl.TEXTURE_2D,texture,0);
        if(gl.checkFramebufferStatus(gl.FRAMEBUFFER)!==gl.FRAMEBUFFER_COMPLETE)throw new Error("Projection basemap target is incomplete");
        gl.viewport(0,0,388,388);gl.clearColor(...(kind==='land'?[0.043,0.078,0.133,1]:[0,0,0,0]) as [number,number,number,number]);gl.clear(gl.COLOR_BUFFER_BIT);
        const b=tileBounds(tile.z,tile.x,tile.y);
        gl.blendFuncSeparate(gl.SRC_ALPHA,gl.ONE_MINUS_SRC_ALPHA,gl.ONE,gl.ONE_MINUS_SRC_ALPHA);
        draw({projection:"equirectangular",centerLon:(b.west+b.east)/2,centerLat:(b.north+b.south)/2,pxPerDeg:384/(b.east-b.west)}, {width:388,height:388},kind);
        gl.blendFunc(gl.SRC_ALPHA,gl.ONE_MINUS_SRC_ALPHA);
        gl.bindFramebuffer(gl.FRAMEBUFFER,null);gl.viewport(0,0,view.width,view.height);
        base={texture};this.baseTiles.set(key,base);
        if(this.baseTiles.size>384){const oldest=this.baseTiles.keys().next().value!;gl.deleteTexture(this.baseTiles.get(oldest)!.texture);this.baseTiles.delete(oldest);}
      }
      return base.texture;
    }
  }
  /** Clip the mesh against an affine georeferenced image. Kept by placement:
   * the cut is of the plane mesh, so it outlives every pan and zoom. */
  bindImage(camera:Camera,view:Viewport,image:ImageDraw):number {
    this.ensure(camera,view);
    const key=JSON.stringify([image.placeLon,image.placeLat]);
    const kept=this.imageMeshes.get(key);
    if(kept){this.gl.bindVertexArray(kept.vao);return kept.count;}
    const [a,b,c]=image.placeLon,[d,e,f]=image.placeLat,det=a*e-b*d;
    if(Math.abs(det)<1e-15)return 0;
    const centre=c+(a+b)/2,data:number[]=[];
    for(const triangle of this.mesh!.triangles){
      const mean=triangle.reduce((sum,p)=>sum+p.lon,0)/3;
      const offset=Math.round((centre-mean)/360)*360;
      let polygon:MeshVertex[]=triangle.map(p=>{
        const lon=p.lon+offset-c,lat=p.lat-f;
        return {...p,lon:(lon*e-lat*b)/det,lat:(lat*a-lon*d)/det};
      });
      // Most of the mesh is nowhere near the image: ask before cutting.
      if(polygon.every(p=>p.lon<0)||polygon.every(p=>p.lon>1)||polygon.every(p=>p.lat<0)||polygon.every(p=>p.lat>1))continue;
      polygon=clipPolygon(polygon,'lon',0,true);polygon=clipPolygon(polygon,'lon',1,false);
      polygon=clipPolygon(polygon,'lat',0,true);polygon=clipPolygon(polygon,'lat',1,false);
      for(let i=1;i+1<polygon.length;i++)for(const p of [polygon[0]!,polygon[i]!,polygon[i+1]!])data.push(p.lon,p.lat,p.x,p.y);
    }
    const mesh=this.makeMesh(new Float32Array(data));
    this.imageMeshes.set(key,mesh);
    // A placement being dragged makes a new cut a frame; keep the last few.
    if(this.imageMeshes.size>8){const oldest=this.imageMeshes.keys().next().value!;this.drop(this.imageMeshes.get(oldest)!);this.imageMeshes.delete(oldest);}
    this.gl.bindVertexArray(mesh.vao);return mesh.count;
  }
  /** Graticule segments on the mesh's virtual canvas, adaptively curved and
   * clipped to the map. Made once a mesh and a spacing, like the mesh itself. */
  bindGraticule(camera:Camera,view:Viewport,step:number):number {
    this.ensure(camera,view);
    const mesh=this.mesh!;
    const key=`${step}`;
    if(key===this.lineKey){this.gl.bindVertexArray(this.lines.vao);return this.lines.count;}
    this.lineKey=key;
    const data:number[]=[],bounds=mesh.bounds;
    const width=(mesh.region[2]-mesh.region[0])*mesh.scale,height=(mesh.region[3]-mesh.region[1])*mesh.scale;
    const limit=Math.hypot(width,height)*2;
    const point=(lon:number,lat:number)=>mesh.toVirtual({lon,lat})??{x:NaN,y:NaN};
    function segment(lon0:number,lat0:number,lon1:number,lat1:number,depth=0):void {
      const a=point(lon0,lat0),b=point(lon1,lat1),m=point((lon0+lon1)/2,(lat0+lat1)/2);
      const av=Number.isFinite(a.x),bv=Number.isFinite(b.x),mv=Number.isFinite(m.x);
      if(!av&&!bv&&!mv)return;
      if(av&&bv&&mv && ([a,b,m].every(p=>p.x < -8) || [a,b,m].every(p=>p.x > width+8)
        || [a,b,m].every(p=>p.y < -8) || [a,b,m].every(p=>p.y > height+8)))return;
      const error=av&&bv&&mv?Math.hypot(m.x-(a.x+b.x)/2,m.y-(a.y+b.y)/2):Infinity;
      if(depth<12&&(error>0.3||!av||!bv||Math.hypot(a.x-b.x,a.y-b.y)>limit)){
        segment(lon0,lat0,(lon0+lon1)/2,(lat0+lat1)/2,depth+1);segment((lon0+lon1)/2,(lat0+lat1)/2,lon1,lat1,depth+1);
      }else if(av&&bv&&Math.hypot(a.x-b.x,a.y-b.y)<limit){data.push(a.x,a.y,0,0,b.x,b.y,0,0);}
    }
    const west=bounds.west-10,east=bounds.east+10,south=Math.max(-90,bounds.south-5),north=Math.min(90,bounds.north+5);
    for(let lon=Math.ceil(west/step)*step;lon<=east;lon+=step)for(let lat=south;lat<north;lat+=5)segment(lon,lat,lon,Math.min(north,lat+5));
    for(let lat=Math.ceil(south/step)*step;lat<=north;lat+=step)for(let lon=west;lon<east;lon+=5)segment(lon,lat,Math.min(east,lon+5),lat);
    const gl=this.gl;gl.bindVertexArray(this.lines.vao);gl.bindBuffer(gl.ARRAY_BUFFER,this.lines.buffer);gl.bufferData(gl.ARRAY_BUFFER,new Float32Array(data),gl.DYNAMIC_DRAW);
    this.lines.count=data.length/4;
    return this.lines.count;
  }
  dispose():void {
    for(const mesh of this.meshes.values())this.drop(mesh);
    for(const mesh of this.baseMeshes.values())this.drop(mesh);
    for(const mesh of this.imageMeshes.values())this.drop(mesh);
    for(const tile of this.baseTiles.values())this.gl.deleteTexture(tile.texture);
    this.drop(this.imageMesh);this.drop(this.lines);this.gl.deleteFramebuffer(this.framebuffer);this.gl.deleteProgram(this.program);
  }
}
