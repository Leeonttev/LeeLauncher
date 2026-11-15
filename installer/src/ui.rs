//! Простые виджеты в стиле лаунчера. Каждый кадр рисуется заново, а области щелчка
//! запоминаются для следующего ввода.

use crate::gfx::{Align, Canvas, Rect, TextStyle, Weight};
use crate::text::{Lang, t};
use crate::theme::{Color, Palette};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
	/// Нажимается: кнопка, флажок, строка выбора.
	Press,
	/// Перетаскивает окно.
	Drag,
	/// Перекрывает всё под собой (подложка подтверждения).
	Block,
}

#[derive(Clone, Debug)]
pub struct Region {
	pub id: String,
	pub rect: Rect,
	pub kind: Kind,
	pub enabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
	Primary,
	Ghost,
	Outline,
	Destructive,
}

/// Что известно о вводе к началу кадра.
#[derive(Clone, Debug, Default)]
pub struct Input {
	pub hover: Option<String>,
	pub pressed: Option<String>,
	pub focus: Option<String>,
	/// Фокус виден только после клавиатуры.
	pub focus_visible: bool,
}

pub struct Ui<'a> {
	pub c: Canvas<'a>,
	pub p: &'static Palette,
	pub lang: Lang,
	pub input: &'a Input,
	pub now: f64,
	pub reduced: bool,
	pub regions: Vec<Region>,
	pub focusable: Vec<String>,
	/// В кадре есть движение: нужен следующий кадр.
	pub animating: bool,
	pub win10: bool,
}

pub const BUTTON_H: f32 = 36.0;

impl<'a> Ui<'a> {
	pub fn t(&self, key: &str) -> &'static str {
		t(self.lang, key)
	}

	pub fn style(&self, size: f32, line: f32, weight: Weight, color: Color) -> TextStyle {
		TextStyle::new(size, line, weight, color)
	}

	fn hovered(&self, id: &str) -> bool {
		self.input.hover.as_deref() == Some(id)
	}

	fn pressed(&self, id: &str) -> bool {
		self.input.pressed.as_deref() == Some(id) && self.hovered(id)
	}

	pub fn focused(&self, id: &str) -> bool {
		self.input.focus_visible && self.input.focus.as_deref() == Some(id)
	}

	pub fn region(&mut self, id: &str, rect: Rect, kind: Kind, enabled: bool, focusable: bool) {
		self.regions.push(Region {
			id: id.to_owned(),
			rect,
			kind,
			enabled,
		});
		if focusable && enabled {
			self.focusable.push(id.to_owned());
		}
	}

	pub fn drag(&mut self, rect: Rect) {
		self.region("", rect, Kind::Drag, true, false);
	}

	/// Начало подтверждения поверх окна: всё, что было раньше, недоступно.
	pub fn modal(&mut self, rect: Rect) {
		self.focusable.clear();
		self.region("", rect, Kind::Block, true, false);
	}

	fn ring(&self, r: Rect, radius: f32) {
		self.c.stroke_round(
			Rect::new(r.x - 4.0, r.y - 4.0, r.w + 8.0, r.h + 8.0),
			radius + 4.0,
			2.0,
			self.p.primary_text,
		);
	}

	pub fn button_width(&self, label: &str, icon: bool) -> f32 {
		let style = self.style(14.0, 20.0, Weight::Medium, self.p.foreground);
		let (w, _) = self.c.g.measure(label, &style, 1000.0);
		(w + 32.0 + if icon { 24.0 } else { 0.0 }).ceil()
	}

	/// Таблетка-кнопка. `icon` — иконка lucide слева, `spin` — крутится (ожидание).
	#[allow(clippy::too_many_arguments)]
	pub fn button(
		&mut self,
		id: &str,
		r: Rect,
		label: &str,
		variant: Variant,
		enabled: bool,
		icon: Option<&str>,
		spin: bool,
	) {
		self.region(id, r, Kind::Press, enabled, true);
		let p = self.p;
		let hover = enabled && self.hovered(id);
		let (bg, fg, border) = match variant {
			Variant::Primary => (
				if hover {
					p.primary.mix(p.foreground, 0.9)
				} else {
					p.primary
				},
				p.primary_foreground,
				None,
			),
			Variant::Ghost => (
				if hover {
					p.accent
				} else {
					Color { a: 0.0, ..p.accent }
				},
				p.foreground,
				None,
			),
			Variant::Outline => (
				if hover { p.accent } else { p.background },
				p.foreground,
				Some(p.border),
			),
			Variant::Destructive => (
				if hover {
					p.destructive.alpha(0.1)
				} else {
					p.background
				},
				p.destructive,
				Some(p.destructive),
			),
		};
		let fade = if enabled { 1.0 } else { 0.55 };
		let mut rr = r;
		if self.pressed(id) && enabled {
			let k = 0.98;
			rr = Rect::new(
				r.x + r.w * (1.0 - k) / 2.0,
				r.y + r.h * (1.0 - k) / 2.0,
				r.w * k,
				r.h * k,
			);
		}
		let radius = rr.h / 2.0;
		self.c.fill_round(rr, radius, bg.alpha(fade));
		if let Some(b) = border {
			self.c.stroke_round(rr, radius, 1.0, b.alpha(fade));
		}
		let style = self
			.style(14.0, 20.0, Weight::Medium, fg.alpha(fade))
			.align(Align::Center);
		let (tw, _) = self.c.g.measure(label, &style, 1000.0);
		let icon_w = if icon.is_some() || spin { 24.0 } else { 0.0 };
		let total = tw + icon_w;
		let left = rr.x + (rr.w - total) / 2.0;
		if spin {
			self.spinner(left, rr.y + (rr.h - 16.0) / 2.0, 16.0, fg.alpha(fade));
		} else if let Some(name) = icon {
			if name == "play" {
				self.c
					.icon_filled(name, left, rr.y + (rr.h - 16.0) / 2.0, 16.0, fg.alpha(fade));
			} else {
				self.c.icon(
					name,
					left,
					rr.y + (rr.h - 16.0) / 2.0,
					16.0,
					fg.alpha(fade),
					0.0,
				);
			}
		}
		self.c.text_mid(
			label,
			Rect::new(left + icon_w, rr.y, tw + 2.0, rr.h),
			&style.align(Align::Start),
		);
		if self.focused(id) {
			self.ring(r, radius);
		}
	}

	pub fn spinner(&mut self, x: f32, y: f32, size: f32, color: Color) {
		let angle = ((self.now % 1.0) * 360.0) as f32;
		self.c.icon("loader-circle", x, y, size, color, angle);
		self.animating = true;
	}

	/// Флажок 18 с подписью; возвращает правый край.
	pub fn checkbox(&mut self, id: &str, x: f32, y: f32, label: &str, checked: bool, w: f32) {
		let row = Rect::new(x, y, w, 24.0);
		self.region(id, row, Kind::Press, true, true);
		let b = Rect::new(x, y + 3.0, 18.0, 18.0);
		let p = self.p;
		if checked {
			self.c.fill_round(b, 5.0, p.primary);
			self.c.icon(
				"check",
				b.x + 2.0,
				b.y + 2.0,
				14.0,
				p.primary_foreground,
				0.0,
			);
		} else {
			self.c.fill_round(b, 5.0, p.field);
			self.c.stroke_round(
				b,
				5.0,
				1.0,
				if self.hovered(id) {
					p.foreground
				} else {
					p.input
				},
			);
		}
		let style = self.style(14.0, 20.0, Weight::Regular, p.foreground);
		self.c
			.text_mid(label, Rect::new(x + 28.0, y, w - 28.0, 24.0), &style);
		if self.focused(id) {
			self.ring(b, 5.0);
		}
	}

	/// Радио 18.
	pub fn radio_mark(&self, x: f32, y: f32, selected: bool, hover: bool) {
		let p = self.p;
		if selected {
			self.c.fill_circle(x + 9.0, y + 9.0, 9.0, p.primary);
			self.c.fill_circle(x + 9.0, y + 9.0, 4.0, p.field);
		} else {
			self.c.fill_circle(x + 9.0, y + 9.0, 9.0, p.field);
			self.c.stroke_circle(
				x + 9.0,
				y + 9.0,
				9.0,
				1.0,
				if hover { p.foreground } else { p.input },
			);
		}
	}

	/// Строка-вариант: радио, заголовок и пояснение. Выбранная — с фоном.
	pub fn choice(&mut self, id: &str, r: Rect, title: &str, text: &str, selected: bool) {
		self.region(id, r, Kind::Press, true, true);
		let p = self.p;
		let hover = self.hovered(id);
		if selected || hover {
			self.c.fill_round(
				r,
				12.0,
				if selected {
					p.accent
				} else {
					p.accent.alpha(0.6)
				},
			);
		}
		self.radio_mark(r.x + 14.0, r.y + 14.0, selected, hover);
		let title_style = self.style(14.0, 20.0, Weight::Medium, p.foreground);
		self.c.text(
			title,
			Rect::new(r.x + 44.0, r.y + 13.0, r.w - 56.0, 20.0),
			&title_style,
		);
		let hint = self.style(13.0, 18.0, Weight::Regular, p.muted_foreground);
		self.c.text(
			text,
			Rect::new(r.x + 44.0, r.y + 34.0, r.w - 56.0, 18.0),
			&hint,
		);
		if self.focused(id) {
			self.ring(r, 12.0);
		}
	}

	/// Сегменты «Русский | English».
	pub fn segments(
		&mut self,
		id: &str,
		x: f32,
		y: f32,
		items: &[&str],
		selected: usize,
		item_w: f32,
	) {
		let p = self.p;
		let group = Rect::new(x, y, item_w * items.len() as f32 + 6.0, 36.0);
		self.c.fill_round(group, 12.0, p.muted);
		self.c.stroke_round(group, 12.0, 1.0, p.border);
		for (i, label) in items.iter().enumerate() {
			let rid = format!("{id}:{i}");
			let r = Rect::new(x + 3.0 + i as f32 * item_w, y + 3.0, item_w, 30.0);
			self.region(&rid, r, Kind::Press, true, i == selected);
			let on = i == selected;
			if on {
				self.c.fill_round(r, 9.0, p.background);
				self.c.stroke_round(r, 9.0, 1.0, p.border);
			} else if self.hovered(&rid) {
				self.c.fill_round(r, 9.0, p.accent.alpha(0.6));
			}
			let style = self
				.style(
					14.0,
					20.0,
					Weight::Medium,
					if on { p.foreground } else { p.muted_foreground },
				)
				.align(Align::Center);
			self.c.text_mid(label, r, &style);
			if self.input.focus_visible && self.input.focus.as_deref() == Some(rid.as_str()) {
				self.c.stroke_round(r.inset(1.0), 8.0, 2.0, p.primary_text);
			}
		}
	}

	/// Полоса прогресса; `None` — неизвестный ход.
	pub fn progress(&mut self, r: Rect, value: Option<f32>) {
		let p = self.p;
		self.c.fill_round(r, r.h / 2.0, p.border);
		match value {
			Some(v) => {
				let w = (r.w * v.clamp(0.0, 1.0)).max(r.h);
				self.c
					.fill_round(Rect::new(r.x, r.y, w, r.h), r.h / 2.0, p.primary);
			}
			None => {
				let seg = r.w * 0.3;
				let phase = if self.reduced {
					0.35
				} else {
					(self.now % 2.4) / 1.2
				};
				let k = if phase <= 1.0 { phase } else { 2.0 - phase } as f32;
				let x = r.x + (r.w - seg) * k;
				self.c
					.fill_round(Rect::new(x, r.y, seg, r.h), r.h / 2.0, p.primary);
				self.animating = !self.reduced || self.animating;
			}
		}
	}

	/// Кнопки окна 46×40 в правом верхнем углу области `right`.
	pub fn window_buttons(&mut self, right: f32, h: f32, minimize_only: bool) {
		let p = self.p;
		let bw = if minimize_only { 40.0 } else { 46.0 };
		let close = Rect::new(right - bw, 0.0, bw, h);
		let min = if minimize_only {
			close
		} else {
			Rect::new(right - 2.0 * bw, 0.0, bw, h)
		};
		self.region("win:min", min, Kind::Press, true, false);
		if self.hovered("win:min") {
			self.c.fill(min, p.foreground.alpha(0.08));
		}
		let cx = min.x + min.w / 2.0;
		let cy = min.y + min.h / 2.0;
		self.c
			.line(cx - 5.0, cy + 0.5, cx + 5.0, cy + 0.5, 1.0, p.foreground);
		if !minimize_only {
			self.region("win:close", close, Kind::Press, true, false);
			let hover = self.hovered("win:close");
			if hover {
				self.c.fill(close, p.titlebar_close);
			}
			let ink = if hover {
				Color::hex(0xffffff)
			} else {
				p.foreground
			};
			let cx = close.x + close.w / 2.0;
			let cy = close.y + close.h / 2.0;
			self.c
				.line(cx - 5.0, cy - 5.0, cx + 5.0, cy + 5.0, 1.0, ink);
			self.c
				.line(cx - 5.0, cy + 5.0, cx + 5.0, cy - 5.0, 1.0, ink);
		}
	}

	/// Ссылка-кнопка цветом `primary_text`.
	pub fn link(
		&mut self,
		id: &str,
		x: f32,
		y: f32,
		label: &str,
		icon: Option<&str>,
		size: f32,
	) -> f32 {
		let p = self.p;
		let style = self.style(size, size + 5.0, Weight::Medium, p.primary_text);
		let (w, _) = self.c.g.measure(label, &style, 1000.0);
		let total = w + if icon.is_some() { 20.0 } else { 0.0 };
		let r = Rect::new(x, y, total, size + 5.0);
		self.region(id, r, Kind::Press, true, true);
		self.c
			.text(label, Rect::new(x, y, w + 2.0, size + 5.0), &style);
		if let Some(name) = icon {
			self.c.icon(
				name,
				x + w + 4.0,
				y + (size + 5.0 - 14.0) / 2.0,
				14.0,
				p.primary_text,
				0.0,
			);
		}
		if self.hovered(id) {
			self.c.line(
				x,
				y + size + 4.0,
				x + w,
				y + size + 4.0,
				1.0,
				p.primary_text,
			);
		}
		if self.focused(id) {
			self.ring(r, 4.0);
		}
		total
	}

	/// Блок ошибки: плитка, заголовок, текст, «Подробнее». Возвращает высоту.
	#[allow(clippy::too_many_arguments)]
	pub fn error_block(
		&mut self,
		x: f32,
		y: f32,
		w: f32,
		title: &str,
		text: &str,
		details: Option<&str>,
		open: bool,
		copied: bool,
	) -> f32 {
		let p = self.p;
		self.c
			.fill_round(Rect::new(x, y, 40.0, 40.0), 12.0, p.destructive.alpha(0.1));
		self.c
			.icon("octagon-x", x + 10.0, y + 10.0, 20.0, p.destructive, 0.0);
		let mut cy = y + 56.0;
		let h = self.c.text(
			title,
			Rect::new(x, cy, w, 26.0),
			&self.style(20.0, 26.0, Weight::SemiBold, p.foreground),
		);
		cy += h + 8.0;
		let h = self.c.text(
			text,
			Rect::new(x, cy, w, 200.0),
			&self
				.style(14.0, 22.0, Weight::Regular, p.muted_foreground)
				.wrap(),
		);
		cy += h + 12.0;
		if let Some(details) = details {
			let label = if open {
				self.t("common.hideDetails")
			} else {
				self.t("common.details")
			};
			let lw = self.link("details", x, cy, label, Some("chevron-down"), 13.0);
			let _ = lw;
			cy += 26.0;
			if open {
				let style = self
					.style(12.0, 18.0, Weight::Regular, p.foreground)
					.mono()
					.wrap();
				let (_, th) = self.c.g.measure(details, &style, w - 24.0);
				let box_h = th.min(90.0) + 20.0 + 24.0;
				let r = Rect::new(x, cy, w, box_h);
				self.c.fill_round(r, 12.0, p.muted);
				self.c.clip(Rect::new(r.x, r.y, r.w, r.h - 28.0));
				self.c.text(
					details,
					Rect::new(x + 12.0, cy + 10.0, w - 24.0, th),
					&style,
				);
				self.c.unclip();
				let copy = if copied {
					self.t("common.copied")
				} else {
					self.t("common.copy")
				};
				self.link("copy", x + 12.0, cy + box_h - 24.0, copy, None, 13.0);
				cy += box_h + 8.0;
			}
		}
		cy - y
	}

	/// Строка проверки системы.
	#[allow(clippy::too_many_arguments)]
	/// Строка проверки высотой `h`; `reserve` — место справа под кнопку действия.
	#[allow(clippy::too_many_arguments)]
	pub fn check_row(
		&mut self,
		r: Rect,
		icon: &str,
		title: &str,
		hint: &str,
		status: Status,
		reserve: f32,
	) {
		let p = self.p;
		let mid = r.y + (r.h - 1.0) / 2.0;
		self.c
			.icon(icon, r.x, mid - 10.0, 20.0, p.muted_foreground, 0.0);
		let text_w = r.w - 64.0 - reserve;
		self.c.text(
			title,
			Rect::new(r.x + 32.0, mid - 19.0, text_w, 20.0),
			&self.style(14.0, 20.0, Weight::Medium, p.foreground),
		);
		self.c.text(
			hint,
			Rect::new(r.x + 32.0, mid + 1.0, text_w, 18.0),
			&self.style(13.0, 18.0, Weight::Regular, p.muted_foreground),
		);
		let sx = r.right() - 20.0;
		let sy = mid - 10.0;
		match status {
			Status::Waiting => self
				.c
				.fill_circle(sx + 10.0, sy + 10.0, 3.0, p.control_track),
			Status::Running => self.spinner(sx, sy, 20.0, p.primary_text),
			Status::Ok => self.c.icon("circle-check", sx, sy, 20.0, p.success, 0.0),
			Status::Warn => self
				.c
				.icon("triangle-exclamation-point", sx, sy, 20.0, p.warning, 0.0),
			Status::Error => self.c.icon("octagon-x", sx, sy, 20.0, p.destructive, 0.0),
		}
		let line = r.bottom() - 0.5;
		self.c.line(r.x, line, r.right(), line, 1.0, p.border);
	}

	/// Подэтап: точка, спиннер или галочка и подпись.
	pub fn phase_row(&mut self, x: f32, y: f32, label: &str, status: Status) {
		let p = self.p;
		match status {
			Status::Waiting => self.c.fill_circle(x + 8.0, y + 10.0, 3.0, p.control_track),
			Status::Running => self.spinner(x, y + 2.0, 16.0, p.primary_text),
			Status::Error => self.c.icon("x", x, y + 2.0, 16.0, p.destructive, 0.0),
			_ => self.c.icon("check", x, y + 2.0, 16.0, p.success, 0.0),
		}
		self.c.text(
			label,
			Rect::new(x + 28.0, y, 420.0, 20.0),
			&self.style(14.0, 20.0, Weight::Regular, p.foreground),
		);
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
	Waiting,
	Running,
	Ok,
	Warn,
	Error,
}

/// Путь с многоточием посередине, чтобы влез в `max` символов.
pub fn middle_ellipsis(path: &str, max: usize) -> String {
	let chars: Vec<char> = path.chars().collect();
	if chars.len() <= max || max < 8 {
		return path.to_owned();
	}
	let tail = max * 3 / 5;
	let head = max - tail - 1;
	let mut s: String = chars[..head].iter().collect();
	s.push('…');
	s.extend(&chars[chars.len() - tail..]);
	s
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn shortens_paths_in_the_middle() {
		assert_eq!(middle_ellipsis("C:\\a", 20), "C:\\a");
		let s = middle_ellipsis(
			"C:\\Users\\someone\\AppData\\Local\\Programs\\LeeLauncher",
			30,
		);
		assert_eq!(s.chars().count(), 30);
		assert!(s.starts_with("C:\\Users"));
		assert!(s.ends_with("LeeLauncher"));
		assert!(s.contains('…'));
	}
}
