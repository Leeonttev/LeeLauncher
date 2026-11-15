//! Остановка игры вместе с потомками: `javaw.exe` из Oracle `javapath` и шимы менеджеров
//! пакетов запускают настоящую Java дочерним процессом.

/// Останавливает потомков `pid` (не его самого). Процессы старше родителя пропускаются:
/// это чужой процесс с повторно выданным PID.
pub fn kill_descendants(pid: u32) -> usize {
	imp::kill_descendants(pid)
}

#[cfg(windows)]
mod imp {
	use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, INVALID_HANDLE_VALUE};
	use windows_sys::Win32::System::Diagnostics::ToolHelp::{
		CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
		TH32CS_SNAPPROCESS,
	};
	use windows_sys::Win32::System::Threading::{
		GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
		PROCESS_TERMINATE, TerminateProcess, WaitForSingleObject,
	};

	/// Сколько раз обходить дерево: потомок мог успеть запустить ещё один процесс.
	const ROUNDS: usize = 3;
	const WAIT_MS: u32 = 2000;

	struct Handle(HANDLE);

	impl Drop for Handle {
		fn drop(&mut self) {
			// SAFETY: дескриптор получен от OpenProcess/CreateToolhelp32Snapshot и закрывается один раз.
			unsafe {
				CloseHandle(self.0);
			}
		}
	}

	struct Proc {
		pid: u32,
		handle: Handle,
		created: u64,
	}

	fn open(pid: u32, access: u32) -> Option<Proc> {
		// SAFETY: обычный вызов WinAPI; нулевой дескриптор — ошибка.
		let raw = unsafe { OpenProcess(access, 0, pid) };
		if raw.is_null() {
			return None;
		}
		let handle = Handle(raw);
		let mut times = [FILETIME {
			dwLowDateTime: 0,
			dwHighDateTime: 0,
		}; 4];
		let [created, exited, kernel, user] = &mut times;
		// SAFETY: дескриптор открыт с правом QUERY_LIMITED_INFORMATION, указатели на локальные значения.
		let ok = unsafe { GetProcessTimes(handle.0, created, exited, kernel, user) };
		if ok == 0 {
			return None;
		}
		let created =
			(u64::from(times[0].dwHighDateTime) << 32) | u64::from(times[0].dwLowDateTime);
		Some(Proc {
			pid,
			handle,
			created,
		})
	}

	/// Пары «процесс → родитель» всех процессов системы.
	fn snapshot() -> Vec<(u32, u32)> {
		// SAFETY: обычный вызов WinAPI; ошибка — INVALID_HANDLE_VALUE.
		let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
		if raw == INVALID_HANDLE_VALUE || raw.is_null() {
			return Vec::new();
		}
		let snap = Handle(raw);
		let mut entry = PROCESSENTRY32W {
			dwSize: size_of::<PROCESSENTRY32W>() as u32,
			..Default::default()
		};
		let mut out = Vec::new();
		// SAFETY: `entry.dwSize` заполнен, снимок открыт.
		let mut ok = unsafe { Process32FirstW(snap.0, &mut entry) };
		while ok != 0 {
			out.push((entry.th32ProcessID, entry.th32ParentProcessID));
			// SAFETY: то же.
			ok = unsafe { Process32NextW(snap.0, &mut entry) };
		}
		out
	}

	/// Потомки корня в порядке обхода в ширину (родитель раньше детей).
	fn descendants(root: &Proc) -> Vec<Proc> {
		let list = snapshot();
		let mut found: Vec<Proc> = Vec::new();
		let mut parents = vec![(root.pid, root.created)];
		let mut next = 0;
		while next < parents.len() {
			let (parent, parent_created) = parents[next];
			next += 1;
			for &(pid, ppid) in &list {
				if ppid != parent
					|| pid == parent
					|| pid == root.pid
					|| parents.iter().any(|(p, _)| *p == pid)
				{
					continue;
				}
				let access =
					PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE;
				let Some(proc) = open(pid, access) else {
					continue;
				};
				if proc.created < parent_created {
					continue;
				}
				parents.push((pid, proc.created));
				found.push(proc);
			}
		}
		found
	}

	pub fn kill_descendants(pid: u32) -> usize {
		let Some(root) = open(pid, PROCESS_QUERY_LIMITED_INFORMATION) else {
			return 0;
		};
		let mut killed = 0;
		for _ in 0..ROUNDS {
			let list = descendants(&root);
			if list.is_empty() {
				break;
			}
			// Сначала самые дальние: родитель-заглушка не успеет запустить замену.
			for proc in list.iter().rev() {
				// SAFETY: дескриптор открыт с правом TERMINATE.
				if unsafe { TerminateProcess(proc.handle.0, 1) } != 0 {
					killed += 1;
				}
			}
			for proc in &list {
				// SAFETY: дескриптор открыт с правом SYNCHRONIZE.
				unsafe {
					WaitForSingleObject(proc.handle.0, WAIT_MS);
				}
			}
		}
		killed
	}

	/// Процесс ещё работает (для тестов).
	#[cfg(test)]
	pub fn is_alive(pid: u32) -> bool {
		use windows_sys::Win32::Foundation::WAIT_TIMEOUT;
		let Some(proc) = open(pid, PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE) else {
			return false;
		};
		// SAFETY: дескриптор открыт с правом SYNCHRONIZE.
		unsafe { WaitForSingleObject(proc.handle.0, 0) == WAIT_TIMEOUT }
	}

	/// PID всех потомков (для тестов).
	#[cfg(test)]
	pub fn descendant_pids(pid: u32) -> Vec<u32> {
		open(pid, PROCESS_QUERY_LIMITED_INFORMATION)
			.map(|root| descendants(&root).iter().map(|p| p.pid).collect())
			.unwrap_or_default()
	}
}

#[cfg(not(windows))]
mod imp {
	/// Игра запускается напрямую `java`, заглушек между ней и лаунчером нет.
	pub fn kill_descendants(_pid: u32) -> usize {
		0
	}
}

#[cfg(all(test, windows))]
mod tests {
	use std::process::{Command, Stdio};
	use std::time::{Duration, Instant};

	use super::imp::{descendant_pids, is_alive};
	use super::kill_descendants;

	/// Корень — `cmd`, который ждёт другой `cmd`, который ждёт `ping` (как заглушка Java ждёт игру).
	#[test]
	fn kills_whole_tree() {
		let mut root = Command::new("cmd")
			.args(["/d", "/c", "cmd /d /c ping -n 60 127.0.0.1"])
			.stdin(Stdio::null())
			.stdout(Stdio::null())
			.stderr(Stdio::null())
			.spawn()
			.unwrap();
		let pid = root.id();
		let deadline = Instant::now() + Duration::from_secs(10);
		let pids = loop {
			let pids = descendant_pids(pid);
			if pids.len() >= 2 || Instant::now() > deadline {
				break pids;
			}
			std::thread::sleep(Duration::from_millis(50));
		};
		assert!(pids.len() >= 2, "потомки не появились: {pids:?}");

		assert!(kill_descendants(pid) >= 2);
		for child in &pids {
			assert!(!is_alive(*child), "процесс {child} жив");
		}
		// Корень не трогали, но без потомков он выходит сам.
		let status = root.wait().unwrap();
		assert!(!status.success());
		assert_eq!(kill_descendants(pid), 0);
	}

	#[test]
	fn no_process_no_kill() {
		assert_eq!(kill_descendants(u32::MAX - 3), 0);
	}
}
