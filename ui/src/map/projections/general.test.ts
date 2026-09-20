import { describe, expect, it } from "vitest";
import proj4 from "proj4";
import { GENERAL_MAPS, METRES_PER_DEGREE, customMap, generalMap, defaultCentre, mapTransform } from "./general";
import references from "./crs-reference.json";
import { cameraForProjection, project, unproject, validGeo, projectedMesh, zoomAbout, cameraWithAnchor } from "../camera";
import { kmFromPixels, pixelsFromKm } from "../footprint";
import type { ProjectionId } from "../projection";

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
      const mesh=projectedMesh(camera,view);expect(mesh.tiles.length).toBeGreaterThan(0);expect(mesh.tiles.length).toBeLessThanOrEqual(192);
      expect(mesh.tiles.every(t=>t.vertices.every(Number.isFinite))).toBe(true);
      // The GPU interpolates geography linearly inside each triangle. Check
      // interior samples, independently of the mesh's midpoint error tests.
      for(let i=0;i<mesh.triangles.length;i+=Math.max(1,Math.floor(mesh.triangles.length/100))){
        const triangle=mesh.triangles[i]!;
        const centre=triangle.reduce((p,v)=>({lon:p.lon+v.lon/3,lat:p.lat+v.lat/3,x:p.x+v.x/3,y:p.y+v.y/3}),{lon:0,lat:0,x:0,y:0});
        const screen=project(camera,view,centre);
        expect(Math.hypot(screen.x-centre.x,screen.y-centre.y),`${id} mesh sample ${i}`).toBeLessThan(1.1);
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
