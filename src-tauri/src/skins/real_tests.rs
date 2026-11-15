//! Проверка с настоящим Mojang: меняет скин аккаунта и возвращает прежний. Только с согласия
//! владельца аккаунта. Копия скина — `%TEMP%\leelauncher-skin-real-<ник>.png`: если вернуть не
//! получится (429), его надо будет надеть вручную. Токены — из настоящего `accounts.json`.
//!
//! ```text
//! $env:LEELAUNCHER_SKIN_REAL_NICK = '<ник>'
//! cargo test real_skin_change -- --ignored --nocapture --test-threads 1
//! ```

use std::path::Path;
use std::time::Duration;

use super::mojang::Call;
use super::png::test_util::{encode, image_of, paint_gaps, skin};
use super::{AccountKind, ApplyRequest, SkinModel, Skins};
use crate::accounts::Accounts;
use crate::error::{AppError, ErrorCode};

const MAX_WAIT_SECS: u64 = 15 * 60;

fn seconds_in(err: &AppError) -> u64 {
	err.details
		.as_deref()
		.and_then(|text| text.parse().ok())
		.unwrap_or(120)
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn real_skin_change() {
	let Ok(nick) = std::env::var("LEELAUNCHER_SKIN_REAL_NICK") else {
		println!("LEELAUNCHER_SKIN_REAL_NICK не задана: настоящие запросы не отправлены");
		return;
	};
	let accounts = Accounts::load(
		Path::new(&crate::paths::data_dir()),
		crate::http::build_client().unwrap(),
	);
	let account = accounts
		.state()
		.await
		.accounts
		.into_iter()
		.find(|account| account.username == nick)
		.expect("нет аккаунта с таким ником");
	let dir = tempfile::tempdir().unwrap();
	let skins = Skins::load(dir.path().join("skins")).unwrap();
	let accounts = &accounts;
	let id = account.id.as_str();
	let token = move |force| async move { accounts.skin_token(id, force, &|_| {}).await };
	let microsoft = AccountKind::Microsoft;

	let before = skins
		.profile(id, microsoft, true, token)
		.await
		.unwrap()
		.value;
	println!(
		"профиль: скин {:?}, плащей {}, активный плащ {:?}",
		before
			.skin
			.as_ref()
			.map(|s| (&s.key, s.model, s.is_default)),
		before.capes.len(),
		before.active_cape_id,
	);
	let current = before.skin.clone();
	let original = match &current {
		Some(skin) if !skin.is_default => {
			let png = skins
				.store
				.texture(&skin.key)
				.expect("текстура скина не получена: ничего не меняем");
			let path = std::env::temp_dir().join(format!("leelauncher-skin-real-{nick}.png"));
			std::fs::write(&path, &png).expect("копия скина не записана: ничего не меняем");
			println!("копия скина: {}", path.display());
			let entry = skins
				.library
				.add_bytes(&png, &nick, Some(&skin.key))
				.unwrap();
			Some((entry.id, skin.model))
		}
		_ => None,
	};

	let wait_for = |secs: u64| tokio::time::sleep(Duration::from_secs(secs + 1));
	let classic = skin(64, 64, 40);
	let slim = {
		let mut image = image_of(64, 64, 200);
		paint_gaps(&mut image, [0, 0, 0, 0]);
		encode(&image)
	};
	let legacy = skin(64, 32, 90);
	let uploads = [
		("широкие руки 64×64", &classic, SkinModel::Classic),
		("тонкие руки 64×64", &slim, SkinModel::Slim),
		("старый формат 64×32", &legacy, SkinModel::Classic),
	];

	// После 429 менять дальше нельзя: за это как раз могут заблокировать.
	let mut limited = None;
	for (label, png, model) in uploads {
		wait_for(10).await;
		let entry = skins.library.add_bytes(png, label, None).unwrap();
		let request = ApplyRequest {
			skin_id: Some(entry.id),
			model: Some(model),
			cape: None,
			allow_lose_previous: false,
		};
		let result = skins.apply(id, microsoft, &nick, request, token).await;
		match result {
			Ok(outcome) => {
				let worn = outcome.value.profile.skin;
				println!("{label}: {:?}", worn.map(|s| (s.key, s.model, s.skin_id)));
			}
			Err(err) => {
				println!("{label}: {err:?}");
				if err.code == ErrorCode::RateLimited {
					limited = Some(seconds_in(&err));
					break;
				}
			}
		}
	}

	if limited.is_none() {
		wait_for(10).await;
		let wrong_size = skin(100, 100, 120);
		let permit = skins.mojang.begin(id);
		let result = match &permit {
			Ok(permit) => {
				let call = Call::Upload {
					png: &wrong_size,
					model: SkinModel::Classic,
				};
				skins
					.mojang
					.call(Some(permit), call, &token)
					.await
					.map(|parsed| parsed.is_some())
					.map_err(|failed| failed.error)
			}
			Err(err) => Err(err.clone()),
		};
		drop(permit);
		println!("плохой файл 100×100: {result:?}");
		if let Err(err) = &result
			&& err.code == ErrorCode::RateLimited
		{
			limited = Some(seconds_in(err));
		}
	}

	if limited.is_none() && !before.capes.is_empty() {
		// Показывается другой плащ или скрывается текущий; прежний вернётся ниже.
		wait_for(10).await;
		let other = before
			.capes
			.iter()
			.find(|cape| Some(&cape.id) != before.active_cape_id.as_ref())
			.map(|cape| cape.id.clone());
		let request = ApplyRequest {
			cape: Some(other),
			..Default::default()
		};
		let result = skins.apply(id, microsoft, &nick, request, token).await;
		println!(
			"плащ: {:?}",
			result.as_ref().map(|o| &o.value.profile.active_cape_id)
		);
		if let Err(err) = &result
			&& err.code == ErrorCode::RateLimited
		{
			limited = Some(seconds_in(err));
		}
	}

	// Возврат: скин и плащ одним действием либо снятие, если был обычный скин.
	let mut wait = limited.unwrap_or(10);
	let mut restored = false;
	loop {
		if wait > MAX_WAIT_SECS {
			println!("пауза {wait} с: скин не возвращён, верните его позже из файла");
			break;
		}
		wait_for(wait).await;
		let result = match &original {
			Some((skin_id, model)) => {
				let request = ApplyRequest {
					skin_id: Some(skin_id.clone()),
					model: Some(*model),
					cape: Some(before.active_cape_id.clone()),
					allow_lose_previous: false,
				};
				skins
					.apply(id, microsoft, &nick, request, token)
					.await
					.map(|outcome| outcome.value.cape_error)
			}
			None => skins.reset(id, microsoft, token).await.map(|_| None),
		};
		println!("возврат: {result:?}");
		match result {
			Ok(_) => {
				restored = true;
				break;
			}
			// Запрос не уходил: лимитер просит подождать, ждём.
			Err(err) if err.code == ErrorCode::SkinTooOften => wait = seconds_in(&err),
			Err(err) => {
				println!("скин не возвращён ({:?}), верните его из файла", err.code);
				if err.code == ErrorCode::RateLimited {
					limited = limited.or(Some(0));
				}
				break;
			}
		}
	}
	assert!(limited.is_none(), "получен 429, проверка остановлена");
	assert!(restored, "на аккаунте осталась тестовая картинка");
}
