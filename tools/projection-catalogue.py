#!/usr/bin/env python3
"""Regenerate the offline map presets and reference coordinates from OSGeo PROJ.

Usage: PROJ_DATA=/path/to/proj/data python3 tools/projection-catalogue.py
The catalogue covers the national, polar, and zoned grids requested for the map.
No network lookup happens in the application. projinfo/proj are build-time tools.
"""
import json
import os
from pathlib import Path
import subprocess

root = Path(__file__).resolve().parents[1]
env = dict(os.environ, PROJ_DATA=os.environ.get('PROJ_DATA', '/usr/local/share/proj'))
groups = {
    'Polar and sea ice': [3413,3995,3031,3411,3412,6931,6932,32661,32761,*range(3571,3577)],
    'Europe': [27700,2154,*range(3942,3951),3812,2056,28992,5514,*range(3006,3019)],
    'Alaska and Hawaii': [3338,*range(26931,26941),*range(26961,26966)],
    'Canada': [3978,3347],
    'New Zealand': [2193,27200,*range(2105,2133)],
    'Japan': [*range(6669,6688)],
    'Australia': [*range(7846,7860)],
    'UTM': [*range(32601,32661),*range(32701,32761)],
}
rows=[]
refs=[]
for group, codes in groups.items():
    for code in codes:
        text=subprocess.check_output(['projinfo',f'EPSG:{code}','-o','PROJ,PROJJSON'],env=env,text=True)
        definition,raw=text.split('\nPROJJSON:\n')
        definition=definition.split('\n',1)[1].strip().replace(' +type=crs','')
        crs=json.loads(raw)
        b=crs['bbox'];bbox=[b['west_longitude'],b['south_latitude'],b['east_longitude'],b['north_latitude']]
        # Built-in Helmert shifts; fine national grid corrections are intentionally
        # not fetched implicitly by a view setting.
        if code==27700: definition += ' +datum=OSGB36'
        elif code==2056: definition += ' +towgs84=674.374,15.056,405.346,0,0,0,0'
        elif code==28992: definition += ' +towgs84=565.4171,50.3319,465.5524,-0.398957,0.343988,-1.8774,4.0725'
        elif code==5514: definition += ' +towgs84=589,76,480,0,0,0,0'
        elif code==27200: definition += ' +datum=nzgd49'
        row={'id':f'epsg_{code}','code':code,'label':crs['name'],'group':group,'definition':definition,'bbox':bbox}
        countries = {27700:'United Kingdom Britain UK England Scotland Wales',2154:'France French',3812:'Belgium Belgian',2056:'Switzerland Swiss',28992:'Netherlands Dutch',5514:'Czechia Czech Republic Slovakia'}
        row['keywords'] = countries.get(code, 'France French' if 3942 <= code <= 3950 else 'Sweden Swedish' if 3006 <= code <= 3018 else '')
        rows.append(row)
        west,south,east,north=bbox
        if east<west:east+=360
        lon=(west+east)/2;lon=(lon+180)%360-180;lat=(south+north)/2
        # Independent projection-method references (datum conversion is tested
        # separately). Geographic inputs here are on the target CRS ellipsoid.
        projected=subprocess.check_output(['proj','-f','%.10f',*definition.split()],input=f'{lon} {lat}\n',env=env,text=True)
        x,y=map(float,projected.split()[:2]);refs.append({'id':row['id'],'lon':lon,'lat':lat,'x':x,'y':y})
# The Hawaii statewide HARN definition has no dedicated EPSG UTM 4N entry.
rows += [
 {'id':'hawaii_harn_utm4','label':'Hawaii statewide — NAD83(HARN) / UTM 4N','group':'Alaska and Hawaii','definition':'+proj=utm +zone=4 +ellps=GRS80 +units=m +no_defs','bbox':[-160.3,18.8,-154.7,22.3]},
 {'id':'hawaii_albers','label':'Hawaii Albers (USGS Landsat)','group':'Alaska and Hawaii','definition':'+proj=aea +lat_0=3 +lon_0=-157 +lat_1=8 +lat_2=18 +x_0=0 +y_0=0 +ellps=GRS80 +units=m +no_defs','bbox':[-160.3,18.8,-154.7,22.3]},
]
(root/'ui/src/map/projections/crs.json').write_text(json.dumps(rows,indent=2)+'\n')
(root/'ui/src/map/projections/crs-reference.json').write_text(json.dumps(refs,indent=2)+'\n')
print(f'Wrote {len(rows)} presets and {len(refs)} independent reference positions')
