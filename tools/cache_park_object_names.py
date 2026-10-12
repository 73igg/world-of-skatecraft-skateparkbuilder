"""Cache game-object display names from the local realm; this is a read-only database query."""
from pathlib import Path
import argparse,json,subprocess
ROOT=Path(__file__).resolve().parents[1]
def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--docker',default='docker')
    parser.add_argument('--container',default='world-of-skatecraft-database-1')
    parser.add_argument('--out',type=Path,default=ROOT/'benilla-config/park-assets/wow-names.json')
    args=parser.parse_args()
    query="SELECT JSON_OBJECT('display',displayId,'name',name) FROM mangos.gameobject_template WHERE displayId>0 AND name<>'';"
    result=subprocess.run([args.docker,'exec','-i',args.container,'sh','-c','exec mariadb -N --raw --batch -uroot -p"$MARIADB_ROOT_PASSWORD"'],input=query,text=True,encoding='utf-8',capture_output=True,check=True)
    groups={}
    for line in result.stdout.splitlines():
        row=json.loads(line);groups.setdefault(str(row['display']),set()).add(row['name'])
    output={k:sorted(v) for k,v in groups.items()}
    args.out.parent.mkdir(parents=True,exist_ok=True)
    temporary=args.out.with_suffix('.json.new');temporary.write_text(json.dumps(output,ensure_ascii=False),encoding='utf-8');temporary.replace(args.out)
    print(f'Cached searchable WoW names for {len(output)} model displays')
if __name__=='__main__':main()
