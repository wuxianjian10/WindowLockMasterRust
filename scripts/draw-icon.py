"""Render the original dual-monitor lock icon at Windows shell sizes."""
from pathlib import Path
from PIL import Image, ImageDraw

out = Path(__file__).resolve().parents[1] / 'assets'
out.mkdir(exist_ok=True)
canvas = Image.new('RGBA', (768, 768))
scale = 3

def box(coords):
    return tuple(round(x * scale) for x in coords)

def rounded(coords, radius, color, outline=None, width=1):
    ImageDraw.Draw(canvas).rounded_rectangle(box(coords), radius * scale, fill=color,
                                            outline=outline, width=width * scale)

rounded((8, 8, 248, 248), 51, '#132841', '#6DB9CE', 3)
mask = Image.new('L', canvas.size)
ImageDraw.Draw(mask).rounded_rectangle(box((12, 12, 244, 244)), 47 * scale, fill=255)
gradient = Image.new('RGBA', canvas.size)
pen = ImageDraw.Draw(gradient)
for y in range(768):
    t = y / 767
    pen.line((0, y, 768, y), fill=(int(32-17*t), int(75-46*t), int(105-54*t), 255))
canvas.paste(gradient, (0, 0), mask)
rounded((44, 54, 172, 143), 15, '#214B66', '#81B6D9', 10)
rounded((73, 77, 207, 169), 15, '#173B54', '#C6F0FF', 11)
rounded((112, 165, 166, 184), 6, '#C6F0FF')
rounded((93, 181, 183, 196), 7, '#C6F0FF')
rounded((166, 120, 228, 180), 26, '#224960', '#7FE7DB', 11)
rounded((157, 150, 237, 213), 15, '#76EDDB', '#B7FFF2', 4)
rounded((191, 170, 203, 195), 6, '#123C4D')
icon = canvas.resize((256, 256), Image.Resampling.LANCZOS)
icon.save(out / 'window-lock-master.png')
icon.save(out / 'window-lock-master.ico', sizes=[(n, n) for n in (16, 20, 24, 32, 40, 48, 64, 128, 256)])
