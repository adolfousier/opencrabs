from PIL import Image, ImageDraw
from pathlib import Path

base = Path('/Users/moe/Desktop/crabz/desktop/src-tauri/icons')
base.mkdir(parents=True, exist_ok=True)
size = 1024
img = Image.new('RGBA', (size, size), (12, 16, 28, 255))
d = ImageDraw.Draw(img)
margin = 110
# shell
outer = [margin, 180, size - margin, size - 140]
d.rounded_rectangle(outer, radius=170, fill=(255, 106, 61, 255))
# inner body
d.rounded_rectangle([220, 300, size - 220, size - 250], radius=120, fill=(255, 188, 162, 255))
# eyes
for cx in (390, 634):
    d.ellipse([cx - 48, 332, cx + 48, 428], fill=(12, 16, 28, 255))
# claws
d.polygon([(180, 340), (60, 250), (90, 470), (215, 455)], fill=(255, 106, 61, 255))
d.polygon([(844, 340), (964, 250), (934, 470), (809, 455)], fill=(255, 106, 61, 255))
# legs
for x in (250, 360, 664, 774):
    d.rounded_rectangle([x, 710, x + 70, 900], radius=28, fill=(255, 106, 61, 255))

out = base / 'opencrabs-icon.png'
img.save(out)
print(out)
