//! Рисование через Direct2D и DirectWrite: прямоугольники со скруглением, текст Onest, иконки.

use std::cell::RefCell;
use std::collections::HashMap;

use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::core::{Interface, Result, w};
use windows_numerics::{Matrix3x2, Vector2};

use crate::art_data;
use crate::svgpath::{self, Cmd};
use crate::theme::Color;

const FONTS: [&[u8]; 3] = [
	include_bytes!("../assets/fonts/Onest-Regular.ttf"),
	include_bytes!("../assets/fonts/Onest-Medium.ttf"),
	include_bytes!("../assets/fonts/Onest-SemiBold.ttf"),
];

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
	pub x: f32,
	pub y: f32,
	pub w: f32,
	pub h: f32,
}

impl Rect {
	pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
		Rect { x, y, w, h }
	}

	pub fn contains(&self, x: f32, y: f32) -> bool {
		x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
	}

	pub fn right(&self) -> f32 {
		self.x + self.w
	}

	pub fn bottom(&self) -> f32 {
		self.y + self.h
	}

	pub fn inset(&self, d: f32) -> Rect {
		Rect::new(self.x + d, self.y + d, self.w - 2.0 * d, self.h - 2.0 * d)
	}

	fn d2d(&self) -> D2D_RECT_F {
		D2D_RECT_F {
			left: self.x,
			top: self.y,
			right: self.x + self.w,
			bottom: self.y + self.h,
		}
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Weight {
	Regular,
	Medium,
	SemiBold,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
	Start,
	Center,
	End,
}

#[derive(Clone, Copy, Debug)]
pub struct TextStyle {
	pub size: f32,
	pub line: f32,
	pub weight: Weight,
	pub color: Color,
	pub align: Align,
	pub wrap: bool,
	pub mono: bool,
}

impl TextStyle {
	pub fn new(size: f32, line: f32, weight: Weight, color: Color) -> TextStyle {
		TextStyle {
			size,
			line,
			weight,
			color,
			align: Align::Start,
			wrap: false,
			mono: false,
		}
	}

	pub fn wrap(mut self) -> Self {
		self.wrap = true;
		self
	}

	pub fn align(mut self, align: Align) -> Self {
		self.align = align;
		self
	}

	pub fn mono(mut self) -> Self {
		self.mono = true;
		self
	}
}

type FormatKey = (u32, u32, Weight, bool, bool, u8);

pub struct Gfx {
	pub d2d: ID2D1Factory1,
	dw: IDWriteFactory5,
	fonts: Option<IDWriteFontCollection1>,
	round: ID2D1StrokeStyle,
	formats: RefCell<HashMap<FormatKey, IDWriteTextFormat>>,
	paths: RefCell<HashMap<String, ID2D1PathGeometry>>,
}

impl Gfx {
	pub fn new() -> Result<Gfx> {
		unsafe {
			let d2d: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
			let dw: IDWriteFactory5 = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
			let round = d2d.CreateStrokeStyle(
				&D2D1_STROKE_STYLE_PROPERTIES1 {
					startCap: D2D1_CAP_STYLE_ROUND,
					endCap: D2D1_CAP_STYLE_ROUND,
					dashCap: D2D1_CAP_STYLE_ROUND,
					lineJoin: D2D1_LINE_JOIN_ROUND,
					miterLimit: 10.0,
					dashStyle: D2D1_DASH_STYLE_SOLID,
					dashOffset: 0.0,
					transformType: D2D1_STROKE_TRANSFORM_TYPE_NORMAL,
				},
				None,
			)?;
			let fonts = load_fonts(&dw).ok();
			Ok(Gfx {
				d2d,
				dw,
				fonts,
				round: round.cast()?,
				formats: RefCell::default(),
				paths: RefCell::default(),
			})
		}
	}

	fn format(&self, s: &TextStyle) -> Result<IDWriteTextFormat> {
		let key = (
			(s.size * 10.0) as u32,
			(s.line * 10.0) as u32,
			s.weight,
			s.mono,
			s.wrap,
			s.align as u8,
		);
		if let Some(f) = self.formats.borrow().get(&key) {
			return Ok(f.clone());
		}
		let weight = match s.weight {
			Weight::Regular => DWRITE_FONT_WEIGHT_NORMAL,
			Weight::Medium => DWRITE_FONT_WEIGHT_MEDIUM,
			Weight::SemiBold => DWRITE_FONT_WEIGHT_SEMI_BOLD,
		};
		let format = unsafe {
			let format = if s.mono {
				self.dw.CreateTextFormat(
					w!("Consolas"),
					None,
					DWRITE_FONT_WEIGHT_NORMAL,
					DWRITE_FONT_STYLE_NORMAL,
					DWRITE_FONT_STRETCH_NORMAL,
					s.size,
					w!(""),
				)?
			} else {
				match &self.fonts {
					Some(fonts) => self.dw.CreateTextFormat(
						w!("Onest"),
						&fonts.cast::<IDWriteFontCollection>()?,
						weight,
						DWRITE_FONT_STYLE_NORMAL,
						DWRITE_FONT_STRETCH_NORMAL,
						s.size,
						w!(""),
					)?,
					None => self.dw.CreateTextFormat(
						w!("Segoe UI"),
						None,
						weight,
						DWRITE_FONT_STYLE_NORMAL,
						DWRITE_FONT_STRETCH_NORMAL,
						s.size,
						w!(""),
					)?,
				}
			};
			format.SetWordWrapping(if s.wrap {
				DWRITE_WORD_WRAPPING_WRAP
			} else {
				DWRITE_WORD_WRAPPING_NO_WRAP
			})?;
			format.SetTextAlignment(match s.align {
				Align::Start => DWRITE_TEXT_ALIGNMENT_LEADING,
				Align::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
				Align::End => DWRITE_TEXT_ALIGNMENT_TRAILING,
			})?;
			format.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, s.line, s.line * 0.78)?;
			if !s.wrap {
				let sign = self.dw.CreateEllipsisTrimmingSign(&format)?;
				format.SetTrimming(
					&DWRITE_TRIMMING {
						granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
						delimiter: 0,
						delimiterCount: 0,
					},
					&sign,
				)?;
			}
			format
		};
		self.formats.borrow_mut().insert(key, format.clone());
		Ok(format)
	}

	pub fn layout(&self, text: &str, style: &TextStyle, max_w: f32) -> Result<IDWriteTextLayout> {
		let wide: Vec<u16> = text.encode_utf16().collect();
		let format = self.format(style)?;
		unsafe {
			self.dw
				.CreateTextLayout(&wide, &format, max_w.max(1.0), 10_000.0)
		}
	}

	/// Ширина и высота текста.
	pub fn measure(&self, text: &str, style: &TextStyle, max_w: f32) -> (f32, f32) {
		let Ok(layout) = self.layout(text, style, max_w) else {
			return (0.0, style.line);
		};
		let mut m = DWRITE_TEXT_METRICS::default();
		if unsafe { layout.GetMetrics(&mut m) }.is_err() {
			return (0.0, style.line);
		}
		(m.widthIncludingTrailingWhitespace, m.height)
	}

	/// Контур из SVG `d`, разобранный один раз.
	fn path(&self, key: &str, d: &[&str]) -> Option<ID2D1PathGeometry> {
		if let Some(p) = self.paths.borrow().get(key) {
			return Some(p.clone());
		}
		let geometry = unsafe { build_path(&self.d2d, d) }.ok()?;
		self.paths
			.borrow_mut()
			.insert(key.to_owned(), geometry.clone());
		Some(geometry)
	}
}

fn load_fonts(dw: &IDWriteFactory5) -> Result<IDWriteFontCollection1> {
	unsafe {
		let loader = dw.CreateInMemoryFontFileLoader()?;
		dw.RegisterFontFileLoader(&loader)?;
		let builder = dw.CreateFontSetBuilder()?;
		for data in FONTS {
			let file = loader.CreateInMemoryFontFileReference(
				dw,
				data.as_ptr().cast(),
				data.len() as u32,
				None,
			)?;
			builder.AddFontFile(&file)?;
		}
		let set = builder.CreateFontSet()?;
		dw.CreateFontCollectionFromFontSet(&set)
	}
}

fn v(x: f32, y: f32) -> Vector2 {
	Vector2 { X: x, Y: y }
}

unsafe fn build_path(d2d: &ID2D1Factory1, parts: &[&str]) -> Result<ID2D1PathGeometry> {
	unsafe {
		let geometry: ID2D1PathGeometry = d2d.CreatePathGeometry()?.cast()?;
		let sink = geometry.Open()?;
		sink.SetFillMode(D2D1_FILL_MODE_WINDING);
		for d in parts {
			let mut open = false;
			for cmd in svgpath::parse(d) {
				match cmd {
					Cmd::Move(x, y) => {
						if open {
							sink.EndFigure(D2D1_FIGURE_END_OPEN);
						}
						sink.BeginFigure(v(x, y), D2D1_FIGURE_BEGIN_FILLED);
						open = true;
					}
					Cmd::Line(x, y) => sink.AddLine(v(x, y)),
					Cmd::Cubic(x1, y1, x2, y2, x, y) => sink.AddBezier(&D2D1_BEZIER_SEGMENT {
						point1: v(x1, y1),
						point2: v(x2, y2),
						point3: v(x, y),
					}),
					Cmd::Arc(rx, ry, rot, large, sweep, x, y) => sink.AddArc(&D2D1_ARC_SEGMENT {
						point: v(x, y),
						size: D2D_SIZE_F {
							width: rx,
							height: ry,
						},
						rotationAngle: rot,
						sweepDirection: if sweep {
							D2D1_SWEEP_DIRECTION_CLOCKWISE
						} else {
							D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE
						},
						arcSize: if large {
							D2D1_ARC_SIZE_LARGE
						} else {
							D2D1_ARC_SIZE_SMALL
						},
					}),
					Cmd::Close => {
						if open {
							sink.EndFigure(D2D1_FIGURE_END_CLOSED);
							open = false;
						}
					}
				}
			}
			if open {
				sink.EndFigure(D2D1_FIGURE_END_OPEN);
			}
		}
		sink.Close()?;
		Ok(geometry)
	}
}

fn color(c: Color) -> D2D1_COLOR_F {
	D2D1_COLOR_F {
		r: c.r,
		g: c.g,
		b: c.b,
		a: c.a,
	}
}

/// Кадр рисования на любой цели Direct2D (окно или картинка).
pub struct Canvas<'a> {
	pub rt: &'a ID2D1RenderTarget,
	pub g: &'a Gfx,
	/// Множитель прозрачности для всего, что рисуется (появление шага).
	pub opacity: f32,
	base: Matrix3x2,
}

impl<'a> Canvas<'a> {
	pub fn new(rt: &'a ID2D1RenderTarget, g: &'a Gfx) -> Canvas<'a> {
		let mut base = Matrix3x2::default();
		unsafe { rt.GetTransform(&mut base) };
		Canvas {
			rt,
			g,
			opacity: 1.0,
			base,
		}
	}

	fn brush(&self, c: Color) -> Option<ID2D1SolidColorBrush> {
		unsafe {
			self.rt
				.CreateSolidColorBrush(&color(c.alpha(self.opacity)), None)
		}
		.ok()
	}

	/// Сдвиг всего, что рисуется дальше (вместо пересчёта координат).
	pub fn offset(&mut self, dx: f32, dy: f32) {
		self.base = Matrix3x2::translation(dx, dy) * self.base;
		unsafe { self.rt.SetTransform(&self.base) };
	}

	pub fn clear(&self, c: Color) {
		unsafe { self.rt.Clear(Some(&color(c))) };
	}

	pub fn fill(&self, r: Rect, c: Color) {
		if let Some(b) = self.brush(c) {
			unsafe { self.rt.FillRectangle(&r.d2d(), &b) };
		}
	}

	pub fn fill_round(&self, r: Rect, radius: f32, c: Color) {
		if let Some(b) = self.brush(c) {
			let rr = D2D1_ROUNDED_RECT {
				rect: r.d2d(),
				radiusX: radius,
				radiusY: radius,
			};
			unsafe { self.rt.FillRoundedRectangle(&rr, &b) };
		}
	}

	/// Рамка внутрь прямоугольника: `r` — внешний край.
	pub fn stroke_round(&self, r: Rect, radius: f32, width: f32, c: Color) {
		if let Some(b) = self.brush(c) {
			let inner = r.inset(width / 2.0);
			let rr = D2D1_ROUNDED_RECT {
				rect: inner.d2d(),
				radiusX: (radius - width / 2.0).max(0.0),
				radiusY: (radius - width / 2.0).max(0.0),
			};
			unsafe { self.rt.DrawRoundedRectangle(&rr, &b, width, None) };
		}
	}

	pub fn fill_circle(&self, cx: f32, cy: f32, r: f32, c: Color) {
		if let Some(b) = self.brush(c) {
			let e = D2D1_ELLIPSE {
				point: v(cx, cy),
				radiusX: r,
				radiusY: r,
			};
			unsafe { self.rt.FillEllipse(&e, &b) };
		}
	}

	pub fn stroke_circle(&self, cx: f32, cy: f32, r: f32, width: f32, c: Color) {
		if let Some(b) = self.brush(c) {
			let e = D2D1_ELLIPSE {
				point: v(cx, cy),
				radiusX: r - width / 2.0,
				radiusY: r - width / 2.0,
			};
			unsafe { self.rt.DrawEllipse(&e, &b, width, None) };
		}
	}

	pub fn line(&self, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, c: Color) {
		if let Some(b) = self.brush(c) {
			unsafe { self.rt.DrawLine(v(x0, y0), v(x1, y1), &b, width, None) };
		}
	}

	/// Текст в прямоугольнике; по вертикали — сверху. Возвращает высоту.
	pub fn text(&self, s: &str, r: Rect, style: &TextStyle) -> f32 {
		let Ok(layout) = self.g.layout(s, style, r.w) else {
			return 0.0;
		};
		if let Some(b) = self.brush(style.color) {
			unsafe {
				self.rt.DrawTextLayout(
					v(r.x, r.y),
					&layout,
					&b,
					D2D1_DRAW_TEXT_OPTIONS_CLIP | D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT,
				)
			};
		}
		let mut m = DWRITE_TEXT_METRICS::default();
		unsafe { layout.GetMetrics(&mut m) }.ok();
		m.height
	}

	/// Текст по центру строки высотой `r.h`.
	pub fn text_mid(&self, s: &str, r: Rect, style: &TextStyle) {
		let top = r.y + (r.h - style.line) / 2.0;
		self.text(s, Rect::new(r.x, top, r.w, style.line), style);
	}

	fn with_transform(&self, m: Matrix3x2, f: impl FnOnce()) {
		unsafe { self.rt.SetTransform(&(m * self.base)) };
		f();
		unsafe { self.rt.SetTransform(&self.base) };
	}

	/// Иконка lucide (сетка 24, линия 1,5) размером `size` в точке `(x, y)`; `angle` — поворот, градусы.
	pub fn icon(&self, name: &str, x: f32, y: f32, size: f32, c: Color, angle: f32) {
		let Some((_, parts)) = art_data::ICONS.iter().find(|(n, _)| *n == name) else {
			return;
		};
		let Some(path) = self.g.path(&format!("icon:{name}"), parts) else {
			return;
		};
		let Some(b) = self.brush(c) else {
			return;
		};
		let k = size / 24.0;
		let m = Matrix3x2::rotation_around(angle, v(12.0, 12.0))
			* Matrix3x2 {
				M11: k,
				M12: 0.0,
				M21: 0.0,
				M22: k,
				M31: x,
				M32: y,
			};
		self.with_transform(m, || unsafe {
			self.rt.DrawGeometry(&path, &b, 1.5, &self.g.round)
		});
	}

	/// Залитая иконка (кнопка «Запустить»).
	pub fn icon_filled(&self, name: &str, x: f32, y: f32, size: f32, c: Color) {
		let Some((_, parts)) = art_data::ICONS.iter().find(|(n, _)| *n == name) else {
			return;
		};
		let (Some(path), Some(b)) = (self.g.path(&format!("icon:{name}"), parts), self.brush(c))
		else {
			return;
		};
		let k = size / 24.0;
		let m = Matrix3x2 {
			M11: k,
			M12: 0.0,
			M21: 0.0,
			M22: k,
			M31: x,
			M32: y,
		};
		self.with_transform(m, || unsafe {
			self.rt.FillGeometry(&path, &b, None);
			self.rt.DrawGeometry(&path, &b, 1.5, &self.g.round);
		});
	}

	fn fill_paths(&self, key: &str, parts: &[&str], m: Matrix3x2, c: Color) {
		let (Some(path), Some(b)) = (self.g.path(key, parts), self.brush(c)) else {
			return;
		};
		self.with_transform(m, || unsafe { self.rt.FillGeometry(&path, &b, None) });
	}

	/// Лок-ап «искра + leelauncher» высотой `h`. Возвращает ширину.
	pub fn lockup(&self, x: f32, y: f32, h: f32, brand: Color, ink: Color) -> f32 {
		let (lw, lh) = art_data::LOCKUP_SIZE;
		let k = h / lh;
		let m = scale_at(k, x, y);
		for (i, d) in art_data::LOCKUP_SPARK.iter().enumerate() {
			self.fill_paths(
				&format!("lockup:s{i}"),
				&[d],
				m,
				if i == 0 { brand } else { ink },
			);
		}
		for (i, d) in art_data::LOCKUP_WORD.iter().enumerate() {
			self.fill_paths(
				&format!("lockup:w{i}"),
				&[d],
				m,
				if i == 1 { brand } else { ink },
			);
		}
		lw * k
	}

	/// Значок приложения: плитка с искрой, сторона `size`.
	pub fn app_icon(&self, x: f32, y: f32, size: f32, tile: Color, mark: Color, accent: Color) {
		let k = size / 100.0;
		self.fill_paths("tile", &[art_data::ICON_TILE], scale_at(k, x, y), tile);
		let (sw, sh) = art_data::SPARK_SIZE;
		let h = 100.0 * art_data::ICON_MARK_LARGE;
		let w = h * sw / sh;
		let ks = h / sh * k;
		let m = scale_at(ks, x + (100.0 - w) / 2.0 * k, y + (100.0 - h) / 2.0 * k);
		for (i, d) in art_data::SPARK.iter().enumerate() {
			self.fill_paths(
				&format!("spark{i}"),
				&[d],
				m,
				if i == 0 { accent } else { mark },
			);
		}
	}

	pub fn clip(&self, r: Rect) {
		unsafe {
			self.rt
				.PushAxisAlignedClip(&r.d2d(), D2D1_ANTIALIAS_MODE_ALIASED)
		};
	}

	pub fn unclip(&self) {
		unsafe { self.rt.PopAxisAlignedClip() };
	}
}

fn scale_at(k: f32, x: f32, y: f32) -> Matrix3x2 {
	Matrix3x2 {
		M11: k,
		M12: 0.0,
		M21: 0.0,
		M22: k,
		M31: x,
		M32: y,
	}
}
