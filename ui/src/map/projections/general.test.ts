import { describe, expect, it } from "vitest";
import proj4 from "proj4";
import { GENERAL_MAPS, METRES_PER_DEGREE, customMap, generalMap, defaultCentre, mapTransform } from "./general";
import references from "./crs-reference.json";
import { cameraForProjection, meshPlacement, panBy, project, projectedMeshWanted, refreshProjectedMesh, unproject, validGeo, projectedMesh, visibleTiles, tileBounds, zoomAbout, cameraWithAnchor } from "../camera";
import { kmFromPixels, pixelsFromKm } from "../footprint";
import { projectionOf, type ProjectionId } from "../projection";

const view={width:720,height:480};
describe("national and polar projection catalogue",()=>{
  it("accepts offline custom definitions and rejects unsupported codes and geographic CRSs",()=>{
    expect(customMap("EPSG:3413").id).toBe("epsg_3413");
    const map=customMap("+proj=lcc +lat_1=33 +lat_2=45 +lat_0=39 +lon_0=-96 +datum=WGS84 +units=m");
    expect(defaultCentre(map)).toEqual({lon:-96,lat:39});
    expect(generalMap(map.id)?.definition).toBe(map.definition);
    expect(()=>customMap("EPSG:999999")).toThrow(/not bundled/);
    expect(()=>customMap("+proj=longlat +datum=WGS84")).toThrow(/projected map/);
    expect(generalMap("custom:%invalid")).toBeUndefined();
    const wkt=customMap('PROJCS["Regional TM",GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433]],PROJECTION["Transverse_Mercator"],PARAMETER["latitude_of_origin",0],PARAMETER["central_meridian",9],PARAMETER["scale_factor",0.9996],PARAMETER["false_easting",500000],PARAMETER["false_northing",0],UNIT["metre",1]]');
    expect(defaultCentre(wkt).lon).toBeCloseTo(9,8);
    const t=mapTransform(wkt,defaultCentre(wkt));
    expect(t.inverse(t.forward({lon:9,lat:40})!)!.lat).toBeCloseTo(40,6);
  });
  it("matches independent OSGeo PROJ reference coordinates for every EPSG method",()=>{
    for(const reference of references){
      const map=GENERAL_MAPS.find(p=>p.id===reference.id)!;
      // Inputs of these method tests are geographic coordinates on the target
      // ellipsoid. Datum shifts are a separate transformation operation.
      const definition=map.definition.replace(/\+towgs84=\S+|\+datum=\S+/g,"");
      const ellipsoid=/\+ellps=(\S+)/.exec(definition)?.[1];
      const a=/\+a=(\S+)/.exec(definition)?.[1],b=/\+b=(\S+)/.exec(definition)?.[1];
      const rf=/\+rf=(\S+)/.exec(definition)?.[1];
      const source=`+proj=longlat ${ellipsoid?`+ellps=${ellipsoid}`:`+a=${a} ${b?`+b=${b}`:`+rf=${rf}`}`}`;
      const actual=proj4(source,definition,[reference.lon,reference.lat]);
      expect(actual[0],map.label).toBeCloseTo(reference.x,2);
      expect(actual[1],map.label).toBeCloseTo(reference.y,2);
    }
  });
  it("has a finite round-trip at the focus of every map",()=>{
    for(const map of GENERAL_MAPS){
      const centre=defaultCentre(map),transform=mapTransform(map,centre);
      const xy=transform.forward(centre);
      expect(xy,map.label).not.toBeNull();
      const back=transform.inverse(xy!);
      expect(back,map.label).not.toBeNull();
      expect(back!.lat,map.label).toBeCloseTo(centre.lat,5);
      expect(back!.lon,map.label).toBeCloseTo(centre.lon,5);
      expect(Math.abs(xy!.x)*METRES_PER_DEGREE).toBeLessThan(1e9);
    }
  });
});
describe("two-dimensional cameras",()=>{
  for(const id of ["orthographic","mollweide","robinson","winkel_tripel","azimuthal_equal_area","epsg_3413","epsg_3031","epsg_27700","epsg_2056","epsg_5514","epsg_27200"] as ProjectionId[]){
    it(`${id}: projects the pointer, zooms at its anchor, and builds visible tiles`,()=>{
      const camera=cameraForProjection({centerLon:0,centerLat:20,pxPerDeg:2},view,id);
      const anchor={x:view.width*0.6,y:view.height*0.45};
      const geo=unproject(camera,view,anchor);expect(validGeo(geo)).toBe(true);
      const xy=project(camera,view,geo);expect(xy.x).toBeCloseTo(anchor.x,4);expect(xy.y).toBeCloseTo(anchor.y,4);
      const zoomed=zoomAbout(camera,view,anchor,1.5),at=project(zoomed,view,geo);
      expect(at.x).toBeCloseTo(anchor.x,2);expect(at.y).toBeCloseTo(anchor.y,2);
      const tiles=visibleTiles(camera,view);expect(tiles.length).toBeGreaterThan(0);expect(tiles.length).toBeLessThanOrEqual(192);
      // A globe and its kin have no mesh: the GPU projects them (azimuthalShader.test.ts).
      if(!projectionOf(id).general?.movable){
        const mesh=projectedMesh(camera,view),place=meshPlacement(camera,view);
        expect(mesh.tiles.every(t=>t.vertices.every(Number.isFinite))).toBe(true);
        // The GPU interpolates geography linearly inside each triangle, and
        // places the mesh's virtual canvas on the screen. Check interior
        // samples through both, independently of the mesh's own midpoint tests.
        for(let i=0;i<mesh.triangles.length;i+=Math.max(1,Math.floor(mesh.triangles.length/100))){
          const triangle=mesh.triangles[i]!;
          const centre=triangle.reduce((p,v)=>({lon:p.lon+v.lon/3,lat:p.lat+v.lat/3,x:p.x+v.x/3,y:p.y+v.y/3}),{lon:0,lat:0,x:0,y:0});
          const screen=project(camera,view,centre);
          expect(Math.hypot(screen.x-(centre.x*place.scale+place.x),screen.y-(centre.y*place.scale+place.y)),`${id} mesh sample ${i}`).toBeLessThan(1.1);
        }
      }
      const moved=cameraWithAnchor(camera,view,geo,{x:anchor.x+12,y:anchor.y-8});
      const translated=project(moved,view,geo);
      expect(translated.x).toBeCloseTo(anchor.x+12,2);expect(translated.y).toBeCloseTo(anchor.y-8,2);
      const km=kmFromPixels(camera,geo.lat,30,"geodesic",geo.lon);
      expect(km).toBeGreaterThan(0);
      expect(pixelsFromKm(camera,geo.lat,km,"geodesic",geo.lon)).toBeCloseTo(30,5);
    });
  }
  it("rejects the back hemisphere and pixels outside a globe",()=>{
    const camera=cameraForProjection({centerLon:0,centerLat:0,pxPerDeg:2},view,"orthographic");
    expect(Number.isFinite(project(camera,view,{lon:180,lat:0}).x)).toBe(false);
    expect(validGeo(unproject(camera,view,{x:0,y:0}))).toBe(false);
  });
});

/**
 * A fixed projection's mesh is made in the projection's own plane, so that a
 * pan or a zoom moves it rather than remaking it: remaking it was 150 to 400
 * ms a frame. What has to hold is that the *same* mesh, placed for whatever
 * the camera has become, still puts every place where the pointer's own
 * projection says it is — and that the tiles asked for still hold the view.
 */
describe("a plane mesh under a moving camera",()=>{
  const big={width:1200,height:760};
  /** The worst distance, in pixels, between where the mesh draws a place and where it is. */
  function worstMisplacement(camera:Parameters<typeof project>[0]):number {
    const mesh=projectedMesh(camera,big),place=meshPlacement(camera,big);
    let worst=0;
    for(let i=0;i<mesh.triangles.length;i+=Math.max(1,Math.floor(mesh.triangles.length/400))){
      for(const v of mesh.triangles[i]!){
        // At a pole the mesh gives a vertex its neighbours' mean longitude, on
        // purpose: where the pole is a line, that is not where it was sampled.
        if(Math.abs(v.lat)>89.9)continue;
        const truth=project(camera,big,v);
        if(!Number.isFinite(truth.x))continue;
        worst=Math.max(worst,Math.hypot(truth.x-(v.x*place.scale+place.x),truth.y-(v.y*place.scale+place.y)));
      }
    }
    return worst;
  }
  function holdsTheView(camera:Parameters<typeof project>[0]):void {
    const bounds=visibleTiles(camera,big).map(t=>({...tileBounds(t.z,t.x,t.y),offset:t.lonOffset}));
    for(let y=4;y<big.height;y+=63)for(let x=4;x<big.width;x+=71){
      const geo=unproject(camera,big,{x,y});if(!validGeo(geo))continue;
      // Just inside the map's own edge a pixel may belong to no triangle; ask well inside.
      if(![[-6,0],[6,0],[0,-6],[0,6]].every(([dx,dy])=>validGeo(unproject(camera,big,{x:x+dx!,y:y+dy!}))))continue;
      expect(bounds.some(b=>[-360,0,360].some(turn=>geo.lon+turn>=b.west+b.offset-1e-6&&geo.lon+turn<=b.east+b.offset+1e-6)&&geo.lat<=b.north+1e-6&&geo.lat>=b.south-1e-6),
        `${camera.projection} at ${camera.pxPerDeg}: no tile holds ${geo.lon},${geo.lat}`).toBe(true);
    }
  }

  for(const id of ["robinson","epsg_3413","epsg_27700"] as ProjectionId[]){
    it(`${id}: pans and zooms within a band without a new mesh, and stays where the pointer is`,()=>{
      let camera=cameraForProjection({centerLon:0,centerLat:20,pxPerDeg:2},big,id);
      refreshProjectedMesh(camera,big);
      const first=projectedMesh(camera,big);
      expect(worstMisplacement(camera)).toBeLessThan(1);
      holdsTheView(camera);
      // Forty small pans and a breath of zoom: one mesh throughout.
      for(let i=0;i<40;i++){
        camera=panBy(camera,big,7,-3);
        camera={...camera,pxPerDeg:camera.pxPerDeg*1.004};
        expect(projectedMesh(camera,big),`${id}: pan ${i} remade the mesh`).toBe(first);
        expect(worstMisplacement(camera),`${id}: pan ${i}`).toBeLessThan(1);
      }
      holdsTheView(camera);
    });

    it(`${id}: zooming in a band makes do, says so, and is right once refreshed`,()=>{
      const camera=cameraForProjection({centerLon:0,centerLat:20,pxPerDeg:2},big,id);
      refreshProjectedMesh(camera,big);
      const first=projectedMesh(camera,big);
      const closer=zoomAbout(camera,big,{x:big.width/2,y:big.height/2},2.2);
      expect(projectedMesh(closer,big)).toBe(first);
      expect(projectedMeshWanted()).toBe(true);
      // Coarser than it would like, not wrong: a plane mesh is right at any scale.
      expect(worstMisplacement(closer)).toBeLessThan(2.5);
      holdsTheView(closer);
      refreshProjectedMesh(closer,big);
      expect(projectedMesh(closer,big)).not.toBe(first);
      expect(projectedMeshWanted()).toBe(false);
      expect(worstMisplacement(closer)).toBeLessThan(1);
      holdsTheView(closer);
    });
  }

  it("is remade at once when the view leaves it, since there is nothing else to draw",()=>{
    let camera=cameraForProjection({centerLon:0,centerLat:20,pxPerDeg:2},big,"epsg_27700");
    camera=zoomAbout(camera,big,{x:big.width/2,y:big.height/2},6);
    refreshProjectedMesh(camera,big);
    const first=projectedMesh(camera,big);
    const far=panBy(camera,big,big.width*1.4,0);
    expect(projectedMesh(far,big)).not.toBe(first);
    expect(worstMisplacement(far)).toBeLessThan(1);
    holdsTheView(far);
  });

  it("stops asking once the camera is one it does not draw",()=>{
    const camera=cameraForProjection({centerLon:0,centerLat:20,pxPerDeg:2},big,"robinson");
    refreshProjectedMesh(camera,big);
    projectedMesh(zoomAbout(camera,big,{x:600,y:380},2.2),big);
    expect(projectedMeshWanted()).toBe(true);
    // The projection is changed before the pointer rests: no redraw loop.
    refreshProjectedMesh({centerLon:0,centerLat:0,pxPerDeg:4},big);
    expect(projectedMeshWanted()).toBe(false);
  });
});
