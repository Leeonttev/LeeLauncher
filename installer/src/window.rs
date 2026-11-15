//! Окно Win32 без системной рамки, Direct2D-отрисовка и ввод.

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::time::Instant;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::{
	BeginPaint, EndPaint, GetMonitorInfoW, InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO,
	MonitorFromPoint, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{MARGINS, WM_MOUSELEAVE};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, GetDpiForWindow, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, Interface, w};

use crate::app::{App, Event, Spawner};
use crate::gfx::{Canvas, Gfx};
use crate::sys;
use crate::theme::Palette;
use crate::ui::{Input, Kind, Region, Ui};

const WM_APP_EVENT: u32 = WM_APP + 1;
const TIMER: usize = 1;

struct State {
	app: App,
	gfx: Gfx,
	rt: Option<ID2D1HwndRenderTarget>,
	input: Input,
	regions: Vec<Region>,
	focusable: Vec<String>,
	scale: f32,
	start: Instant,
	dark: bool,
	reduced: bool,
	win10: bool,
	rx: Receiver<Event>,
	size: (f32, f32),
}

thread_local! {
	static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Состояние окна. Во время вложенных сообщений (системное окно выбора папки, смена размера)
/// оно уже занято — тогда сообщение уходит в `DefWindowProcW`.
fn with<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
	STATE.with(|s| s.try_borrow_mut().ok()?.as_mut().map(f))
}

pub fn reduced_motion() -> bool {
	let mut on = BOOL(1);
	unsafe {
		let _ = SystemParametersInfoW(
			SPI_GETCLIENTAREAANIMATION,
			0,
			Some((&mut on as *mut BOOL).cast()),
			SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
		);
	}
	!on.as_bool()
}

/// Запускает окно и цикл сообщений. Возвращает, когда окно закрыто.
pub fn run(make: impl FnOnce(Spawner) -> App) -> windows::core::Result<()> {
	let (tx, rx) = channel::<Event>();
	let hwnd_cell = Arc::new(AtomicIsize::new(0));
	let hwnd_for_jobs = hwnd_cell.clone();
	let spawner: Spawner = Arc::new(move |job| {
		let tx = tx.clone();
		let hwnd = hwnd_for_jobs.clone();
		std::thread::spawn(move || {
			let send: crate::app::Send = Arc::new(move |e| {
				let _ = tx.send(e);
				let h = hwnd.load(Ordering::SeqCst);
				if h != 0 {
					unsafe {
						let _ =
							PostMessageW(Some(HWND(h as _)), WM_APP_EVENT, WPARAM(0), LPARAM(0));
					}
				}
			});
			job(send);
		});
	});
	let app = make(spawner);
	let gfx = Gfx::new()?;
	unsafe {
		let instance = GetModuleHandleW(None)?;
		let class = WNDCLASSEXW {
			cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
			style: CS_HREDRAW | CS_VREDRAW,
			lpfnWndProc: Some(wndproc),
			hInstance: instance.into(),
			hIcon: LoadIconW(Some(instance.into()), windows::core::PCWSTR(1 as _))
				.unwrap_or_default(),
			hCursor: LoadCursorW(None, IDC_ARROW)?,
			lpszClassName: w!("LeeLauncherSetup"),
			..Default::default()
		};
		RegisterClassExW(&class);

		// Окно появляется на мониторе, где курсор.
		let mut cursor = POINT::default();
		let _ = GetCursorPos(&mut cursor);
		let monitor = MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST);
		let (mut dx, mut dy) = (96u32, 96u32);
		let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
		let scale = dx as f32 / 96.0;
		let mut info = MONITORINFO {
			cbSize: std::mem::size_of::<MONITORINFO>() as u32,
			..Default::default()
		};
		let _ = GetMonitorInfoW(monitor, &mut info);
		let work = info.rcWork;
		let size = app.size();
		let (pw, ph) = (
			(size.0 * scale).round() as i32,
			(size.1 * scale).round() as i32,
		);
		let x = work.left + (work.right - work.left - pw) / 2;
		let y = work.top + (work.bottom - work.top - ph) / 2;

		let (_, build, _) = sys::windows_version();
		let dark = !sys::light_theme();
		STATE.with(|s| {
			*s.borrow_mut() = Some(State {
				app,
				gfx,
				rt: None,
				input: Input::default(),
				regions: Vec::new(),
				focusable: Vec::new(),
				scale,
				start: Instant::now(),
				dark,
				reduced: reduced_motion(),
				win10: build < 22000,
				rx,
				size,
			})
		});
		let hwnd = CreateWindowExW(
			WS_EX_APPWINDOW,
			w!("LeeLauncherSetup"),
			w!("LeeLauncher"),
			WS_POPUP | WS_SYSMENU | WS_MINIMIZEBOX,
			x,
			y,
			pw,
			ph,
			None,
			None,
			Some(instance.into()),
			None,
		)?;
		hwnd_cell.store(hwnd.0 as isize, Ordering::SeqCst);
		with(|s| s.app.hwnd = hwnd.0 as isize);
		let corner = DWMWCP_ROUND;
		let _ = DwmSetWindowAttribute(
			hwnd,
			DWMWA_WINDOW_CORNER_PREFERENCE,
			(&corner as *const DWM_WINDOW_CORNER_PREFERENCE).cast(),
			4,
		);
		set_dark(hwnd, dark);
		let _ = DwmExtendFrameIntoClientArea(
			hwnd,
			&MARGINS {
				cxLeftWidth: 1,
				cxRightWidth: 1,
				cyTopHeight: 1,
				cyBottomHeight: 1,
			},
		);
		let _ = ShowWindow(hwnd, SW_SHOW);
		let _ = SetForegroundWindow(hwnd);

		let mut msg = MSG::default();
		while GetMessageW(&mut msg, None, 0, 0).as_bool() {
			let _ = TranslateMessage(&msg);
			DispatchMessageW(&msg);
		}
	}
	STATE.with(|s| s.borrow_mut().take());
	Ok(())
}

fn set_dark(hwnd: HWND, dark: bool) {
	let v = BOOL(dark as i32);
	unsafe {
		let _ = DwmSetWindowAttribute(
			hwnd,
			DWMWA_USE_IMMERSIVE_DARK_MODE,
			(&v as *const BOOL).cast(),
			4,
		);
	}
}

fn now(s: &State) -> f64 {
	s.start.elapsed().as_secs_f64()
}

fn point(s: &State, lparam: LPARAM) -> (f32, f32) {
	let x = (lparam.0 & 0xffff) as i16 as f32;
	let y = ((lparam.0 >> 16) & 0xffff) as i16 as f32;
	(x / s.scale, y / s.scale)
}

fn hit(s: &State, x: f32, y: f32) -> Option<&Region> {
	s.regions.iter().rev().find(|r| r.rect.contains(x, y))
}

fn invalidate(hwnd: HWND) {
	unsafe {
		let _ = InvalidateRect(Some(hwnd), None, false);
	}
}

fn paint(hwnd: HWND, s: &mut State) {
	unsafe {
		let mut ps = PAINTSTRUCT::default();
		BeginPaint(hwnd, &mut ps);
		if s.rt.is_none() {
			let mut rc = RECT::default();
			let _ = GetClientRect(hwnd, &mut rc);
			let props = D2D1_RENDER_TARGET_PROPERTIES {
				pixelFormat: D2D1_PIXEL_FORMAT {
					format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
					alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
				},
				dpiX: 96.0 * s.scale,
				dpiY: 96.0 * s.scale,
				..Default::default()
			};
			let hprops = D2D1_HWND_RENDER_TARGET_PROPERTIES {
				hwnd,
				pixelSize: D2D_SIZE_U {
					width: (rc.right - rc.left) as u32,
					height: (rc.bottom - rc.top) as u32,
				},
				presentOptions: D2D1_PRESENT_OPTIONS_NONE,
			};
			s.rt = s.gfx.d2d.CreateHwndRenderTarget(&props, &hprops).ok();
		}
		let Some(rt) = s.rt.clone() else {
			let _ = EndPaint(hwnd, &ps);
			return;
		};
		let target: ID2D1RenderTarget = rt.cast().unwrap();
		target.BeginDraw();
		target.SetTransform(&windows_numerics::Matrix3x2::identity());
		let t = now(s);
		s.app.tick(t);
		let palette = Palette::get(s.dark);
		let (regions, focusable, animating) = {
			let mut ui = Ui {
				c: Canvas::new(&target, &s.gfx),
				p: palette,
				lang: s.app.lang,
				input: &s.input,
				now: t,
				reduced: s.reduced,
				regions: Vec::new(),
				focusable: Vec::new(),
				animating: false,
				win10: s.win10,
			};
			s.app.paint(&mut ui);
			(ui.regions, ui.focusable, ui.animating)
		};
		if target.EndDraw(None, None).is_err() {
			s.rt = None;
		}
		s.regions = regions;
		if s.input
			.focus
			.as_ref()
			.is_some_and(|f| !focusable.contains(f))
		{
			s.input.focus = None;
		}
		s.focusable = focusable;
		let _ = EndPaint(hwnd, &ps);
		if animating || s.app.needs_tick() {
			SetTimer(Some(hwnd), TIMER, 16, None);
		} else {
			let _ = KillTimer(Some(hwnd), TIMER);
		}
	}
}

/// После действия: закрыть окно или поменять его размер.
fn after(hwnd: HWND, s: &mut State) {
	if s.app.close {
		unsafe {
			let _ = DestroyWindow(hwnd);
		}
		return;
	}
	let size = s.app.size();
	if size != s.size {
		s.size = size;
		let (pw, ph) = (
			(size.0 * s.scale).round() as i32,
			(size.1 * s.scale).round() as i32,
		);
		unsafe {
			let _ = SetWindowPos(hwnd, None, 0, 0, pw, ph, SWP_NOMOVE | SWP_NOZORDER);
		}
		s.rt = None;
	}
	invalidate(hwnd);
}

fn activate(hwnd: HWND, s: &mut State, id: &str) {
	if id == "win:min" {
		unsafe {
			let _ = ShowWindow(hwnd, SW_MINIMIZE);
		}
		return;
	}
	s.app.click(id);
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
	let handled = match msg {
		WM_PAINT => with(|s| paint(hwnd, s)).map(|_| LRESULT(0)),
		WM_ERASEBKGND => Some(LRESULT(1)),
		WM_NCHITTEST => with(|s| {
			let mut pt = POINT {
				x: (lparam.0 & 0xffff) as i16 as i32,
				y: ((lparam.0 >> 16) & 0xffff) as i16 as i32,
			};
			unsafe {
				let _ = windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut pt);
			}
			let (x, y) = (pt.x as f32 / s.scale, pt.y as f32 / s.scale);
			match hit(s, x, y).map(|r| r.kind) {
				Some(Kind::Drag) => LRESULT(HTCAPTION as isize),
				_ => LRESULT(HTCLIENT as isize),
			}
		}),
		WM_MOUSEMOVE => with(|s| {
			let (x, y) = point(s, lparam);
			let hover = hit(s, x, y)
				.filter(|r| r.kind == Kind::Press && r.enabled)
				.map(|r| r.id.clone());
			if hover != s.input.hover {
				s.input.hover = hover;
				invalidate(hwnd);
			}
			let mut track = TRACKMOUSEEVENT {
				cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
				dwFlags: TME_LEAVE,
				hwndTrack: hwnd,
				dwHoverTime: 0,
			};
			unsafe {
				let _ = TrackMouseEvent(&mut track);
			}
			LRESULT(0)
		}),
		WM_MOUSELEAVE | WM_NCMOUSEMOVE => with(|s| {
			if s.input.hover.take().is_some() {
				invalidate(hwnd);
			}
			LRESULT(0)
		})
		.filter(|_| msg == WM_MOUSELEAVE),
		WM_LBUTTONDOWN => with(|s| {
			s.input.pressed = s.input.hover.clone();
			s.input.focus_visible = false;
			unsafe { SetCapture(hwnd) };
			invalidate(hwnd);
			LRESULT(0)
		}),
		WM_LBUTTONUP => with(|s| {
			unsafe {
				let _ = ReleaseCapture();
			}
			let pressed = s.input.pressed.take();
			if let Some(id) = pressed.filter(|p| s.input.hover.as_ref() == Some(p)) {
				s.input.focus = Some(id.clone());
				activate(hwnd, s, &id);
			}
			after(hwnd, s);
			LRESULT(0)
		}),
		WM_KEYDOWN => with(|s| {
			let key = VIRTUAL_KEY(wparam.0 as u16);
			let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
			match key {
				VK_TAB => {
					s.input.focus_visible = true;
					if !s.focusable.is_empty() {
						let n = s.focusable.len();
						let i = s
							.input
							.focus
							.as_ref()
							.and_then(|f| s.focusable.iter().position(|x| x == f));
						let next = match (i, shift) {
							(None, false) => 0,
							(None, true) => n - 1,
							(Some(i), false) => (i + 1) % n,
							(Some(i), true) => (i + n - 1) % n,
						};
						s.input.focus = Some(s.focusable[next].clone());
					}
				}
				VK_RETURN | VK_SPACE => {
					let focus = s.input.focus.clone().filter(|_| s.input.focus_visible);
					match focus {
						Some(id) => activate(hwnd, s, &id),
						None if key == VK_RETURN => s.app.default_action(),
						None => {}
					}
				}
				VK_ESCAPE => s.app.escape(),
				VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN => {
					if let Some(f) = s.input.focus.clone()
						&& let Some(next) = s.app.arrow(&f, matches!(key, VK_RIGHT | VK_DOWN))
					{
						s.input.focus = Some(next);
						s.input.focus_visible = true;
					}
				}
				_ => {}
			}
			after(hwnd, s);
			LRESULT(0)
		}),
		WM_TIMER => with(|s| {
			let t = now(s);
			s.app.tick(t);
			after(hwnd, s);
			LRESULT(0)
		}),
		WM_APP_EVENT => with(|s| {
			while let Ok(ev) = s.rx.try_recv() {
				s.app.on_event(ev);
			}
			after(hwnd, s);
			LRESULT(0)
		}),
		WM_SETTINGCHANGE => with(|s| {
			let dark = !sys::light_theme();
			if dark != s.dark {
				s.dark = dark;
				set_dark(hwnd, dark);
			}
			s.reduced = reduced_motion();
			invalidate(hwnd);
			LRESULT(0)
		}),
		WM_DPICHANGED => with(|s| {
			s.scale = (wparam.0 & 0xffff) as f32 / 96.0;
			let r = unsafe { &*(lparam.0 as *const RECT) };
			unsafe {
				let _ = SetWindowPos(
					hwnd,
					None,
					r.left,
					r.top,
					r.right - r.left,
					r.bottom - r.top,
					SWP_NOZORDER | SWP_NOACTIVATE,
				);
			}
			s.rt = None;
			invalidate(hwnd);
			LRESULT(0)
		}),
		WM_SIZE => with(|s| {
			if let Some(rt) = &s.rt {
				let size = D2D_SIZE_U {
					width: (lparam.0 & 0xffff) as u32,
					height: ((lparam.0 >> 16) & 0xffff) as u32,
				};
				if unsafe { rt.Resize(&size) }.is_err() {
					s.rt = None;
				}
			}
			LRESULT(0)
		}),
		WM_CLOSE => with(|s| {
			if !s.app.close {
				s.app.escape();
			}
			after(hwnd, s);
			LRESULT(0)
		}),
		WM_DESTROY => {
			unsafe { PostQuitMessage(0) };
			Some(LRESULT(0))
		}
		_ => None,
	};
	match handled {
		Some(r) => r,
		None => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
	}
}

/// Масштаб окна по его монитору (для первого кадра).
#[allow(dead_code)]
pub fn window_scale(hwnd: HWND) -> f32 {
	unsafe { GetDpiForWindow(hwnd) as f32 / 96.0 }
}
