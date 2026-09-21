"""Verify generated local links and image targets in a built site (stdlib only)."""
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import urlsplit, unquote
import json, sys
if len(sys.argv) != 2:
    sys.exit('usage: verify_links.py BUILT_SITE_DIRECTORY')
root = Path(sys.argv[1]).resolve()
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
        dst=(root/unquote(url.path).lstrip('/')) if url.path.startswith('/') else (f.parent/unquote(url.path)) if url.path else f
        dst=dst.resolve()
        if dst.is_dir(): dst=dst/'index.html'
        if not dst.exists() and not dst.suffix: dst=dst.with_suffix('.html')
        checked+=1
        if not dst.exists(): errors.append({'page':str(f.relative_to(root)),'link':ref,'reason':'missing file'})
        elif url.fragment and dst in pages and unquote(url.fragment) not in pages[dst].ids:
            errors.append({'page':str(f.relative_to(root)),'link':ref,'reason':'missing anchor'})
print(json.dumps({'pages':len(pages),'local_targets_checked':checked,'errors':errors},ensure_ascii=False,indent=2))
if errors: sys.exit(1)
