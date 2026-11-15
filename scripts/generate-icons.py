"""用 Chromium 保留 Figma foreignObject/三维图层，再生成 Windows 图标。

运行：python -I scripts/generate-icons.py <source.svg> <chrome.exe>
只生成资产，不编译应用；原始图稿保留，等比缩放与透明补边。
"""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import xml.etree.ElementTree as ET
from PIL import Image

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('source', type=Path)
    parser.add_argument('browser', type=Path)
    args = parser.parse_args()
    raw = args.source.read_bytes()
    tree = ET.fromstring(raw)
    for element in tree.iter():
        if element.tag.split('}')[-1] in {'script', 'iframe', 'object', 'embed'}:
            raise ValueError('图稿含主动内容，拒绝渲染')
        for key, value in element.attrib.items():
            if key.startswith('on') or (key.endswith('href') and not value.startswith(('#', 'data:'))):
                raise ValueError('图稿含外部资源或事件处理器，拒绝渲染')
    viewbox = [float(n) for n in tree.attrib['viewBox'].replace(',', ' ').split()]
    width, height = viewbox[2:]
    side = 1088
    left, top = 85.6436 - 32, 236.888 - 32
    import re
    artwork = re.sub(r'<rect width="1158" height="1267" fill="#1E1E1E"\s*/>', '', raw.decode('utf-8'), count=1)
    artwork = re.sub(r'viewBox="0 0 1158 1267"', f'viewBox="{left} {top} {side} {side}"', artwork, count=1)
    artwork = re.sub(r'<svg width="1158" height="1267"', f'<svg width="{side}" height="{side}"', artwork, count=1)
    outline = list(tree)[1].attrib['d']
    artwork = re.sub(r'(<svg\b[^>]*>)', r'\1<g clip-path="url(#celesvault-icon-boundary)">', artwork, count=1)
    position = artwork.rfind('</svg>')
    artwork = artwork[:position] + f'</g><defs><clipPath id="celesvault-icon-boundary"><path d="{outline}" /></clipPath></defs>' + artwork[position:]
    branding = ROOT / 'assets' / 'branding'
    branding.mkdir(parents=True, exist_ok=True)
    (branding / 'CelesVault.original.svg').write_bytes(raw)
    (branding / 'CelesVault.svg').write_text(artwork, encoding='utf-8')
    icons = ROOT / 'src-tauri' / 'icons'
    icons.mkdir(parents=True, exist_ok=True)
    public = ROOT / 'public'
    public.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='CelesVault-icon-') as scratch:
        scratch = Path(scratch)
        svg = artwork
        width = height = side
        page = scratch / 'render.html'
        page.write_text(f'<!doctype html><html><head><meta charset="UTF-8"><style>html,body{{margin:0;width:{side}px;height:{side}px;background:transparent;overflow:hidden}}body>svg{{position:absolute;left:{(side-width)/2}px;top:{(side-height)/2}px;width:{width}px;height:{height}px}}</style></head><body>{svg}</body></html>', encoding='utf-8')
        rendered = scratch / 'render.png'
        subprocess.run([str(args.browser), '--headless', '--no-first-run', '--no-default-browser-check',
                        '--disable-extensions', '--disable-background-networking', '--hide-scrollbars',
                        '--default-background-color=00000000', '--force-device-scale-factor=2',
                        '--allow-file-access-from-files', f'--user-data-dir={scratch / "profile"}',
                        f'--window-size={side},{side}', '--virtual-time-budget=2000',
                        f'--screenshot={rendered}', page.as_uri()], check=True, timeout=90,
                        stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        with Image.open(rendered) as image:
            master = image.convert('RGBA')
            if master.width != master.height:
                raise ValueError('浏览器未输出方形图稿')
            master = master.resize((1024, 1024), Image.Resampling.LANCZOS)
        master.save(branding / 'CelesVault-1024.png')
        sizes = [16, 24, 32, 48, 64, 128, 256, 512, 1024]
        for size in sizes:
            master.resize((size, size), Image.Resampling.LANCZOS).save(icons / f'{size}x{size}.png')
        master.resize((256, 256), Image.Resampling.LANCZOS).save(icons / '128x128@2x.png')
        tray = master.resize((32, 32), Image.Resampling.LANCZOS)
        tray.save(icons / 'tray.png')
        from PIL import ImageOps
        paused = ImageOps.grayscale(tray).convert('RGBA')
        paused.putalpha(tray.getchannel('A'))
        paused.save(icons / 'tray-paused.png')
        error = ImageOps.colorize(ImageOps.grayscale(tray), '#641e1e', '#f08080').convert('RGBA')
        error.putalpha(tray.getchannel('A'))
        error.save(icons / 'tray-error.png')
        master.save(icons / 'icon.ico', format='ICO', sizes=[(n,n) for n in [16,24,32,48,64,128,256]])
        shutil.copyfile(icons / 'icon.ico', public / 'favicon.ico')
        shutil.copyfile(icons / '32x32.png', public / 'favicon-32.png')
        shutil.copyfile(icons / '192x192.png', public / 'icon-192.png') if (icons / '192x192.png').exists() else master.resize((192,192), Image.Resampling.LANCZOS).save(public / 'icon-192.png')
        shutil.copyfile(icons / '512x512.png', public / 'icon-512.png')
        outputs = [branding / 'CelesVault.original.svg', branding / 'CelesVault.svg', branding / 'CelesVault-1024.png', *sorted(icons.glob('*')), *[public / n for n in ['favicon.ico','favicon-32.png','icon-192.png','icon-512.png']]]
        manifest = {'source': '用户提供的 CelesVault.svg', 'source_sha256': hashlib.sha256(raw).hexdigest(),
                    'original_viewbox': viewbox, 'square_canvas': side, 'renderer': 'Chromium headless, 2x raster',
                    'operations': '保留原稿；应用派生图移除Figma画板背景，以原1024方形圆角主体居中加32单位透明安全边距，保留三维图层与纹理。PNG RGBA，ICO 多尺寸。',
                    'files': [{'path': p.relative_to(ROOT).as_posix(), 'sha256': hashlib.sha256(p.read_bytes()).hexdigest()} for p in outputs]}
        (branding / 'manifest.json').write_text(json.dumps(manifest, ensure_ascii=False, indent=2)+'\n', encoding='utf-8')
        print(json.dumps({'files':len(outputs), 'source_sha256':manifest['source_sha256'], 'ico_sizes':[16,24,32,48,64,128,256]}, ensure_ascii=False))

if __name__ == '__main__':
    main()
