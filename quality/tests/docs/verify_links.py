"""Verify generated local links and image targets in a built site (stdlib only)."""
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import urlsplit, unquote
import argparse, json, sys
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('built_site_directory')
parser.add_argument('--base', default='/', help='absolute site path ending in / (default: /)')
args = parser.parse_args()
base = args.base
if not base.startswith('/') or not base.endswith('/') or base.startswith('//') or any(part in ('.', '..') for part in base.split('/')) or any(c in base for c in ('?', '#', '\\')):
    parser.error('--base must be an absolute site path ending in /')
root = Path(args.built_site_directory).resolve()
if not root.is_dir():
    sys.exit(f'built site directory does not exist: {root}')
class Page(HTMLParser):
    def __init__(self, file):
        super().__init__(); self.ids=set(); self.links=[]; self.file=file
    def handle_starttag(self, tag, attrs):
        attrs=dict(attrs)
        if attrs.get('id'): self.ids.add(attrs['id'])
        if tag in ('a','img','script','link'):
            value=attrs.get('href' if tag in ('a','link') else 'src')
            if value: self.links.append((tag,value))
pages={}
for f in root.rglob('*.html'):
    p=Page(f); p.feed(f.read_text()); pages[f]=p
if not pages:
    sys.exit(f'no HTML pages found under {root}')
errors=[]; checked=0
for f,p in pages.items():
    for tag,ref in p.links:
        url=urlsplit(ref)
        if url.scheme or url.netloc: continue
        url_path = unquote(url.path)
        if url_path.startswith('/'):
            if base != '/' and not url_path.startswith(base):
                errors.append({'page':str(f.relative_to(root)),'link':ref,'reason':'outside site base'})
                continue
            dst = root / url_path[len(base):]
        else:
            dst = f.parent / url_path if url_path else f
        dst=dst.resolve()
        if not dst.is_relative_to(root):
            errors.append({'page':str(f.relative_to(root)),'link':ref,'reason':'outside site directory'})
            continue
        if dst.is_dir(): dst=dst/'index.html'
        if not dst.exists() and not dst.suffix: dst=dst.with_suffix('.html')
        checked+=1
        if not dst.exists(): errors.append({'page':str(f.relative_to(root)),'link':ref,'reason':'missing file'})
        elif url.fragment and dst in pages and unquote(url.fragment) not in pages[dst].ids:
            errors.append({'page':str(f.relative_to(root)),'link':ref,'reason':'missing anchor'})
print(json.dumps({'pages':len(pages),'local_targets_checked':checked,'errors':errors},ensure_ascii=False,indent=2))
if errors: sys.exit(1)
