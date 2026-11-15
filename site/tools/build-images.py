# Скриншоты сайта в WebP нескольких ширин: python site/tools/build-images.py (нужен Pillow).
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parents[2]
SRC = ROOT / ".github" / "assets"
OUT = ROOT / "site" / "public" / "assets" / "img"
# Первый экран крупнее галереи, поэтому у него на одну ширину больше.
IMAGES = {
	"home-dark": [640, 1080, 1600, 2160],
	"home-light": [640, 1080, 1600, 2160],
	"catalog-dark": [640, 1080, 1600],
	"instance-dark": [640, 1080, 1600],
	"skins-dark": [640, 1080, 1600],
	"instances-light": [640, 1080, 1600],
}

for name, widths in IMAGES.items():
	image = Image.open(SRC / f"{name}.png").convert("RGB")
	for width in widths:
		height = round(image.height * width / image.width)
		image.resize((width, height), Image.LANCZOS).save(OUT / f"{name}-{width}.webp", "WEBP", quality=82, method=6)
	print(name)
