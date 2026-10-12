"""Convert placeable objects from the owner's Skate 3 files into the private builder cache.

Uses the same pinned, installed RX2 readers as setup. Never changes the game install.
"""
from __future__ import annotations
import argparse, hashlib, json, re, struct, sys, zlib
import xml.etree.ElementTree as ET
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ENGINE = ROOT / '.setup/skate-engine'
sys.path.insert(0, str(ENGINE))
sys.path.insert(0, str(ENGINE/'tools/vendor/utt'))
sys.path.insert(0, str(ENGINE/'tools/vendor/university/tools/vanilla_map_extraction/tools'))

def asset_id(path):
    return (2 << 62) | (int.from_bytes(hashlib.sha1(path.replace('\\','/').lower().encode()).digest()[:8],'little') & ((1 << 62)-1))

def friendly(name):
    name=re.sub(r'^[a-z]{2}_','',name)
    name=re.sub(r'([a-z])([A-Z])',r'\1 \2',name).replace('_',' ')
    replacements={'dmo':'','glbl':'Global','gl':'Global','lrg':'Large','lg':'Large','med':'Medium','sml':'Small','sm':'Small','dt':'Downtown','id':'Industrial','qp':'Quarter Pipe'}
    return ' '.join(replacements.get(w.lower(),w) for w in name.split() if replacements.get(w.lower(),w)).strip()

def write_json(path, data):
    temp=path.with_suffix(path.suffix+'.new')
    temp.write_text(json.dumps(data,ensure_ascii=False,allow_nan=False),encoding='utf-8');temp.replace(path)

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--game',type=Path)
    p.add_argument('--out',type=Path,default=ROOT/'benilla-config/park-assets')
    p.add_argument('--limit',type=int,default=0,help='Explicit diagnostic subset; not published as the installed catalog')
    a=p.parse_args()
    if not a.game:a.game=Path((ROOT/'.setup/skate3-path').read_text(encoding='utf-8-sig').strip())
    game=a.game.resolve();out=a.out.resolve()
    if out==game or out.is_relative_to(game):raise RuntimeError('Object cache must be outside the read-only game install')
    if not (game/'default.xex').is_file():raise RuntimeError('Skate 3 source folder is missing default.xex')
    out.mkdir(parents=True,exist_ok=True);(out/'models').mkdir(exist_ok=True);(out/'textures/skate').mkdir(parents=True,exist_ok=True)
    from tools.owned_game.big import BigArchive
    from mdl_parser import parse_rx2
    import rx2_parser
    import numpy as np
    from PIL import Image
    import prepare_hawaiian_dream as prep
    from retail_collision_mesh import decode_rx2_clustered_meshes
    from retail_grind_splines import decode_grind_splines
    from tools.asset_pipeline.dynamic_props import template_meshes
    park=BigArchive(game/'data/content/parkassets.big')
    models={Path(e.path).stem.lower():e for e in park.entries if '/model/' in e.path and e.path.lower().endswith('.rx2')}
    texture_sources={}
    archives=[park]
    # Recipe channels name cooked texture IDs. Shared textures may live in the boot banks.
    for bank in ['data/big/miscload.big','data/big/miscboot.big']:
        if (game/bank).is_file():archives.append(BigArchive(game/bank))
    for archive in archives:
        for e in archive.entries:
            if '/texture/' in e.path.lower() and e.path.lower().endswith('.rx2'):
                texture_sources.setdefault(Path(e.path).stem.lower(),(archive,e))
    variants=[];materials={}
    for e in park.entries:
        if not e.path.endswith('.xml'):continue
        recipe=ET.fromstring(park.read(e))
        for m in recipe.findall('mat'):
            materials[m.get('id').lower()]={'type':m.get('type',''),'channels':{s.get('chn'):s.get('id').lower() for s in m.findall('sp')}}
        for category in recipe.findall('comp'):
            for model in category.findall('mod'):
                lod=next((l for l in model.findall('lod') if l.get('idx')=='0'),None)
                if lod is None:continue
                aid=lod.get('arenaid','').lower()
                variants.append(dict(name=model.get('n'),category=category.get('n'),source=aid,template=lod.get('id','').lower(),id=model.get('id').lower(),materials=[m.get('id').lower() for m in lod.findall('matinst/matvar')]))
    print(f'Skate catalog: {len(variants)} native object variants, {len(models)} model resources',flush=True)
    exported=[];failures=[];editor_markers=[];decoded={};texture_cache={}
    def texture(key):
        if key in texture_cache:return texture_cache[key]
        source=texture_sources.get(key)
        if source is None:raise ValueError('Missing object texture '+key)
        raw=source[0].read(source[1]);parsed=rx2_parser.parse_rx2(raw)
        if not parsed.textures:raise ValueError('Texture resource contains no texture '+key)
        t=parsed.textures[0]
        if t.width*t.height>16_777_216:raise ValueError('Unreasonable object texture dimensions')
        relative='textures/skate/'+key+'.png';path=out/relative
        if not path.is_file():
            image=Image.frombytes('RGBA',(t.width,t.height),t.rgba)
            temp=path.with_suffix('.png.new');image.save(temp,format='PNG');temp.replace(path)
        texture_cache[key]=relative;return relative
    for n,v in enumerate(variants[:a.limit or None]):
        logical='skatepark/'+v['id'];identifier=asset_id(logical)
        try:
            entry=models.get(v['source'])
            if entry is None:raise ValueError('Missing native model resource '+v['source'])
            if v['source'] not in decoded:
                raw=park.read(entry)
                if raw[0x58:0x5c] == b'\x00\x00\x00\x01':
                    count,table=struct.unpack_from('>I',raw,32)[0],struct.unpack_from('>I',raw,48)[0]
                    types=[struct.unpack_from('>I',raw,table+j*24+20)[0] for j in range(count)]
                    if 0xEB0023 not in types and 0x200E9 not in types:
                        editor_markers.append(dict(object=v['name'],source=v['source'],kind='native editor marker without render geometry'))
                        continue
                model=parse_rx2(raw,strict=False)
                groups,bindings=prep._bind_material_groups_by_guid(raw,prep._group_material_parameters(model.materials),len(model.meshes),allow_import_order_fallback=False)
                templates=template_meshes(raw)
                collisions=decode_rx2_clustered_meshes(raw)
                splines=decode_grind_splines(raw)
                decoded[v['source']]=(raw,model,groups,bindings,templates,collisions,splines)
            raw,model,groups,bindings,templates,collisions,splines=decoded[v['source']]
            template=templates.get(v['template'].removeprefix('0x').upper())
            transform=np.eye(4) if template is None else template['model_matrix'] @ template['matrix']
            if not np.isfinite(transform).all():raise ValueError('Non-finite source transform')
            def points(values):return (np.asarray(values) @ transform[:3,:3] + transform[3,:3])/0.9144
            normal_matrix=np.linalg.inv(transform[:3,:3]).T
            batches=[]
            for i,mesh in enumerate(model.meshes):
                if not mesh.vertex_count or not mesh.triangle_count:continue
                if template is not None and mesh.source_offsets['mesh_info'] not in template['mesh_info']:continue
                group=groups[i];original=re.search(r'(0x[0-9a-fA-F]{16})$',group.get('Name',[''])[0])
                material_key=original.group(1).lower() if original else None
                if material_key not in v['materials'] and len(v['materials'])==len(groups):material_key=v['materials'][bindings[i]['group_index']]
                mat=materials.get(material_key)
                if mat is None:raise ValueError('Unresolved recipe material '+str(material_key))
                channels=mat['channels'];albedo=channels.get('diffuse',channels.get('transparent',channels.get('transparency')))
                normals=mesh.normals
                if normals is None:
                    normals=np.zeros_like(mesh.vertices);face=np.cross(mesh.vertices[mesh.faces[:,1]]-mesh.vertices[mesh.faces[:,0]],mesh.vertices[mesh.faces[:,2]]-mesh.vertices[mesh.faces[:,0]])
                    for corner in range(3):np.add.at(normals,mesh.faces[:,corner],face)
                normals=np.asarray(normals) @ normal_matrix;normals/=np.maximum(np.linalg.norm(normals,axis=1,keepdims=True),1e-20)
                pos=points(mesh.vertices).astype('f4');faces=mesh.faces
                if np.linalg.det(transform[:3,:3])<0:faces=faces[:,[0,2,1]]
                metadata=prep._material_metadata(groups,i)
                batches.append(dict(positions=pos.tolist(),normals=normals.astype('f4').tolist(),uvs=(mesh.uvs if mesh.uvs is not None else np.zeros((len(pos),2))).tolist(),indices=faces.reshape(-1).tolist(),texture=texture(albedo) if albedo else None,alpha=metadata['alpha_mode'],cutoff=.5,two_sided=False,repeat_u=True,repeat_v=True))
            if not batches:raise ValueError('Native model has no renderable meshes')
            all_points=np.concatenate([np.array(b['positions'],dtype='f4') for b in batches]);low=all_points.min(axis=0);high=all_points.max(axis=0)
            origin=np.array([(low[0]+high[0])/2,low[1],(low[2]+high[2])/2],dtype='f4')
            collision=[]
            for c in collisions:
                for t in c.triangles:
                    tri=(points([t.a,t.b,t.c])-origin).astype('f4')
                    if np.linalg.det(transform[:3,:3])<0:tri=tri[[0,2,1]]
                    collision.append(tri.tolist())
            for b in batches:b['positions']=(np.array(b['positions'],dtype='f4')-origin).astype('f4').tolist()
            if not collision:
                for b in batches:
                    verts=np.array(b['positions'],dtype='f4');faces=np.array(b['indices'],dtype='u4').reshape(-1,3)
                    collision.extend(verts[faces].tolist())
            rails=[]
            # Authored spline records are retained when present; edge-derived rails fill models without them at runtime.
            for spline in splines:
                samples=[]
                for payload in spline['native_segment_payloads']:
                    values=np.array(struct.unpack('>30f',bytes.fromhex(payload)))
                    aa,bb,cc,dd=(values[i:i+3] for i in [0,4,8,12])
                    samples.extend((dd+cc*t+bb*t*t+aa*t*t*t).tolist() for t in np.linspace(0,1,17)[not not samples:])
                if len(samples)>1:rails.append((points(samples)-origin).astype('f4').tolist())
            package=dict(version=3,id=identifier,batches=batches,collision=collision,rails=rails,bounds=[(low-origin).tolist(),(high-origin).tolist()])
            target=out/'models'/f'{identifier:016x}.spk';temporary=target.with_suffix('.spk.new')
            temporary.write_bytes(b'SPK3'+zlib.compress(json.dumps(package,separators=(',',':'),allow_nan=False).encode(),1));temporary.replace(target)
            exported.append(dict(id=identifier,game='Skate 3',name=friendly(v['name']),category=friendly(v['category']),path=logical,aliases=[v['name'],entry.path,v['source'],v['id']],package='models/'+target.name,bounds=package['bounds']))
        except Exception as error:
            failures.append(dict(object=v['name'],source=v['source'],error=str(error)))
        if n%50==0:print(f'Skate objects: {n+1}/{min(a.limit or len(variants),len(variants))}, converted {len(exported)}, unresolved {len(failures)}',flush=True)
    report=dict(version=3,entries=exported,unresolved=failures,editor_markers=editor_markers,source_count=len(variants))
    write_json(out/('skate-diagnostic.json' if a.limit else 'skate-catalog.json'),report)
    print(f'Converted {len(exported)} objects; {len(failures)} unresolved',flush=True)
    if failures:
        for error in failures[:12]:print(error,flush=True)
        raise SystemExit(2)

if __name__=='__main__':main()
