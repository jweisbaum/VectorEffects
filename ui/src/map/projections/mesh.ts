/** Adaptive inverse projection. A screen triangle carries geographic coordinates;
 * clipping it to source tiles prevents antimeridian triangles from crossing the map.
 * This also lets every supported CRS share the same raster shaders. */
import type { LL, XY } from "./general";
export type MeshVertex = XY & LL;
export type MeshTriangle = [MeshVertex, MeshVertex, MeshVertex];
export interface MeshTile {
  z: number; x: number; y: number; lonOffset: number;
  /** Interleaved tile UV and screen XY, ready for WebGL attribute locations 0/1. */
  vertices: Float32Array;
}
export interface GeographicMesh {
  tiles: MeshTile[];
  triangles: MeshTriangle[];
}
const wrap = (lon:number) => ((lon+180)%360+360)%360-180;
/** How many times a coarse cell may be halved. */
const LATTICE_DEPTH = 12;
/** Finest cells along a coarse cell's side, halved once more for midpoints. */
const LATTICE = 2 << LATTICE_DEPTH;
/** Room for any lattice count beside another in one key: 2^26 of them. */
const KEY_STRIDE = 1 << 26;
function blend(a:MeshVertex,b:MeshVertex,t:number):MeshVertex {
  return {x:a.x+(b.x-a.x)*t,y:a.y+(b.y-a.y)*t,lon:a.lon+(b.lon-a.lon)*t,lat:a.lat+(b.lat-a.lat)*t};
}
export function clipPolygon(points:MeshVertex[],axis:"lon"|"lat",value:number,greater:boolean):MeshVertex[] {
  const result:MeshVertex[]=[];
  for(let i=0;i<points.length;i++){
    const a=points[i]!,b=points[(i+1)%points.length]!;
    const insideA=greater?a[axis]>=value:a[axis]<=value;
    const insideB=greater?b[axis]>=value:b[axis]<=value;
    if(insideA)result.push(a);
    if(insideA!==insideB)result.push(blend(a,b,(value-a[axis])/(b[axis]-a[axis])));
  }
  return result;
}

export function geographicMesh(
  width:number,height:number,pxPerDeg:number,
  inverse:(point:XY)=>LL|null,forward:(point:LL)=>XY|null,budget=192,spacing=48,
):GeographicMesh {
  const triangles:MeshTriangle[]=[];
  // A sample is asked for by every cell that shares it, up to four at each
  // depth, so each is taken once and remembered. Remembered by number: a
  // position is a whole count of the finest cell there can be (a coarse cell
  // halved LATTICE_DEPTH times, and once more for its midpoints), and two
  // counts pack into one key. Keyed by a string built from the coordinates,
  // this lookup was the largest single cost of the mesh.
  const cols=Math.max(1,Math.ceil(width/spacing)),rows=Math.max(1,Math.ceil(height/spacing));
  const unitX=width/cols/LATTICE,unitY=height/rows/LATTICE;
  const samples=new Map<number,MeshVertex|null>();
  const at=(i:number,j:number):MeshVertex|null=>{
    const key=i*KEY_STRIDE+j;
    const known=samples.get(key);
    if(known!==undefined)return known;
    const x=i*unitX,y=j*unitY;
    const ll=inverse({x,y});
    const point=ll&&Number.isFinite(ll.lon)&&Number.isFinite(ll.lat)?{...ll,x,y}:null;
    samples.set(key,point);return point;
  };
  const emit=(a:MeshVertex,b:MeshVertex,c:MeshVertex)=>{
    const verts:[MeshVertex,MeshVertex,MeshVertex]=[a,{...b,lon:a.lon+wrap(b.lon-a.lon)},{...c,lon:a.lon+wrap(c.lon-a.lon)}];
    // At an exact pole longitude is arbitrary. Give that shared point the
    // neighbouring meridians' mean so it does not create an all-world sliver.
    for(let i=0;i<3;i++)if(Math.abs(verts[i]!.lat)>89.999999){
      const next=verts[(i+1)%3]!,last=verts[(i+2)%3]!;
      verts[i]={...verts[i]!,lon:(next.lon+last.lon)/2};
    }
    triangles.push(verts);
  };
  // A cell in lattice counts: its corner, and its side, which halves with depth.
  function cell(i0:number,j0:number,size:number,depth:number):void {
    const half=size/2,i1=i0+size,j1=j0+size,im=i0+half,jm=j0+half;
    const a=at(i0,j0),b=at(i1,j0),c=at(i1,j1),d=at(i0,j1),m=at(im,jm);
    const top=at(im,j0),right=at(i1,jm),bottom=at(im,j1),left=at(i0,jm);
    const points=[a,b,c,d,m,top,right,bottom,left];
    if(points.every(p=>p===null))return;
    const boundary=points.some(p=>p===null);
    const small=Math.max(size*unitX,size*unitY)<=(boundary?1:0.25) || depth>=LATTICE_DEPTH;
    let refine=boundary;
    if(!refine){
      // Test the diagonal and each edge: bilinear-only tests miss curvature
      // within the two triangles used by the GPU.
      for(const [p,q,actual] of [[a,c,m],[a,b,top],[b,c,right],[d,c,bottom],[a,d,left]] as const){
        const mid={lon:p!.lon+wrap(q!.lon-p!.lon)/2,lat:(p!.lat+q!.lat)/2};
        const projected=forward(mid);
        if(!projected||Math.hypot(projected.x-actual!.x,projected.y-actual!.y)>0.3){refine=true;break;}
      }
    }
    if(refine&&!small){
      cell(i0,j0,half,depth+1);cell(im,j0,half,depth+1);
      cell(i0,jm,half,depth+1);cell(im,jm,half,depth+1);
    }else{
      if(a&&b&&c)emit(a,b,c);if(a&&c&&d)emit(a,c,d);
    }
  }
  // The modest initial spacing samples disconnected valid regions as well as
  // the main outline. Refinement follows the projection, not a fixed tile mesh.
  for(let j=0;j<rows;j++)for(let i=0;i<cols;i++)cell(i*LATTICE,j*LATTICE,LATTICE,0);
  let z=Math.min(12,Math.max(0,Math.ceil(Math.log2(360*pxPerDeg/256))-1));
  for(;;z--){
    const tileData=new Map<string,{z:number;x:number;y:number;lonOffset:number;data:number[]}>();
    const columns=2<<z,rows=1<<z,span=360/columns;
    for(const triangle of triangles){
      const [p0,p1,p2]=triangle;
      const west=Math.min(p0.lon,p1.lon,p2.lon),east=Math.max(p0.lon,p1.lon,p2.lon);
      const south=Math.min(p0.lat,p1.lat,p2.lat),north=Math.max(p0.lat,p1.lat,p2.lat);
      const firstCol=Math.floor((west+180)/span),lastCol=Math.floor((east+180-1e-10)/span);
      const firstRow=Math.max(0,Math.floor((90-north)/span)),lastRow=Math.min(rows-1,Math.floor((90-south-1e-10)/span));
      // Nearly every triangle is a few pixels across and lies in one tile,
      // where there is nothing to clip it against.
      const whole=firstCol===lastCol&&firstRow===lastRow;
      for(let row=firstRow;row<=lastRow;row++)for(let col=firstCol;col<=lastCol;col++){
        const w=-180+col*span,n=90-row*span;
        let polygon:MeshVertex[]=triangle;
        if(!whole){
          polygon=clipPolygon(polygon,'lon',w,true);polygon=clipPolygon(polygon,'lon',w+span,false);
          polygon=clipPolygon(polygon,'lat',n-span,true);polygon=clipPolygon(polygon,'lat',n,false);
        }
        if(polygon.length<3)continue;
        const x=((col%columns)+columns)%columns,lonOffset=(col-x)*span,key=`${x}/${row}/${lonOffset}`;
        let tile=tileData.get(key);
        if(!tile){tile={z,x,y:row,lonOffset,data:[]};tileData.set(key,tile);}
        for(let i=1;i+1<polygon.length;i++)for(const p of [polygon[0]!,polygon[i]!,polygon[i+1]!]){
          tile.data.push((p.lon-w)/span,(n-p.lat)/span,p.x,p.y);
        }
      }
    }
    if(tileData.size<=budget||z===0)return {triangles,tiles:[...tileData.values()].map(({data,...tile})=>({...tile,vertices:new Float32Array(data)}))};
  }
}
