# Спрайт пустого списка сборок (16x16, рисуется x4). Остальные буквы — SPRITE_PALETTE из build-pixel.py.

# Тёплые цвета — только у этого спрайта: дерево в три тона и золото.
WARM = {
	"t": "#d9a066",  # дерево, светлое
	"b": "#a86a3d",  # дерево, среднее
	"r": "#6e4122",  # дерево, тёмное
	"y": "#ffcf4a",  # золото
	"h": "#dc9a1c",  # золото, тень
}

# Пустой список сборок: открытый пустой ящик, из него выглядывает искра.
EMPTY = [
	"..........y.....",
	".........ywy....",
	"..........y.....",
	"................",
	".oooo......oooo.",
	"ottto......orro.",
	".oootoooooooroo.",
	"..ottttttttttro.",
	"..obbbbbbbbbbro.",
	"..obbbbbbbbbbro.",
	"..obbbooooobbro.",
	"..obbbbbbbbbbro.",
	"..obbbbbbbbbbro.",
	"..orrrrrrrrrroo.",
	"..oooooooooooo..",
	"................",
]


def _rgb(h):
	h = h.lstrip("#")
	return (int(h[0:2], 16), int(h[2:4], 16), int(h[4:6], 16), 255)


def _image(rows, colors, Image):
	assert len(rows) == 16, len(rows)
	im = Image.new("RGBA", (16, 16), (0, 0, 0, 0))
	for y, row in enumerate(rows):
		assert len(row) == 16, (y, row)
		for x, ch in enumerate(row):
			if ch not in ". ":
				im.putpixel((x, y), colors[ch])
	return im


def build(colors, Image):
	colors = {**colors, **{k: _rgb(v) for k, v in WARM.items()}}
	return {"sprite-instances-empty": _image(EMPTY, colors, Image)}
