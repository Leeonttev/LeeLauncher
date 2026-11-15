use serde::Serialize;

const MB: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemMemory {
	/// Объём ОЗУ компьютера в МБ (0 — не удалось определить).
	pub total_mb: u64,
	/// Рекомендуемый максимум памяти для игры в МБ.
	pub recommended_max_mb: u32,
}

pub fn system_memory() -> SystemMemory {
	let total_mb = total_bytes().map_or(0, |b| b / MB);
	SystemMemory {
		total_mb,
		recommended_max_mb: recommended_max_mb(total_mb),
	}
}

/// Четверть ОЗУ в пределах 2–8 ГБ, но не больше половины ОЗУ; кратно 256 МБ.
/// ОЗУ неизвестно — 2 ГБ.
pub fn recommended_max_mb(total_mb: u64) -> u32 {
	if total_mb == 0 {
		return 2048;
	}
	let value = (total_mb / 4).clamp(2048, 8192).min(total_mb / 2).max(1024);
	(value - value % 256) as u32
}

#[cfg(windows)]
fn total_bytes() -> Option<u64> {
	use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

	let mut status = MEMORYSTATUSEX {
		dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
		..unsafe { std::mem::zeroed() }
	};
	// SAFETY: структура инициализирована, dwLength задан, указатель живёт до конца вызова.
	let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
	(ok != 0).then_some(status.ullTotalPhys)
}

#[cfg(target_os = "linux")]
fn total_bytes() -> Option<u64> {
	let text = std::fs::read_to_string("/proc/meminfo").ok()?;
	let line = text.lines().find(|l| l.starts_with("MemTotal:"))?;
	let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
	Some(kb * 1024)
}

#[cfg(not(any(windows, target_os = "linux")))]
fn total_bytes() -> Option<u64> {
	None
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn recommended_memory() {
		assert_eq!(recommended_max_mb(0), 2048);
		assert_eq!(recommended_max_mb(4096), 2048);
		assert_eq!(recommended_max_mb(3000), 1280);
		assert_eq!(recommended_max_mb(16 * 1024), 4096);
		assert_eq!(recommended_max_mb(12 * 1024), 3072);
		assert_eq!(recommended_max_mb(64 * 1024), 8192);
	}

	#[test]
	fn detects_memory() {
		let mem = system_memory();
		if cfg!(windows) {
			assert!(mem.total_mb > 512);
		}
	}
}
