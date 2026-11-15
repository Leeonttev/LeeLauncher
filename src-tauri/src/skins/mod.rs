//! Скины: общая библиотека PNG, текущий скин и плащи аккаунта Microsoft с серверов Mojang.
//! Офлайн-аккаунтам раздел закрыт.

pub mod commands;
mod library;
mod mojang;
mod png;
mod profile;
#[cfg(test)]
mod real_tests;
mod textures;

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use self::library::Library;
use self::mojang::{Call, Failed, Limiter, Mojang, too_often};
use self::profile::{ParsedProfile, ProfileStore, StoredProfile, StoredSkin};
use self::textures::{Download, TEXTURES_BASE, Textures};
use crate::accounts::AccountKind;
use crate::error::{AppError, ErrorCode};

pub const EVENT_CHANGED: &str = "skins-changed";

/// Профиль не запрашивается чаще раза в минуту, «Повторить» — чаще раза в 10 с.
const PROFILE_EVERY: Duration = Duration::from_secs(60);
const REFRESH_EVERY: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkinModel {
	Classic,
	Slim,
}

impl SkinModel {
	pub fn as_str(self) -> &'static str {
		match self {
			Self::Classic => "classic",
			Self::Slim => "slim",
		}
	}
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkinEntry {
	pub id: String,
	pub name: String,
	pub model: SkinModel,
	pub legacy: bool,
	pub added_at: String,
	/// `data:image/png;base64,…`
	pub texture: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WornSkin {
	pub key: String,
	/// Скин библиотеки; `None` — его там нет.
	pub skin_id: Option<String>,
	pub model: SkinModel,
	/// Обычный скин Minecraft: в ответе был `alias`.
	pub is_default: bool,
	/// `data:image/png;base64,…`; `None` — текстура не получена.
	pub texture: Option<String>,
	pub fetched_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cape {
	pub id: String,
	pub name: String,
	pub texture: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkinProfile {
	pub account_id: String,
	pub skin: Option<WornSkin>,
	pub capes: Vec<Cape>,
	pub active_cape_id: Option<String>,
	/// Сервер не ответил или ответ не разобрался: показан сохранённый.
	pub stale: bool,
	pub error: Option<AppError>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkinApplied {
	pub profile: SkinProfile,
	/// Скин надет, плащ нет.
	pub cape_error: Option<AppError>,
	/// Несохранённый скин, который был на аккаунте, положен в библиотеку.
	pub saved_previous: Option<SkinEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectReason {
	NotPng,
	BadSize,
	TooLarge,
	Unreadable,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RejectedFile {
	pub path: String,
	pub reason: RejectReason,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub width: Option<u32>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub height: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkinsAdded {
	pub added: Vec<SkinEntry>,
	pub duplicates: Vec<SkinEntry>,
	pub rejected: Vec<RejectedFile>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkinPatch {
	#[serde(default)]
	pub name: Option<String>,
	#[serde(default)]
	pub model: Option<SkinModel>,
}

/// Что поменять. Поля нет — не меняется; `cape: Some(None)` — скрыть плащ.
#[derive(Debug, Clone, Default)]
pub struct ApplyRequest {
	pub skin_id: Option<String>,
	pub model: Option<SkinModel>,
	pub cape: Option<Option<String>>,
	/// Пользователь согласен заменить скин, который нельзя сохранить.
	pub allow_lose_previous: bool,
}

/// Результат и признак «изменился текущий скин аккаунта»: тогда уходит `skins-changed`.
#[derive(Debug, Clone)]
pub struct Outcome<T> {
	pub value: T,
	pub changed: bool,
}

/// Работа с файлами вне потока команд.
async fn blocking<T: Send + 'static>(
	work: impl FnOnce() -> Result<T, AppError> + Send + 'static,
) -> Result<T, AppError> {
	tokio::task::spawn_blocking(work)
		.await
		.map_err(|err| AppError::unknown(err.to_string()))?
}

async fn with_library<T: Send + 'static>(
	library: &Arc<Library>,
	work: impl FnOnce(&Library) -> Result<T, AppError> + Send + 'static,
) -> Result<T, AppError> {
	let library = library.clone();
	blocking(move || work(&library)).await
}

fn now_iso() -> String {
	Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn data_url(png: &[u8]) -> String {
	use base64::Engine;
	format!(
		"data:image/png;base64,{}",
		base64::prelude::BASE64_STANDARD.encode(png)
	)
}

fn view_of(
	store: &ProfileStore,
	library: &Library,
	account_id: &str,
	stale: bool,
	error: Option<AppError>,
) -> Option<SkinProfile> {
	let stored = store.read(account_id)?;
	let skin = stored.skin.as_ref().map(|skin| {
		let bytes = store.texture(&skin.key);
		let id = bytes
			.as_deref()
			.and_then(|bytes| png::parse(bytes).ok())
			.map(|parsed| parsed.id);
		WornSkin {
			key: skin.key.clone(),
			skin_id: library.skin_id_for(&skin.key, id.as_deref()),
			model: skin.model,
			is_default: skin.default,
			texture: bytes.as_deref().map(data_url),
			fetched_at: stored.fetched_at.clone(),
		}
	});
	let capes = stored
		.capes
		.iter()
		.map(|cape| Cape {
			id: cape.id.clone(),
			name: cape.name.clone(),
			texture: cape
				.key
				.as_deref()
				.and_then(|key| store.texture(key))
				.as_deref()
				.map(data_url),
		})
		.collect();
	Some(SkinProfile {
		account_id: account_id.to_owned(),
		skin,
		capes,
		active_cape_id: stored.active_cape_id,
		stale,
		error,
	})
}

fn same_skin(old: Option<&SkinProfile>, new: Option<&SkinProfile>) -> bool {
	let parts = |profile: Option<&SkinProfile>| {
		profile
			.and_then(|profile| profile.skin.as_ref())
			.map(|skin| {
				(
					skin.key.clone(),
					skin.skin_id.clone(),
					skin.model,
					skin.texture.is_some(),
				)
			})
	};
	parts(old) == parts(new)
}

/// Последний профиль аккаунта. Замок держится на время запроса, второй вызов берёт его результат.
#[derive(Default)]
struct ProfileSlot {
	last_attempt: Option<Instant>,
	last_error: Option<AppError>,
	/// Ответ на изменение не разобрался: следующий `skin_profile` идёт на сервер.
	refetch: bool,
}

struct Policy {
	every: Duration,
	refresh_every: Duration,
}

/// Кладётся в состояние Tauri.
pub struct Skins {
	library: Arc<Library>,
	store: Arc<ProfileStore>,
	mojang: Mojang,
	textures: Textures,
	slots: std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<ProfileSlot>>>>,
	policy: Policy,
}

impl Skins {
	pub fn load(dir: PathBuf) -> Result<Self, AppError> {
		let client = |built: reqwest::Result<reqwest::Client>| {
			built.map_err(|err| AppError::unknown(err.to_string()))
		};
		Ok(Self::with_parts(
			dir,
			Mojang::new(
				client(mojang::build_client())?,
				mojang::API_BASE,
				Limiter::default(),
			),
			Textures::new(client(textures::build_client())?, TEXTURES_BASE),
		))
	}

	fn with_parts(dir: PathBuf, mojang: Mojang, textures: Textures) -> Self {
		Self {
			store: Arc::new(ProfileStore::new(&dir)),
			library: Arc::new(Library::new(dir)),
			mojang,
			textures,
			slots: Default::default(),
			policy: Policy {
				every: PROFILE_EVERY,
				refresh_every: REFRESH_EVERY,
			},
		}
	}

	fn slot(&self, account_id: &str) -> Arc<tokio::sync::Mutex<ProfileSlot>> {
		self.slots
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner)
			.entry(account_id.to_owned())
			.or_default()
			.clone()
	}

	async fn view(
		&self,
		account_id: &str,
		stale: bool,
		error: Option<AppError>,
	) -> Result<Option<SkinProfile>, AppError> {
		let (store, library, account) = (
			self.store.clone(),
			self.library.clone(),
			account_id.to_owned(),
		);
		blocking(move || Ok(view_of(&store, &library, &account, stale, error))).await
	}

	/// Докачка, запись профиля и уборка кэша. `local` — текстура, которая уже есть у нас.
	async fn commit(
		&self,
		account_id: &str,
		stored: StoredProfile,
		downloads: Vec<Download>,
		local: Option<(String, Vec<u8>)>,
	) -> Result<(), AppError> {
		let _guard = self.store.lock.lock().await;
		if let Some((key, png)) = local {
			let store = self.store.clone();
			if let Err(err) = blocking(move || store.write_texture(&key, &png)).await {
				log::warn!("текстура не записана: {err}");
			}
		}
		self.textures.ensure(&self.store, downloads).await;
		let (store, account) = (self.store.clone(), account_id.to_owned());
		blocking(move || {
			store.write(&account, &stored)?;
			store.collect_garbage();
			Ok(())
		})
		.await
	}

	/// Текущий скин и плащи. Сервер спрашивается не чаще `policy`, ответы при сбое — из сохранённого.
	pub async fn profile<F, Fut>(
		&self,
		account_id: &str,
		kind: AccountKind,
		refresh: bool,
		token: F,
	) -> Result<Outcome<SkinProfile>, AppError>
	where
		F: Fn(bool) -> Fut,
		Fut: Future<Output = Result<String, AppError>>,
	{
		ensure_microsoft(kind)?;
		let slot = self.slot(account_id);
		let mut slot = slot.lock().await;
		let saved = self.view(account_id, false, None).await?;
		let unchanged = |value| Outcome {
			value,
			changed: false,
		};
		let degraded = |error: AppError| match &saved {
			Some(profile) => Ok(unchanged(SkinProfile {
				stale: true,
				error: Some(error),
				..profile.clone()
			})),
			None => Err(error),
		};

		// `refetch` снимает окно только для первого чтения: если и оно не удалось, окно действует.
		let must_read = slot.refetch && slot.last_error.is_none();
		if !must_read && let Some(last) = slot.last_attempt {
			let window = if refresh {
				self.policy.refresh_every
			} else {
				self.policy.every
			};
			let wait = window.saturating_sub(last.elapsed());
			if !wait.is_zero() {
				match (&slot.last_error, &saved) {
					(None, Some(profile)) => return Ok(unchanged(profile.clone())),
					(None, None) => {}
					(Some(_), _) if refresh => return degraded(too_often(wait)),
					(Some(err), _) => return degraded(err.clone()),
				}
			}
		}
		if let Some(wait) = self.mojang.pause_left() {
			return degraded(too_often(wait));
		}

		let parsed = match self.mojang.call(None, Call::GetProfile, &token).await {
			Ok(Some(parsed)) => parsed,
			Ok(None) => {
				let err = AppError::unknown("ответ сервера не похож на профиль");
				slot.last_attempt = Some(Instant::now());
				slot.last_error = Some(err.clone());
				return degraded(err);
			}
			Err(Failed { error, delivered }) => {
				// Не запоминаем: запрос не дошёл до сервера, а после входа токен будет новый.
				if delivered && error.code != ErrorCode::NeedsRelogin {
					slot.last_attempt = Some(Instant::now());
					slot.last_error = Some(error.clone());
				}
				return degraded(error);
			}
		};
		let (stored, downloads) = parsed.into_stored(now_iso());
		if let Err(err) = self.commit(account_id, stored, downloads, None).await {
			return degraded(err);
		}
		*slot = ProfileSlot {
			last_attempt: Some(Instant::now()),
			last_error: None,
			refetch: false,
		};
		let value = self
			.view(account_id, false, None)
			.await?
			.ok_or_else(|| AppError::storage("профиль не записан"))?;
		let changed = !same_skin(saved.as_ref(), Some(&value));
		Ok(Outcome { value, changed })
	}

	/// Сохранённые профили без сети: только скины с текстурой.
	pub async fn worn(&self) -> Result<BTreeMap<String, WornSkin>, AppError> {
		let (store, library) = (self.store.clone(), self.library.clone());
		blocking(move || {
			library.ensure_readable()?;
			Ok(store
				.accounts()
				.into_iter()
				.filter_map(|account| {
					let skin = view_of(&store, &library, &account, false, None)?.skin?;
					skin.texture.is_some().then_some((account, skin))
				})
				.collect())
		})
		.await
	}

	/// Кладёт текущий скин в библиотеку: текстура из кэша, имя — ник. Уже есть — та же запись.
	pub async fn save_current(
		&self,
		account_id: &str,
		kind: AccountKind,
		nick: &str,
	) -> Result<SkinEntry, AppError> {
		ensure_microsoft(kind)?;
		let (store, library, account, nick) = (
			self.store.clone(),
			self.library.clone(),
			account_id.to_owned(),
			nick.to_owned(),
		);
		blocking(move || {
			let skin = store
				.read(&account)
				.and_then(|profile| profile.skin)
				.ok_or_else(|| AppError::new(ErrorCode::SkinNotFound))?;
			if skin.default {
				return Err(AppError::coded(
					ErrorCode::InvalidValue,
					"обычный скин Minecraft не сохраняется",
				));
			}
			let png = store
				.texture(&skin.key)
				.ok_or_else(|| AppError::new(ErrorCode::SkinNotFound))?;
			library.add_bytes(&png, &nick, Some(&skin.key))
		})
		.await
	}

	/// Сначала скин, потом плащ, не больше двух запросов и только то, что попросили.
	/// Скин не прошёл — плащ не отправляется. Плащ не прошёл после скина — `cape_error`.
	pub async fn apply<F, Fut>(
		&self,
		account_id: &str,
		kind: AccountKind,
		nick: &str,
		request: ApplyRequest,
		token: F,
	) -> Result<Outcome<SkinApplied>, AppError>
	where
		F: Fn(bool) -> Fut,
		Fut: Future<Output = Result<String, AppError>>,
	{
		ensure_microsoft(kind)?;
		let ApplyRequest {
			skin_id,
			model,
			cape,
			allow_lose_previous,
		} = request;
		if skin_id.is_none() && model.is_none() && cape.is_none() {
			return Err(AppError::coded(ErrorCode::InvalidValue, "пустой запрос"));
		}
		let cape = cape.map(normalize_cape).transpose()?;
		with_library(&self.library, Library::ensure_readable).await?;
		let permit = self.mojang.begin(account_id)?;
		let slot = self.slot(account_id);
		let mut slot = slot.lock().await;
		// Только под замком: идущее чтение могло как раз узнать, что сейчас на аккаунте.
		let saved = self.view(account_id, false, None).await?;
		let current = saved.as_ref().and_then(|profile| profile.skin.as_ref());
		if skin_id.is_none() && model.is_some() && current.is_some_and(|skin| skin.is_default) {
			return Err(AppError::coded(
				ErrorCode::InvalidValue,
				"обычный скин Minecraft не загружается как свой",
			));
		}
		let upload = self
			.prepare_upload(account_id, skin_id.as_deref(), model, saved.as_ref())
			.await?;
		// Страховка: новый скин заменит несохранённый, а другой копии у него нет.
		let previous = if skin_id.is_some() {
			self.previous_skin(saved.as_ref(), allow_lose_previous)
				.await?
		} else {
			None
		};

		let (store, account) = (self.store.clone(), account_id.to_owned());
		let mut draft = blocking(move || Ok(store.read(&account).unwrap_or_default())).await?;
		let mut downloads = Vec::new();
		let mut unparsed = false;
		let mut absorb = |draft: &mut StoredProfile, parsed: ParsedProfile| {
			let (stored, more) = parsed.into_stored(now_iso());
			*draft = stored;
			downloads.extend(more);
		};

		let mut uploaded = None;
		if let Some(upload) = upload {
			let call = Call::Upload {
				png: &upload.png,
				model: upload.model,
			};
			match self.mojang.call(Some(&permit), call, &token).await {
				Ok(Some(parsed)) => absorb(&mut draft, parsed),
				Ok(None) => {
					unparsed = true;
					draft.skin = Some(StoredSkin {
						key: upload.id.clone(),
						model: upload.model,
						default: false,
					});
				}
				Err(failed) => return Err(failed.error),
			}
			uploaded = Some(upload);
		}

		let mut cape_error = None;
		if let Some(cape) = cape {
			let call = match &cape {
				Some(id) => Call::ShowCape(id),
				None => Call::HideCape,
			};
			match self.mojang.call(Some(&permit), call, &token).await {
				Ok(Some(parsed)) => absorb(&mut draft, parsed),
				Ok(None) => {
					unparsed = true;
					draft.active_cape_id = cape;
				}
				Err(failed) if uploaded.is_some() => cape_error = Some(failed.error),
				Err(failed) => return Err(failed.error),
			}
		}
		drop(permit);

		draft.fetched_at = now_iso();
		let new_key = draft.skin.as_ref().map(|skin| skin.key.clone());
		let local = uploaded
			.as_ref()
			.zip(new_key.clone())
			.map(|(upload, key)| (key, upload.png.clone()));
		self.commit(account_id, draft, downloads, local)
			.await
			.map_err(|err| server_changed_but(&err))?;
		*slot = ProfileSlot {
			last_attempt: Some(Instant::now()),
			last_error: None,
			refetch: unparsed,
		};

		let library_id = uploaded
			.as_ref()
			.and_then(|upload| upload.library_id.clone());
		let saved_previous = self
			.after_upload(library_id, new_key.as_deref(), previous, nick)
			.await;

		let value = self
			.view(account_id, unparsed, None)
			.await?
			.ok_or_else(|| server_changed_but(&AppError::storage("профиль не записан")))?;
		let changed = !same_skin(saved.as_ref(), Some(&value));
		Ok(Outcome {
			changed: changed || saved_previous.is_some(),
			value: SkinApplied {
				profile: value,
				cape_error,
				saved_previous,
			},
		})
	}

	/// Текстура несохранённого скина, который заменит загрузка. Если сохранить его нечем
	/// (нет профиля или текстуры), без согласия пользователя запрос не уходит.
	async fn previous_skin(
		&self,
		saved: Option<&SkinProfile>,
		allow_lose: bool,
	) -> Result<Option<(String, Vec<u8>)>, AppError> {
		let lost = |reason: &str| {
			if allow_lose {
				Ok(None)
			} else {
				Err(AppError::coded(ErrorCode::SkinPreviousUnknown, reason))
			}
		};
		let Some(profile) = saved else {
			return lost("no_profile");
		};
		let Some(skin) = profile
			.skin
			.as_ref()
			.filter(|skin| !skin.is_default && skin.skin_id.is_none())
		else {
			return Ok(None);
		};
		let (store, key) = (self.store.clone(), skin.key.clone());
		match blocking(move || Ok(store.texture(&key).map(|png| (key, png)))).await? {
			Some(previous) => Ok(Some(previous)),
			None => lost("no_texture"),
		}
	}

	async fn prepare_upload(
		&self,
		account_id: &str,
		skin_id: Option<&str>,
		model: Option<SkinModel>,
		saved: Option<&SkinProfile>,
	) -> Result<Option<Upload>, AppError> {
		if let Some(id) = skin_id {
			let id = id.to_owned();
			return with_library(&self.library, move |library| {
				let png = library.read_png(&id)?;
				let model = match model {
					Some(model) => model,
					None => library.model_of(&id)?,
				};
				Ok(Some(Upload {
					id: id.clone(),
					library_id: Some(id),
					png,
					model,
				}))
			})
			.await;
		}
		let Some(model) = model else {
			return Ok(None);
		};
		// Другие руки у текущего скина: загружается его текстура из кэша.
		let library_id = saved
			.and_then(|profile| profile.skin.as_ref())
			.and_then(|skin| skin.skin_id.clone());
		let (store, account) = (self.store.clone(), account_id.to_owned());
		blocking(move || {
			let not_found = || AppError::new(ErrorCode::SkinNotFound);
			let skin = store
				.read(&account)
				.and_then(|profile| profile.skin)
				.ok_or_else(not_found)?;
			let png = store.texture(&skin.key).ok_or_else(not_found)?;
			let id = png::parse(&png).map_err(|_| not_found())?.id;
			Ok(Some(Upload {
				id,
				library_id,
				png,
				model,
			}))
		})
		.await
	}

	/// После успеха: запомнить ключ за скином и сохранить прежний. Сервер уже изменён,
	/// поэтому сбой здесь только в лог.
	async fn after_upload(
		&self,
		library_id: Option<String>,
		new_key: Option<&str>,
		previous: Option<(String, Vec<u8>)>,
		nick: &str,
	) -> Option<SkinEntry> {
		let new_key = new_key.map(str::to_owned);
		let nick = nick.to_owned();
		let library = self.library.clone();
		let saved = blocking(move || {
			if let (Some(id), Some(key)) = (&library_id, &new_key)
				&& let Err(err) = library.remember_key(key, id)
			{
				log::warn!("ключ текстуры не записан: {err}");
			}
			let Some((key, png)) = previous.filter(|(key, _)| Some(key) != new_key.as_ref()) else {
				return Ok(None);
			};
			library.add_bytes(&png, &nick, Some(&key)).map(Some)
		})
		.await;
		saved.unwrap_or_else(|err| {
			log::warn!("прежний скин не сохранён: {err}");
			None
		})
	}

	/// На аккаунте будет обычный скин Minecraft. Ответ сразу заменяет профиль.
	pub async fn reset<F, Fut>(
		&self,
		account_id: &str,
		kind: AccountKind,
		token: F,
	) -> Result<Outcome<SkinProfile>, AppError>
	where
		F: Fn(bool) -> Fut,
		Fut: Future<Output = Result<String, AppError>>,
	{
		ensure_microsoft(kind)?;
		// `DELETE` не отменить, поэтому сначала убеждаемся, что результат будет куда записать.
		with_library(&self.library, Library::ensure_readable).await?;
		let saved = self.view(account_id, false, None).await?;
		let permit = self.mojang.begin(account_id)?;
		let slot = self.slot(account_id);
		let mut slot = slot.lock().await;
		let parsed = self
			.mojang
			.call(Some(&permit), Call::ResetSkin, &token)
			.await
			.map_err(|failed| failed.error)?;
		drop(permit);

		let unparsed = parsed.is_none();
		let (stored, downloads) = match parsed {
			Some(parsed) => parsed.into_stored(now_iso()),
			None => {
				let (store, account) = (self.store.clone(), account_id.to_owned());
				let mut stored =
					blocking(move || Ok(store.read(&account).unwrap_or_default())).await?;
				stored.skin = None;
				stored.fetched_at = now_iso();
				(stored, Vec::new())
			}
		};
		self.commit(account_id, stored, downloads, None)
			.await
			.map_err(|err| server_changed_but(&err))?;
		*slot = ProfileSlot {
			last_attempt: Some(Instant::now()),
			last_error: None,
			refetch: unparsed,
		};
		let value = self
			.view(account_id, unparsed, None)
			.await?
			.ok_or_else(|| server_changed_but(&AppError::storage("профиль не записан")))?;
		let changed = !same_skin(saved.as_ref(), Some(&value));
		Ok(Outcome { value, changed })
	}

	/// Аккаунт удалён: профиль и его текстуры больше не нужны. Библиотека остаётся.
	pub async fn forget(&self, account_id: &str) -> Result<bool, AppError> {
		self.slots
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner)
			.remove(account_id);
		let (store, account) = (self.store.clone(), account_id.to_owned());
		blocking(move || {
			let had = store.read(&account).is_some();
			let _guard = store.lock.blocking_lock();
			store.forget(&account);
			Ok(had)
		})
		.await
	}
}

struct Upload {
	/// sha1 картинки.
	id: String,
	library_id: Option<String>,
	png: Vec<u8>,
	model: SkinModel,
}

fn normalize_cape(cape: Option<String>) -> Result<Option<String>, AppError> {
	cape.map(|id| {
		Uuid::parse_str(&id)
			.map(|id| id.hyphenated().to_string())
			.map_err(|_| AppError::coded(ErrorCode::InvalidValue, "capeId"))
	})
	.transpose()
}

/// Раздел «Скины» закрыт для офлайн-аккаунтов.
fn ensure_microsoft(kind: AccountKind) -> Result<(), AppError> {
	match kind {
		AccountKind::Microsoft => Ok(()),
		AccountKind::Offline => Err(AppError::coded(
			ErrorCode::InvalidValue,
			"скины доступны только аккаунтам Microsoft",
		)),
	}
}

fn server_changed_but(err: &AppError) -> AppError {
	let details = err.details.as_deref().unwrap_or_default();
	AppError::storage(format!("{SERVER_CHANGED}. {details}"))
}

const SERVER_CHANGED: &str = "на сервере скин уже сменён";

#[cfg(test)]
mod tests {
	use std::net::SocketAddr;
	use std::sync::Mutex;
	use std::time::Duration;

	use tokio::io::{AsyncReadExt, AsyncWriteExt};

	use super::png::test_util::skin;
	use super::*;

	#[derive(Debug, Clone)]
	struct Seen {
		method: String,
		path: String,
		/// Заголовки в нижнем регистре.
		head: String,
		body: Vec<u8>,
	}

	type Log = Arc<Mutex<Vec<Seen>>>;

	fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
		hay.windows(needle.len()).position(|w| w == needle)
	}

	async fn read_request(socket: &mut tokio::net::TcpStream) -> Option<Seen> {
		let mut bytes = Vec::new();
		let mut buf = [0u8; 8192];
		let head_end = loop {
			if let Some(at) = find(&bytes, b"\r\n\r\n") {
				break at + 4;
			}
			let n = socket.read(&mut buf).await.ok()?;
			if n == 0 {
				return None;
			}
			bytes.extend_from_slice(&buf[..n]);
		};
		let head = String::from_utf8_lossy(&bytes[..head_end]).to_lowercase();
		let length: usize = head
			.lines()
			.find_map(|line| line.strip_prefix("content-length:"))
			.and_then(|value| value.trim().parse().ok())
			.unwrap_or(0);
		while bytes.len() < head_end + length {
			let n = socket.read(&mut buf).await.ok()?;
			if n == 0 {
				break;
			}
			bytes.extend_from_slice(&buf[..n]);
		}
		let first = String::from_utf8_lossy(&bytes[..head_end])
			.lines()
			.next()
			.unwrap_or_default()
			.to_owned();
		let mut parts = first.split(' ');
		Some(Seen {
			method: parts.next().unwrap_or_default().to_owned(),
			path: parts.next().unwrap_or_default().to_owned(),
			head,
			body: bytes[head_end..].to_vec(),
		})
	}

	/// Отвечает по очереди, записывая каждый запрос; очередь кончилась — 500.
	async fn server(responses: Vec<String>) -> (String, Log) {
		server_with(|_| responses).await
	}

	/// То же, ответы могут ссылаться на адрес самого сервера.
	async fn server_with(build: impl FnOnce(&str) -> Vec<String>) -> (String, Log) {
		let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
		let addr: SocketAddr = listener.local_addr().unwrap();
		let base = format!("http://{addr}");
		let responses = build(&base);
		let log: Log = Arc::default();
		let seen = log.clone();
		tokio::spawn(async move {
			let mut responses = responses.into_iter();
			while let Ok((mut socket, _)) = listener.accept().await {
				let Some(request) = read_request(&mut socket).await else {
					continue;
				};
				seen.lock().unwrap().push(request);
				let response = responses
					.next()
					.unwrap_or_else(|| reply("500 Internal Server Error", "", ""));
				let _ = socket.write_all(response.as_bytes()).await;
				let _ = socket.shutdown().await;
			}
		});
		(base, log)
	}

	fn reply(status: &str, headers: &str, body: &str) -> String {
		format!(
			"HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}",
			body.len()
		)
	}

	fn ok() -> String {
		reply("200 OK", "", "{}")
	}

	struct Fixture {
		_dir: tempfile::TempDir,
		skins: Skins,
		skin: SkinEntry,
	}

	/// Запросы только к подставному API: на этот адрес текстур никто не отвечает.
	fn with_mojang(dir: PathBuf, mojang: Mojang) -> Skins {
		let textures = Textures::new(textures::build_client().unwrap(), "http://127.0.0.1:9");
		Skins::with_parts(dir, mojang, textures)
	}

	impl Skins {
		/// Надеть скин библиотеки.
		async fn wear<F, Fut>(
			&self,
			account_id: &str,
			kind: AccountKind,
			skin_id: &str,
			model: SkinModel,
			token: F,
		) -> Result<SkinApplied, AppError>
		where
			F: Fn(bool) -> Fut,
			Fut: Future<Output = Result<String, AppError>>,
		{
			let request = ApplyRequest {
				skin_id: Some(skin_id.to_owned()),
				model: Some(model),
				cape: None,
				allow_lose_previous: true,
			};
			self.apply(account_id, kind, "Steve", request, token)
				.await
				.map(|outcome| outcome.value)
		}
	}

	fn fixture_with(base: &str, limiter: Limiter) -> Fixture {
		let dir = tempfile::tempdir().unwrap();
		let http = mojang::build_client().unwrap();
		let skins = with_mojang(dir.path().join("skins"), Mojang::new(http, base, limiter));
		let source = dir.path().join("Рыцарь.png");
		std::fs::write(&source, skin(64, 64, 5)).unwrap();
		let skin = skins
			.library
			.add_files(&[source.display().to_string()])
			.unwrap()
			.added
			.remove(0);
		Fixture {
			_dir: dir,
			skins,
			skin,
		}
	}

	/// Без пауз: проверяется ответ сервера, а не частота.
	fn fixture(base: &str) -> Fixture {
		fixture_with(base, Limiter::new(Duration::ZERO, Duration::ZERO))
	}

	async fn fixed_token(_force: bool) -> Result<String, AppError> {
		Ok("tok-1".to_owned())
	}

	impl Fixture {
		async fn apply(&self, account: &str) -> Result<SkinApplied, AppError> {
			self.skins
				.wear(
					account,
					AccountKind::Microsoft,
					&self.skin.id,
					SkinModel::Slim,
					fixed_token,
				)
				.await
		}

		fn worn_count(&self) -> usize {
			let store = &self.skins.store;
			store
				.accounts()
				.iter()
				.filter(|account| {
					view_of(store, &self.skins.library, account, false, None)
						.is_some_and(|profile| profile.skin.is_some())
				})
				.count()
		}
	}

	#[tokio::test]
	async fn upload_form_and_headers() {
		let (base, log) = server(vec![ok()]).await;
		let f = fixture(&base);
		let worn = f.apply("acc").await.unwrap().profile.skin.unwrap();
		assert_eq!(worn.skin_id.as_deref(), Some(f.skin.id.as_str()));
		assert_eq!(worn.model, SkinModel::Slim);

		let seen = log.lock().unwrap().clone();
		assert_eq!(seen.len(), 1);
		let request = &seen[0];
		assert_eq!(request.method, "POST");
		assert_eq!(request.path, "/minecraft/profile/skins");
		assert!(request.head.contains("authorization: bearer tok-1"));
		assert!(request.head.contains("accept: application/json"));
		assert!(request.head.contains("user-agent: leelauncher/"));
		assert!(request.head.contains("content-type: multipart/form-data"));

		let body = String::from_utf8_lossy(&request.body);
		assert!(body.contains("name=\"variant\"\r\n\r\nslim\r\n"), "{body}");
		assert!(
			body.contains("name=\"file\"; filename=\"skin.png\""),
			"{body}"
		);
		assert!(body.contains("Content-Type: image/png"), "{body}");
		let png = f.skins.library.read_png(&f.skin.id).unwrap();
		assert!(find(&request.body, &png).is_some());
	}

	#[tokio::test]
	async fn classic_variant_is_sent() {
		let (base, log) = server(vec![ok()]).await;
		let f = fixture(&base);
		f.skins
			.wear(
				"acc",
				AccountKind::Microsoft,
				&f.skin.id,
				SkinModel::Classic,
				fixed_token,
			)
			.await
			.unwrap();
		let seen = log.lock().unwrap().clone();
		assert!(String::from_utf8_lossy(&seen[0].body).contains("\r\n\r\nclassic\r\n"));
	}

	#[tokio::test]
	async fn reset_request() {
		let (base, log) = server(vec![ok(), ok()]).await;
		let f = fixture(&base);
		f.apply("acc").await.unwrap();
		assert_eq!(f.worn_count(), 1);
		f.skins
			.reset("acc", AccountKind::Microsoft, fixed_token)
			.await
			.unwrap();
		assert_eq!(f.worn_count(), 0);
		let seen = log.lock().unwrap().clone();
		assert_eq!(seen[1].method, "DELETE");
		assert_eq!(seen[1].path, "/minecraft/profile/skins/active");
		assert!(seen[1].head.contains("authorization: bearer tok-1"));

		let err = f
			.skins
			.reset("acc", AccountKind::Offline, fixed_token)
			.await
			.unwrap_err();
		assert_eq!(err.code, ErrorCode::InvalidValue);
		assert_eq!(log.lock().unwrap().len(), 2);
	}

	#[tokio::test]
	async fn status_codes() {
		let cases = [
			(
				reply("400 Bad Request", "", r#"{"errorMessage":"bad skin"}"#),
				ErrorCode::SkinRejected,
				Some("bad skin"),
			),
			(
				reply("404 Not Found", "", "{}"),
				ErrorCode::NoMinecraft,
				None,
			),
			(
				reply("429 Too Many Requests", "", ""),
				ErrorCode::RateLimited,
				None,
			),
			(
				reply("500 Internal Server Error", "", ""),
				ErrorCode::ServiceUnavailable,
				None,
			),
			(
				reply("503 Service Unavailable", "", ""),
				ErrorCode::ServiceUnavailable,
				None,
			),
			(
				reply("403 Forbidden", "", "nope"),
				ErrorCode::Unknown,
				Some("HTTP 403: nope"),
			),
			(
				reply("418 Teapot", "", ""),
				ErrorCode::Unknown,
				Some("HTTP 418"),
			),
		];
		for (response, code, details) in cases {
			let (base, log) = server(vec![response, ok()]).await;
			let f = fixture(&base);
			let err = f.apply("acc").await.unwrap_err();
			assert_eq!(err.code, code, "{details:?}");
			if let Some(details) = details {
				assert!(err.details.as_deref().unwrap().contains(details), "{err:?}");
			}
			// Без повторов и без записи.
			assert_eq!(log.lock().unwrap().len(), 1, "{code:?}");
			assert_eq!(f.worn_count(), 0);
		}
	}

	#[tokio::test]
	async fn redirect_is_not_followed() {
		let (base, log) = server(vec![reply("302 Found", "Location: /stolen\r\n", ""), ok()]).await;
		let f = fixture(&base);
		let err = f.apply("acc").await.unwrap_err();
		assert_eq!(err.code, ErrorCode::Unknown);
		tokio::time::sleep(Duration::from_millis(100)).await;
		assert_eq!(log.lock().unwrap().len(), 1);
		assert_eq!(f.worn_count(), 0);
	}

	#[tokio::test]
	async fn token_is_masked_in_details() {
		let (base, _log) = server(vec![reply("400 Bad Request", "", "echo tok-1 end")]).await;
		let f = fixture(&base);
		let err = f.apply("acc").await.unwrap_err();
		let details = err.details.unwrap();
		assert!(!details.contains("tok-1"), "{details}");
		assert!(details.contains("***"));
	}

	#[tokio::test]
	async fn long_error_body_is_cut() {
		let body = "я".repeat(5000);
		let (base, _log) = server(vec![reply("400 Bad Request", "", &body)]).await;
		let f = fixture(&base);
		let details = f.apply("acc").await.unwrap_err().details.unwrap();
		assert!(details.len() < 700, "{}", details.len());
	}

	fn recording_token(
		calls: &Mutex<Vec<bool>>,
		fail_force: Option<ErrorCode>,
	) -> impl Fn(bool) -> std::future::Ready<Result<String, AppError>> {
		move |force| {
			calls.lock().unwrap().push(force);
			std::future::ready(match (force, fail_force) {
				(true, Some(code)) => Err(AppError::new(code)),
				(true, None) => Ok("tok-2".to_owned()),
				(false, _) => Ok("tok-1".to_owned()),
			})
		}
	}

	#[tokio::test]
	async fn retries_once_after_401() {
		let (base, log) = server(vec![reply("401 Unauthorized", "", ""), ok()]).await;
		let f = fixture(&base);
		let calls = Mutex::new(Vec::new());
		f.skins
			.wear(
				"acc",
				AccountKind::Microsoft,
				&f.skin.id,
				SkinModel::Slim,
				recording_token(&calls, None),
			)
			.await
			.unwrap();
		assert_eq!(*calls.lock().unwrap(), [false, true]);
		let seen = log.lock().unwrap().clone();
		assert_eq!(seen.len(), 2);
		assert!(seen[0].head.contains("bearer tok-1"));
		assert!(seen[1].head.contains("bearer tok-2"));
		// Форма собирается заново и при повторе полная.
		assert_eq!(seen[0].body.len(), seen[1].body.len());
		assert_eq!(f.worn_count(), 1);
	}

	#[tokio::test]
	async fn second_401_needs_relogin() {
		let unauthorized = || reply("401 Unauthorized", "", "");
		let (base, log) = server(vec![unauthorized(), unauthorized(), ok()]).await;
		let f = fixture(&base);
		let calls = Mutex::new(Vec::new());
		let err = f
			.skins
			.reset("acc", AccountKind::Microsoft, recording_token(&calls, None))
			.await
			.unwrap_err();
		assert_eq!(err.code, ErrorCode::NeedsRelogin);
		assert_eq!(log.lock().unwrap().len(), 2);
		assert_eq!(*calls.lock().unwrap(), [false, true]);
	}

	#[tokio::test]
	async fn revoked_token_stops_retry() {
		let (base, log) = server(vec![reply("401 Unauthorized", "", ""), ok()]).await;
		let f = fixture(&base);
		let calls = Mutex::new(Vec::new());
		let err = f
			.skins
			.wear(
				"acc",
				AccountKind::Microsoft,
				&f.skin.id,
				SkinModel::Slim,
				recording_token(&calls, Some(ErrorCode::NeedsRelogin)),
			)
			.await
			.unwrap_err();
		assert_eq!(err.code, ErrorCode::NeedsRelogin);
		assert_eq!(log.lock().unwrap().len(), 1);
		assert_eq!(f.worn_count(), 0);
	}

	#[tokio::test]
	async fn no_token_no_request_no_pause() {
		let (base, log) = server(vec![ok()]).await;
		let f = fixture_with(&base, Limiter::default());
		let err = f
			.skins
			.wear(
				"acc",
				AccountKind::Microsoft,
				&f.skin.id,
				SkinModel::Slim,
				|_| async { Err(AppError::new(ErrorCode::NeedsRelogin)) },
			)
			.await
			.unwrap_err();
		assert_eq!(err.code, ErrorCode::NeedsRelogin);
		assert!(log.lock().unwrap().is_empty());
		// Запрос не уходил, поэтому паузы нет.
		f.apply("acc").await.unwrap();
	}

	#[tokio::test]
	async fn ten_second_pause_blocks_second_request() {
		let (base, log) = server(vec![ok(), ok(), ok()]).await;
		let f = fixture_with(&base, Limiter::default());
		f.apply("acc").await.unwrap();

		let err = f.apply("acc").await.unwrap_err();
		assert_eq!(err.code, ErrorCode::SkinTooOften);
		let wait: u64 = err.details.unwrap().parse().unwrap();
		assert!((9..=10).contains(&wait), "{wait}");
		let err = f
			.skins
			.reset("acc", AccountKind::Microsoft, fixed_token)
			.await
			.unwrap_err();
		assert_eq!(err.code, ErrorCode::SkinTooOften);
		assert_eq!(log.lock().unwrap().len(), 1);

		// У другого аккаунта своя пауза.
		f.apply("other").await.unwrap();
		assert_eq!(log.lock().unwrap().len(), 2);
	}

	#[tokio::test]
	async fn request_allowed_after_interval() {
		let (base, log) = server(vec![ok(), ok()]).await;
		let limiter = Limiter::new(Duration::from_millis(150), Duration::ZERO);
		let f = fixture_with(&base, limiter);
		f.apply("acc").await.unwrap();
		assert_eq!(
			f.apply("acc").await.unwrap_err().code,
			ErrorCode::SkinTooOften
		);
		tokio::time::sleep(Duration::from_millis(200)).await;
		f.apply("acc").await.unwrap();
		assert_eq!(log.lock().unwrap().len(), 2);
	}

	#[tokio::test]
	async fn one_request_at_a_time() {
		let (base, log) = server(vec![ok(), ok()]).await;
		let f = fixture_with(&base, Limiter::new(Duration::ZERO, Duration::ZERO));
		let (entered_tx, entered_rx) = tokio::sync::oneshot::channel::<()>();
		let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
		let gate = Mutex::new(Some((entered_tx, release_rx)));
		let slow = f.skins.wear(
			"acc",
			AccountKind::Microsoft,
			&f.skin.id,
			SkinModel::Slim,
			|_| {
				let parts = gate.lock().unwrap().take();
				async move {
					if let Some((entered, release)) = parts {
						let _ = entered.send(());
						let _ = release.await;
					}
					Ok("tok-1".to_owned())
				}
			},
		);
		let second = async {
			let _ = entered_rx.await;
			let err = f.apply("acc").await.unwrap_err();
			let _ = release_tx.send(());
			err
		};
		let (first, err) = tokio::join!(slow, second);
		first.unwrap();
		assert_eq!(err.code, ErrorCode::SkinTooOften);
		assert_eq!(log.lock().unwrap().len(), 1);
	}

	#[tokio::test]
	async fn rate_limit_pauses_without_retry() {
		let (base, log) = server(vec![reply("429 Too Many Requests", "", ""), ok()]).await;
		let f = fixture_with(&base, Limiter::default());
		let err = f.apply("acc").await.unwrap_err();
		assert_eq!(err.code, ErrorCode::RateLimited);
		// Заголовка нет, но настоящая пауза известна.
		assert_eq!(err.details.as_deref(), Some("120"));
		tokio::time::sleep(Duration::from_millis(100)).await;
		assert_eq!(log.lock().unwrap().len(), 1, "без автоматического повтора");

		let err = f.apply("acc").await.unwrap_err();
		assert_eq!(err.code, ErrorCode::SkinTooOften);
		let wait: u64 = err.details.unwrap().parse().unwrap();
		assert!((118..=120).contains(&wait), "{wait}");
		// И для других аккаунтов.
		assert_eq!(
			f.apply("other").await.unwrap_err().code,
			ErrorCode::SkinTooOften
		);
		assert_eq!(log.lock().unwrap().len(), 1);
	}

	#[tokio::test]
	async fn retry_after_extends_pause() {
		let (base, log) = server(vec![
			reply("429 Too Many Requests", "Retry-After: 300\r\n", ""),
			ok(),
		])
		.await;
		let f = fixture_with(&base, Limiter::default());
		let err = f.apply("acc").await.unwrap_err();
		assert_eq!(err.code, ErrorCode::RateLimited);
		assert_eq!(err.details.as_deref(), Some("300"));
		let err = f.apply("acc").await.unwrap_err();
		let wait: u64 = err.details.unwrap().parse().unwrap();
		assert!((298..=300).contains(&wait), "{wait}");
		assert_eq!(log.lock().unwrap().len(), 1);
	}

	#[tokio::test]
	async fn short_retry_after_keeps_minimum() {
		let (base, _log) = server(vec![reply(
			"429 Too Many Requests",
			"Retry-After: 5\r\n",
			"",
		)])
		.await;
		let f = fixture_with(&base, Limiter::default());
		let err = f.apply("acc").await.unwrap_err();
		assert_eq!(err.details.as_deref(), Some("120"));
		let wait: u64 = f
			.apply("acc")
			.await
			.unwrap_err()
			.details
			.unwrap()
			.parse()
			.unwrap();
		assert!((118..=120).contains(&wait), "{wait}");
	}

	fn seconds_in(err: &AppError) -> u64 {
		err.details.as_deref().unwrap().parse().unwrap()
	}

	/// A получил 429 на час, пока запрос B был в пути; ответ B (429 без заголовка) не сокращает паузу.
	#[tokio::test]
	async fn later_429_does_not_shorten_pause() {
		let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
		let base = format!("http://{}", listener.local_addr().unwrap());
		let log: Log = Arc::default();
		let seen = log.clone();
		tokio::spawn(async move {
			let mut number = 0;
			while let Ok((mut socket, _)) = listener.accept().await {
				number += 1;
				let (delay, headers) = if number == 1 {
					(300, "")
				} else {
					(0, "Retry-After: 3600\r\n")
				};
				let seen = seen.clone();
				tokio::spawn(async move {
					if let Some(request) = read_request(&mut socket).await {
						seen.lock().unwrap().push(request);
					}
					tokio::time::sleep(Duration::from_millis(delay)).await;
					let response = reply("429 Too Many Requests", headers, "");
					let _ = socket.write_all(response.as_bytes()).await;
					let _ = socket.shutdown().await;
				});
			}
		});
		let f = fixture_with(&base, Limiter::default());
		let second = async {
			tokio::time::sleep(Duration::from_millis(100)).await;
			f.apply("a").await
		};
		let (slow, fast) = tokio::join!(f.apply("b"), second);
		let (slow, fast) = (slow.unwrap_err(), fast.unwrap_err());
		assert_eq!(fast.code, ErrorCode::RateLimited);
		assert_eq!(fast.details.as_deref(), Some("3600"));
		assert_eq!(slow.code, ErrorCode::RateLimited);
		// Настоящая пауза, а не минимум 120 с.
		assert!(seconds_in(&slow) > 3500, "{slow:?}");

		let next = f.apply("c").await.unwrap_err();
		assert_eq!(next.code, ErrorCode::SkinTooOften);
		assert!(seconds_in(&next) > 3500, "{next:?}");
		assert_eq!(log.lock().unwrap().len(), 2);
	}

	#[tokio::test]
	async fn rate_limit_on_token_refresh_starts_pause() {
		let (base, log) = server(vec![ok()]).await;
		let f = fixture_with(&base, Limiter::default());
		let err = f
			.skins
			.wear(
				"acc",
				AccountKind::Microsoft,
				&f.skin.id,
				SkinModel::Slim,
				|_| async {
					Err(AppError::coded(
						ErrorCode::RateLimited,
						"minecraft token: HTTP 429 тело ответа",
					))
				},
			)
			.await
			.unwrap_err();
		assert_eq!(err.code, ErrorCode::RateLimited);
		// Только число секунд, без тела ответа.
		assert_eq!(err.details.as_deref(), Some("120"));
		assert!(log.lock().unwrap().is_empty());

		let next = f.apply("other").await.unwrap_err();
		assert_eq!(next.code, ErrorCode::SkinTooOften);
		assert!(log.lock().unwrap().is_empty());
	}

	#[tokio::test]
	async fn rate_limit_on_forced_refresh_starts_pause() {
		let (base, log) = server(vec![reply("401 Unauthorized", "", ""), ok()]).await;
		let f = fixture_with(&base, Limiter::default());
		let calls = Mutex::new(Vec::new());
		let err = f
			.skins
			.wear(
				"acc",
				AccountKind::Microsoft,
				&f.skin.id,
				SkinModel::Slim,
				recording_token(&calls, Some(ErrorCode::RateLimited)),
			)
			.await
			.unwrap_err();
		assert_eq!(err.code, ErrorCode::RateLimited);
		assert_eq!(err.details.as_deref(), Some("120"));
		assert_eq!(log.lock().unwrap().len(), 1);
		let next = f.apply("other").await.unwrap_err();
		assert_eq!(next.code, ErrorCode::SkinTooOften);
	}

	/// Запрос аккаунта B ждёт токен (`wait_on_force`: при повторе после 401), тем временем A получает 429.
	async fn pause_while_waiting_for_token(responses: Vec<String>, wait_on_force: bool) -> usize {
		let (base, log) = server(responses).await;
		let f = fixture_with(&base, Limiter::default());
		let (entered_tx, entered_rx) = tokio::sync::oneshot::channel::<()>();
		let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
		let gate = Mutex::new(Some((entered_tx, release_rx)));
		let waiting = f.skins.wear(
			"b",
			AccountKind::Microsoft,
			&f.skin.id,
			SkinModel::Slim,
			|force| {
				let parts = (force == wait_on_force)
					.then(|| gate.lock().unwrap().take())
					.flatten();
				async move {
					if let Some((entered, release)) = parts {
						let _ = entered.send(());
						let _ = release.await;
					}
					Ok("tok-1".to_owned())
				}
			},
		);
		let limited = async {
			let _ = entered_rx.await;
			let err = f.apply("a").await.unwrap_err();
			let _ = release_tx.send(());
			err
		};
		let (waiting, limited) = tokio::join!(waiting, limited);
		assert_eq!(limited.code, ErrorCode::RateLimited);
		let waiting = waiting.unwrap_err();
		assert_eq!(waiting.code, ErrorCode::SkinTooOften);
		assert!(seconds_in(&waiting) >= 118, "{waiting:?}");
		log.lock().unwrap().len()
	}

	#[tokio::test]
	async fn pause_checked_before_first_send() {
		let responses = vec![reply("429 Too Many Requests", "", ""), ok()];
		assert_eq!(pause_while_waiting_for_token(responses, false).await, 1);
	}

	#[tokio::test]
	async fn pause_checked_before_retry_after_401() {
		let responses = vec![
			reply("401 Unauthorized", "", ""),
			reply("429 Too Many Requests", "", ""),
			ok(),
		];
		assert_eq!(pause_while_waiting_for_token(responses, true).await, 2);
	}

	/// Первый запрос дошёл (401), повтор не смог соединиться: пауза от первого остаётся.
	#[tokio::test]
	async fn lost_retry_keeps_ten_second_pause() {
		let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
		let base = format!("http://{}", listener.local_addr().unwrap());
		tokio::spawn(async move {
			let Ok((mut socket, _)) = listener.accept().await else {
				return;
			};
			let _ = read_request(&mut socket).await;
			// Порт закрывается раньше ответа: повтору соединиться уже не с чем.
			drop(listener);
			let response = reply("401 Unauthorized", "", "");
			let _ = socket.write_all(response.as_bytes()).await;
			let _ = socket.shutdown().await;
		});
		let f = fixture_with(&base, Limiter::default());
		let err = f.apply("acc").await.unwrap_err();
		assert_eq!(err.code, ErrorCode::Network);
		let next = f.apply("acc").await.unwrap_err();
		assert_eq!(next.code, ErrorCode::SkinTooOften);
	}

	const CAPE_A: &str = "11111111-2222-3333-4444-555555555555";
	const CAPE_B: &str = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";

	/// Ответ Mojang по образцу Wiki и `D:\www\Chunk`.
	fn profile_json(key: &str, variant: &str, active_cape: Option<&str>) -> String {
		let cape = |id: &str, name: &str| {
			let state = if active_cape == Some(id) {
				"ACTIVE"
			} else {
				"INACTIVE"
			};
			format!(
				r#"{{"id":"{id}","state":"{state}","url":"http://textures.minecraft.net/texture/cape{name}","alias":"Cape {name}"}}"#
			)
		};
		format!(
			r#"{{"id":"069a79f444e94726a5befca90e38aaf5","name":"Steve","skins":[{{"id":"6a6e65e5-76dd-4c3c-a625-162924514568","state":"ACTIVE","url":"http://textures.minecraft.net/texture/{key}","textureKey":"{key}","variant":"{variant}"}}],"capes":[{},{}]}}"#,
			cape(CAPE_A, "a"),
			cape(CAPE_B, "b"),
		)
	}

	fn profile_reply(key: &str, variant: &str, active_cape: Option<&str>) -> String {
		reply("200 OK", "", &profile_json(key, variant, active_cape))
	}

	fn png_reply(png: &[u8]) -> Vec<u8> {
		let mut out = format!(
			"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
			png.len()
		)
		.into_bytes();
		out.extend_from_slice(png);
		out
	}

	/// Сервер текстур: отвечает по пути, неизвестный путь — 404.
	async fn texture_server(files: Vec<(String, Vec<u8>)>) -> (String, Log) {
		let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
		let base = format!("http://{}", listener.local_addr().unwrap());
		let log: Log = Arc::default();
		let seen = log.clone();
		tokio::spawn(async move {
			while let Ok((mut socket, _)) = listener.accept().await {
				let Some(request) = read_request(&mut socket).await else {
					continue;
				};
				let path = request.path.clone();
				seen.lock().unwrap().push(request);
				let response = files
					.iter()
					.find(|(name, _)| *name == path)
					.map(|(_, bytes)| bytes.clone())
					.unwrap_or_else(|| reply("404 Not Found", "", "").into_bytes());
				let _ = socket.write_all(&response).await;
				let _ = socket.shutdown().await;
			}
		});
		(base, log)
	}

	fn cape_png() -> Vec<u8> {
		skin(64, 32, 200)
	}

	/// Сервер текстур с настоящими картинками для `key`, `capea`, `capeb`.
	async fn textures_for(key: &str, seed: u8) -> (String, Log) {
		texture_server(vec![
			(format!("/texture/{key}"), png_reply(&skin(64, 64, seed))),
			("/texture/capea".to_owned(), png_reply(&cape_png())),
			("/texture/capeb".to_owned(), png_reply(&cape_png())),
		])
		.await
	}

	fn full_fixture(base: &str, textures_base: &str, limiter: Limiter) -> Fixture {
		let dir = tempfile::tempdir().unwrap();
		let skins = Skins::with_parts(
			dir.path().join("skins"),
			Mojang::new(mojang::build_client().unwrap(), base, limiter),
			Textures::new(textures::build_client().unwrap(), textures_base),
		);
		let source = dir.path().join("Рыцарь.png");
		std::fs::write(&source, skin(64, 64, 5)).unwrap();
		let skin = skins
			.library
			.add_files(&[source.display().to_string()])
			.unwrap()
			.added
			.remove(0);
		Fixture {
			_dir: dir,
			skins,
			skin,
		}
	}

	fn free_limiter() -> Limiter {
		Limiter::new(Duration::ZERO, Duration::ZERO)
	}

	impl Fixture {
		async fn read(&self, refresh: bool) -> Result<Outcome<SkinProfile>, AppError> {
			self.skins
				.profile("acc", AccountKind::Microsoft, refresh, fixed_token)
				.await
		}

		async fn send(&self, request: ApplyRequest) -> Result<Outcome<SkinApplied>, AppError> {
			self.skins
				.apply("acc", AccountKind::Microsoft, "Steve", request, fixed_token)
				.await
		}

		fn textures_dir(&self) -> PathBuf {
			self._dir.path().join("skins/textures")
		}
	}

	#[tokio::test]
	async fn profile_request_and_view() {
		let (base, log) = server(vec![profile_reply("abc", "SLIM", Some(CAPE_A))]).await;
		let (textures_base, texture_log) = textures_for("abc", 9).await;
		let f = full_fixture(&base, &textures_base, free_limiter());
		let outcome = f.read(false).await.unwrap();
		assert!(outcome.changed);
		let profile = outcome.value;
		assert!(!profile.stale && profile.error.is_none());
		let worn = profile.skin.unwrap();
		assert_eq!(worn.key, "abc");
		assert_eq!(worn.model, SkinModel::Slim);
		assert_eq!(worn.skin_id, None);
		assert!(!worn.is_default);
		assert!(worn.texture.unwrap().starts_with("data:image/png;base64,"));
		assert_eq!(profile.active_cape_id.as_deref(), Some(CAPE_A));
		assert_eq!(profile.capes.len(), 2);
		assert_eq!(profile.capes[0].name, "Cape a");
		assert!(profile.capes[1].texture.is_some());

		let seen = log.lock().unwrap().clone();
		assert_eq!(seen.len(), 1);
		assert_eq!(seen[0].method, "GET");
		assert_eq!(seen[0].path, "/minecraft/profile");
		assert!(seen[0].head.contains("authorization: bearer tok-1"));
		assert!(seen[0].head.contains("accept: application/json"));
		assert!(seen[0].head.contains("user-agent: leelauncher/"));

		// Токен на сервер текстур не уходит.
		let seen = texture_log.lock().unwrap().clone();
		assert_eq!(seen.len(), 3);
		for request in &seen {
			assert_eq!(request.method, "GET");
			assert!(request.path.starts_with("/texture/"));
			assert!(!request.head.contains("authorization"), "{}", request.head);
			assert!(!request.head.contains("tok-1"));
		}
	}

	#[tokio::test]
	async fn cached_texture_is_not_downloaded_again() {
		let responses = vec![
			profile_reply("abc", "CLASSIC", None),
			profile_reply("abc", "SLIM", None),
		];
		let (base, log) = server(responses).await;
		let (textures_base, texture_log) = textures_for("abc", 9).await;
		let mut f = full_fixture(&base, &textures_base, free_limiter());
		f.skins.policy.every = Duration::ZERO;
		f.skins.policy.refresh_every = Duration::ZERO;
		f.read(false).await.unwrap();
		let count = texture_log.lock().unwrap().len();
		let outcome = f.read(true).await.unwrap();
		assert_eq!(log.lock().unwrap().len(), 2);
		assert_eq!(texture_log.lock().unwrap().len(), count);
		// Изменились только руки, и это изменение текущего скина.
		assert!(outcome.changed);
	}

	#[tokio::test]
	async fn same_answer_reports_no_change() {
		let responses = vec![
			profile_reply("abc", "CLASSIC", None),
			profile_reply("abc", "CLASSIC", Some(CAPE_A)),
		];
		let (base, _log) = server(responses).await;
		let (textures_base, _tlog) = textures_for("abc", 9).await;
		let mut f = full_fixture(&base, &textures_base, free_limiter());
		f.skins.policy.every = Duration::ZERO;
		assert!(f.read(false).await.unwrap().changed);
		// Сменился только плащ: голова и скин те же, перечитывать нечего.
		let second = f.read(false).await.unwrap();
		assert!(!second.changed);
		assert_eq!(second.value.active_cape_id.as_deref(), Some(CAPE_A));
	}

	#[tokio::test]
	async fn foreign_texture_hosts_are_never_requested() {
		let body = format!(
			r#"{{"skins":[{{"state":"ACTIVE","url":"https://textures.minecraft.net.evil.com/texture/abc","variant":"CLASSIC"}}],"capes":[{{"id":"{CAPE_A}","state":"ACTIVE","url":"https://user@textures.minecraft.net/texture/capea","alias":"A"}},{{"id":"{CAPE_B}","state":"INACTIVE","url":"https://textures.minecraft.net:8443/texture/capeb","alias":"B"}}]}}"#
		);
		let (base, _log) = server(vec![reply("200 OK", "", &body)]).await;
		let (textures_base, texture_log) = textures_for("abc", 9).await;
		let f = full_fixture(&base, &textures_base, free_limiter());
		let profile = f.read(false).await.unwrap().value;
		tokio::time::sleep(Duration::from_millis(100)).await;
		assert!(texture_log.lock().unwrap().is_empty());
		assert_eq!(profile.skin.unwrap().texture, None);
		assert!(profile.capes.iter().all(|cape| cape.texture.is_none()));
	}

	#[tokio::test]
	async fn bad_texture_responses_leave_null() {
		let mut large = skin(64, 64, 1);
		large.resize(1024 * 1024 + 1, 0);
		let redirect = reply("302 Found", "Location: /stolen\r\n", "").into_bytes();
		let oversized_body = png_reply(&large);
		let wrong_size = png_reply(&skin(128, 128, 1));
		let cases: [(&str, Vec<u8>); 4] = [
			("large", oversized_body),
			("square", wrong_size),
			("moved", redirect),
			(
				"error",
				reply("500 Internal Server Error", "", "").into_bytes(),
			),
		];
		for (key, response) in cases {
			let (base, log) = server(vec![profile_reply(key, "CLASSIC", None)]).await;
			let (textures_base, texture_log) =
				texture_server(vec![(format!("/texture/{key}"), response)]).await;
			let f = full_fixture(&base, &textures_base, Limiter::default());
			let profile = f.read(false).await.unwrap().value;
			assert_eq!(profile.skin.unwrap().texture, None, "{key}");
			assert!(
				!f.textures_dir().join(format!("{key}.png")).exists(),
				"{key}"
			);
			// Перенаправление не выполнено, и ошибка хоста текстур паузу не включает.
			let paths: Vec<_> = texture_log
				.lock()
				.unwrap()
				.iter()
				.filter(|r| r.path == format!("/texture/{key}") || r.path == "/stolen")
				.map(|r| r.path.clone())
				.collect();
			assert_eq!(paths, [format!("/texture/{key}")], "{key}");
			assert!(f.skins.mojang.pause_left().is_none(), "{key}");
			assert_eq!(log.lock().unwrap().len(), 1);
		}
	}

	#[tokio::test]
	async fn cape_sizes_are_checked() {
		let (base, _log) = server(vec![profile_reply("abc", "CLASSIC", None)]).await;
		let (textures_base, _tlog) = texture_server(vec![
			("/texture/capea".to_owned(), png_reply(&skin(64, 64, 1))),
			("/texture/capeb".to_owned(), png_reply(&skin(46, 22, 1))),
		])
		.await;
		let f = full_fixture(&base, &textures_base, free_limiter());
		let profile = f.read(false).await.unwrap().value;
		assert_eq!(profile.capes[0].texture, None);
		assert!(profile.capes[1].texture.is_some());
	}

	#[tokio::test]
	async fn profile_status_codes_without_saved() {
		let cases = [
			(reply("404 Not Found", "", "{}"), ErrorCode::NoMinecraft),
			(
				reply("429 Too Many Requests", "", ""),
				ErrorCode::RateLimited,
			),
			(
				reply("503 Service Unavailable", "", ""),
				ErrorCode::ServiceUnavailable,
			),
			(reply("403 Forbidden", "", "нет"), ErrorCode::Unknown),
			(reply("200 OK", "", "не профиль"), ErrorCode::Unknown),
			(
				reply("302 Found", "Location: /stolen\r\n", ""),
				ErrorCode::Unknown,
			),
		];
		for (response, code) in cases {
			let (base, log) = server(vec![response, ok()]).await;
			let f = full_fixture(&base, "http://127.0.0.1:9", Limiter::default());
			let err = f.read(false).await.unwrap_err();
			assert_eq!(err.code, code);
			assert_eq!(log.lock().unwrap().len(), 1, "{code:?}");
		}
	}

	#[tokio::test]
	async fn profile_retries_once_after_401() {
		let (base, log) = server(vec![
			reply("401 Unauthorized", "", ""),
			profile_reply("abc", "CLASSIC", None),
		])
		.await;
		let (textures_base, _tlog) = textures_for("abc", 9).await;
		let f = full_fixture(&base, &textures_base, free_limiter());
		let calls = Mutex::new(Vec::new());
		f.skins
			.profile(
				"acc",
				AccountKind::Microsoft,
				false,
				recording_token(&calls, None),
			)
			.await
			.unwrap();
		assert_eq!(*calls.lock().unwrap(), [false, true]);
		let seen = log.lock().unwrap().clone();
		assert!(seen[1].head.contains("bearer tok-2"));
	}

	#[tokio::test]
	async fn needs_relogin_uses_no_network() {
		let (base, log) = server(vec![ok()]).await;
		let f = full_fixture(&base, "http://127.0.0.1:9", Limiter::default());
		let err = f
			.skins
			.profile("acc", AccountKind::Microsoft, false, |_| async {
				Err(AppError::new(ErrorCode::NeedsRelogin))
			})
			.await
			.unwrap_err();
		assert_eq!(err.code, ErrorCode::NeedsRelogin);
		assert!(log.lock().unwrap().is_empty());
	}

	#[tokio::test]
	async fn profile_is_asked_once_a_minute() {
		let (base, log) = server(vec![
			profile_reply("abc", "CLASSIC", None),
			profile_reply("new", "CLASSIC", None),
		])
		.await;
		let (textures_base, _tlog) = textures_for("abc", 9).await;
		let mut f = full_fixture(&base, &textures_base, Limiter::default());
		f.skins.policy.refresh_every = Duration::from_millis(200);
		f.read(false).await.unwrap();
		// Без `refresh` — из сохранённого, сервер не спрашивается.
		let again = f.read(false).await.unwrap();
		assert!(!again.changed && !again.value.stale && again.value.error.is_none());
		assert_eq!(log.lock().unwrap().len(), 1);
		// `refresh` раньше своего срока тоже не спрашивает.
		let early = f.read(true).await.unwrap();
		assert_eq!(early.value.skin.unwrap().key, "abc");
		assert_eq!(log.lock().unwrap().len(), 1);
		tokio::time::sleep(Duration::from_millis(250)).await;
		// Без `refresh` всё ещё минута.
		f.read(false).await.unwrap();
		assert_eq!(log.lock().unwrap().len(), 1);
		let late = f.read(true).await.unwrap();
		assert_eq!(late.value.skin.unwrap().key, "new");
		assert!(late.changed);
		assert_eq!(log.lock().unwrap().len(), 2);
	}

	#[tokio::test]
	async fn concurrent_reads_share_one_request() {
		let (base, log) = server(vec![profile_reply("abc", "CLASSIC", None), ok()]).await;
		let (textures_base, _tlog) = textures_for("abc", 9).await;
		let f = full_fixture(&base, &textures_base, Limiter::default());
		let (first, second) = tokio::join!(f.read(false), f.read(false));
		assert_eq!(first.unwrap().value.skin.unwrap().key, "abc");
		assert_eq!(second.unwrap().value.skin.unwrap().key, "abc");
		assert_eq!(log.lock().unwrap().len(), 1);
	}

	#[tokio::test]
	async fn failure_shows_saved_profile_as_stale() {
		let (base, log) = server(vec![
			profile_reply("abc", "CLASSIC", None),
			reply("503 Service Unavailable", "", ""),
		])
		.await;
		let (textures_base, _tlog) = textures_for("abc", 9).await;
		let mut f = full_fixture(&base, &textures_base, Limiter::default());
		f.skins.policy.every = Duration::ZERO;
		f.skins.policy.refresh_every = Duration::from_secs(60);
		f.read(false).await.unwrap();
		let stale = f.read(false).await.unwrap().value;
		assert!(stale.stale);
		assert_eq!(stale.error.unwrap().code, ErrorCode::ServiceUnavailable);
		assert_eq!(stale.skin.unwrap().key, "abc");
		assert_eq!(log.lock().unwrap().len(), 2);
		// «Повторить» сразу после сбоя: сервер не спрашивается, ответ — сколько ждать.
		let wait = f.read(true).await.unwrap().value;
		assert!(wait.stale);
		let error = wait.error.unwrap();
		assert_eq!(error.code, ErrorCode::SkinTooOften);
		assert!(seconds_in(&error) > 50);
		assert_eq!(log.lock().unwrap().len(), 2);
	}

	#[tokio::test]
	async fn pause_is_shared_by_profile_and_changes() {
		// 429 на чтении останавливает изменения.
		let (base, log) = server(vec![reply("429 Too Many Requests", "", ""), ok()]).await;
		let f = full_fixture(&base, "http://127.0.0.1:9", Limiter::default());
		let err = f.read(false).await.unwrap_err();
		assert_eq!(err.code, ErrorCode::RateLimited);
		assert_eq!(err.details.as_deref(), Some("120"));
		assert_eq!(
			f.apply("acc").await.unwrap_err().code,
			ErrorCode::SkinTooOften
		);
		assert_eq!(log.lock().unwrap().len(), 1);

		// 429 на изменении останавливает чтение: сначала ошибка, с сохранённым — он с `stale`.
		let (base, log) = server(vec![
			profile_reply("abc", "CLASSIC", None),
			reply("429 Too Many Requests", "Retry-After: 300\r\n", ""),
			ok(),
		])
		.await;
		let (textures_base, _tlog) = textures_for("abc", 9).await;
		let mut f = full_fixture(&base, &textures_base, Limiter::default());
		f.skins.policy.every = Duration::ZERO;
		f.skins.policy.refresh_every = Duration::ZERO;
		f.read(false).await.unwrap();
		assert_eq!(
			f.apply("acc").await.unwrap_err().code,
			ErrorCode::RateLimited
		);
		let stale = f.read(true).await.unwrap().value;
		assert!(stale.stale);
		let error = stale.error.unwrap();
		assert_eq!(error.code, ErrorCode::SkinTooOften);
		assert!(seconds_in(&error) >= 298, "{error:?}");
		assert_eq!(log.lock().unwrap().len(), 2);
		// Без сохранённого — ошибка.
		let other = f
			.skins
			.profile("other", AccountKind::Microsoft, false, fixed_token)
			.await
			.unwrap_err();
		assert_eq!(other.code, ErrorCode::SkinTooOften);
		assert_eq!(log.lock().unwrap().len(), 2);
	}

	#[tokio::test]
	async fn apply_changes_skin_then_cape_from_response() {
		let (base, log) = server(vec![
			profile_reply("new", "SLIM", None),
			profile_reply("new", "SLIM", Some(CAPE_B)),
			ok(),
		])
		.await;
		let (textures_base, texture_log) = textures_for("zzz", 9).await;
		let f = full_fixture(&base, &textures_base, free_limiter());
		let outcome = f
			.send(ApplyRequest {
				skin_id: Some(f.skin.id.clone()),
				model: Some(SkinModel::Slim),
				cape: Some(Some(CAPE_B.to_uppercase())),
				allow_lose_previous: true,
			})
			.await
			.unwrap();
		assert!(outcome.changed);
		let applied = outcome.value;
		assert!(applied.cape_error.is_none());
		assert!(applied.saved_previous.is_none());
		let worn = applied.profile.skin.as_ref().unwrap();
		assert_eq!(worn.key, "new");
		assert_eq!(worn.skin_id.as_deref(), Some(f.skin.id.as_str()));
		assert_eq!(applied.profile.active_cape_id.as_deref(), Some(CAPE_B));
		// Текстура — наш файл, скачивать её незачем.
		assert_eq!(worn.texture.as_deref(), Some(f.skin.texture.as_str()));
		assert!(
			texture_log
				.lock()
				.unwrap()
				.iter()
				.all(|request| request.path != "/texture/new")
		);

		let seen = log.lock().unwrap().clone();
		assert_eq!(seen.len(), 2, "без лишнего GET");
		assert_eq!(seen[0].method, "POST");
		assert_eq!(seen[0].path, "/minecraft/profile/skins");
		assert_eq!(seen[1].method, "PUT");
		assert_eq!(seen[1].path, "/minecraft/profile/capes/active");
		assert!(seen[1].head.contains("content-type: application/json"));
		assert!(seen[1].head.contains("authorization: bearer tok-1"));
		let body: serde_json::Value = serde_json::from_slice(&seen[1].body).unwrap();
		assert_eq!(body, serde_json::json!({ "capeId": CAPE_B }));
	}

	#[tokio::test]
	async fn fresh_response_replaces_read_for_a_minute() {
		let (base, log) = server(vec![profile_reply("new", "SLIM", None), ok()]).await;
		let (textures_base, _tlog) = textures_for("zzz", 9).await;
		let f = full_fixture(&base, &textures_base, free_limiter());
		f.apply("acc").await.unwrap();
		let again = f.read(false).await.unwrap().value;
		assert_eq!(again.skin.unwrap().key, "new");
		assert_eq!(log.lock().unwrap().len(), 1);
	}

	#[tokio::test]
	async fn key_from_response_finds_library_skin_later() {
		// Mojang мог пересохранить файл: хеш другой, но ключ записан при загрузке.
		let (base, _log) = server(vec![
			profile_reply("resaved", "SLIM", None),
			profile_reply("resaved", "SLIM", None),
		])
		.await;
		let (textures_base, _tlog) = textures_for("zzz", 9).await;
		let mut f = full_fixture(&base, &textures_base, free_limiter());
		f.skins.policy.every = Duration::ZERO;
		f.apply("acc").await.unwrap();
		// Подменяем кэш другой картинкой: совпасть может только ключ.
		std::fs::write(f.textures_dir().join("resaved.png"), skin(64, 64, 77)).unwrap();
		let profile = f.read(false).await.unwrap().value;
		assert_eq!(
			profile.skin.unwrap().skin_id.as_deref(),
			Some(f.skin.id.as_str())
		);
	}

	#[tokio::test]
	async fn same_texture_id_finds_library_skin() {
		let (base, _log) = server(vec![profile_reply("abc", "CLASSIC", None)]).await;
		let (textures_base, _tlog) = textures_for("abc", 5).await;
		let f = full_fixture(&base, &textures_base, free_limiter());
		let profile = f.read(false).await.unwrap().value;
		assert_eq!(
			profile.skin.unwrap().skin_id.as_deref(),
			Some(f.skin.id.as_str())
		);
	}

	#[tokio::test]
	async fn failed_skin_sends_no_cape() {
		let (base, log) = server(vec![
			reply("400 Bad Request", "", r#"{"errorMessage":"bad"}"#),
			ok(),
		])
		.await;
		let f = full_fixture(&base, "http://127.0.0.1:9", free_limiter());
		let err = f
			.send(ApplyRequest {
				skin_id: Some(f.skin.id.clone()),
				model: None,
				cape: Some(None),
				allow_lose_previous: true,
			})
			.await
			.unwrap_err();
		assert_eq!(err.code, ErrorCode::SkinRejected);
		assert_eq!(log.lock().unwrap().len(), 1);
		assert_eq!(f.worn_count(), 0);
	}

	#[tokio::test]
	async fn rate_limit_on_skin_sends_no_cape() {
		let (base, log) = server(vec![reply("429 Too Many Requests", "", ""), ok()]).await;
		let f = full_fixture(&base, "http://127.0.0.1:9", Limiter::default());
		let err = f
			.send(ApplyRequest {
				skin_id: Some(f.skin.id.clone()),
				model: None,
				cape: Some(None),
				allow_lose_previous: true,
			})
			.await
			.unwrap_err();
		assert_eq!(err.code, ErrorCode::RateLimited);
		tokio::time::sleep(Duration::from_millis(100)).await;
		assert_eq!(log.lock().unwrap().len(), 1);
	}

	#[tokio::test]
	async fn failed_cape_is_partial_success() {
		let (base, log) = server(vec![
			profile_reply("new", "CLASSIC", None),
			reply(
				"400 Bad Request",
				"",
				r#"{"errorMessage":"profile does not own cape"}"#,
			),
		])
		.await;
		let (textures_base, _tlog) = textures_for("zzz", 9).await;
		let f = full_fixture(&base, &textures_base, free_limiter());
		let applied = f
			.send(ApplyRequest {
				skin_id: Some(f.skin.id.clone()),
				model: None,
				cape: Some(Some(CAPE_A.to_owned())),
				allow_lose_previous: true,
			})
			.await
			.unwrap()
			.value;
		let error = applied.cape_error.unwrap();
		assert_eq!(error.code, ErrorCode::SkinRejected);
		assert!(error.details.unwrap().contains("does not own cape"));
		assert_eq!(applied.profile.skin.unwrap().key, "new");
		assert_eq!(applied.profile.active_cape_id, None);
		assert_eq!(log.lock().unwrap().len(), 2);
	}

	#[tokio::test]
	async fn cape_only_requests() {
		let (base, log) = server(vec![
			profile_reply("abc", "CLASSIC", None),
			reply("500 Internal Server Error", "", ""),
		])
		.await;
		let (textures_base, _tlog) = textures_for("abc", 9).await;
		let f = full_fixture(&base, &textures_base, free_limiter());
		let hide = ApplyRequest {
			cape: Some(None),
			..Default::default()
		};
		let applied = f.send(hide.clone()).await.unwrap().value;
		assert_eq!(applied.profile.active_cape_id, None);
		let seen = log.lock().unwrap().clone();
		assert_eq!(seen[0].method, "DELETE");
		assert_eq!(seen[0].path, "/minecraft/profile/capes/active");
		// Скин не менялся, поэтому сбой плаща — обычная ошибка.
		let err = f.send(hide).await.unwrap_err();
		assert_eq!(err.code, ErrorCode::ServiceUnavailable);
	}

	#[tokio::test]
	async fn apply_validates_before_network() {
		let (base, log) = server(vec![ok()]).await;
		let f = full_fixture(&base, "http://127.0.0.1:9", Limiter::default());
		let empty = f.send(ApplyRequest::default()).await.unwrap_err();
		assert_eq!(empty.code, ErrorCode::InvalidValue);
		let bad_cape = f
			.send(ApplyRequest {
				cape: Some(Some("../evil".to_owned())),
				..Default::default()
			})
			.await
			.unwrap_err();
		assert_eq!(bad_cape.code, ErrorCode::InvalidValue);
		// Руки без скина, а текстуры текущего нет.
		let no_texture = f
			.send(ApplyRequest {
				model: Some(SkinModel::Slim),
				..Default::default()
			})
			.await
			.unwrap_err();
		assert_eq!(no_texture.code, ErrorCode::SkinNotFound);
		assert!(log.lock().unwrap().is_empty());
		// И ничего из этого не занимает паузу.
		f.apply("acc").await.unwrap();
	}

	#[tokio::test]
	async fn model_only_uploads_cached_texture() {
		let (base, log) = server(vec![
			profile_reply("abc", "CLASSIC", None),
			profile_reply("abc", "SLIM", None),
		])
		.await;
		let (textures_base, _tlog) = textures_for("abc", 9).await;
		let f = full_fixture(&base, &textures_base, free_limiter());
		f.read(false).await.unwrap();
		let applied = f
			.send(ApplyRequest {
				model: Some(SkinModel::Slim),
				..Default::default()
			})
			.await
			.unwrap()
			.value;
		// Та же картинка: страховка не нужна.
		assert!(applied.saved_previous.is_none());
		assert_eq!(applied.profile.skin.unwrap().model, SkinModel::Slim);
		let seen = log.lock().unwrap().clone();
		let cached = std::fs::read(f.textures_dir().join("abc.png")).unwrap();
		assert!(find(&seen[1].body, &cached).is_some());
		assert!(String::from_utf8_lossy(&seen[1].body).contains("\r\n\r\nslim\r\n"));
	}

	#[tokio::test]
	async fn wearing_over_unsaved_skin_saves_it() {
		let (base, _log) = server(vec![
			profile_reply("unsaved", "CLASSIC", None),
			profile_reply("new", "SLIM", None),
		])
		.await;
		let (textures_base, _tlog) = textures_for("unsaved", 9).await;
		let f = full_fixture(&base, &textures_base, free_limiter());
		let before = f.read(false).await.unwrap().value.skin.unwrap();
		assert_eq!(before.skin_id, None);
		let applied = f
			.send(ApplyRequest {
				skin_id: Some(f.skin.id.clone()),
				..Default::default()
			})
			.await
			.unwrap()
			.value;
		let saved = applied.saved_previous.unwrap();
		assert_eq!(saved.name, "Steve");
		assert_eq!(Some(saved.texture.clone()), before.texture);
		assert_eq!(f.skins.library.skins().unwrap().len(), 2);
		// Кэш старой текстуры убран: на неё больше никто не ссылается, копия — в библиотеке.
		assert!(!f.textures_dir().join("unsaved.png").exists());
	}

	#[tokio::test]
	async fn library_or_default_skin_is_not_saved_again() {
		// Обычный скин.
		let default_body = r#"{"skins":[{"state":"ACTIVE","url":"https://textures.minecraft.net/texture/steve","variant":"CLASSIC","alias":"STEVE"}],"capes":[]}"#;
		let (base, _log) = server(vec![
			reply("200 OK", "", default_body),
			profile_reply("new", "SLIM", None),
		])
		.await;
		let (textures_base, _tlog) = textures_for("steve", 9).await;
		let f = full_fixture(&base, &textures_base, free_limiter());
		let before = f.read(false).await.unwrap().value.skin.unwrap();
		assert!(before.is_default);
		let err = f
			.skins
			.save_current("acc", AccountKind::Microsoft, "Steve")
			.await
			.unwrap_err();
		assert_eq!(err.code, ErrorCode::InvalidValue);
		let applied = f
			.send(ApplyRequest {
				skin_id: Some(f.skin.id.clone()),
				..Default::default()
			})
			.await
			.unwrap()
			.value;
		assert!(applied.saved_previous.is_none());
		assert_eq!(f.skins.library.skins().unwrap().len(), 1);
	}

	#[tokio::test]
	async fn unparsed_answer_is_success_and_marks_for_reread() {
		let (base, log) = server(vec![
			reply("200 OK", "", "не json"),
			profile_reply("real", "SLIM", None),
		])
		.await;
		let (textures_base, _tlog) = textures_for("real", 9).await;
		let f = full_fixture(&base, &textures_base, free_limiter());
		let applied = f.apply("acc").await.unwrap();
		assert!(applied.profile.stale);
		assert!(applied.profile.error.is_none());
		let worn = applied.profile.skin.unwrap();
		assert_eq!(worn.skin_id.as_deref(), Some(f.skin.id.as_str()));
		assert_eq!(worn.texture.as_deref(), Some(f.skin.texture.as_str()));
		// Следующее чтение идёт на сервер, хотя минута не прошла.
		let read = f.read(false).await.unwrap().value;
		assert_eq!(read.skin.unwrap().key, "real");
		assert!(!read.stale);
		assert_eq!(log.lock().unwrap().len(), 2);
	}

	#[tokio::test]
	async fn reset_returns_profile_from_response() {
		let default_body = r#"{"skins":[{"state":"ACTIVE","url":"https://textures.minecraft.net/texture/steve","variant":"CLASSIC","alias":"STEVE"}],"capes":[]}"#;
		let (base, log) = server(vec![reply("200 OK", "", default_body)]).await;
		let (textures_base, _tlog) = textures_for("steve", 9).await;
		let f = full_fixture(&base, &textures_base, free_limiter());
		let outcome = f
			.skins
			.reset("acc", AccountKind::Microsoft, fixed_token)
			.await
			.unwrap();
		assert!(outcome.changed);
		let worn = outcome.value.skin.unwrap();
		assert!(worn.is_default);
		assert!(worn.texture.is_some());
		assert_eq!(log.lock().unwrap().len(), 1);
	}

	#[tokio::test]
	async fn save_current_and_worn() {
		let (base, _log) = server(vec![profile_reply("cur", "SLIM", None)]).await;
		let (textures_base, _tlog) = textures_for("cur", 9).await;
		let f = full_fixture(&base, &textures_base, free_limiter());
		let missing = f
			.skins
			.save_current("acc", AccountKind::Microsoft, "Steve")
			.await
			.unwrap_err();
		assert_eq!(missing.code, ErrorCode::SkinNotFound);

		f.read(false).await.unwrap();
		let worn = f.skins.worn().await.unwrap();
		assert_eq!(worn.keys().collect::<Vec<_>>(), ["acc"]);
		assert_eq!(worn["acc"].skin_id, None);

		let entry = f
			.skins
			.save_current("acc", AccountKind::Microsoft, "Steve")
			.await
			.unwrap();
		assert_eq!(entry.name, "Steve");
		let again = f
			.skins
			.save_current("acc", AccountKind::Microsoft, "Другой")
			.await
			.unwrap();
		assert_eq!(again.id, entry.id);
		assert_eq!(again.name, "Steve");
		assert_eq!(f.skins.library.skins().unwrap().len(), 2);
		let worn = f.skins.worn().await.unwrap();
		assert_eq!(worn["acc"].skin_id.as_deref(), Some(entry.id.as_str()));
		// Без сети: другого обращения нет, а при удалении скина профиль остаётся.
		f.skins.library.remove(&entry.id, |_| Ok(())).unwrap();
		let worn = f.skins.worn().await.unwrap();
		assert_eq!(worn["acc"].skin_id, None);
		assert!(worn["acc"].texture.is_some());
	}

	#[tokio::test]
	async fn worn_lists_only_skins_with_texture() {
		let (base, _log) = server(vec![profile_reply("gone", "SLIM", None)]).await;
		let (textures_base, _tlog) = texture_server(Vec::new()).await;
		let f = full_fixture(&base, &textures_base, free_limiter());
		assert_eq!(
			f.read(false).await.unwrap().value.skin.unwrap().texture,
			None
		);
		assert!(f.skins.worn().await.unwrap().is_empty());
	}

	#[tokio::test]
	async fn forget_removes_profile_and_cache_only() {
		let (base, _log) = server(vec![profile_reply("abc", "SLIM", None)]).await;
		let (textures_base, _tlog) = textures_for("abc", 9).await;
		let f = full_fixture(&base, &textures_base, free_limiter());
		f.read(false).await.unwrap();
		assert!(f.skins.forget("acc").await.unwrap());
		assert!(!f.skins.forget("acc").await.unwrap());
		assert!(f.skins.worn().await.unwrap().is_empty());
		assert!(!f.textures_dir().join("abc.png").exists());
		assert_eq!(f.skins.library.skins().unwrap().len(), 1);
	}

	#[tokio::test]
	async fn offline_is_refused_without_anything() {
		let (base, log) = server(vec![ok()]).await;
		let f = fixture(&base);
		let token =
			|_| async { panic!("токен офлайн-аккаунту не нужен") };
		let offline = AccountKind::Offline;
		let codes = [
			f.skins
				.profile("off", offline, true, token)
				.await
				.unwrap_err()
				.code,
			f.skins
				.apply(
					"off",
					offline,
					"Steve",
					ApplyRequest {
						skin_id: Some(f.skin.id.clone()),
						..Default::default()
					},
					token,
				)
				.await
				.unwrap_err()
				.code,
			f.skins.reset("off", offline, token).await.unwrap_err().code,
			f.skins
				.save_current("off", offline, "Steve")
				.await
				.unwrap_err()
				.code,
		];
		assert_eq!(codes, [ErrorCode::InvalidValue; 4]);
		assert!(log.lock().unwrap().is_empty());
		assert_eq!(f.worn_count(), 0);
		assert!(!f._dir.path().join("skins/profiles").exists());
	}

	#[tokio::test]
	async fn failed_record_after_upload_says_so() {
		let (base, _log) = server(vec![ok()]).await;
		let f = fixture(&base);
		// Файл вместо папки: профиль записать некуда.
		std::fs::write(f._dir.path().join("skins/profiles"), b"x").unwrap();
		let err = f.apply("acc").await.unwrap_err();
		assert_eq!(err.code, ErrorCode::Storage);
		assert!(err.details.unwrap().contains(SERVER_CHANGED));
	}

	#[test]
	fn types_for_frontend() {
		let worn = WornSkin {
			key: "k".into(),
			skin_id: None,
			model: SkinModel::Slim,
			is_default: true,
			texture: None,
			fetched_at: "t".into(),
		};
		let profile = SkinProfile {
			account_id: "a".into(),
			skin: Some(worn),
			capes: vec![Cape {
				id: "c".into(),
				name: "n".into(),
				texture: None,
			}],
			active_cape_id: None,
			stale: true,
			error: Some(AppError::new(ErrorCode::Network)),
		};
		let applied = serde_json::to_value(SkinApplied {
			profile,
			cape_error: None,
			saved_previous: None,
		})
		.unwrap();
		assert!(applied["capeError"].is_null() && applied["savedPrevious"].is_null());
		let profile = &applied["profile"];
		assert_eq!(profile["accountId"], "a");
		assert!(profile["activeCapeId"].is_null());
		assert_eq!(profile["error"]["code"], "network");
		assert_eq!(profile["skin"]["isDefault"], true);
		assert_eq!(profile["skin"]["fetchedAt"], "t");
		assert!(profile["skin"]["skinId"].is_null());
		assert!(profile["skin"]["texture"].is_null());
		let rejected = serde_json::to_value(RejectedFile {
			path: "p".into(),
			reason: RejectReason::BadSize,
			width: Some(100),
			height: Some(100),
		})
		.unwrap();
		assert_eq!(rejected["reason"], "bad_size");
		assert_eq!(rejected["width"], 100);
		let none = serde_json::to_value(RejectedFile {
			path: "p".into(),
			reason: RejectReason::NotPng,
			width: None,
			height: None,
		})
		.unwrap();
		assert!(none.get("width").is_none());
		let patch: SkinPatch = serde_json::from_str(r#"{"model":"classic"}"#).unwrap();
		assert_eq!(patch.model, Some(SkinModel::Classic));
		assert!(patch.name.is_none());
		assert!(serde_json::from_str::<SkinPatch>(r#"{"model":"wide"}"#).is_err());
	}

	/// Как `server`, но первый ответ задерживается: запросы успевают пересечься.
	async fn slow_first_server(delay_ms: u64, responses: Vec<String>) -> (String, Log) {
		let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
		let base = format!("http://{}", listener.local_addr().unwrap());
		let log: Log = Arc::default();
		let seen = log.clone();
		tokio::spawn(async move {
			let mut responses = responses.into_iter();
			let mut delay = delay_ms;
			while let Ok((mut socket, _)) = listener.accept().await {
				let Some(request) = read_request(&mut socket).await else {
					continue;
				};
				seen.lock().unwrap().push(request);
				let response = responses
					.next()
					.unwrap_or_else(|| reply("500 Internal Server Error", "", ""));
				tokio::time::sleep(Duration::from_millis(delay)).await;
				delay = 0;
				let _ = socket.write_all(response.as_bytes()).await;
				let _ = socket.shutdown().await;
			}
		});
		(base, log)
	}

	fn wear_request(f: &Fixture) -> ApplyRequest {
		ApplyRequest {
			skin_id: Some(f.skin.id.clone()),
			..Default::default()
		}
	}

	/// «Надеть» пока идёт первое чтение: страховка считается уже по его результату.
	#[tokio::test]
	async fn apply_during_first_read_keeps_unsaved_skin() {
		let (base, log) = slow_first_server(
			300,
			vec![
				profile_reply("unsaved", "CLASSIC", None),
				profile_reply("new", "SLIM", None),
			],
		)
		.await;
		let (textures_base, _tlog) = textures_for("unsaved", 9).await;
		let f = full_fixture(&base, &textures_base, free_limiter());
		let wear = async {
			tokio::time::sleep(Duration::from_millis(80)).await;
			f.send(wear_request(&f)).await
		};
		let (read, wear) = tokio::join!(f.read(false), wear);
		assert_eq!(read.unwrap().value.skin.unwrap().skin_id, None);
		let saved = wear.unwrap().value.saved_previous.unwrap();
		assert_eq!(saved.name, "Steve");
		assert_eq!(f.skins.library.skins().unwrap().len(), 2);
		let methods: Vec<_> = log
			.lock()
			.unwrap()
			.iter()
			.map(|r| r.method.clone())
			.collect();
		assert_eq!(methods, ["GET", "POST"]);
	}

	#[tokio::test]
	async fn missing_texture_needs_confirmation() {
		let (base, log) = server(vec![
			profile_reply("unsaved", "CLASSIC", None),
			profile_reply("new", "SLIM", None),
		])
		.await;
		let (textures_base, _tlog) = texture_server(Vec::new()).await;
		let f = full_fixture(&base, &textures_base, free_limiter());
		let read = f.read(false).await.unwrap().value;
		assert_eq!(read.skin.unwrap().texture, None);

		let err = f.send(wear_request(&f)).await.err().unwrap();
		assert_eq!(err.code, ErrorCode::SkinPreviousUnknown);
		assert_eq!(err.details.as_deref(), Some("no_texture"));
		assert_eq!(log.lock().unwrap().len(), 1);

		// Подтверждённая загрузка проходит.
		let confirmed = ApplyRequest {
			allow_lose_previous: true,
			..wear_request(&f)
		};
		let applied = f.send(confirmed).await.unwrap().value;
		assert!(applied.saved_previous.is_none());
		assert_eq!(log.lock().unwrap().len(), 2);
	}

	#[tokio::test]
	async fn unknown_profile_needs_confirmation() {
		let (base, log) = server(vec![
			reply("503 Service Unavailable", "", ""),
			profile_reply("abc", "CLASSIC", Some(CAPE_A)),
			profile_reply("new", "SLIM", None),
		])
		.await;
		let (textures_base, _tlog) = textures_for("zzz", 9).await;
		let f = full_fixture(&base, &textures_base, free_limiter());
		// Чтение не удалось, профиля нет.
		assert_eq!(
			f.read(false).await.unwrap_err().code,
			ErrorCode::ServiceUnavailable
		);

		let err = f.send(wear_request(&f)).await.err().unwrap();
		assert_eq!(err.code, ErrorCode::SkinPreviousUnknown);
		assert_eq!(err.details.as_deref(), Some("no_profile"));
		assert_eq!(log.lock().unwrap().len(), 1);

		// Только плащ страховки не касается.
		let cape = ApplyRequest {
			cape: Some(Some(CAPE_A.to_owned())),
			..Default::default()
		};
		f.send(cape).await.unwrap();
		assert_eq!(log.lock().unwrap().len(), 2);
	}

	#[tokio::test]
	async fn known_empty_profile_needs_no_confirmation() {
		// Активного скина нет: терять нечего.
		let empty = reply("200 OK", "", r#"{"skins":[],"capes":[]}"#);
		let (base, _log) = server(vec![empty, profile_reply("new", "SLIM", None)]).await;
		let f = full_fixture(&base, "http://127.0.0.1:9", free_limiter());
		f.read(false).await.unwrap();
		assert!(f.send(wear_request(&f)).await.is_ok());
	}

	#[tokio::test]
	async fn model_only_refused_for_default_skin() {
		let default_body = r#"{"skins":[{"state":"ACTIVE","url":"https://textures.minecraft.net/texture/steve","variant":"CLASSIC","alias":"STEVE"}],"capes":[]}"#;
		let (base, log) = server(vec![reply("200 OK", "", default_body), ok()]).await;
		let (textures_base, _tlog) = textures_for("steve", 9).await;
		let f = full_fixture(&base, &textures_base, free_limiter());
		f.read(false).await.unwrap();
		let err = f
			.send(ApplyRequest {
				model: Some(SkinModel::Slim),
				allow_lose_previous: true,
				..Default::default()
			})
			.await
			.err()
			.unwrap();
		assert_eq!(err.code, ErrorCode::InvalidValue);
		assert_eq!(log.lock().unwrap().len(), 1);
	}

	/// После повторного входа токен новый: прошлый `needs_relogin` не должен отвечать вместо сервера.
	#[tokio::test]
	async fn needs_relogin_is_not_remembered() {
		let unauthorized = || reply("401 Unauthorized", "", "");
		let (base, log) = server(vec![
			unauthorized(),
			unauthorized(),
			profile_reply("abc", "CLASSIC", None),
		])
		.await;
		let (textures_base, _tlog) = textures_for("abc", 9).await;
		let f = full_fixture(&base, &textures_base, Limiter::default());
		assert_eq!(
			f.read(false).await.unwrap_err().code,
			ErrorCode::NeedsRelogin
		);
		assert_eq!(log.lock().unwrap().len(), 2);
		let read = f.read(false).await.unwrap().value;
		assert_eq!(read.skin.unwrap().key, "abc");
		assert_eq!(log.lock().unwrap().len(), 3);
	}

	/// Ответ на изменение не разобрался: чтение идёт на сервер один раз, а при сбоях окно действует.
	#[tokio::test]
	async fn refetch_keeps_window_while_failing() {
		let failure = || reply("500 Internal Server Error", "", "");
		let (base, log) = server(vec![ok(), failure(), failure(), failure()]).await;
		let f = fixture_with(&base, free_limiter());
		f.apply("acc").await.unwrap();
		for _ in 0..3 {
			let read = f.read(false).await.unwrap().value;
			assert!(read.stale);
			assert_eq!(read.error.unwrap().code, ErrorCode::ServiceUnavailable);
		}
		assert_eq!(log.lock().unwrap().len(), 2);
	}

	/// Файл версии 1 с настоящим надеванием: скины целы, запись `worn` пропала, ключ записан.
	#[tokio::test]
	async fn version_1_library_through_apply() {
		let (base, _log) = server(vec![profile_reply("k1", "CLASSIC", None)]).await;
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path().join("skins");
		std::fs::create_dir_all(&root).unwrap();
		let one = png::parse(&skin(64, 64, 5)).unwrap();
		let two = png::parse(&skin(64, 64, 6)).unwrap();
		std::fs::write(root.join(format!("{}.png", one.id)), &one.png).unwrap();
		std::fs::write(root.join(format!("{}.png", two.id)), &two.png).unwrap();
		let index = root.join("library.json");
		let file = serde_json::json!({
			"version": 1,
			"skins": [
				{ "id": one.id, "name": "One", "model": "slim", "legacy": false, "addedAt": "t" },
				{ "id": two.id, "name": "Two", "model": "classic", "legacy": false, "addedAt": "t" },
			],
			"worn": { "acc": { "skinId": one.id, "model": "slim", "wornAt": "t" } },
		});
		std::fs::write(&index, file.to_string()).unwrap();
		let http = mojang::build_client().unwrap();
		let skins = with_mojang(root.clone(), Mojang::new(http, &base, free_limiter()));
		assert!(skins.worn().await.unwrap().is_empty());
		let request = ApplyRequest {
			skin_id: Some(two.id.clone()),
			allow_lose_previous: true,
			..Default::default()
		};
		let applied = skins
			.apply("acc", AccountKind::Microsoft, "Nick", request, fixed_token)
			.await
			.unwrap()
			.value;
		assert_eq!(
			applied.profile.skin.unwrap().skin_id.as_deref(),
			Some(two.id.as_str())
		);
		let raw: serde_json::Value =
			serde_json::from_slice(&std::fs::read(&index).unwrap()).unwrap();
		assert_eq!(raw["version"], 2);
		assert!(raw.get("worn").is_none());
		assert_eq!(raw["skins"].as_array().unwrap().len(), 2);
		assert_eq!(raw["keys"]["k1"], serde_json::json!(two.id));
	}

	#[tokio::test]
	async fn reset_checks_library_before_request() {
		let (base, log) = server(vec![ok()]).await;
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path().join("skins");
		// Папка вместо файла: библиотека не читается.
		std::fs::create_dir_all(root.join("library.json")).unwrap();
		let http = mojang::build_client().unwrap();
		let skins = with_mojang(root, Mojang::new(http, &base, Limiter::default()));
		let err = skins
			.reset("acc", AccountKind::Microsoft, fixed_token)
			.await
			.unwrap_err();
		assert_eq!(err.code, ErrorCode::Storage);
		assert!(log.lock().unwrap().is_empty());
	}

	#[tokio::test]
	async fn no_connection_is_network_without_pause() {
		let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
		let base = format!("http://{}", listener.local_addr().unwrap());
		drop(listener);
		let f = fixture_with(&base, Limiter::default());
		for _ in 0..2 {
			let err = f.apply("acc").await.unwrap_err();
			assert_eq!(err.code, ErrorCode::Network);
		}
		assert_eq!(f.worn_count(), 0);
	}

	#[tokio::test]
	async fn unknown_skin_sends_nothing() {
		let (base, log) = server(vec![ok()]).await;
		let f = fixture(&base);
		let err = f
			.skins
			.wear(
				"acc",
				AccountKind::Microsoft,
				&"0".repeat(40),
				SkinModel::Slim,
				fixed_token,
			)
			.await
			.unwrap_err();
		assert_eq!(err.code, ErrorCode::SkinNotFound);
		assert!(log.lock().unwrap().is_empty());
	}
}
