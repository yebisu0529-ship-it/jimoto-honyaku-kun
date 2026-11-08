"""Regenerate the original code-drawn translation mark (developer tool: Pillow/fontTools)."""
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont
from fontTools.ttLib import TTFont
from fontTools.pens.svgPathPen import SVGPathPen

root = Path(__file__).resolve().parent.parent
assets = root / "assets" / "icons"
extension = root / "extension" / "icons"
assets.mkdir(parents=True, exist_ok=True)
extension.mkdir(parents=True, exist_ok=True)
font_path = root / "assets/fonts/NotoSansJP-Regular.ttf"
font = TTFont(font_path)
glyphs = font.getGlyphSet()
upem = font["head"].unitsPerEm

def glyph(char, x, baseline, size, color):
    name = font.getBestCmap()[ord(char)]
    pen = SVGPathPen(glyphs)
    glyphs[name].draw(pen)
    advance = font["hmtx"].metrics[name][0]
    scale = size / upem
    return f'<path fill="{color}" transform="translate({x-advance*scale/2:.3f} {baseline}) scale({scale:.5f} {-scale:.5f})" d="{pen.getCommands()}"/>'

svg = ('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 128 128">'
       '<title>Talk風Live翻訳</title><rect width="128" height="128" rx="28" fill="#3CBFA4"/>'
       '<rect x="15" y="18" width="62" height="57" rx="13" fill="#F4FAF8"/>'
       '<path d="M24 67v18l19-15" fill="#F4FAF8"/>'
       + glyph("A", 46, 61, 40, "#142029")
       + '<rect x="51" y="48" width="62" height="57" rx="13" fill="#142029"/>'
       '<path d="M103 98v15L84 101" fill="#142029"/>'
       + glyph("文", 82, 90, 36, "#F4FAF8") + '</svg>')
(assets / "jimoto.svg").write_text(svg, encoding="utf-8")

scale = 8
image = Image.new("RGBA", (128*scale, 128*scale))
draw = ImageDraw.Draw(image)
box = lambda xy: tuple(int(x*scale) for x in xy)
draw.rounded_rectangle(box((0,0,128,128)), radius=28*scale, fill="#3CBFA4")
draw.rounded_rectangle(box((15,18,77,75)), radius=13*scale, fill="#F4FAF8")
draw.polygon([box(p) for p in [(24,67),(24,85),(43,70)]], fill="#F4FAF8")
draw.text((46*scale,61*scale), "A", font=ImageFont.truetype(str(font_path),40*scale), anchor="ms", fill="#142029")
draw.rounded_rectangle(box((51,48,113,105)), radius=13*scale, fill="#142029")
draw.polygon([box(p) for p in [(103,98),(103,113),(84,101)]], fill="#142029")
draw.text((82*scale,90*scale), "文", font=ImageFont.truetype(str(font_path),36*scale), anchor="ms", fill="#F4FAF8")
for size in [16,32,48,64,128,256,512]:
    icon = image.resize((size,size), Image.Resampling.LANCZOS)
    icon.save(assets/f"icon-{size}.png")
    if size in [16,32,48,128]:
        icon.save(extension/f"icon-{size}.png")
image.save(assets/"jimoto.ico", sizes=[(x,x) for x in [16,24,32,48,64,128,256]])
print("Generated SVG, PNG sizes, Windows ICO and extension icons.")
