"""Generate assets/icon.ico (a document with a pair of scissors-style cut line)."""

from pathlib import Path

from PIL import Image, ImageDraw

S = 256
img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
d = ImageDraw.Draw(img)
# rounded blue background
d.rounded_rectangle((8, 8, S - 8, S - 8), radius=48, fill=(37, 99, 235, 255))
# two page halves, slightly separated
d.rounded_rectangle((62, 40, 194, 118), radius=10, fill=(255, 255, 255, 255))
d.rounded_rectangle((62, 138, 194, 216), radius=10, fill=(255, 255, 255, 255))
for y in (62, 84, 160, 182):
    d.rounded_rectangle((84, y, 172, y + 10), radius=5, fill=(191, 219, 254, 255))
# dashed cut line
for x in range(30, S - 30, 22):
    d.rounded_rectangle((x, 124, x + 12, 132), radius=3, fill=(250, 204, 21, 255))

out = Path(__file__).resolve().parent.parent / "assets"
out.mkdir(exist_ok=True)
img.save(out / "icon.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
img.save(out / "icon.png")
print("written", out)
