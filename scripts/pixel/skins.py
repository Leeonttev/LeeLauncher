# Манекен (развёртка 64x64) и сцена подиума (110x400, рисуется x4): день для светлой темы, ночь для тёмной.
import random

# (x0, y0, ширина, высота, глубина) коробки на развёртке
PARTS = {
	"head": (0, 0, 8, 8, 8),
	"body": (16, 16, 8, 12, 4),
	"rarm": (40, 16, 4, 12, 4),
	"larm": (32, 48, 4, 12, 4),
	"rleg": (0, 16, 4, 12, 4),
	"lleg": (16, 48, 4, 12, 4),
}

MANNEQUIN = {"a": "#c8d0d8", "b": "#b4bdc5", "c": "#9aa4af", "d": "#7b8793", "mark": "#32ade6", "eye": "#5c6873"}
# Знак на груди, координаты передней грани тела.
PLAY_MARK = {(3, 2), (3, 3), (4, 3), (3, 4), (4, 4), (5, 4), (3, 5), (4, 5), (3, 6)}

STAGE_W = 110
STAGE_H = 400
SKY_ROWS = 118  # полосы неба занимают эти строки нижних 160 строк сцены
LOWER = STAGE_H - 160
HILLS_FAR = [116, 115, 113, 112, 110, 108, 108, 107, 106, 106, 107, 108, 110, 112, 113, 114, 114, 113, 111, 109, 108, 106, 105, 104, 104, 105, 107, 109, 111, 113, 114]
HILLS_NEAR = [122, 122, 121, 120, 120, 119, 118, 118, 118, 119, 120, 121, 122, 122, 123, 123, 122, 121, 120, 119, 118, 118, 119, 120, 121, 122]
PLATES_TOP = 128  # верх плит: 32 строки от низа, x4 = 128 px

STAGES = {
	"stage-day": {
		"sky": ["#1bace9", "#5bbcee", "#82ccf6", "#a8dbf9"],
		"far": "#73bde6",
		"near": "#4fb5e8",
		"plate_top": "#eaf3fb",
		"plate": "#b3dcf4",
		"grout": "#92cced",
		"disc": ("#ffffff", "#d7e8f8"),
		"stars": None,
		"spots": [],
	},
	"stage-night": {
		"sky": ["#00162c", "#00203b", "#002a4c", "#00365f"],
		"far": "#004373",
		"near": "#00355d",
		"plate_top": "#007dac",
		"plate": "#00487a",
		"grout": "#00365f",
		"disc": ("#dde9f5", "#b7daef"),
		"stars": ["#56a3cc", "#b7daef", "#ffffff"],
		"spots": [(69, 279), (73, 283), (72, 279)],
	},
}


def _rgb(s):
	s = s.lstrip("#")
	return (int(s[0:2], 16), int(s[2:4], 16), int(s[4:6], 16))


def _faces(x0, y0, w, h, d):
	yield "top", x0 + d, y0, w, d
	yield "bottom", x0 + d + w, y0, w, d
	yield "right", x0, y0 + d, d, h
	yield "front", x0 + d, y0 + d, w, h
	yield "left", x0 + d + w, y0 + d, d, h
	yield "back", x0 + d + w + d, y0 + d, w, h


def _block(face, u, v, h):
	if face in ("top", "bottom"):
		return "b"
	if v == 0:
		return "a"
	if v == h - 1:
		return "c"
	return "a" if (u + v) % 7 else "b"


def _head(face, u, v, w, h):
	if face == "front" and (u, v) in {(2, 4), (5, 4)}:
		return "eye"
	if face == "front" and v == 6 and 3 <= u <= 4:
		return "c"
	if face == "top":
		return "a"
	return _block(face, u, v, h)


def _body(face, u, v, w, h):
	if face == "front" and (u, v) in PLAY_MARK:
		return "mark"
	if v == 8:
		return "d"
	if face in ("front", "back") and u in (0, w - 1) and v < 8:
		return "b"
	return _block(face, u, v, h)


def _limb(face, u, v, w, h):
	if v in (4, 5) and face not in ("top", "bottom"):
		return "d" if v == 4 else "c"
	return _block(face, u, v, h)


def mannequin(Image):
	im = Image.new("RGBA", (64, 64), (0, 0, 0, 0))
	px = im.load()
	painters = {"head": _head, "body": _body, "rarm": _limb, "larm": _limb, "rleg": _limb, "lleg": _limb}
	for part, paint in painters.items():
		for face, fx, fy, fw, fh in _faces(*PARTS[part]):
			for v in range(fh):
				for u in range(fw):
					px[fx + u, fy + v] = _rgb(MANNEQUIN[paint(face, u, v, fw, fh)]) + (255,)
	return im


def stage(Image, spec):
	im = Image.new("RGB", (STAGE_W, STAGE_H), _rgb(spec["sky"][0]))
	px = im.load()
	sky = spec["sky"]
	n = len(sky)
	for y in range(160):
		k = min(n - 1, y * n // SKY_ROWS)
		for x in range(STAGE_W):
			c = sky[k]
			if k + 1 < n and (y * n) % SKY_ROWS > SKY_ROWS - n * 3 and (x + y) % 2 == 0:
				c = sky[k + 1]
			px[x, LOWER + y] = _rgb(c)
	if spec["stars"]:
		rnd = random.Random(42)
		for _ in range(120):
			x, y = rnd.randrange(STAGE_W), rnd.randrange(0, LOWER + 96)
			px[x, y] = _rgb(rnd.choice(spec["stars"]))
	# Солнце или луна левее кнопок подиума: под них не попадает при любой высоте.
	inner, outer = spec["disc"]
	cx, cy = 71, 281
	for y in range(cy - 6, cy + 7):
		for x in range(cx - 6, cx + 7):
			d = (x - cx) ** 2 + (y - cy) ** 2
			if d <= 30:
				px[x, y] = _rgb(inner if d < 18 else outer)
	for x, y in spec["spots"]:
		px[x, y] = _rgb(outer)
	for x in range(STAGE_W):
		far = HILLS_FAR[(x // 4) % len(HILLS_FAR)]
		near = HILLS_NEAR[(x // 4 + 5) % len(HILLS_NEAR)]
		for y in range(far, 160):
			px[x, LOWER + y] = _rgb(spec["far"])
		for y in range(near, 160):
			px[x, LOWER + y] = _rgb(spec["near"])
	for y in range(PLATES_TOP, 160):
		dy = y - PLATES_TOP
		for x in range(STAGE_W):
			if dy < 2:
				c = spec["plate_top"]
			elif dy == 2:
				c = spec["plate"]
			else:
				row = (dy - 3) // 7
				grout = (dy - 3) % 7 == 6 or (x + (row % 2) * 7) % 14 == 13
				c = spec["grout"] if grout else spec["plate"]
			px[x, LOWER + y] = _rgb(c)
	return im


def build(Image):
	out = {"mannequin": mannequin(Image)}
	for name, spec in STAGES.items():
		out[name] = stage(Image, spec)
	return out