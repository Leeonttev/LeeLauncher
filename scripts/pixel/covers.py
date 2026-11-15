# Обложки сборок: карточка (CARD, x4 = 400x160) и баннер (-wide, 640x60). Два слоя на размер: back и front, интерфейс сдвигает их для параллакса.
import math
import random

CARD = (100, 40)
BANNER = (420, 60)
EDGE = 110

# Порядок = порядок в выборе обложки.
ORDER = ["meadow", "forest", "peaks", "ocean", "desert", "sakura", "village", "castle", "caves", "volcano", "cosmos"]

CLEAR = (0, 0, 0, 0)


def rgb(h):
	h = h.lstrip("#")
	return (int(h[0:2], 16), int(h[2:4], 16), int(h[4:6], 16), 255)


def P(d):
	return {k: rgb(v) for k, v in d.items()}


class Canvas:
	"""w — ширина композиции, pad — поля по бокам: x от -pad до w + pad - 1."""

	def __init__(self, Image, w, h, pad=0):
		self.im = Image.new("RGBA", (w + 2 * pad, h), CLEAR)
		self.w, self.h, self.pad = w, h, pad
		self.x0, self.x1 = -pad, w + pad

	def xs(self):
		return range(self.x0, self.x1)

	def put(self, x, y, c):
		if self.x0 <= x < self.x1 and 0 <= y < self.h:
			self.im.putpixel((x + self.pad, y), c)

	def get(self, x, y):
		if self.x0 <= x < self.x1 and 0 <= y < self.h:
			return self.im.getpixel((x + self.pad, y))
		return CLEAR

	def stamp(self, rows, ox, oy, colors):
		for y, row in enumerate(rows):
			for x, ch in enumerate(row):
				if ch in colors:
					self.put(ox + x, oy + y, colors[ch])


# ---------- общие приёмы ----------

def sky(cv, stops, top=0, bottom=None):
	"""Полосы сверху вниз; на стыке две строки перехода (шахматка и редкие точки)."""
	bottom = cv.h if bottom is None else bottom
	n = len(stops)
	band = (bottom - top) / n
	for y in range(top, bottom):
		i = min(n - 1, int((y - top) / band))
		edge = top + int(round((i + 1) * band))
		for x in cv.xs():
			c = stops[i]
			if i + 1 < n:
				if y == edge - 1 and (x + y) % 2 == 0:
					c = stops[i + 1]
				elif y == edge - 2 and x % 4 == (y % 2) * 2:
					c = stops[i + 1]
			cv.put(x, y, c)


def disc(cv, cx, cy, r, c, ring=None, hi=None):
	for y in range(-r - 1, r + 1):
		for x in range(-r - 1, r + 1):
			d = (x + 0.5) ** 2 + (y + 0.5) ** 2
			if d <= r * r:
				cv.put(cx + x, cy + y, c)
			elif ring and d <= (r + 1) ** 2:
				cv.put(cx + x, cy + y, ring)
	if hi and r > 3:
		for x, y in ((1, -2), (2, 0), (-2, 1), (0, 2), (1, 1)):
			cv.put(cx + x, cy + y, hi)


def peak(cv, cx, cy, base, lit, shade, snow=None, slope=(2, 3), snow_rows=3):
	"""Гора: на slope[1] пикселей вбок спускается на slope[0]; левый склон светлее, сверху снег."""
	a, b = slope
	reach = (base - cy) * b // a + 3
	for x in range(cx - reach, cx + reach + 1):
		d = abs(x - cx)
		top = cy + (max(0, d - 1) * a + b - 1) // b
		if top >= base:
			continue
		for y in range(top, base):
			c = lit if x <= cx else shade
			if snow:
				if y - cy < snow_rows and y - top < 2:
					c = snow
				elif y - cy == snow_rows and y - top < 2 and x % 2 == 0:
					c = snow
			cv.put(x, y, c)


class Line:
	"""Линия рельефа: высота для любого x, в том числе в полях."""

	def __init__(self, f):
		self.f = f

	def __getitem__(self, x):
		return self.f(x)


def wave(rng, w, base, amp, period):
	p1, p2 = rng.random() * 6.28, rng.random() * 6.28

	def f(x):
		v = 0.65 * math.sin(x * 6.283 / period + p1) + 0.35 * math.sin(x * 6.283 / (period * 0.43) + p2)
		return int(round(base - amp * (0.5 + 0.5 * v)))

	return Line(f)


def hills(cv, line, fill, hi, dk=None, pattern=None):
	for x in cv.xs():
		top = line[x]
		for y in range(top, cv.h):
			c = fill
			if y == top:
				c = hi
			elif y == top + 1 and x % 3 == 0:
				c = hi
			elif dk and pattern and pattern(x, y - top):
				c = dk
			cv.put(x, y, c)


def spots(rng, w, n, avoid=None, margin=3, gap=6):
	xs = []
	tries = 0
	while len(xs) < n and tries < 4000:
		tries += 1
		x = rng.randrange(margin, w - margin)
		if avoid and avoid[0] <= x <= avoid[1]:
			continue
		if all(abs(x - o) >= gap for o in xs):
			xs.append(x)
	return sorted(xs)


def pine(cv, x, base, h, lit, mid, dark, trunk=None, snow=None):
	"""Ель высотой h: ярусы, левый край светлый, правый тёмный."""
	trunk_h = 1 if h < 9 else 2
	body = h - trunk_h
	top = base - h
	tier = max(3, body // max(2, h // 4))
	for y in range(body):
		k = y % tier
		half = max(0, int(round(y * 0.42)) - (tier - 1 - k) // 2) if y else 0
		half = min(half, 1 + y // 2)
		for dx in range(-half, half + 1):
			c = mid
			if dx == -half and half:
				c = lit
			elif dx == half and half:
				c = dark
			elif dx > 0 and (y + dx) % 3 == 0:
				c = dark
			if snow and k == 0 and dx < half:
				c = snow
			cv.put(x + dx, top + y, c)
	for y in range(trunk_h):
		cv.put(x, base - trunk_h + y, trunk or dark)


ROUND = [
	"..LLLM...",
	".LLMMMMD.",
	"LLMMLMMMD",
	"LMMMMMMDD",
	"MLMMMDMDD",
	".MDMMDDD.",
	"...TU....",
	"...TU....",
]
ROUND_BIG = [
	"....LLLLM.....",
	"..LLLMMMMMD...",
	".LLMMMLMMMMDD.",
	"LLMMMMMMMMMMDD",
	"LMMLMMMMMMDMDD",
	"MMMMMMMDMMMDDD",
	"MLMMMMMMMDMDDD",
	".MMDMMMMDDDDD.",
	"..MDDMDDDDDD..",
	".....TTU......",
	".....TTU......",
	".....TTU......",
	"....TTTUU.....",
]
BUSH = [
	".LLM.",
	"LMMMD",
	"MMDDD",
]
PALM = [
	"..LLLM.MMMD..",
	".LL..LMMD.DD.",
	"L....MLMD...D",
	"....M.T.D....",
	"......T......",
	".......T.....",
	".......T.....",
	"......T......",
	"......T......",
	".....TT......",
]
CACTUS = [
	"..LM..",
	"..LM..",
	"L.LM..",
	"L.LM.D",
	"LLLM.D",
	"..LMDD",
	"..LM..",
	"..LM..",
]


def tree(cv, rows, x, base, pal):
	cv.stamp(rows, x - len(rows[0]) // 2, base - len(rows), pal)


def cloud(cv, x, y, w, c, sh):
	a = w // 3
	rows = [
		"." * a + "x" * (w // 4) + "." * (w - a - w // 4),
		"." * (w // 6) + "x" * (w - w // 6 - 2) + "..",
		"x" * w,
		"s" * w,
	]
	cv.stamp(rows, x, y, {"x": c, "s": sh})


def ground(cv, y, top, mid, dark):
	for x in cv.xs():
		cv.put(x, y, top)
		cv.put(x, y + 1, mid if x % 5 in (0, 1, 3) else top)
		for yy in range(y + 2, cv.h):
			cv.put(x, yy, dark if (x % 7 == 3 and yy % 3 == 0) or yy == cv.h - 1 else mid)


def stars(cv, rng, n, c, dim, ymax, big=0):
	for _ in range(n):
		cv.put(rng.randrange(cv.w), rng.randrange(ymax), dim if rng.random() < 0.6 else c)
	for _ in range(big):
		x, y = rng.randrange(2, cv.w - 2), rng.randrange(2, ymax)
		for dx, dy in ((0, -1), (-1, 0), (1, 0), (0, 1)):
			cv.put(x + dx, y + dy, dim)
		cv.put(x, y, c)


def layers(Image, size, pad=0):
	w, h = size
	return Canvas(Image, w, h, pad), Canvas(Image, w, h, pad)


def clouds_row(cv, rng, p, ymax):
	for x in spots(rng, cv.w, max(2, cv.w // 40), gap=30, margin=0):
		cloud(cv, x - 8, rng.randrange(2, max(3, ymax)), rng.choice([12, 16, 20]), p["cl"], p["clS"])


# Поля баннера: детали берут числа из своего генератора, чтобы середина сцены не менялась.

def edge_rng(name):
	return random.Random(name + "-edge")


def per_side(cv, n):
	"""Сколько деталей в одном поле при n деталях на всю композицию."""
	return int(round(n * cv.pad / cv.w))


def edge_spots(rng, cv, n, gap=6, margin=0, inner=0):
	"""По n точек в каждом поле; inner — отступ от середины."""
	if not cv.pad:
		return []
	out = []
	for lo, hi in ((cv.x0 + margin, -inner), (cv.w + inner, cv.x1 - margin)):
		xs = []
		tries = 0
		while len(xs) < n and tries < 4000:
			tries += 1
			x = rng.randrange(lo, hi)
			if all(abs(x - o) >= gap for o in xs):
				xs.append(x)
		out += xs
	return sorted(out)


def continue_row(cv, start, stop, step):
	"""Продолжение ряда range(start, stop, step) в оба поля."""
	if not cv.pad:
		return []
	left = list(range(start - step, cv.x0 - step, -step))
	last = start + (stop - 1 - start) // step * step
	return left + list(range(last + step, cv.x1 + step, step))


def edge_xs(cv):
	return [x for x in cv.xs() if x < 0 or x >= cv.w]


def edge_stars(cv, rng, n, c, dim, ymax, big=0):
	"""Звёзды в полях: n и big — на всю композицию, как в stars."""
	xs = edge_xs(cv)
	if not xs:
		return
	for _ in range(per_side(cv, n) * 2):
		cv.put(rng.choice(xs), rng.randrange(ymax), dim if rng.random() < 0.6 else c)
	for x in edge_spots(rng, cv, per_side(cv, big), gap=8, margin=2):
		y = rng.randrange(2, ymax)
		for dx, dy in ((0, -1), (-1, 0), (1, 0), (0, 1)):
			cv.put(x + dx, y + dy, dim)
		cv.put(x, y, c)


def edge_clouds(cv, rng, p, ymax):
	for x in edge_spots(rng, cv, per_side(cv, max(2, cv.w // 40)), gap=30):
		cloud(cv, x - 8, rng.randrange(2, max(3, ymax)), rng.choice([12, 16, 20]), p["cl"], p["clS"])


def edge_dots(cv, rng, n, ylo, yhi, colors):
	"""Россыпь точек в полях (лепестки, снежинки, блики): n — на всю композицию."""
	xs = edge_xs(cv)
	if not xs:
		return
	for _ in range(per_side(cv, n) * 2):
		cv.put(rng.choice(xs), rng.randrange(ylo, yhi), rng.choice(colors))


# Каждая сцена: (Image, size) -> (back, front). Главный предмет — по центру ширины.

def meadow(Image, size, pad=0):
	p = P({
		"s1": "#6ec0ff", "s2": "#9bd6ff", "s3": "#c8ecff", "s4": "#eef9e4",
		"sun": "#fff6c2", "sunR": "#fffbe0", "cl": "#ffffff", "clS": "#d9f0ff",
		"far": "#8fc7c9", "farD": "#79b4ba", "snow": "#f4fbff",
		"h1": "#7cc96a", "h1H": "#9be07f", "h1D": "#68b45a",
		"h2": "#4fae4f", "h2H": "#72c85c", "h2D": "#3f9645",
		"lL": "#6fcf55", "lM": "#3d9a3f", "lD": "#2b7a37", "tr": "#7a4f2e", "trD": "#5b3a22",
		"g": "#4caa48", "gH": "#86dd63", "gD": "#3a8a3c",
		"f1": "#ffffff", "f2": "#ffd84a", "f3": "#ff7aa8",
	})
	w, h = size
	rng = random.Random("meadow%d" % w)
	er = edge_rng("meadow")
	back, front = layers(Image, size, pad)
	sky(back, [p["s1"], p["s2"], p["s3"], p["s4"]])
	disc(back, int(w * 0.8), h // 5, 4, p["sun"], ring=p["sunR"])
	clouds_row(back, rng, p, h // 4)
	edge_clouds(back, er, p, h // 4)
	gy = h - 4
	for x in spots(rng, w, max(3, w // 22), gap=16, margin=0):
		peak(back, x, int(h * 0.36) + rng.randrange(0, 5), gy, p["far"], p["farD"], snow=p["snow"])
	for x in edge_spots(er, back, per_side(back, max(3, w // 22)), gap=16):
		peak(back, x, int(h * 0.36) + er.randrange(0, 5), gy, p["far"], p["farD"], snow=p["snow"])
	line = wave(rng, w, int(h * 0.64), 5, 70)
	hills(front, line, p["h1"], p["h1H"], p["h1D"], pattern=lambda x, d: x % 6 == 2 and d % 4 == 3)
	line2 = wave(rng, w, int(h * 0.8), 3, 45)
	hills(front, line2, p["h2"], p["h2H"], p["h2D"], pattern=lambda x, d: x % 5 == 1 and d % 3 == 2)
	pal = {"L": p["lL"], "M": p["lM"], "D": p["lD"], "T": p["tr"], "U": p["trD"]}
	c = w // 2
	for x in spots(rng, w, w // 16, avoid=(c - 13, c + 13), gap=9):
		if rng.random() < 0.5:
			tree(front, rng.choice([ROUND, BUSH]), x, line[x] + 2, pal)
	for x in spots(rng, w, w // 18, avoid=(c - 13, c + 13), gap=11):
		tree(front, rng.choice([ROUND, BUSH, ROUND]), x, line2[x] + 2, pal)
	for x in edge_spots(er, front, per_side(front, w // 16), gap=9, inner=4):
		if er.random() < 0.5:
			tree(front, er.choice([ROUND, BUSH]), x, line[x] + 2, pal)
	for x in edge_spots(er, front, per_side(front, w // 18), gap=11, inner=6):
		tree(front, er.choice([ROUND, BUSH, ROUND]), x, line2[x] + 2, pal)
	if pad:
		tree(front, ROUND_BIG, -64, line2[-64] + 3, pal)
		tree(front, ROUND_BIG, w + 58, line2[w + 58] + 3, pal)
		tree(front, BUSH, w + 66, line2[w + 66] + 2, pal)
	ground(front, h - 4, p["gH"], p["g"], p["gD"])
	tree(front, ROUND_BIG, c, h - 3, pal)
	if w > 200:
		tree(front, ROUND_BIG, c - 34, line2[c - 34] + 3, pal)
		tree(front, ROUND, c + 30, line2[c + 30] + 2, pal)
	for x in range(1, w, 2):
		if rng.random() < 0.35 and abs(x - c) > 4:
			y = rng.randrange(line2[x] + 2, h - 1)
			front.put(x, y, rng.choice([p["f1"], p["f2"], p["f3"]]))
	for x in edge_xs(front):
		if x % 2 and er.random() < 0.35:
			front.put(x, er.randrange(line2[x] + 2, h - 1), er.choice([p["f1"], p["f2"], p["f3"]]))
	return back, front


def forest(Image, size, pad=0):
	p = P({
		"s1": "#2d5a6e", "s2": "#4e8a86", "s3": "#9bc49a", "s4": "#f2d9a0",
		"sun": "#fff1c4", "sunR": "#ffe6a0",
		"m1": "#4a8a74", "m1D": "#3f7a68", "mtn": "#6a9a8e", "mtnD": "#5c8c82",
		"p1L": "#3f8a5c", "p1M": "#2f7450", "p1D": "#245f44",
		"p2L": "#2e7048", "p2M": "#215a3b", "p2D": "#174632",
		"p3L": "#24603c", "p3M": "#164a30", "p3D": "#0e3424", "tr": "#4a2f1c",
		"g": "#1c4a30", "gH": "#2a6a40", "gD": "#123726",
		"tent": "#e8743a", "tentL": "#ff9a55", "tentD": "#b8532a", "o": "#3a2416",
		"fire": "#ff8a2a", "fireY": "#ffe066", "glow": "#3f7a3a", "log": "#6e4122",
	})
	w, h = size
	rng = random.Random("forest%d" % w)
	er = edge_rng("forest")
	back, front = layers(Image, size, pad)
	sky(back, [p["s1"], p["s2"], p["s3"], p["s4"]])
	disc(back, int(w * 0.3), int(h * 0.42), 5, p["sun"], ring=p["sunR"])
	for x in spots(rng, w, max(3, w // 20), gap=14, margin=0):
		peak(back, x, int(h * 0.3) + rng.randrange(0, 6), h, p["mtn"], p["mtnD"])
	for x in edge_spots(er, back, per_side(back, max(3, w // 20)), gap=14):
		peak(back, x, int(h * 0.3) + er.randrange(0, 6), h, p["mtn"], p["mtnD"])
	line = wave(rng, w, int(h * 0.62), 6, 90)
	hills(back, line, p["m1"], p["m1"], p["m1D"], pattern=lambda x, d: x % 7 == 1 and d % 4 == 2)
	c = w // 2
	for x in range(-2, w + 2, 3):
		pine(back, x + rng.randrange(0, 2), int(h * 0.74), rng.randrange(8, 13), p["p1L"], p["p1M"], p["p1D"])
	for x in continue_row(back, -2, w + 2, 3):
		pine(back, x + er.randrange(0, 2), int(h * 0.74), er.randrange(8, 13), p["p1L"], p["p1M"], p["p1D"])
	for x in range(-2, w + 2, 5):
		if abs(x - c) < 12:
			continue
		pine(front, x + rng.randrange(0, 3), int(h * 0.88), rng.randrange(11, 17), p["p2L"], p["p2M"], p["p2D"], p["tr"])
	for x in continue_row(front, -2, w + 2, 5):
		pine(front, x + er.randrange(0, 3), int(h * 0.88), er.randrange(11, 17), p["p2L"], p["p2M"], p["p2D"], p["tr"])
	ground(front, h - 5, p["gH"], p["g"], p["gD"])
	for x in spots(rng, w, w // 16, avoid=(c - 18, c + 16), gap=9):
		pine(front, x, h - 3, rng.randrange(16, 24), p["p3L"], p["p3M"], p["p3D"], p["tr"])
	for x in edge_spots(er, front, per_side(front, w // 16), gap=9, inner=4):
		pine(front, x, h - 3, er.randrange(16, 24), p["p3L"], p["p3M"], p["p3D"], p["tr"])
	tent = [
		"........o........",
		".......oLo.......",
		"......oLTDo......",
		".....oLTTTDo.....",
		"....oLTTTTTDo....",
		"...oLTTToTTTDo...",
		"..oLTTTooTTTTDo..",
		".oLTTToooTTTTTDo.",
		"oLTTTTooooTTTTTDo",
	]
	front.stamp(tent, c - 11, h - 5 - len(tent), {"o": p["o"], "L": p["tentL"], "T": p["tent"], "D": p["tentD"]})
	fx = c + 8
	for dx in range(-3, 8):
		front.put(fx + dx, h - 5, p["glow"])
	fire = [
		"..y..",
		".yfy.",
		".fYf.",
		"fYYYf",
		"lLlLl",
	]
	front.stamp(fire, fx, h - 5 - 4, {"y": p["fire"], "f": p["fire"], "Y": p["fireY"], "l": p["log"], "L": p["o"]})
	return back, front


def peaks(Image, size, pad=0):
	p = P({
		"s1": "#4f9fe6", "s2": "#7dbdf2", "s3": "#b2dcfa", "s4": "#e6f6ff",
		"far": "#a9c8e8", "farD": "#93b6dc", "mid": "#6f95c9", "midD": "#5679b0", "snow": "#ffffff", "snowS": "#dcecf8",
		"pL": "#3f8f86", "pM": "#2f7470", "pD": "#215a5a", "tr": "#4a3a30",
		"g": "#d8eaf7", "gH": "#ffffff", "gD": "#bcd6ec",
		"pole": "#4a3a30", "flag": "#ff5a4a", "flagD": "#cc3f33", "flake": "#ffffff",
	})
	w, h = size
	rng = random.Random("peaks%d" % w)
	er = edge_rng("peaks")
	back, front = layers(Image, size, pad)
	sky(back, [p["s1"], p["s2"], p["s3"], p["s4"]])
	gy = h - 6
	for x in spots(rng, w, max(3, w // 18), gap=14, margin=0):
		peak(back, x, int(h * 0.3) + rng.randrange(0, 6), gy, p["far"], p["farD"], snow=p["snowS"], snow_rows=4)
	for x in edge_spots(er, back, per_side(back, max(3, w // 18)), gap=14):
		peak(back, x, int(h * 0.3) + er.randrange(0, 6), gy, p["far"], p["farD"], snow=p["snowS"], snow_rows=4)
	c = w // 2
	for x in spots(rng, w, max(2, w // 34), avoid=(c - 26, c + 26), gap=28, margin=0):
		peak(front, x, int(h * 0.42) + rng.randrange(0, 4), gy, p["mid"], p["midD"], snow=p["snow"], slope=(1, 1), snow_rows=5)
	for x in edge_spots(er, front, per_side(front, max(2, w // 34)), gap=28, inner=10):
		peak(front, x, int(h * 0.42) + er.randrange(0, 4), gy, p["mid"], p["midD"], snow=p["snow"], slope=(1, 1), snow_rows=5)
	pk = 6
	peak(front, c, pk, gy, p["mid"], p["midD"], snow=p["snow"], slope=(1, 1), snow_rows=8)
	for y in range(pk - 5, pk):
		front.put(c, y, p["pole"])
	front.stamp(["ff", "fF", "f."], c + 1, pk - 5, {"f": p["flag"], "F": p["flagD"]})
	ground(front, h - 6, p["gH"], p["g"], p["gD"])
	for x in spots(rng, w, w // 6, avoid=(c - 3, c + 3), gap=4):
		pine(front, x, h - 2 - rng.randrange(0, 3), rng.randrange(7, 13), p["pL"], p["pM"], p["pD"], p["tr"], snow=p["snow"])
	for x in edge_spots(er, front, per_side(front, w // 6), gap=4, inner=2):
		pine(front, x, h - 2 - er.randrange(0, 3), er.randrange(7, 13), p["pL"], p["pM"], p["pD"], p["tr"], snow=p["snow"])
	for _ in range(w // 6):
		back.put(rng.randrange(w), rng.randrange(h // 2), p["flake"])
	edge_dots(back, er, w // 6, 0, h // 2, [p["flake"]])
	return back, front


def ocean(Image, size, pad=0):
	p = P({
		"s1": "#22aee6", "s2": "#58c8f0", "s3": "#9ee2f5", "s4": "#e2f8f6",
		"sun": "#fffbe0", "sunR": "#fff3b0", "cl": "#ffffff", "clS": "#c9eef8",
		"sea": "#1497b8", "seaD": "#0e7c9e", "seaL": "#3dbad3", "seaHi": "#ffffff",
		"isl": "#f3d792", "islD": "#d9b46a", "lL": "#4fcf6a", "lM": "#2fae55", "lD": "#208a45", "tr": "#9a6a3c",
		"hull": "#8a4e2c", "hullD": "#6b3a20", "sail": "#ffffff", "sailS": "#d8e8f0", "mast": "#5a3a24", "flag": "#ff5a4a",
	})
	w, h = size
	rng = random.Random("ocean%d" % w)
	er = edge_rng("ocean")
	back, front = layers(Image, size, pad)
	hz = int(h * 0.5)
	sky(back, [p["s1"], p["s2"], p["s3"], p["s4"]], 0, hz)
	sx = int(w * 0.25)
	disc(back, sx, int(h * 0.22), 4, p["sun"], ring=p["sunR"])
	clouds_row(back, rng, p, hz - 10)
	edge_clouds(back, er, p, hz - 10)
	for y in range(hz, h):
		for x in back.xs():
			back.put(x, y, p["seaL"] if y < hz + 2 else p["sea"] if y < h - 8 else p["seaD"])
	for y in range(hz + 1, h - 2, 2):
		ww = 2 + (y - hz) // 4
		off = ((y * 7) % 5) - 2
		for dx in range(-ww + off, ww + off - 1):
			if (dx + y) % 6 != 0:
				back.put(sx + dx, y, p["seaHi"] if abs(dx - off) < ww - 1 else p["seaL"])
	for _ in range(w // 3):
		x, y = rng.randrange(w), rng.randrange(hz + 3, h)
		back.put(x, y, p["seaL"])
		back.put(x + 1, y, p["seaL"])
	if pad:
		xs = edge_xs(back)
		for _ in range(per_side(back, w // 3) * 2):
			x, y = er.choice(xs), er.randrange(hz + 3, h)
			back.put(x, y, p["seaL"])
			back.put(x + 1, y, p["seaL"])
		far_boat = [
			"..o..",
			".So..",
			"SSoW.",
			"hhhhh",
		]
		back.stamp(far_boat, w + 64, hz - 2, {"o": p["mast"], "S": p["sailS"], "W": p["sail"], "h": p["hullD"]})
	c = w // 2
	for x in spots(rng, w, max(2, w // 70), avoid=(c - 24, c + 18), gap=48, margin=8):
		iw = rng.randrange(14, 22)
		for yy in range(3):
			for xx in range(-iw // 2 + yy * 2 - 2, iw // 2 - yy * 2 + 3):
				front.put(x + xx, hz + 4 - yy, p["isl"] if yy else p["islD"])
		tree(front, PALM, x + 2, hz + 2, {"L": p["lL"], "M": p["lM"], "D": p["lD"], "T": p["tr"]})
	if pad:
		for x, iw in ((-58, 18), (w + 28, 12)):
			for yy in range(3):
				for xx in range(-iw // 2 + yy * 2 - 2, iw // 2 - yy * 2 + 3):
					front.put(x + xx, hz + 4 - yy, p["isl"] if yy else p["islD"])
			tree(front, PALM, x + 2, hz + 2, {"L": p["lL"], "M": p["lM"], "D": p["lD"], "T": p["tr"]})
	for x in front.xs():
		if x % 8 in (0, 1, 2):
			front.put(x, h - 3 - (x // 8) % 2, p["seaL"])
	boat_y = int(h * 0.82)
	boat = [
		".......o..........",
		".......off........",
		".......oWff.......",
		"......SoWWW.......",
		".....SSoWWWW......",
		"....SSSoWWWWW.....",
		"...SSSSoWWWWWW....",
		"..SSSSSoWWWWWWW...",
		".......o..........",
		"HHHHHHHHHHHHHHHHHH",
		".hhhhhhhhhhhhhhhh.",
		"..hhhhhhhhhhhhhh..",
	]
	front.stamp(boat, c - 9, boat_y - len(boat) + 2, {"o": p["mast"], "f": p["flag"], "S": p["sailS"], "W": p["sail"], "H": p["hull"], "h": p["hullD"]})
	for dx in range(-11, 11, 3):
		front.put(c + dx, boat_y + 2, p["seaHi"])
	return back, front


def desert(Image, size, pad=0):
	p = P({
		"s1": "#ff8a5a", "s2": "#ffaa66", "s3": "#ffcc80", "s4": "#ffecb8",
		"sun": "#fffbe8", "sunR": "#ffe9a8",
		"d1": "#f0b465", "d1D": "#e2a152", "d2": "#e8a14e", "d2H": "#f5c070", "d2D": "#cf8a3c",
		"pyL": "#f9dc98", "pyM": "#e2b064", "pyD": "#c48d45", "pyS": "#b07a3a", "door": "#5a3a24",
		"cL": "#6fbf5a", "cM": "#4f9f45", "cD": "#3a7f38", "rock": "#b0703a",
	})
	w, h = size
	rng = random.Random("desert%d" % w)
	er = edge_rng("desert")
	back, front = layers(Image, size, pad)
	sky(back, [p["s1"], p["s2"], p["s3"], p["s4"]])
	c = w // 2
	disc(back, c - 10, int(h * 0.3), 8, p["sun"], ring=p["sunR"])
	if pad:
		for fx, fh in ((-72, 7), (w + 52, 5), (w + 62, 3)):
			for y in range(fh):
				for dx in range(-y - 1, y + 2):
					back.put(fx + dx, int(h * 0.66) - fh + y, p["d1D"] if dx > 0 else p["d2H"])
	line = wave(rng, w, int(h * 0.68), 4, 60)
	hills(back, line, p["d1"], p["d1"], p["d1D"], pattern=lambda x, d: (x + d) % 9 == 0)
	ph = 15 if h < 50 else 20
	base = int(h * 0.82)

	def pyramid(cx, hh):
		for y in range(hh):
			half = y + 1
			for dx in range(-half, half + 1):
				cc = p["pyL"] if dx < 0 else p["pyM"]
				if dx == half:
					cc = p["pyD"]
				if y % 3 == 2:
					cc = p["pyM"] if dx < 0 else p["pyS"]
				front.put(cx + dx, base - hh + y, cc)

	pyramid(c + 14, int(ph * 0.6))
	pyramid(c, ph)
	for y in range(3):
		front.put(c - 1, base - 3 + y, p["door"])
		front.put(c, base - 3 + y, p["door"])
	line2 = wave(rng, w, int(h * 0.9), 3, 50)
	hills(front, line2, p["d2"], p["d2H"], p["d2D"], pattern=lambda x, d: (x * 2 + d) % 11 == 0)
	for x in spots(rng, w, max(3, w // 34), avoid=(c - 18, c + 26), gap=16):
		tree(front, CACTUS, x, line2[x] + 2, {"L": p["cL"], "M": p["cM"], "D": p["cD"]})
	for x in edge_spots(er, front, max(2, per_side(front, max(3, w // 34))), gap=16, inner=6):
		tree(front, CACTUS, x, line2[x] + 2, {"L": p["cL"], "M": p["cM"], "D": p["cD"]})
	for x in spots(rng, w, w // 12, gap=5):
		front.put(x, line2[x] + 2, p["rock"])
		front.put(x + 1, line2[x] + 2, p["rock"])
	for x in edge_spots(er, front, per_side(front, w // 12), gap=5):
		front.put(x, line2[x] + 2, p["rock"])
		front.put(x + 1, line2[x] + 2, p["rock"])
	return back, front


def sakura(Image, size, pad=0):
	p = P({
		"s1": "#8a7bd8", "s2": "#c08ad8", "s3": "#f2a2c4", "s4": "#ffd6c8",
		"sun": "#fff3e0", "sunR": "#ffe0d0",
		"far": "#b58ac8", "farD": "#a079b8",
		"h1": "#7fc48a", "h1H": "#9ad89c", "h1D": "#69ad78",
		"lL": "#ffd6e8", "lM": "#ffa8cc", "lD": "#ec7aac", "tr": "#6e4a5a", "trD": "#523642",
		"torii": "#e8463a", "toriiD": "#b8322a", "beam": "#3a2a30",
		"pond": "#7ab8e8", "pondL": "#b0daf8",
		"g": "#6cb878", "gH": "#8fd394", "gD": "#5aa068", "petal": "#ffc0dc",
	})
	w, h = size
	rng = random.Random("sakura%d" % w)
	er = edge_rng("sakura")
	back, front = layers(Image, size, pad)
	sky(back, [p["s1"], p["s2"], p["s3"], p["s4"]])
	c = w // 2
	disc(back, c, int(h * 0.42), 7, p["sun"], ring=p["sunR"])
	gy = h - 6
	for x in spots(rng, w, max(3, w // 24), gap=18, margin=0):
		peak(back, x, int(h * 0.45) + rng.randrange(0, 4), gy, p["far"], p["farD"], slope=(1, 2))
	for x in edge_spots(er, back, per_side(back, max(3, w // 24)), gap=18):
		peak(back, x, int(h * 0.45) + er.randrange(0, 4), gy, p["far"], p["farD"], slope=(1, 2))
	line = wave(rng, w, int(h * 0.72), 3, 60)
	hills(front, line, p["h1"], p["h1H"], p["h1D"], pattern=lambda x, d: x % 6 == 1 and d % 3 == 2)
	pal = {"L": p["lL"], "M": p["lM"], "D": p["lD"], "T": p["tr"], "U": p["trD"]}
	for x in spots(rng, w, w // 16, avoid=(c - 19, c + 19), gap=8):
		tree(front, rng.choice([ROUND_BIG, ROUND, ROUND_BIG]), x, line[x] + 2 + rng.randrange(0, 5), pal)
	for x in edge_spots(er, front, per_side(front, w // 16), gap=8, inner=4):
		tree(front, er.choice([ROUND_BIG, ROUND, ROUND_BIG]), x, line[x] + 2 + er.randrange(0, 5), pal)
	ground(front, h - 3, p["gH"], p["g"], p["gD"])
	py = h - 4
	for dy in range(3):
		for dx in range(-14 + dy * 2, 15 - dy * 2):
			front.put(c + dx, py - 2 + dy, p["pondL"] if dy == 0 and dx % 4 == 0 else p["pond"])
	tw, th = 19, 14
	top = py - 3 - th
	x0 = c - tw // 2
	for x in range(x0 - 1, x0 + tw + 1):
		front.put(x, top, p["beam"])
	front.put(x0 - 2, top - 1, p["beam"])
	front.put(x0 + tw, top - 1, p["beam"])
	for x in range(x0, x0 + tw):
		front.put(x, top + 1, p["torii"])
		front.put(x, top + 2, p["toriiD"])
	for x in range(x0 + 1, x0 + tw - 1):
		front.put(x, top + 4, p["torii"])
	for px in (x0 + 3, x0 + tw - 5):
		for y in range(top + 3, py - 2):
			front.put(px, y, p["torii"])
			front.put(px + 1, y, p["toriiD"])
	front.put(c, top + 3, p["toriiD"])
	for _ in range(w // 4):
		back.put(rng.randrange(w), rng.randrange(int(h * 0.7)), p["petal"])
	edge_dots(back, er, w // 4, 0, int(h * 0.7), [p["petal"]])
	for _ in range(w // 10):
		x = rng.randrange(w)
		if abs(x - c) > 15:
			front.put(x, rng.randrange(h - 3, h), p["petal"])
	edge_dots(front, er, w // 10, h - 3, h, [p["petal"]])
	return back, front


def village(Image, size, pad=0):
	p = P({
		"s1": "#5aaef0", "s2": "#86c6f5", "s3": "#b6defa", "s4": "#e8f6ff",
		"sun": "#fff6c2", "sunR": "#fffbe0", "cl": "#ffffff", "clS": "#d9eefa",
		"far": "#8ac0a8", "farD": "#78ad98",
		"h1": "#86c95e", "h1H": "#a3dc74", "h1D": "#72b54f",
		"wheat": "#f2cc52", "wheatD": "#d9ad3a", "field": "#5fae4a", "fieldD": "#4f9a3e",
		"wall": "#f5e6c8", "wallD": "#d9c6a2", "roof": "#d9533a", "roofD": "#a83c2a", "win": "#5a7ab8", "door": "#7a4f2e", "o": "#5a3a24",
		"mill": "#efe2c6", "millD": "#cdbd9c", "blade": "#8a5a34", "sailc": "#ffffff",
		"lL": "#5fbf4d", "lM": "#3d9a3f", "lD": "#2b7a37", "tr": "#7a4f2e", "smoke": "#ffffff",
	})
	w, h = size
	rng = random.Random("village%d" % w)
	er = edge_rng("village")
	back, front = layers(Image, size, pad)
	sky(back, [p["s1"], p["s2"], p["s3"], p["s4"]])
	disc(back, int(w * 0.84), h // 5, 4, p["sun"], ring=p["sunR"])
	clouds_row(back, rng, p, h // 4)
	edge_clouds(back, er, p, h // 4)
	line0 = wave(rng, w, int(h * 0.56), 5, 80)
	hills(back, line0, p["far"], p["far"], p["farD"], pattern=lambda x, d: x % 8 == 3 and d % 4 == 1)
	line = wave(rng, w, int(h * 0.7), 2, 70)
	hills(front, line, p["h1"], p["h1H"], p["h1D"], pattern=lambda x, d: x % 6 == 2 and d % 4 == 3)
	fy = int(h * 0.84)
	for y in range(fy, h):
		for x in front.xs():
			band = ((x + (y - fy) * 2) // 6) % 2
			if (y - fy) % 3:
				cc = p["wheat"] if band else p["field"]
			else:
				cc = p["wheatD"] if band else p["fieldD"]
			front.put(x, y, cc)
	house = [
		"....rr....",
		"...rRRr...",
		"..rRRRRr..",
		".rRRRRRRr.",
		"rRRRRRRRRr",
		".wwwwwwwW.",
		".wbbwwddW.",
		".wbbwwddW.",
		".wwwwwddW.",
	]
	hp = {"r": p["roofD"], "R": p["roof"], "w": p["wall"], "W": p["wallD"], "b": p["win"], "d": p["door"]}
	pal = {"L": p["lL"], "M": p["lM"], "D": p["lD"], "T": p["tr"], "U": p["tr"]}
	c = w // 2
	for x in spots(rng, w, w // 16, avoid=(c - 12, c + 12), gap=10):
		tree(front, rng.choice([ROUND, BUSH]), x, line[x] + 2, pal)
	for x in edge_spots(er, front, per_side(front, w // 16), gap=10, inner=4):
		tree(front, er.choice([ROUND, BUSH]), x, line[x] + 2, pal)
	for x in spots(rng, w, max(2, w // 28), avoid=(c - 14, c + 14), gap=13):
		front.stamp(house, x - 5, line[x] + 2 - len(house), hp)
		if rng.random() < 0.6:
			for k in range(3):
				front.put(x + 2 + k % 2, line[x] - 9 - k * 2, p["smoke"])
	for x in edge_spots(er, front, per_side(front, max(2, w // 28)), gap=13, margin=6, inner=8):
		front.stamp(house, x - 5, line[x] + 2 - len(house), hp)
		if er.random() < 0.6:
			for k in range(3):
				front.put(x + 2 + k % 2, line[x] - 9 - k * 2, p["smoke"])
	mb = line[c] + 2
	for y in range(14):
		half = 2 + y // 5
		for dx in range(-half, half + 1):
			front.put(c + dx, mb - 14 + y, p["millD"] if dx == half else p["mill"])
	front.stamp(["..rr..", ".rRRr.", "rRRRRr"], c - 3, mb - 17, hp)
	front.put(c, mb - 2, p["door"])
	front.put(c, mb - 1, p["door"])
	front.put(c, mb - 8, p["win"])
	hx, hy = c, mb - 13
	for i in range(1, 10):
		for sx, sy in ((1, 1), (-1, -1), (1, -1), (-1, 1)):
			front.put(hx + sx * i, hy + sy * i, p["blade"])
			if i > 2:
				front.put(hx + sx * i - sy, hy + sy * i + sx, p["sailc"])
	front.put(hx, hy, p["o"])
	return back, front


def castle(Image, size, pad=0):
	p = P({
		"s1": "#141a4a", "s2": "#22286a", "s3": "#363a88", "s4": "#5a4a9a",
		"moon": "#f5f0d8", "moonS": "#d8d2b8", "star": "#ffffff", "starD": "#9a9ad8",
		"far": "#2a2e6a", "farD": "#24285e",
		"hill": "#1e3a5a", "hillH": "#2c5274", "hillD": "#18304c",
		"wall": "#8a8fb0", "wallL": "#a8adcc", "wallD": "#6a6f92", "roof": "#4a5ab8", "roofD": "#34408e",
		"win": "#ffd45a", "flag": "#ff5a6a",
		"pL": "#1f4a5a", "pM": "#173a4a", "pD": "#102c3a", "tr": "#0e2230",
	})
	w, h = size
	rng = random.Random("castle%d" % w)
	er = edge_rng("castle")
	back, front = layers(Image, size, pad)
	sky(back, [p["s1"], p["s2"], p["s3"], p["s4"]])
	stars(back, rng, w // 2, p["star"], p["starD"], int(h * 0.6), big=max(2, w // 50))
	edge_stars(back, er, w // 2, p["star"], p["starD"], int(h * 0.6), big=max(2, w // 50))
	disc(back, int(w * 0.75), int(h * 0.25), 5, p["moon"], hi=p["moonS"])
	gy = h - 4
	for x in spots(rng, w, max(3, w // 20), gap=16, margin=0):
		peak(back, x, int(h * 0.5) + rng.randrange(0, 5), gy, p["far"], p["farD"], slope=(1, 2))
	for x in edge_spots(er, back, per_side(back, max(3, w // 20)), gap=16):
		peak(back, x, int(h * 0.5) + er.randrange(0, 5), gy, p["far"], p["farD"], slope=(1, 2))
	c = w // 2
	line = Line(lambda x: min(h - 3, int(h * 0.66 + max(0, abs(x - c) - 16) * 0.3)))
	hills(front, line, p["hill"], p["hillH"], p["hillD"], pattern=lambda x, d: x % 6 == 3 and d % 3 == 1)
	base = line[c] + 1
	ww, wh = 22, 9
	for y in range(wh):
		for dx in range(-ww // 2, ww // 2 + 1):
			cc = p["wall"]
			if dx == -ww // 2:
				cc = p["wallL"]
			elif dx == ww // 2:
				cc = p["wallD"]
			elif (dx + y * 3) % 7 == 0 and y % 2 == 0:
				cc = p["wallD"]
			front.put(c + dx, base - wh + y, cc)
	for dx in range(-ww // 2, ww // 2 + 1, 2):
		front.put(c + dx, base - wh - 1, p["wall"])
	for dx in (-7, -4, 4, 7):
		front.put(c + dx, base - 6, p["win"])
	for dx in range(-1, 2):
		for y in range(4):
			front.put(c + dx, base - 4 + y, p["hillD"])
	front.put(c, base - 5, p["hillD"])

	def tower(tx, th, tw=3):
		for y in range(th):
			for dx in range(-tw, tw + 1):
				cc = p["wallL"] if dx == -tw else p["wallD"] if dx == tw else p["wall"]
				front.put(tx + dx, base - th + y, cc)
		rh = tw + 3
		for y in range(rh):
			half = min(tw + 1, (y * (tw + 1) + rh - 2) // (rh - 1))
			for dx in range(-half, half + 1):
				front.put(tx + dx, base - th - rh + y, p["roof"] if dx <= 0 else p["roofD"])
		for y in range(3):
			front.put(tx, base - th - rh - 1 - y, p["wallD"])
		front.stamp(["ff", "f."], tx + 1, base - th - rh - 3, {"f": p["flag"]})
		front.put(tx, base - th + 3, p["win"])
		front.put(tx, base - th + 4, p["win"])

	tower(c - 13, 15)
	tower(c + 13, 13)
	tower(c, 20, 3)
	for x in spots(rng, w, w // 7, avoid=(c - 19, c + 19), gap=4):
		pine(front, x, min(h, line[x] + rng.randrange(3, 6)), rng.randrange(8, 15), p["pL"], p["pM"], p["pD"], p["tr"])
	for x in edge_spots(er, front, per_side(front, w // 7), gap=4, inner=2):
		pine(front, x, min(h, line[x] + er.randrange(3, 6)), er.randrange(8, 15), p["pL"], p["pM"], p["pD"], p["tr"])
	return back, front


def caves(Image, size, pad=0):
	p = P({
		"bg": "#1a1238", "bg2": "#221848", "bg3": "#2c2058",
		"rock": "#3a2c66", "rockL": "#4e3e86", "rockD": "#2a1f50", "far": "#2a1f50",
		"c1L": "#d2fdff", "c1": "#5ee6f5", "c1D": "#2aa8c8",
		"c2L": "#ffd8ff", "c2": "#e070f0", "c2D": "#a040c0",
		"c3L": "#fff2b0", "c3": "#ffc94a", "c3D": "#d0902a",
		"glow": "#3e3384", "drip": "#5ee6f5", "moss": "#4fd18a", "mush": "#ff7ab0",
	})
	w, h = size
	rng = random.Random("caves%d" % w)
	er = edge_rng("caves")
	back, front = layers(Image, size, pad)
	sky(back, [p["bg"], p["bg2"], p["bg3"], p["bg2"]])
	for x in back.xs():
		for y in range(h):
			if (x * 7 + y * 13) % 29 == 0:
				back.put(x, y, p["far"])
	ceil = wave(rng, w, 6, 6, 30)
	floor = wave(rng, w, h - 3, 5, 40)
	c = w // 2
	for x in front.xs():
		ct = 16 - ceil[x]
		for y in range(0, ct):
			front.put(x, y, p["rock"] if y == ct - 1 else p["rockD"])
		for y in range(floor[x], h):
			front.put(x, y, p["rockL"] if y == floor[x] else p["rock"] if y < floor[x] + 3 else p["rockD"])
	for x in spots(rng, w, w // 6, gap=4, margin=0):
		top = 16 - ceil[x]
		ln = rng.randrange(2, 7)
		for y in range(ln):
			front.put(x, top + y, p["rock"])
			if y < ln - 2:
				front.put(x + 1, top + y, p["rockD"])
		if rng.random() < 0.4:
			front.put(x, top + ln + 1, p["drip"])
	for x in edge_spots(er, front, per_side(front, w // 6), gap=4, inner=2):
		top = 16 - ceil[x]
		ln = er.randrange(2, 7)
		for y in range(ln):
			front.put(x, top + y, p["rock"])
			if y < ln - 2:
				front.put(x + 1, top + y, p["rockD"])
		if er.random() < 0.4:
			front.put(x, top + ln + 1, p["drip"])

	def crystal(cx, base, hh, col):
		L, M, D = p[col + "L"], p[col], p[col + "D"]
		for y in range(hh):
			half = 0 if y == 0 else 1
			for dx in range(-half, half + 1):
				cc = L if dx < 0 or y == 0 else D if dx > 0 else M
				front.put(cx + dx, base - hh + y, cc)
		for y in range(-1, 2):
			for dx in range(-3 + abs(y), 4 - abs(y)):
				back.put(cx + dx, base - hh // 2 + y, p["glow"])

	for dx, hh, col in ((-9, 4, "c3"), (9, 5, "c3"), (-6, 8, "c2"), (6, 9, "c1"), (-3, 12, "c1"), (3, 13, "c2"), (0, 17, "c1")):
		crystal(c + dx, floor[c + dx] + 1, hh, col)
	for x in spots(rng, w, w // 14, avoid=(c - 14, c + 14), gap=7):
		col = rng.choice(["c1", "c2", "c3"])
		crystal(x, floor[x] + 1, rng.randrange(3, 8), col)
		if rng.random() < 0.6 and x + 2 < w:
			crystal(x + 2, floor[x + 2] + 1, rng.randrange(2, 5), col)
	for x in spots(rng, w, w // 10, avoid=(c - 12, c + 12), gap=5):
		front.put(x, floor[x] - 1, p["moss"])
		if rng.random() < 0.4 and x + 1 < w:
			front.put(x + 1, floor[x + 1] - 1, p["mush"])
			front.put(x + 1, floor[x + 1] - 2, p["mush"])
	for x in edge_spots(er, front, per_side(front, w // 14), gap=7, inner=4):
		col = er.choice(["c1", "c2", "c3"])
		crystal(x, floor[x] + 1, er.randrange(3, 8), col)
		if er.random() < 0.6:
			crystal(x + 2, floor[x + 2] + 1, er.randrange(2, 5), col)
	for x in edge_spots(er, front, per_side(front, w // 10), gap=5, inner=2):
		front.put(x, floor[x] - 1, p["moss"])
		if er.random() < 0.4:
			front.put(x + 1, floor[x + 1] - 1, p["mush"])
			front.put(x + 1, floor[x + 1] - 2, p["mush"])
	for _ in range(w // 5):
		back.put(rng.randrange(w), rng.randrange(14, h - 8), rng.choice([p["c1"], p["c2"], p["glow"]]))
	edge_dots(back, er, w // 5, 14, h - 8, [p["c1"], p["c2"], p["glow"]])
	return back, front


def volcano(Image, size, pad=0):
	p = P({
		"s1": "#2a1028", "s2": "#4a1630", "s3": "#7a2a32", "s4": "#c2483a",
		"smoke": "#4a3040", "smokeD": "#3a2434", "ash": "#ffb84a",
		"far": "#3a1a2a", "farD": "#2e1422",
		"rock": "#3a2a30", "rockL": "#4e3a40",
		"lava": "#ff6a2a", "lavaY": "#ffd24a", "lavaD": "#d03a1a",
		"g": "#2a1e24", "gH": "#4e3a40",
	})
	w, h = size
	rng = random.Random("volcano%d" % w)
	er = edge_rng("volcano")
	back, front = layers(Image, size, pad)
	sky(back, [p["s1"], p["s2"], p["s3"], p["s4"]])
	gy = h - 4
	for x in spots(rng, w, max(3, w // 22), gap=16, margin=0):
		peak(back, x, int(h * 0.52) + rng.randrange(0, 5), gy, p["far"], p["farD"], slope=(1, 2))
	for x in edge_spots(er, back, per_side(back, max(3, w // 22)), gap=16):
		peak(back, x, int(h * 0.52) + er.randrange(0, 5), gy, p["far"], p["farD"], slope=(1, 2))
	if pad:
		for fx, fy in ((-70, int(h * 0.42)), (w + 70, int(h * 0.46))):
			peak(back, fx, fy, gy, p["far"], p["farD"], slope=(2, 3))
			for dx in (-1, 0, 1):
				back.put(fx + dx, fy + 1, p["lavaD"])
			for k in range(6):
				for dx in range(-1 - k // 2, 2 + k // 2):
					if er.random() < 0.6:
						back.put(fx + k // 2 + dx, fy - 1 - k, p["smokeD"])
	c = w // 2
	ty = 10 if h < 50 else 16
	peak(front, c, ty - 3, h, p["rockL"], p["rock"], slope=(3, 4))
	for dx in range(-4, 5):
		for y in range(ty - 3, ty - 1):
			front.put(c + dx, y, CLEAR)
	for dx in range(-4, 5):
		front.put(c + dx, ty, p["lavaY"] if abs(dx) < 3 else p["lava"])
		front.put(c + dx, ty - 1, p["lava"] if abs(dx) < 4 else (p["rockL"] if dx < 0 else p["rock"]))
	for start, dirx in ((c - 2, -1), (c + 2, 1), (c, 0)):
		x = start
		for y in range(ty + 1, h):
			if front.get(x, y)[3]:
				front.put(x, y, p["lava"] if y % 3 else p["lavaY"])
				nx = x + (1 if dirx >= 0 else -1)
				if front.get(nx, y)[3]:
					front.put(nx, y, p["lavaD"])
			if dirx and y % 2 == 0:
				x += dirx
			elif dirx == 0 and y % 5 == 0:
				x += rng.choice([-1, 1])
	for i in range(14):
		sx = c + i // 2
		sy = ty - 3 - i
		if sy < 0:
			break
		for dx in range(-2 - i // 3, 3 + i // 3):
			if rng.random() < 0.8:
				back.put(sx + dx, sy, p["smoke"] if rng.random() < 0.6 else p["smokeD"])
	for _ in range(16):
		back.put(c + rng.randrange(-12, 13), rng.randrange(0, ty), p["ash"])
	line = wave(rng, w, h - 4, 3, 30)
	for x in front.xs():
		if abs(x - c) < 22:
			continue
		for y in range(line[x], h):
			front.put(x, y, p["gH"] if y == line[x] else p["g"])
	for x in spots(rng, w, w // 12, avoid=(c - 26, c + 26), gap=6):
		for k, col in enumerate(("lava", "lavaY", "lava")):
			front.put(x + k, line[x] + 1, p[col])
	for x in edge_spots(er, front, per_side(front, w // 12), gap=6, inner=2):
		for k, col in enumerate(("lava", "lavaY", "lava")):
			front.put(x + k, line[x] + 1, p[col])
	return back, front


def cosmos(Image, size, pad=0):
	p = P({
		"s1": "#0d0826", "s2": "#1a0f3e", "s3": "#2c1458", "s4": "#4a1a6a",
		"star": "#ffffff", "starD": "#8a7ad8",
		"pl": "#ff9ad0", "plL": "#ffc4e4", "plD": "#d06aa8", "ring": "#ffe08a", "ringD": "#d8b05a",
		"pl2": "#6ad8ff", "pl2D": "#3aa0d8",
		"grass": "#5ff0b0", "grassD": "#3ac08a", "rock": "#5a3a8a", "rockL": "#6e4ea0", "rockD": "#422a6e",
		"cr": "#7af0ff", "crD": "#3ac0e0", "lL": "#c0a0ff", "lM": "#8a5ae8", "lD": "#6a3ac8", "tr": "#4a2a6a",
	})
	w, h = size
	rng = random.Random("cosmos%d" % w)
	er = edge_rng("cosmos")
	back, front = layers(Image, size, pad)
	sky(back, [p["s1"], p["s2"], p["s3"], p["s4"]])
	stars(back, rng, w, p["star"], p["starD"], h, big=max(3, w // 30))
	edge_stars(back, er, w, p["star"], p["starD"], h, big=max(3, w // 30))
	if pad:
		disc(back, w + 58, int(h * 0.7), 3, p["pl2"])
		for dx, dy in ((1, -1), (2, 0), (1, 1), (0, 2)):
			back.put(w + 58 + dx, int(h * 0.7) + dy, p["pl2D"])
		disc(back, -82, int(h * 0.3), 1, p["plD"])
	px, py, r = int(w * 0.76), int(h * 0.32), 7
	for y in range(-r, r):
		for x in range(-r, r):
			if (x + 0.5) ** 2 + (y + 0.5) ** 2 <= r * r:
				cc = p["pl"]
				if x + y < -r + 1:
					cc = p["plL"]
				elif x + y > r - 3:
					cc = p["plD"]
				elif y % 3 == 0:
					cc = p["plL"] if x < 0 else p["pl"]
				back.put(px + x, py + y, cc)
	ra, rb, tilt = r + 5, 2.2, -0.35
	seen = set()
	for i in range(720):
		t = i * math.pi / 360
		ex, ey = ra * math.cos(t), rb * math.sin(t)
		x = int(round(ex * math.cos(tilt) - ey * math.sin(tilt)))
		y = int(round(ex * math.sin(tilt) + ey * math.cos(tilt)))
		if (x, y) in seen:
			continue
		seen.add((x, y))
		inside = (x + 0.5) ** 2 + (y + 0.5) ** 2 <= r * r
		if math.sin(t) < 0 and inside:
			continue
		back.put(px + x, py + y, p["ring"] if x < 2 else p["ringD"])
	disc(back, int(w * 0.14), int(h * 0.2), 2, p["pl2"])
	back.put(int(w * 0.14) + 1, int(h * 0.2), p["pl2D"])

	def island(cx, top, iw, depth):
		for y in range(depth):
			half = int(iw / 2 * (1 - y / depth) ** 0.8)
			for dx in range(-half, half + 1):
				cc = p["rockL"] if dx < -half + 2 else p["rockD"] if dx > half - 2 else p["rock"]
				if y == 0:
					cc = p["grass"]
				elif y == 1:
					cc = p["grassD"] if dx % 3 else p["grass"]
				front.put(cx + dx, top + y, cc)

	c = w // 2
	top = int(h * 0.58)
	island(c, top, 30, 13)
	tree(front, ROUND, c - 7, top, {"L": p["lL"], "M": p["lM"], "D": p["lD"], "T": p["tr"], "U": p["tr"]})
	for dx, hh in ((5, 9), (8, 6), (3, 5)):
		for y in range(hh):
			half = 0 if y == 0 else 1
			for xx in range(-half, half + 1):
				front.put(c + dx + xx, top - hh + y, p["cr"] if xx <= 0 else p["crD"])
	for x in spots(rng, w, max(2, w // 60), avoid=(c - 26, c + 26), gap=40, margin=8):
		t = rng.randrange(int(h * 0.3), int(h * 0.7))
		island(x, t, rng.randrange(10, 18), rng.randrange(5, 9))
		if rng.random() < 0.6:
			front.put(x, t - 1, p["cr"])
			front.put(x, t - 2, p["cr"])
	for x in edge_spots(er, front, per_side(front, max(2, w // 60)), gap=40, margin=8, inner=14):
		t = er.randrange(int(h * 0.3), int(h * 0.7))
		island(x, t, er.randrange(10, 18), er.randrange(5, 9))
		if er.random() < 0.6:
			tree(front, BUSH, x - 2, t, {"L": p["lL"], "M": p["lM"], "D": p["lD"]})
		else:
			front.put(x, t - 1, p["cr"])
			front.put(x, t - 2, p["cr"])
	return back, front


SCENES = {
	"meadow": meadow, "forest": forest, "peaks": peaks, "ocean": ocean, "desert": desert,
	"sakura": sakura, "village": village, "castle": castle, "caves": caves, "volcano": volcano, "cosmos": cosmos,
}


def build(Image):
	out = {}
	for name in ORDER:
		for suffix, size, pad in (("", CARD, 0), ("-wide", BANNER, EDGE)):
			back, front = SCENES[name](Image, size, pad)
			out["cover-%s%s-back" % (name, suffix)] = back.im
			out["cover-%s%s-front" % (name, suffix)] = front.im
	return out
