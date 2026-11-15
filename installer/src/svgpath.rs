//! Разбор атрибута `d` у SVG-контуров в абсолютные команды.

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cmd {
	Move(f32, f32),
	Line(f32, f32),
	Cubic(f32, f32, f32, f32, f32, f32),
	/// rx, ry, поворот, большая дуга, по часовой, x, y.
	Arc(f32, f32, f32, bool, bool, f32, f32),
	Close,
}

struct Lexer<'a> {
	s: &'a [u8],
	i: usize,
}

impl Lexer<'_> {
	fn skip(&mut self) {
		while self.i < self.s.len()
			&& (self.s[self.i].is_ascii_whitespace() || self.s[self.i] == b',')
		{
			self.i += 1;
		}
	}

	fn command(&mut self) -> Option<u8> {
		self.skip();
		let c = *self.s.get(self.i)?;
		if c.is_ascii_alphabetic() {
			self.i += 1;
			Some(c)
		} else {
			None
		}
	}

	fn has_number(&mut self) -> bool {
		self.skip();
		matches!(self.s.get(self.i), Some(c) if c.is_ascii_digit() || matches!(c, b'-' | b'+' | b'.'))
	}

	fn number(&mut self) -> Option<f32> {
		self.skip();
		let start = self.i;
		let mut seen_dot = false;
		let mut seen_exp = false;
		if matches!(self.s.get(self.i), Some(b'-' | b'+')) {
			self.i += 1;
		}
		while let Some(&c) = self.s.get(self.i) {
			match c {
				b'0'..=b'9' => self.i += 1,
				b'.' if !seen_dot && !seen_exp => {
					seen_dot = true;
					self.i += 1;
				}
				b'e' | b'E' if !seen_exp => {
					seen_exp = true;
					self.i += 1;
					if matches!(self.s.get(self.i), Some(b'-' | b'+')) {
						self.i += 1;
					}
				}
				_ => break,
			}
		}
		std::str::from_utf8(&self.s[start..self.i])
			.ok()?
			.parse()
			.ok()
	}

	/// Флаг дуги: «0» или «1», можно без разделителя.
	fn flag(&mut self) -> Option<bool> {
		self.skip();
		let c = *self.s.get(self.i)?;
		self.i += 1;
		match c {
			b'0' => Some(false),
			b'1' => Some(true),
			_ => None,
		}
	}
}

pub fn parse(d: &str) -> Vec<Cmd> {
	let mut lx = Lexer {
		s: d.as_bytes(),
		i: 0,
	};
	let mut out = Vec::new();
	let (mut x, mut y) = (0.0f32, 0.0f32);
	let (mut sx, mut sy) = (0.0f32, 0.0f32);
	let mut last_ctrl: Option<(f32, f32)> = None;
	let mut last_quad: Option<(f32, f32)> = None;
	let mut cmd = b'M';
	loop {
		if let Some(c) = lx.command() {
			cmd = c;
		} else if !lx.has_number() {
			break;
		}
		let rel = cmd.is_ascii_lowercase();
		let (ox, oy) = if rel { (x, y) } else { (0.0, 0.0) };
		let up = cmd.to_ascii_uppercase();
		let mut ctrl = None;
		let mut quad = None;
		let ok = (|| -> Option<()> {
			match up {
				b'M' => {
					x = ox + lx.number()?;
					y = oy + lx.number()?;
					sx = x;
					sy = y;
					out.push(Cmd::Move(x, y));
					// Следующие пары после M — это L.
					cmd = if rel { b'l' } else { b'L' };
				}
				b'L' => {
					x = ox + lx.number()?;
					y = oy + lx.number()?;
					out.push(Cmd::Line(x, y));
				}
				b'H' => {
					x = ox + lx.number()?;
					out.push(Cmd::Line(x, y));
				}
				b'V' => {
					y = oy + lx.number()?;
					out.push(Cmd::Line(x, y));
				}
				b'C' => {
					let (x1, y1) = (ox + lx.number()?, oy + lx.number()?);
					let (x2, y2) = (ox + lx.number()?, oy + lx.number()?);
					x = ox + lx.number()?;
					y = oy + lx.number()?;
					out.push(Cmd::Cubic(x1, y1, x2, y2, x, y));
					ctrl = Some((x2, y2));
				}
				b'S' => {
					let (x1, y1) = match last_ctrl {
						Some((cx, cy)) => (2.0 * x - cx, 2.0 * y - cy),
						None => (x, y),
					};
					let (x2, y2) = (ox + lx.number()?, oy + lx.number()?);
					x = ox + lx.number()?;
					y = oy + lx.number()?;
					out.push(Cmd::Cubic(x1, y1, x2, y2, x, y));
					ctrl = Some((x2, y2));
				}
				b'Q' | b'T' => {
					let (qx, qy) = if up == b'Q' {
						(ox + lx.number()?, oy + lx.number()?)
					} else {
						match last_quad {
							Some((cx, cy)) => (2.0 * x - cx, 2.0 * y - cy),
							None => (x, y),
						}
					};
					let (nx, ny) = (ox + lx.number()?, oy + lx.number()?);
					out.push(Cmd::Cubic(
						x + 2.0 / 3.0 * (qx - x),
						y + 2.0 / 3.0 * (qy - y),
						nx + 2.0 / 3.0 * (qx - nx),
						ny + 2.0 / 3.0 * (qy - ny),
						nx,
						ny,
					));
					x = nx;
					y = ny;
					quad = Some((qx, qy));
				}
				b'A' => {
					let rx = lx.number()?;
					let ry = lx.number()?;
					let rot = lx.number()?;
					let large = lx.flag()?;
					let sweep = lx.flag()?;
					x = ox + lx.number()?;
					y = oy + lx.number()?;
					out.push(Cmd::Arc(rx, ry, rot, large, sweep, x, y));
				}
				b'Z' => {
					x = sx;
					y = sy;
					out.push(Cmd::Close);
				}
				_ => return None,
			}
			Some(())
		})();
		if ok.is_none() {
			break;
		}
		last_ctrl = ctrl;
		last_quad = quad;
		// После Z числа без команды не бывают: иначе цикл не сдвинется.
		if up == b'Z' && lx.has_number() {
			break;
		}
	}
	out
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parses_relative_and_implicit() {
		let c = parse("m16 9-5.5 5.5L8 12");
		assert_eq!(
			c,
			vec![
				Cmd::Move(16.0, 9.0),
				Cmd::Line(10.5, 14.5),
				Cmd::Line(8.0, 12.0)
			]
		);
	}

	#[test]
	fn parses_arcs_with_packed_flags() {
		let c = parse("M2 12a10 10 0 1 0 20 0z");
		assert_eq!(c[1], Cmd::Arc(10.0, 10.0, 0.0, true, false, 22.0, 12.0));
		assert_eq!(c[2], Cmd::Close);
		let c = parse("M0 0a1 1 0 0119 9");
		assert_eq!(c[1], Cmd::Arc(1.0, 1.0, 0.0, false, true, 19.0, 9.0));
	}

	#[test]
	fn parses_hv_and_exponent() {
		let c = parse("M1 1H5V-2e1h1");
		assert_eq!(
			c,
			vec![
				Cmd::Move(1.0, 1.0),
				Cmd::Line(5.0, 1.0),
				Cmd::Line(5.0, -20.0),
				Cmd::Line(6.0, -20.0)
			]
		);
	}
}
