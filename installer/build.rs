use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
	let out = PathBuf::from(env::var("OUT_DIR").unwrap());
	let app_toml = "../src-tauri/Cargo.toml";
	println!("cargo:rerun-if-changed={app_toml}");
	let text = fs::read_to_string(app_toml).expect("src-tauri/Cargo.toml");
	let version = text
		.lines()
		.find_map(|l| l.strip_prefix("version = \""))
		.and_then(|l| l.strip_suffix('"'))
		.expect("версия в src-tauri/Cargo.toml");
	println!("cargo:rustc-env=APP_VERSION={version}");

	// Лаунчер внутри установщика. Без него установщик собирается, но ставить ему нечего.
	println!("cargo:rerun-if-env-changed=LEELAUNCHER_PAYLOAD");
	let payload = match env::var_os("LEELAUNCHER_PAYLOAD") {
		Some(path) => {
			println!("cargo:rerun-if-changed={}", PathBuf::from(&path).display());
			println!(
				"cargo:rustc-env=LEELAUNCHER_PAYLOAD={}",
				PathBuf::from(&path).display()
			);
			let raw = fs::read(&path).expect("LEELAUNCHER_PAYLOAD");
			let packed = miniz_oxide::deflate::compress_to_vec(&raw, 9);
			let mut out = (raw.len() as u64).to_le_bytes().to_vec();
			out.extend_from_slice(&packed);
			out
		}
		None => Vec::new(),
	};
	fs::write(out.join("payload.bin"), payload).unwrap();

	// Загрузчик WebView2 от Microsoft. Нет — установщик скачает его сам.
	println!("cargo:rerun-if-env-changed=LEELAUNCHER_WEBVIEW2");
	let webview = match env::var_os("LEELAUNCHER_WEBVIEW2") {
		Some(path) => {
			println!("cargo:rerun-if-changed={}", PathBuf::from(&path).display());
			fs::read(&path).expect("LEELAUNCHER_WEBVIEW2")
		}
		None => Vec::new(),
	};
	fs::write(out.join("webview2.bin"), webview).unwrap();

	println!("cargo:rerun-if-changed=installer.rc");
	println!("cargo:rerun-if-changed=installer.manifest");
	embed_resource::compile("installer.rc", embed_resource::NONE)
		.manifest_required()
		.unwrap();
}
