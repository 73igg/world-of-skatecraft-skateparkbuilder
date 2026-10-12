"""Prepare the pinned skate host with native camera preferences and raw input handoff."""
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parent.parent
ENGINE = ROOT / '.setup/skate-camera-engine'
PATCH = ROOT / 'setup/skate-camera.patch'
REVISION = '099b1948e9c08394b20c760ef123da38ef347278'
URL = 'https://github.com/Kimmo3223/2010-rust-rewrite-mashup'

def git(*args, **kwargs):
    return subprocess.run(['git', '-C', str(ENGINE), *args], **kwargs)

if not (ENGINE / '.git').exists():
    ENGINE.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(['git', 'clone', '--quiet', URL, str(ENGINE)], check=True)
    git('checkout', '--quiet', REVISION, check=True)
revision = git('rev-parse', 'HEAD', capture_output=True, text=True, check=True).stdout.strip()
if revision != REVISION:
    raise SystemExit('The skate camera host must use the pinned revision ' + REVISION)
if git('apply', '--reverse', '--check', str(PATCH), capture_output=True).returncode != 0:
    git('apply', '--check', str(PATCH), check=True)
    git('apply', str(PATCH), check=True)
print('Pinned skate host ready: native Low/High camera selection enabled.')
