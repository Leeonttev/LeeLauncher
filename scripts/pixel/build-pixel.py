# Пиксель-арт игры: правь сетки и палитры и запусти `python scripts/pixel/build-pixel.py` (нужен Pillow), PNG пересоберутся в src/assets/pixel.
import os
from PIL import Image

import covers
import empty_sprite
import skins

W, H = 110, 42
GROUND = 36
SPRITE_POS = (47, GROUND - 16)

# Одинаковые значения у разных ролей намеренны: сцена держится в 18 (день) и 20 (ночь) цветах.
PALETTES = {
	"day": {
		"sky3": "#82ccf6", "sky2": "#a8dbf9", "sky1": "#d7e8f8",
		"orb": "#fff0b3", "orbHi": "#ffffff", "orbRing": "#d7e8f8", "orbSpot": "#ffe08a",
		"far": "#73bde6", "farLit": "#92cced", "snow": "#ffffff",
		"mid": "#0093ca", "midLit": "#1bace9", "midHi": "#1bace9", "midDk": "#008abd",
		"leaf": "#006b94", "leafHi": "#008abd", "leafDk": "#004a7e",
		"trunk": "#4a5663", "trunkDk": "#333d47",
		"grassHi": "#1bace9", "grass": "#007aa9", "dirt": "#004a7e", "dirtDk": "#003359", "dirtSp": "#006b94",
		"tower": "#7c8b9a", "towerDk": "#4a5663", "lamp": "#ffffff", "flag": "#007aa9", "beam": "#ffffff", "glow": "#ffffff",
		"star": "#ffffff", "starDim": "#ffffff", "bird": "#0093ca",
		"cloud": "#ffffff", "cloudSh": "#d7e8f8",
		"storm": "#8d9caa", "stormDk": "#707f8d", "stormHi": "#a6b2bf", "rain": "#008ec3",
		"flower": "#ffffff", "spark": "#007aa9", "sparkHi": "#ffffff",
	},
	"night": {
		"sky3": "#00162c", "sky2": "#00203b", "sky1": "#002a4c",
		"orb": "#dceaf8", "orbHi": "#ffffff", "orbRing": "#002a4c", "orbSpot": "#a1cee9",
		"far": "#00365f", "farLit": "#00487a", "snow": "#56a3cc",
		"mid": "#004373", "midLit": "#00709b", "midHi": "#00709b", "midDk": "#00558f",
		"leaf": "#003d6a", "leafHi": "#00558f", "leafDk": "#002b4c",
		"trunk": "#28333d", "trunkDk": "#1a232c",
		"grassHi": "#00709b", "grass": "#0080b1", "dirt": "#002b4c", "dirtDk": "#00203c", "dirtSp": "#003d6a",
		"tower": "#2e4861", "towerDk": "#28333d", "lamp": "#ffffff", "flag": "#0080b1", "beam": "#00aded", "glow": "#00487a",
		"star": "#ffffff", "starDim": "#56a3cc", "bird": "#00203b",
		"cloud": "#00365d", "cloudSh": "#002b4c",
		"storm": "#31485e", "stormDk": "#233648", "stormHi": "#385c70", "rain": "#00aded",
		"flower": "#a1cee9", "spark": "#00aded", "sparkHi": "#ffffff",
	},
}

SPRITE_PALETTE = {
	"o": "#003359", "a": "#007aa9", "l": "#35b6f2", "d": "#006085", "w": "#e7f1fa",
	"n": "#eef1f6", "m": "#c7d0d8", "k": "#8c9aa7", "g": "#00a2de",
}

# Горы: (вершина x, вершина y). Склон — 3 пикселя вбок на 2 вниз, левый склон светлее, сверху снег.
MOUNTAINS = [(12, 15), (36, 19), (60, 14), (84, 11), (104, 16)]
# Холмы среднего плана: ступени (x начала, верх); None — холма нет.
HILLS = [(0, 27), (6, 26), (12, 27), (17, 29), (22, 31), (26, 33), (30, None), (78, 33), (82, 31), (86, 29), (91, 27), (98, 26), (104, 28)]
# Деревья: (вид, x, земля). Вид — ключ TREE_SHAPES.
TREES = [("oak", 3, 26), ("pine", 13, 27), ("bush", 31, 36), ("oak", 92, 26), ("pine", 85, 29), ("bush", 75, 36)]
TREE_SHAPES = {
	"oak": [
		"..LLLM...",
		".LLMMMMD.",
		"LLMMLMMMD",
		"LMMMMMMDD",
		"MLMMMDMDD",
		".MDMMDDD.",
		"...TU....",
		"...TU....",
		"...TU....",
	],
	"pine": [
		"..L...",
		".LMD..",
		"..MD..",
		".LMMD.",
		"LMMMDD",
		"..TU..",
		"..TU..",
	],
	"bush": [
		".LLM.",
		"LMMMD",
		"MMDDD",
	],
}
FLOWERS = [37, 40, 66, 70]
TUFTS = [24, 34, 44, 63, 73, 80, 89]
TOWER_X = 35
ORB = (74, 3)
ORB_ROWS = ["..xxxx..", ".xxxxxx.", "xxxxxxxx", "xxxxxxxx", "xxxxxxxx", "xxxxxxxx", ".xxxxxx.", "..xxxx.."]
BIRDS = [(20, 11), (25, 8)]
STARS = [(3, 18), (9, 3), (16, 9), (28, 5), (33, 14), (45, 2), (52, 9), (64, 4), (68, 15), (86, 15), (90, 2), (102, 16), (107, 6), (58, 17), (39, 11)]
STARS_BIG = [(14, 15), (60, 12), (95, 9)]
STARS_TWINKLE = [(6, 9), (24, 13), (42, 6), (55, 3), (71, 11), (87, 8), (104, 12), (35, 17), (12, 6)]

CLOUD_L = [
	".....xxxx.........",
	"...xxxxxxxx.xxx...",
	"..xxxxxxxxxxxxxxx.",
	"xxxxxxxxxxxxxxxxxx",
	"ssssssssssssssssss",
]
CLOUD_S = [
	"..xxx....",
	".xxxxxxx.",
	"xxxxxxxxx",
	"sssssssss",
]
CLOUD_M = [
	"....xxx.....",
	"..xxxxxxxx..",
	"xxxxxxxxxxxx",
	"ssssssssssss",
]
CLOUDS = [(CLOUD_L, 4, 6), (CLOUD_S, 46, 2), (CLOUD_M, 84, 14)]

STORM = [
	".......hhhh.........",
	"...hhhhxxxxhh.hhh...",
	".hhxxxxxxxxxxhxxxh..",
	"xxxxxxxxxxxxxxxxxxxx",
	"xxxxxxxxxxxxxxxxxxxx",
	"ssssssssssssssssssss",
]
STORM_POS = (45, 7)
RAIN_COLS = [47, 50, 53, 56, 59, 62]

SPARKS_A = [(41, 22), (39, 31), (67, 13), (52, 14)]
SPARKS_B = [(69, 24), (71, 31), (44, 13), (60, 16)]
SHOOTING_STAR = [(36, 8, "sparkHi"), (37, 8, "sparkHi"), (38, 7, "spark"), (39, 7, "spark"), (40, 6, "glow"), (41, 6, "glow"), (42, 5, "glow")]

SPRITES = {
	"choose": [
		"......oooo......",
		".....owllao.....",
		"......oaao......",
		".......oo.......",
		"..oooooooooooo..",
		".olllllllllllao.",
		".olaaaaaaaaaado.",
		".olaaaaaaaaaado.",
		".olawoaaaawoado.",
		".olaooaaaaooado.",
		".olaaaaaaaaaado.",
		".olaaaaddaaaado.",
		".olaaaaaaaaaado.",
		".oadddddddddddo.",
		"..oooooooooooo..",
		"................",
	],
	"success": [
		"......oooo......",
		".....owllao.....",
		"......oaao......",
		".......oo.......",
		"..oooooooooooo..",
		".olllllllllllao.",
		".olaaaaaaaaaado.",
		".olaaaaaaaaaado.",
		".olaooaaaaooado.",
		".oloaaoaaoaaodo.",
		".olaaoaaaaoaado.",
		".olaaaooooaaado.",
		".olaaaaaaaaaado.",
		".oadddddddddddo.",
		"..oooooooooooo..",
		"................",
	],
	"microsoft": [
		"................",
		"................",
		"................",
		"................",
		"..oooo..........",
		".ollllo.........",
		"olwooaaooooooo..",
		"olo..oaallllllo.",
		"oao..oaddddddddo",
		"oaaooadoooddoddo",
		".oddddo...oo.oo.",
		"..oooo..........",
		"................",
		"................",
		"................",
		"................",
	],
	"offline": [
		"................",
		".oooooooooooooo.",
		"onnnnnnnnnnnnnmo",
		"ononnoonoonannmo",
		"ononnonnonnannmo",
		"onoonoonoonannmo",
		"ommmmmmmmmmmmmmo",
		".oooooooooooooo.",
		"......omko......",
		"......omko......",
		"......omko......",
		"......omko......",
		"......omko......",
		"......omko......",
		"......omko......",
		"....g.omkog.....",
	],
	"error": [
		"................",
		".oooooooooooooo.",
		".onnnnnnonnnnko.",
		".onmmmmmommmmko.",
		".onmmmmonmmmmko.",
		".onmmmmommmmmko.",
		".onmmmoonmmmmko.",
		".onmmmmoommmmko.",
		".onmmmmmonmmmko.",
		".onmmmmmoommmko.",
		".onmmmmmmonmmko.",
		".onmmmmmmommmko.",
		".onmmmmmmmmmmko.",
		".okkkkkkkkkkkko.",
		".oooooooooooooo.",
		"................",
	],
}
SPRITE_FRAMES_B = {
	"offline": {(11, 3): "n", (11, 4): "n", (11, 5): "n"},
	"microsoft": {},
	"success": {},
}
MS_SPARK = [(13, 1, "l"), (12, 2, "l"), (13, 2, "w"), (14, 2, "l"), (13, 3, "l")]
SUCCESS_SPARKS_A = [(1, 0, "l"), (0, 1, "l"), (1, 1, "w"), (2, 1, "l"), (1, 2, "l")]
SUCCESS_SPARKS_B = [(13, 1, "l"), (12, 2, "l"), (13, 2, "w"), (14, 2, "l"), (13, 3, "l")]
STANDING = {"choose", "success", "offline", "error"}


def rgb(h):
	h = h.lstrip("#")
	return (int(h[0:2], 16), int(h[2:4], 16), int(h[4:6], 16), 255)


def pal(theme):
	return {k: rgb(v) for k, v in PALETTES[theme].items()}


def canvas():
	return Image.new("RGBA", (W, H), (0, 0, 0, 0))


def put(im, x, y, c):
	if 0 <= x < W and 0 <= y < H:
		im.putpixel((x, y), c)


def runs(spec, end):
	for i, (x, top) in enumerate(spec):
		nx = spec[i + 1][0] if i + 1 < len(spec) else end
		yield x, nx, top


def stamp(im, rows, ox, oy, colors, wrap=False):
	for y, row in enumerate(rows):
		for x, ch in enumerate(row):
			if ch in colors:
				put(im, (ox + x) % W if wrap else ox + x, oy + y, colors[ch])


def plus(im, x, y, arm, center):
	for dx, dy in ((0, -1), (-1, 0), (1, 0), (0, 1)):
		put(im, x + dx, y + dy, arm)
	put(im, x, y, center)


def in_orb(x, y):
	return 0 <= y < 8 and 0 <= x < 8 and ORB_ROWS[y][x] == "x"


def scene(theme):
	p = pal(theme)
	im = canvas()
	for y in range(H):
		for x in range(W):
			if y < 8:
				c = p["sky3"]
			elif y < 10:
				c = p["sky2"] if (x + y) % 2 == 0 or (y == 9 and x % 4 == 1) else p["sky3"]
			elif y < 16:
				c = p["sky2"]
			elif y < 18:
				c = p["sky1"] if (x + y) % 2 == 0 or (y == 17 and x % 4 == 1) else p["sky2"]
			else:
				c = p["sky1"]
			im.putpixel((x, y), c)

	if theme == "night":
		for x, y in STARS:
			put(im, x, y, p["starDim"])
		for x, y in STARS_BIG:
			plus(im, x, y, p["starDim"], p["star"])
	else:
		for x, y in BIRDS:
			for dx, dy in ((0, 0), (1, 1), (2, 0), (3, 1), (4, 0)):
				if dx != 2:
					put(im, x + dx, y + dy, p["bird"])

	ox, oy = ORB
	for y in range(-1, 9):
		for x in range(-1, 9):
			if not in_orb(x, y) and any(in_orb(x + dx, y + dy) for dx, dy in ((0, 1), (0, -1), (1, 0), (-1, 0))):
				put(im, ox + x, oy + y, p["orbRing"])
	stamp(im, ORB_ROWS, ox, oy, {"x": p["orb"]})
	if theme == "day":
		hi, spots = ((2, 1), (3, 1), (1, 2), (1, 3)), ((6, 5), (5, 6), (6, 6), (4, 6))
	else:
		hi, spots = ((2, 1), (1, 2)), ((2, 2), (3, 2), (2, 3), (5, 4), (5, 5), (3, 6))
	for x, y in hi:
		put(im, ox + x, oy + y, p["orbHi"])
	for x, y in spots:
		put(im, ox + x, oy + y, p["orbSpot"])

	tops = [GROUND] * W
	for cx, cy in MOUNTAINS:
		for x in range(W):
			d = abs(x - cx)
			top = cy + (max(0, d - 1) * 2 + 2) // 3
			if top >= GROUND:
				continue
			tops[x] = min(tops[x], top)
			for y in range(top, GROUND):
				if y - cy < 3 and y - top < 2 and d <= 3:
					c = p["snow"]
				elif x <= cx:
					c = p["farLit"]
				else:
					c = p["far"]
				put(im, x, y, c)

	tx = TOWER_X
	base = tops[tx + 1] + 1
	if theme == "night":
		for y in range(0, base - 8):
			put(im, tx + 1, y, p["beam"])
			put(im, tx, y, p["glow"])
			put(im, tx + 2, y, p["glow"])
	else:
		put(im, tx + 2, base - 9, p["flag"])
		put(im, tx + 3, base - 9, p["flag"])
		put(im, tx + 2, base - 8, p["flag"])
		put(im, tx + 1, base - 9, p["towerDk"])
	put(im, tx + 1, base - 8, p["lamp"])
	put(im, tx, base - 7, p["lamp"])
	put(im, tx + 1, base - 7, p["lamp"])
	put(im, tx + 2, base - 7, p["lamp"])
	for x in range(tx - 1, tx + 4):
		put(im, x, base - 6, p["towerDk"])
	for y in range(base - 5, base):
		put(im, tx, y, p["tower"])
		put(im, tx + 1, y, p["tower"])
		put(im, tx + 2, y, p["towerDk"])

	for a, b, top in runs(HILLS, W):
		if top is None:
			continue
		for x in range(a, b):
			for y in range(top, GROUND):
				if y == top:
					c = p["midHi"]
				elif y == top + 1:
					c = p["midHi"] if x % 3 == 0 else p["midLit"]
				elif y == top + 2 and x % 3 == 1:
					c = p["midLit"]
				elif x % 4 == 1 and (y - top) % 4 == 0:
					c = p["midDk"]
				else:
					c = p["mid"]
				put(im, x, y, c)

	for kind, x0, ground in TREES:
		rows = TREE_SHAPES[kind]
		stamp(im, rows, x0, ground - len(rows), {"L": p["leafHi"], "M": p["leaf"], "D": p["leafDk"], "T": p["trunk"], "U": p["trunkDk"]})

	for x in range(W):
		put(im, x, GROUND, p["grassHi"])
		put(im, x, GROUND + 1, p["grass"])
		put(im, x, GROUND + 2, p["grass"] if x % 5 in (0, 1, 3) else p["dirt"])
		for y in range(GROUND + 3, H):
			put(im, x, y, p["dirtDk"] if x % 8 == 7 or y == H - 1 else p["dirt"])
	for x, y in ((3, 39), (12, 40), (19, 39), (27, 40), (34, 39), (45, 40), (50, 39), (61, 40), (66, 39), (77, 40), (83, 39), (90, 40), (98, 39), (105, 40)):
		put(im, x, y, p["dirtSp"])
	for x in TUFTS:
		put(im, x, GROUND - 1, p["grassHi"])
	for x in FLOWERS:
		put(im, x, GROUND - 1, p["grassHi"])
		put(im, x, GROUND - 2, p["flower"])
	return im


def clouds(theme):
	p = pal(theme)
	im = canvas()
	for rows, x, y in CLOUDS:
		stamp(im, rows, x, y, {"x": p["cloud"], "s": p["cloudSh"]}, wrap=True)
	return im


def twinkle():
	p = pal("night")
	im = canvas()
	for x, y in STARS_TWINKLE:
		put(im, x, y, p["star"])
	return im


def storm(theme):
	p = pal(theme)
	im = canvas()
	stamp(im, STORM, *STORM_POS, {"x": p["storm"], "s": p["stormDk"], "h": p["stormHi"]})
	return im


def rain(theme, frame):
	p = pal(theme)
	im = canvas()
	top = STORM_POS[1] + len(STORM)
	for i, x in enumerate(RAIN_COLS):
		start = top + 1 + (i % 2 * 2 + frame * 2) % 4
		for y in range(start, SPRITE_POS[1] + 1, 4):
			put(im, x, y, p["rain"])
			put(im, x, y + 1, p["rain"])
	return im


def sparkles(theme, frame):
	p = pal(theme)
	im = canvas()
	for x, y in SPARKS_A if frame == 0 else SPARKS_B:
		plus(im, x, y, p["spark"], p["sparkHi"])
	if theme == "night" and frame == 0:
		for x, y, k in SHOOTING_STAR:
			put(im, x, y, p[k])
	return im


def sprite(name, frame=0):
	colors = {k: rgb(v) for k, v in SPRITE_PALETTE.items()}
	rows = [list(r) for r in SPRITES[name]]
	if frame == 1:
		for (x, y), ch in SPRITE_FRAMES_B.get(name, {}).items():
			rows[y][x] = ch
	extra = []
	if name == "microsoft" and frame == 0:
		extra = MS_SPARK
	elif name == "success":
		extra = SUCCESS_SPARKS_A if frame == 0 else SUCCESS_SPARKS_B
	for x, y, ch in extra:
		rows[y][x] = ch
	if name in STANDING:
		while all(c == "." for c in rows[-1]):
			rows = [["."] * 16] + rows[:-1]
	im = Image.new("RGBA", (16, 16), (0, 0, 0, 0))
	for y, row in enumerate(rows):
		for x, ch in enumerate(row):
			if ch != ".":
				im.putpixel((x, y), colors[ch])
	return im


def build_all():
	out = {}
	for t in ("day", "night"):
		out["scene-%s" % t] = scene(t)
		out["clouds-%s" % t] = clouds(t)
		out["storm-%s" % t] = storm(t)
		out["rain-%s-a" % t] = rain(t, 0)
		out["rain-%s-b" % t] = rain(t, 1)
		out["sparkle-%s-a" % t] = sparkles(t, 0)
		out["sparkle-%s-b" % t] = sparkles(t, 1)
	out["stars-night"] = twinkle()
	for n in SPRITES:
		out["sprite-%s" % n] = sprite(n)
		if n in SPRITE_FRAMES_B:
			out["sprite-%s-b" % n] = sprite(n, 1)
	out.update(empty_sprite.build({k: rgb(v) for k, v in SPRITE_PALETTE.items()}, Image))
	out.update(covers.build(Image))
	out.update(skins.build(Image))
	return out


if __name__ == "__main__":
	root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
	dst = os.path.join(root, "src", "assets", "pixel")
	os.makedirs(dst, exist_ok=True)
	for name, im in build_all().items():
		im.save(os.path.join(dst, name + ".png"), optimize=True)
	print("saved to", dst)
