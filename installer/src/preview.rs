//! Снимки экранов в PNG без окна: Direct2D рисует в картинку WIC. Систему не меняет.

use std::path::{Path, PathBuf};

use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::core::{GUID, HSTRING, Result};

use crate::app::{App, Failure, Mode, Step, Un, Wv};
use crate::gfx::{Canvas, Gfx};
use crate::sys::Existing;
use crate::text::Lang;
use crate::theme::Palette;
use crate::ui::{Input, Status, Ui};

fn state(name: &str, lang: Lang) -> App {
	let mut a = App::new(Mode::Wizard, lang, None);
	a.dir = PathBuf::from(r"C:\Users\User\AppData\Local\Programs\LeeLauncher");
	a.free = Some(84 * 1024 * 1024 * 1024);
	let ok = |h: &str| (Status::Ok, h.to_owned());
	match name {
		"welcome" => {}
		"existing" => {
			a.existing = Some(Existing {
				dir: a.dir.clone(),
				version: "0.0.9".into(),
			});
		}
		"folder" => a.step = Step::Folder,
		"folder-error" => {
			a.step = Step::Folder;
			a.dir = PathBuf::from(r"C:\Users\User\AppData\Local\LeeLauncher");
			a.dir_error = Some("folder.errData");
		}
		"checks" => {
			a.step = Step::Checks;
			a.checks_running = true;
			a.checks = vec![
				ok("Windows 11 (26100), 64-бит"),
				ok("Свободно 84 ГБ, нужно 200 МБ"),
				(Status::Running, "Проверяем…".into()),
				(Status::Waiting, String::new()),
				(Status::Waiting, String::new()),
			];
		}
		"checks-problems" => {
			a.step = Step::Checks;
			a.need_webview = true;
			a.checks = vec![
				ok("Windows 10 (19045), 64-бит"),
				ok("Свободно 84 ГБ, нужно 200 МБ"),
				ok("Можно записывать файлы"),
				(
					Status::Warn,
					"Не найден. Установим на следующем шаге".into(),
				),
				(
					Status::Error,
					"LeeLauncher открыт. Закройте его, чтобы продолжить".into(),
				),
				ok("Есть доступ к Microsoft"),
			];
		}
		"webview" => {
			a.step = Step::WebView;
			a.need_webview = true;
		}
		"webview-running" => {
			a.step = Step::WebView;
			a.need_webview = true;
			a.wv = Wv::Running(-42.0);
		}
		"webview-error" => {
			a.step = Step::WebView;
			a.need_webview = true;
			a.wv = Wv::Error("exit code 0x80072ee7".into());
			a.details_open = true;
		}
		"install" => {
			a.step = Step::Install;
			a.progress = 0.42;
			a.phases = [Status::Running, Status::Waiting, Status::Waiting];
		}
		"install-error" => {
			a.step = Step::Install;
			a.failure = Some(Failure {
				title: "err.copyTitle",
				text: crate::text::t(lang, "err.copyText").into(),
				details: r"замена C:\Users\User\AppData\Local\Programs\LeeLauncher\LeeLauncher.exe: Отказано в доступе. (os error 5)"
					.into(),
				second: Some(("otherFolder", "checks.otherFolder")),
			});
			a.details_open = true;
		}
		"test" => {
			a.step = Step::Test;
			a.test = [Status::Ok, Status::Running, Status::Waiting];
			a.test_hint = ["Ответила за 1,8 с".into(), String::new(), String::new()];
		}
		"test-error" => {
			a.step = Step::Test;
			a.failure = Some(Failure {
				title: "err.testTitle",
				text: crate::text::t(lang, "err.testText").into(),
				details: "exit code 2\nerror=timeout".into(),
				second: Some(("logs", "err.logs")),
			});
		}
		"done" => a.step = Step::Done,
		"cancel" => {
			a.step = Step::Install;
			a.progress = 0.7;
			a.phases = [Status::Ok, Status::Running, Status::Waiting];
			a.modal = Some(crate::app::Modal::Cancel);
		}
		"uninstall" => {
			a.mode = Mode::Uninstall;
			a.data_size = Some(3_400_000_000);
		}
		"uninstall-data" => {
			a.mode = Mode::Uninstall;
			a.data_size = Some(3_400_000_000);
			a.delete_data = true;
		}
		"uninstall-done" => {
			a.mode = Mode::Uninstall;
			a.un = Un::Done;
		}
		"silent" => {
			a.mode = Mode::Silent;
			a.progress = 0.64;
		}
		"silent-error" => {
			a.mode = Mode::Silent;
			a.silent_error = Some("self-test 2".into());
		}
		_ => {}
	}
	a
}

pub const SCREENS: &[&str] = &[
	"welcome",
	"existing",
	"folder",
	"folder-error",
	"checks",
	"checks-problems",
	"webview",
	"webview-running",
	"webview-error",
	"install",
	"install-error",
	"cancel",
	"test",
	"test-error",
	"done",
	"uninstall",
	"uninstall-data",
	"uninstall-done",
	"silent",
	"silent-error",
];

fn render(
	gfx: &Gfx,
	wic: &IWICImagingFactory,
	app: &mut App,
	dark: bool,
	scale: f32,
	out: &Path,
) -> Result<()> {
	let (w, h) = app.size();
	let (pw, ph) = ((w * scale).round() as u32, (h * scale).round() as u32);
	unsafe {
		let bitmap =
			wic.CreateBitmap(pw, ph, &GUID_WICPixelFormat32bppPBGRA, WICBitmapCacheOnLoad)?;
		let props = D2D1_RENDER_TARGET_PROPERTIES {
			pixelFormat: D2D1_PIXEL_FORMAT {
				format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
				alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
			},
			dpiX: 96.0 * scale,
			dpiY: 96.0 * scale,
			..Default::default()
		};
		let rt = gfx.d2d.CreateWicBitmapRenderTarget(&bitmap, &props)?;
		rt.BeginDraw();
		let input = Input::default();
		let mut ui = Ui {
			c: Canvas::new(&rt, gfx),
			p: Palette::get(dark),
			lang: app.lang,
			input: &input,
			now: 0.3,
			reduced: true,
			regions: Vec::new(),
			focusable: Vec::new(),
			animating: false,
			win10: false,
		};
		app.paint(&mut ui);
		rt.EndDraw(None, None)?;

		let stream = wic.CreateStream()?;
		stream.InitializeFromFilename(&HSTRING::from(out.as_os_str()), 0x4000_0000)?;
		let encoder = wic.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
		encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
		let mut frame = None;
		encoder.CreateNewFrame(&mut frame, std::ptr::null_mut())?;
		let frame = frame.unwrap();
		frame.Initialize(None)?;
		frame.SetSize(pw, ph)?;
		let mut format: GUID = GUID_WICPixelFormat32bppBGRA;
		frame.SetPixelFormat(&mut format)?;
		frame.WriteSource(&bitmap, std::ptr::null())?;
		frame.Commit()?;
		encoder.Commit()?;
	}
	Ok(())
}

/// Все экраны в обеих темах (×1), часть — ещё на английском и в ×1,5.
pub fn run(dir: &Path) -> Result<Vec<PathBuf>> {
	crate::sys::com_init();
	std::fs::create_dir_all(dir).ok();
	let gfx = Gfx::new()?;
	let wic: IWICImagingFactory =
		unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)? };
	let mut out = Vec::new();
	for name in SCREENS {
		for dark in [false, true] {
			let path = dir.join(format!(
				"{name}-{}.png",
				if dark { "dark" } else { "light" }
			));
			render(&gfx, &wic, &mut state(name, Lang::Ru), dark, 1.0, &path)?;
			out.push(path);
		}
	}
	for name in [
		"welcome",
		"checks-problems",
		"done",
		"uninstall-data",
		"silent",
	] {
		let path = dir.join(format!("{name}-en-light.png"));
		render(&gfx, &wic, &mut state(name, Lang::En), false, 1.0, &path)?;
		out.push(path);
	}
	for name in ["welcome", "folder", "checks-problems"] {
		for dark in [false, true] {
			let path = dir.join(format!(
				"{name}-150-{}.png",
				if dark { "dark" } else { "light" }
			));
			render(&gfx, &wic, &mut state(name, Lang::Ru), dark, 1.5, &path)?;
			out.push(path);
		}
	}
	Ok(out)
}
