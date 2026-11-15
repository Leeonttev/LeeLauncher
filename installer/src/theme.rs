//! Цвета светлой и тёмной темы — те же, что у лаунчера.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
	pub r: f32,
	pub g: f32,
	pub b: f32,
	pub a: f32,
}

impl Color {
	pub const fn hex(v: u32) -> Color {
		Color {
			r: ((v >> 16) & 0xff) as f32 / 255.0,
			g: ((v >> 8) & 0xff) as f32 / 255.0,
			b: (v & 0xff) as f32 / 255.0,
			a: 1.0,
		}
	}

	pub fn alpha(self, a: f32) -> Color {
		Color {
			a: self.a * a,
			..self
		}
	}

	/// Смесь `self` с `other`: `k` — доля `self`.
	pub fn mix(self, other: Color, k: f32) -> Color {
		Color {
			r: self.r * k + other.r * (1.0 - k),
			g: self.g * k + other.g * (1.0 - k),
			b: self.b * k + other.b * (1.0 - k),
			a: 1.0,
		}
	}
}

#[derive(Clone, Copy, Debug)]
pub struct Palette {
	pub dark: bool,
	pub background: Color,
	pub foreground: Color,
	pub popover: Color,
	pub popover_border: Color,
	pub primary: Color,
	pub primary_foreground: Color,
	pub primary_text: Color,
	pub muted: Color,
	pub muted_foreground: Color,
	pub accent: Color,
	pub destructive: Color,
	pub warning: Color,
	pub success: Color,
	pub border: Color,
	pub input: Color,
	pub control_track: Color,
	pub field: Color,
	pub sidebar: Color,
	pub titlebar_close: Color,
	pub brand: Color,
	pub brand_icon: Color,
	pub brand_tile: Color,
	pub brand_mark: Color,
}

impl Palette {
	pub const LIGHT: Palette = Palette {
		dark: false,
		background: Color::hex(0xffffff),
		foreground: Color::hex(0x1d1d1f),
		popover: Color::hex(0xffffff),
		popover_border: Color::hex(0xd2d2d7),
		primary: Color::hex(0x028ecc),
		primary_foreground: Color::hex(0x1d1d1f),
		primary_text: Color::hex(0x016799),
		muted: Color::hex(0xf5f5f7),
		muted_foreground: Color::hex(0x68686d),
		accent: Color::hex(0xececf0),
		destructive: Color::hex(0xc4261d),
		warning: Color::hex(0x8a5806),
		success: Color::hex(0x177534),
		border: Color::hex(0xe8e8ed),
		input: Color::hex(0x8e8e93),
		control_track: Color::hex(0xc7c7cc),
		field: Color::hex(0xffffff),
		sidebar: Color::hex(0xf5f5f7),
		titlebar_close: Color::hex(0xc42b1c),
		brand: Color::hex(0x028ecc),
		brand_icon: Color::hex(0x32ade6),
		brand_tile: Color::hex(0x1d1d1f),
		brand_mark: Color::hex(0xffffff),
	};

	pub const DARK: Palette = Palette {
		dark: true,
		background: Color::hex(0x18181b),
		foreground: Color::hex(0xf5f5f7),
		popover: Color::hex(0x29292d),
		popover_border: Color::hex(0x3e3e43),
		primary: Color::hex(0x32ade6),
		primary_foreground: Color::hex(0x1d1d1f),
		primary_text: Color::hex(0x32ade6),
		muted: Color::hex(0x222225),
		muted_foreground: Color::hex(0xa1a1a6),
		accent: Color::hex(0x313135),
		destructive: Color::hex(0xff6b62),
		warning: Color::hex(0xe0b04a),
		success: Color::hex(0x3fb950),
		border: Color::hex(0x323236),
		input: Color::hex(0x737378),
		control_track: Color::hex(0x5a5a5f),
		field: Color::hex(0x131315),
		sidebar: Color::hex(0x111113),
		titlebar_close: Color::hex(0xc42b1c),
		brand: Color::hex(0x32ade6),
		brand_icon: Color::hex(0x32ade6),
		brand_tile: Color::hex(0x1d1d1f),
		brand_mark: Color::hex(0xffffff),
	};

	pub fn get(dark: bool) -> &'static Palette {
		if dark {
			&Palette::DARK
		} else {
			&Palette::LIGHT
		}
	}
}
