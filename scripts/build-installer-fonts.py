"""Статические Onest 400/500/600 (латиница + кириллица) для установщика.

Источник — переменный Onest из @fontsource-variable/onest (woff2). Нужны fontTools и brotli.
Запуск: python scripts/build-installer-fonts.py
"""
import io
import pathlib

from fontTools.merge import Merger
from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont

ROOT = pathlib.Path(__file__).resolve().parent.parent
SRC = ROOT / "node_modules" / "@fontsource-variable" / "onest"
OUT = ROOT / "installer" / "assets" / "fonts"
SUBSETS = ["latin", "latin-ext", "cyrillic", "cyrillic-ext"]
WEIGHTS = {400: "Regular", 500: "Medium", 600: "SemiBold"}


def instance(subset, weight):
	font = TTFont(SRC / "files" / f"onest-{subset}-wght-normal.woff2")
	static = instantiateVariableFont(font, {"wght": weight}, updateFontNames=False)
	static.flavor = None
	buf = io.BytesIO()
	static.save(buf)
	buf.seek(0)
	return buf


OUT.mkdir(parents=True, exist_ok=True)
for weight, style in WEIGHTS.items():
	parts = [instance(s, weight) for s in SUBSETS]
	merged = Merger().merge(parts)
	name = merged["name"]
	for rec in list(name.names):
		if rec.nameID in (1, 16):
			name.setName("Onest", rec.nameID, rec.platformID, rec.platEncID, rec.langID)
		elif rec.nameID in (2, 17):
			name.setName(style, rec.nameID, rec.platformID, rec.platEncID, rec.langID)
		elif rec.nameID in (4,):
			name.setName(f"Onest {style}", rec.nameID, rec.platformID, rec.platEncID, rec.langID)
		elif rec.nameID == 6:
			name.setName(f"Onest-{style}", rec.nameID, rec.platformID, rec.platEncID, rec.langID)
	merged["OS/2"].usWeightClass = weight
	path = OUT / f"Onest-{style}.ttf"
	merged.save(path)
	print(path.name, path.stat().st_size)

(OUT / "OFL.txt").write_text((SRC / "LICENSE").read_text(encoding="utf-8"), encoding="utf-8")
