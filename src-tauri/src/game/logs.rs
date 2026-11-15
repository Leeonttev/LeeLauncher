//! Лог игры: разбор вывода (XML log4j или обычные строки), кольцевой буфер последних строк.

use std::collections::VecDeque;

use chrono::{Local, TimeZone};
use serde::Serialize;

/// Сколько последних строк лога хранится в памяти для `instance_log`.
pub const BUFFER_LINES: usize = 10_000;
/// Строка длиннее — обрезается (защита от «строки» на сотни мегабайт без переводов строки).
const MAX_LINE_BYTES: usize = 64 * 1024;
/// XML-событие длиннее стольких строк считаем сломанным и выводим как есть.
const MAX_EVENT_LINES: usize = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
	Info,
	Warn,
	Error,
	Debug,
}

impl Level {
	fn parse(text: &str) -> Option<Self> {
		match text.trim().to_ascii_uppercase().as_str() {
			"INFO" => Some(Self::Info),
			"WARN" | "WARNING" => Some(Self::Warn),
			"ERROR" | "FATAL" | "SEVERE" => Some(Self::Error),
			"DEBUG" | "TRACE" | "FINE" => Some(Self::Debug),
			_ => None,
		}
	}

	fn label(self) -> &'static str {
		match self {
			Self::Info => "INFO",
			Self::Warn => "WARN",
			Self::Error => "ERROR",
			Self::Debug => "DEBUG",
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stream {
	Stdout,
	Stderr,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LogLine {
	/// Местное время `ЧЧ:ММ:СС`.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub time: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub level: Option<Level>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub thread: Option<String>,
	/// Текст. У события log4j со стеком ошибки — несколько строк через `\n`.
	pub text: String,
	pub stream: Stream,
}

impl LogLine {
	pub fn plain(text: String, stream: Stream) -> Self {
		parse_plain(&text, stream).unwrap_or(Self {
			time: None,
			level: None,
			thread: None,
			text,
			stream,
		})
	}

	/// Строка для файла `launcher-output.log`.
	pub fn format(&self) -> String {
		let mut out = String::new();
		if let Some(time) = &self.time {
			out.push_str(&format!("[{time}] "));
		}
		match (&self.thread, self.level) {
			(Some(thread), Some(level)) => out.push_str(&format!("[{thread}/{}]: ", level.label())),
			(None, Some(level)) => out.push_str(&format!("[{}]: ", level.label())),
			(Some(thread), None) => out.push_str(&format!("[{thread}]: ")),
			(None, None) => {}
		}
		out.push_str(&self.text);
		out
	}
}

/// Обычная строка log4j/Fabric: `[12:34:56] [Render thread/INFO]: текст`.
fn parse_plain(line: &str, stream: Stream) -> Option<LogLine> {
	let rest = line.strip_prefix('[')?;
	let (time, rest) = rest.split_once("] [")?;
	if time.len() != 8 || !time.chars().all(|c| c.is_ascii_digit() || c == ':') {
		return None;
	}
	let (head, text) = rest.split_once("]: ")?;
	let (thread, level) = head.rsplit_once('/')?;
	Some(LogLine {
		time: Some(time.to_owned()),
		level: Some(Level::parse(level)?),
		thread: Some(thread.to_owned()),
		text: text.to_owned(),
		stream,
	})
}

fn local_time(millis: i64) -> Option<String> {
	Local
		.timestamp_millis_opt(millis)
		.single()
		.map(|t| t.format("%H:%M:%S").to_string())
}

/// Значение атрибута `name="…"` в открывающем теге.
fn attribute(tag: &str, name: &str) -> Option<String> {
	let pattern = format!(" {name}=\"");
	let start = tag.find(&pattern)? + pattern.len();
	let end = tag[start..].find('"')? + start;
	Some(unescape(&tag[start..end]))
}

fn unescape(text: &str) -> String {
	if !text.contains('&') {
		return text.to_owned();
	}
	let mut out = String::with_capacity(text.len());
	let mut rest = text;
	while let Some(i) = rest.find('&') {
		out.push_str(&rest[..i]);
		let tail = &rest[i..];
		let Some(end) = tail.find(';').filter(|&e| e <= 10) else {
			out.push('&');
			rest = &tail[1..];
			continue;
		};
		let entity = &tail[1..end];
		let decoded = match entity {
			"lt" => Some('<'),
			"gt" => Some('>'),
			"amp" => Some('&'),
			"quot" => Some('"'),
			"apos" => Some('\''),
			_ => entity
				.strip_prefix("#x")
				.and_then(|h| u32::from_str_radix(h, 16).ok())
				.or_else(|| entity.strip_prefix('#').and_then(|d| d.parse().ok()))
				.and_then(char::from_u32),
		};
		match decoded {
			Some(c) => {
				out.push(c);
				rest = &tail[end + 1..];
			}
			None => {
				out.push('&');
				rest = &tail[1..];
			}
		}
	}
	out.push_str(rest);
	out
}

/// Содержимое элемента `<tag>…</tag>`: секции CDATA как есть, остальное с разбором сущностей.
fn element(xml: &str, tag: &str) -> Option<String> {
	let open = format!("<{tag}>");
	let close = format!("</{tag}>");
	let start = xml.find(&open)? + open.len();
	let end = xml[start..].find(&close)? + start;
	let mut body = &xml[start..end];
	let mut out = String::new();
	while let Some(i) = body.find("<![CDATA[") {
		out.push_str(&unescape(&body[..i]));
		let after = &body[i + 9..];
		match after.find("]]>") {
			Some(e) => {
				out.push_str(&after[..e]);
				body = &after[e + 3..];
			}
			None => {
				out.push_str(after);
				body = "";
			}
		}
	}
	out.push_str(&unescape(body));
	Some(out)
}

/// Событие log4j `XMLLayout` целиком (от `<log4j:Event` до `</log4j:Event>`).
fn parse_event(xml: &str) -> Option<LogLine> {
	let tag_end = xml.find('>')?;
	let tag = &xml[..tag_end];
	let mut text = element(xml, "log4j:Message").unwrap_or_default();
	if let Some(throwable) = element(xml, "log4j:Throwable") {
		let throwable = throwable.trim_end();
		if !throwable.is_empty() {
			if !text.is_empty() {
				text.push('\n');
			}
			text.push_str(throwable);
		}
	}
	Some(LogLine {
		time: attribute(tag, "timestamp")
			.and_then(|t| t.parse().ok())
			.and_then(local_time),
		level: attribute(tag, "level").and_then(|l| Level::parse(&l)),
		thread: attribute(tag, "thread"),
		text: text.trim_end().to_owned(),
		stream: Stream::Stdout,
	})
}

/// Построчный разбор вывода: XML-события log4j вперемешку с обычными строками.
/// Сломанный XML выводится как есть.
#[derive(Default)]
pub struct Parser {
	event: Option<Vec<String>>,
}

impl Parser {
	pub fn push(&mut self, line: String, stream: Stream) -> Vec<LogLine> {
		let line = truncate(line);
		if stream == Stream::Stderr {
			return vec![LogLine::plain(line, stream)];
		}
		if let Some(event) = &mut self.event {
			let done = line.contains("</log4j:Event>");
			event.push(line);
			if done || event.len() >= MAX_EVENT_LINES {
				return self.finish_event();
			}
			return Vec::new();
		}
		let trimmed = line.trim_start();
		if trimmed.starts_with("<log4j:Event") {
			let done = trimmed.contains("</log4j:Event>");
			self.event = Some(vec![line]);
			if done {
				return self.finish_event();
			}
			return Vec::new();
		}
		if line.trim().is_empty() {
			return Vec::new();
		}
		vec![LogLine::plain(line, stream)]
	}

	/// Конец вывода: недописанное событие — как есть.
	pub fn finish(&mut self) -> Vec<LogLine> {
		match self.event.take() {
			Some(lines) => lines
				.into_iter()
				.filter(|l| !l.trim().is_empty())
				.map(|l| LogLine::plain(l, Stream::Stdout))
				.collect(),
			None => Vec::new(),
		}
	}

	fn finish_event(&mut self) -> Vec<LogLine> {
		let Some(lines) = self.event.take() else {
			return Vec::new();
		};
		let xml = lines.join("\n");
		if xml.contains("</log4j:Event>")
			&& let Some(event) = parse_event(xml.trim())
		{
			return vec![event];
		}
		lines
			.into_iter()
			.filter(|l| !l.trim().is_empty())
			.map(|l| LogLine::plain(l, Stream::Stdout))
			.collect()
	}
}

fn truncate(mut line: String) -> String {
	if line.len() > MAX_LINE_BYTES {
		let mut cut = MAX_LINE_BYTES;
		while !line.is_char_boundary(cut) {
			cut -= 1;
		}
		line.truncate(cut);
		line.push('…');
	}
	line
}

/// Последние [`BUFFER_LINES`] строк лога.
#[derive(Default)]
pub struct Buffer {
	lines: VecDeque<LogLine>,
}

impl Buffer {
	pub fn push(&mut self, line: LogLine) {
		if self.lines.len() >= BUFFER_LINES {
			self.lines.pop_front();
		}
		self.lines.push_back(line);
	}

	pub fn lines(&self) -> Vec<LogLine> {
		self.lines.iter().cloned().collect()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn feed(parser: &mut Parser, text: &str) -> Vec<LogLine> {
		text.lines()
			.flat_map(|l| parser.push(l.to_owned(), Stream::Stdout))
			.collect()
	}

	#[test]
	fn parses_log4j_xml() {
		let mut parser = Parser::default();
		let out = feed(
			&mut parser,
			r#"<log4j:Event logger="net.minecraft.client.Minecraft" timestamp="1700000000000" level="INFO" thread="Render thread">
  <log4j:Message><![CDATA[Setting user: Steve]]></log4j:Message>
</log4j:Event>
[LWJGL] plain line from System.out
<log4j:Event logger="x" timestamp="1700000000500" level="ERROR" thread="Worker &amp; 1">
  <log4j:Message><![CDATA[Boom <here>]]></log4j:Message>
  <log4j:Throwable><![CDATA[java.lang.RuntimeException: x
	at a.b(C.java:1)
]]></log4j:Throwable>
</log4j:Event>"#,
		);
		assert_eq!(out.len(), 3);
		assert_eq!(out[0].text, "Setting user: Steve");
		assert_eq!(out[0].level, Some(Level::Info));
		assert_eq!(out[0].thread.as_deref(), Some("Render thread"));
		assert_eq!(out[0].time.as_ref().map(String::len), Some(8));
		assert_eq!(out[1].text, "[LWJGL] plain line from System.out");
		assert_eq!(out[1].level, None);
		assert_eq!(out[2].level, Some(Level::Error));
		assert_eq!(out[2].thread.as_deref(), Some("Worker & 1"));
		assert_eq!(
			out[2].text,
			"Boom <here>\njava.lang.RuntimeException: x\n\tat a.b(C.java:1)"
		);
		assert!(parser.finish().is_empty());
	}

	#[test]
	fn broken_xml_is_not_lost() {
		let mut parser = Parser::default();
		assert!(feed(&mut parser, "<log4j:Event level=\"INFO\">\n  half").is_empty());
		let rest = parser.finish();
		assert_eq!(rest.len(), 2);
		assert_eq!(rest[1].text, "  half");
	}

	#[test]
	fn parses_plain_lines() {
		let line = LogLine::plain(
			"[12:34:56] [Render thread/WARN]: Missing sound".into(),
			Stream::Stdout,
		);
		assert_eq!(line.time.as_deref(), Some("12:34:56"));
		assert_eq!(line.level, Some(Level::Warn));
		assert_eq!(line.thread.as_deref(), Some("Render thread"));
		assert_eq!(line.text, "Missing sound");
		assert_eq!(
			line.format(),
			"[12:34:56] [Render thread/WARN]: Missing sound"
		);

		let raw = LogLine::plain("Exception in thread \"main\"".into(), Stream::Stderr);
		assert_eq!(raw.level, None);
		assert_eq!(raw.text, "Exception in thread \"main\"");
		assert_eq!(raw.format(), "Exception in thread \"main\"");
	}

	#[test]
	fn unescapes_entities() {
		assert_eq!(
			unescape("a &lt;b&gt; &#1046; &#x41; & c &bogus;"),
			"a <b> Ж A & c &bogus;"
		);
	}

	#[test]
	fn buffer_keeps_last_lines() {
		let mut buffer = Buffer::default();
		for i in 0..BUFFER_LINES + 5 {
			buffer.push(LogLine::plain(i.to_string(), Stream::Stdout));
		}
		let lines = buffer.lines();
		assert_eq!(lines.len(), BUFFER_LINES);
		assert_eq!(lines[0].text, "5");
	}
}
