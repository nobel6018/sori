mod actions;
pub mod ask;
pub mod engine;
pub mod meeting;
mod voices;
pub mod download;
pub mod models;
mod recorder;
pub mod store;
mod location;
mod calendar;
pub mod search;
pub mod summarize;
mod tagger;
mod sync;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use whisper_rs::WhisperContext;

use engine::{ProcessOptions, Stage, Transcript};
use store::{Recording, Settings, Status, Store};

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

struct AppState {
    store: Store,
    home: PathBuf,
    /// 1.6GB 모델을 녹음마다 다시 읽지 않도록 경로와 함께 들고 있는다
    whisper: Mutex<Option<(PathBuf, Arc<WhisperContext>)>>,
    cancels: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// GPU와 CPU를 다 쓰는 작업이라 한 번에 하나만 처리한다
    pipeline: tokio::sync::Mutex<()>,
    /// 녹음은 한 번에 하나만. 녹음 id와 녹음 스레드 핸들
    recorder: Mutex<Option<(String, recorder::ActiveRecording)>>,
    /// 요약 중인 녹음 id → 취소 신호
    summaries: Mutex<HashMap<String, tokio::sync::oneshot::Sender<()>>>,
    /// 모델 내려받기 중이면 취소 신호
    model_install: Mutex<Option<Arc<AtomicBool>>>,
    search: search::Index,
    /// 시작할 때 전역 단축키를 등록하지 못했으면 그 이유. 화면이 준비되면 알린다
    shortcut_error: Mutex<Option<String>>,
    voices: voices::VoiceBook,
    /// 지금 녹음을 시작한 시각. 메모와 북마크의 시각을 잰다
    recording_started: Mutex<Option<Instant>>,
    meeting: Mutex<meeting::Detector>,
    /// 지금 띄운 회의 질문 창의 내용
    meeting_prompt: Mutex<Option<meeting::Prompt>>,
    syncer: sync::Syncer,
}

impl AppState {
    fn whisper(&self, path: &Path) -> eyre::Result<Arc<WhisperContext>> {
        let mut slot = self.whisper.lock().unwrap();
        if let Some((loaded, ctx)) = slot.as_ref() {
            if loaded == path {
                return Ok(ctx.clone());
            }
        }
        let ctx = Arc::new(engine::load_whisper(path)?);
        *slot = Some((path.to_path_buf(), ctx.clone()));
        Ok(ctx)
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ProgressEvent {
    id: String,
    stage: Stage,
    progress: f32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecordingDetail {
    recording: Recording,
    audio_path: PathBuf,
    transcript: Option<Transcript>,
    summary: Option<summarize::Summary>,
    /// 오래된 것부터
    summaries: Vec<summarize::Summary>,
    /// 오디오가 이 기기에 있는지. 다른 기기에서 온 녹음은 받기 전까지 없다
    audio_available: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProcessRequest {
    pub(crate) id: String,
    pub(crate) num_speakers: Option<i32>,
    pub(crate) languages: Vec<String>,
    /// None이면 설정의 기본 방식(자동 처리)
    #[serde(default)]
    pub(crate) diarize: Option<bool>,
}

#[tauri::command]
fn get_settings(state: State<AppState>) -> Settings {
    state.store.get_settings(&state.home)
}

#[tauri::command]
fn save_settings(app: AppHandle, state: State<AppState>, settings: Settings) -> CmdResult<()> {
    let previous = state.store.get_settings(&state.home);
    if previous.shortcuts != settings.shortcuts {
        // 등록에 실패하면 이전 단축키로 되돌리고 저장하지 않는다
        actions::apply_shortcuts(&app, &settings.shortcuts, Some(&previous.shortcuts))?;
        state.shortcut_error.lock().unwrap().take();
    }
    state.store.save_settings(&settings).map_err(err)?;
    // 동기화 방식을 바꿨으면 바로 새 방식으로 맞춘다
    sync::request(&app);
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReadyInfo {
    pending: Vec<actions::Action>,
    shortcut_error: Option<String>,
}

/// 화면이 app-action 이벤트를 구독한 뒤 부른다. 앱을 띄운 딥링크처럼 그 전에 들어온 요청을 넘겨준다
#[tauri::command]
fn frontend_ready(app: AppHandle, state: State<AppState>) -> ReadyInfo {
    ReadyInfo {
        pending: actions::take_pending(&app),
        shortcut_error: state.shortcut_error.lock().unwrap().clone(),
    }
}

/// 설정에서 단축키를 새로 받는 동안 전역 단축키를 잠시 끈다. 끄지 않으면 OS가 키를 먼저 가로채
/// 받기 칸에 오지 않고, 녹음 토글 조합이면 녹음이 시작된다
#[tauri::command]
fn suspend_shortcuts(app: AppHandle) {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    let _ = app.global_shortcut().unregister_all();
}

/// 저장된 설정대로 전역 단축키를 다시 등록한다
#[tauri::command]
fn resume_shortcuts(app: AppHandle, state: State<AppState>) -> CmdResult<()> {
    let settings = state.store.get_settings(&state.home);
    actions::apply_shortcuts(&app, &settings.shortcuts, None)
}

#[tauri::command]
async fn search_recordings(app: AppHandle, query: String) -> CmdResult<Vec<search::RecordingHits>> {
    // 녹음이 많으면 파일을 읽는 데 시간이 걸리므로 화면 스레드를 막지 않는다
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        search::search(&state.store, &state.search, &query)
    })
    .await
    .map_err(err)?
    .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
fn model_catalog(state: State<AppState>) -> models::Catalog {
    models::catalog(&state.store.models_dir, &state.home)
}

/// 고른 Whisper 모델과 없는 화자 분리 모델을 받는다. 진행률은 model-download 이벤트로 보낸다
#[tauri::command]
async fn install_models(app: AppHandle, whisper: Vec<String>) -> CmdResult<models::Catalog> {
    let state = app.state::<AppState>();
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut slot = state.model_install.lock().unwrap();
        if slot.is_some() {
            return Err("이미 모델을 받는 중입니다".into());
        }
        *slot = Some(cancel.clone());
    }
    let worker = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let state = worker.state::<AppState>();
        let plan = models::install_plan(&state.store.models_dir, &state.home, &whisper).map_err(|e| eyre::eyre!(e))?;
        download::download_all(&plan, &cancel, |p| {
            let _ = worker.emit("model-download", p);
        })
    })
    .await
    .map_err(err);
    state.model_install.lock().unwrap().take();
    result?.map_err(|e| format!("{e:#}"))?;

    // 설정이 없는 파일을 가리키고 있을 때만 새로 받은 파일로 바꾼다. 사용자가 고른 경로는 두고
    let catalog = models::catalog(&state.store.models_dir, &state.home);
    let mut settings = state.store.get_settings(&state.home);
    let m = &mut settings.models;
    if !m.whisper.is_file() {
        if let Some(p) = catalog.whisper.iter().find_map(|w| w.path.clone()) {
            m.whisper = p;
        }
    }
    if !m.segmentation.is_file() {
        if let Some(p) = catalog.segmentation.clone() {
            m.segmentation = p;
        }
    }
    if !m.embedding.is_file() {
        if let Some(p) = catalog.embedding.clone() {
            m.embedding = p;
        }
    }
    state.store.save_settings(&settings).map_err(err)?;
    Ok(catalog)
}

#[tauri::command]
fn cancel_model_install(state: State<AppState>) {
    if let Some(flag) = state.model_install.lock().unwrap().as_ref() {
        flag.store(true, Ordering::Relaxed);
    }
}

#[tauri::command]
fn list_recordings(state: State<AppState>) -> CmdResult<Vec<Recording>> {
    state.store.list().map_err(err)
}

#[tauri::command]
fn import_recordings(state: State<AppState>, paths: Vec<PathBuf>) -> CmdResult<Vec<Recording>> {
    paths.iter().map(|p| state.store.import(p).map_err(err)).collect()
}

#[tauri::command]
fn get_recording(state: State<AppState>, id: String) -> CmdResult<RecordingDetail> {
    let recording = state.store.get(&id).map_err(err)?;
    let audio_path = state.store.audio_path(&recording).map_err(err)?;
    Ok(RecordingDetail {
        audio_available: audio_path.is_file(),
        audio_path,
        transcript: state.store.get_transcript(&id).map_err(err)?,
        summary: state.store.get_summary(&id).map_err(err)?,
        summaries: state.store.get_summaries(&id).map_err(err)?,
        recording,
    })
}

/// 사람이 태그를 고친다. 고친 시각을 남겨 AI 자동 분류가 덮어쓰지 않게 한다(docs/organize.md)
#[tauri::command]
fn set_tags(app: AppHandle, state: State<AppState>, id: String, tags: Vec<String>) -> CmdResult<Recording> {
    let recording = state
        .store
        .update(&id, |r| {
            r.tags = sori_sync::tags::normalize_tags(&tags);
            r.tags_edited_at = Some(chrono::Utc::now());
        })
        .map_err(err)?;
    sync::request(&app);
    Ok(recording)
}

#[tauri::command]
fn update_recording(app: AppHandle, state: State<AppState>, recording: Recording) -> CmdResult<()> {
    // 상태와 오류는 처리 파이프라인이, 화자 이름과 메모는 각자의 명령이 바꾼다.
    // 화면이 들고 있던 예전 값으로 덮어쓰지 않도록 제목, 화자 수, 언어만 받는다
    state
        .store
        .update(&recording.id, |r| {
            // 사람이 제목을 고쳤으면 AI 정리가 다시 바꾸지 않게 남긴다
            if r.title != recording.title {
                r.title_edited_at = Some(chrono::Utc::now());
            }
            r.title = recording.title;
            r.num_speakers = recording.num_speakers;
            r.languages = recording.languages;
        })
        .map_err(err)?;
    sync::request(&app);
    Ok(())
}

#[tauri::command]
fn delete_recording(app: AppHandle, state: State<AppState>, id: String) -> CmdResult<()> {
    if state.recorder.lock().unwrap().as_ref().is_some_and(|(r, _)| r == &id) {
        return Err("녹음 중인 항목은 삭제할 수 없습니다. 먼저 녹음을 멈춰 주세요".into());
    }
    if state.cancels.lock().unwrap().contains_key(&id) {
        return Err("처리 중인 녹음은 삭제할 수 없습니다. 먼저 취소해 주세요".into());
    }
    state.store.delete(&id).map_err(err)?;
    sync::request(&app);
    Ok(())
}

#[tauri::command]
fn cancel_processing(state: State<AppState>, id: String) {
    if let Some(flag) = state.cancels.lock().unwrap().get(&id) {
        flag.store(true, Ordering::Relaxed);
    }
}

#[tauri::command]
fn list_input_devices() -> CmdResult<Vec<recorder::InputDevice>> {
    recorder::list_input_devices().map_err(err)
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct LevelEvent {
    id: String,
    /// 트랙 순서(마이크, 시스템 소리)대로 0..1
    levels: Vec<f32>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct InterruptedEvent {
    id: String,
    reason: String,
}

// 마이크를 열고 녹음 스레드를 합치는 동안 메인 스레드(화면)가 멈추지 않도록 async로 둔다
#[tauri::command]
async fn start_recording(app: AppHandle, state: State<'_, AppState>) -> CmdResult<Recording> {
    let mut slot = state.recorder.lock().unwrap();
    if slot.is_some() {
        return Err("이미 녹음 중입니다".into());
    }
    let settings = state.store.get_settings(&state.home);
    let mut sources = vec![("mic.wav", recorder::Source::Mic(settings.input_device_id.clone()))];
    if settings.capture_system_audio {
        sources.push(("system.wav", recorder::Source::System));
    }
    let title = format!("녹음 {}", chrono::Local::now().format("%Y-%m-%d %H:%M"));
    let source_name = if settings.capture_system_audio { "마이크 + 시스템 소리" } else { "마이크 녹음" };
    let recording = state
        .store
        .create_for_recording(
            title,
            source_name.to_string(),
            sources.iter().map(|(f, _)| f.to_string()).collect(),
        )
        .map_err(err)?;
    let tracks = state
        .store
        .track_paths(&recording)
        .map_err(err)?
        .into_iter()
        .zip(sources)
        .map(|(path, (_, source))| recorder::Track { source, path })
        .collect();

    let id = recording.id.clone();
    let level_app = app.clone();
    let location_app = app.clone();
    let interrupted_id = recording.id.clone();
    let started = recorder::start(
        tracks,
        move |levels| {
            let _ = level_app.emit("recording-level", LevelEvent { id: id.clone(), levels });
        },
        move |reason| {
            let _ = app.emit("recording-interrupted", InterruptedEvent { id: interrupted_id, reason });
        },
    );
    match started {
        Ok(active) => {
            *slot = Some((recording.id.clone(), active));
            *state.recording_started.lock().unwrap() = Some(Instant::now());
            if settings.record_location {
                record_location(location_app, recording.id.clone());
            }
            Ok(recording)
        }
        Err(e) => {
            // 장치를 못 열었으면 빈 항목을 남기지 않는다
            let _ = state.store.delete(&recording.id);
            Err(format!("{e:#}"))
        }
    }
}

/// 녹음을 시작한 곳을 따로 받아 남긴다. 최대 약 18초 걸리므로 녹음을 막지 않게 작업 스레드에서 한다
fn record_location(app: AppHandle, id: String) {
    std::thread::spawn(move || {
        let Some(location) = location::capture() else { return };
        let state = app.state::<AppState>();
        if state.store.update(&id, |r| r.location = Some(location)).is_ok() {
            let _ = app.emit("recording-changed", RecordingChanged { id });
            sync::request(&app);
        }
    });
}

/// 트랙을 섞어 재생과 전사에 쓸 audio.wav를 만든다. 녹음 길이(초)를 돌려준다
fn finalize_tracks(store: &Store, recording: &Recording) -> eyre::Result<f32> {
    let tracks = store.track_paths(recording)?;
    let existing: Vec<PathBuf> = tracks.into_iter().filter(|p| p.exists()).collect();
    engine::audio::mix_wav_tracks(&existing, &store.audio_path(recording)?)
}

#[tauri::command]
async fn stop_recording(app: AppHandle) -> CmdResult<Recording> {
    let state = app.state::<AppState>();
    let (id, active) = state.recorder.lock().unwrap().take().ok_or("녹음 중이 아닙니다")?;
    state.recording_started.lock().unwrap().take();
    let result = active.stop();

    let worker_app = app.clone();
    let worker_id = id.clone();
    // 1시간 녹음이면 트랙을 섞는 데 몇 초 걸리므로 블로킹 스레드에서 한다
    let mixed = tauri::async_runtime::spawn_blocking(move || {
        let state = worker_app.state::<AppState>();
        let recording = state.store.get(&worker_id)?;
        finalize_tracks(&state.store, &recording)
    })
    .await
    // 패닉해도 녹음 상태는 아래에서 마무리한다. 여기서 빠져나가면 "녹음 중"에 묶인다
    .unwrap_or_else(|e| Err(eyre::eyre!("트랙을 합치다 멈췄습니다: {e}")));

    let error = match (&result, &mixed) {
        (Err(e), _) => Some(format!("녹음을 마무리하지 못했습니다: {e:#}")),
        (_, Err(e)) => Some(format!("트랙을 합치지 못했습니다: {e:#}")),
        (Ok(f), Ok(_)) => f
            .interrupted
            .as_ref()
            .map(|e| format!("녹음 장치 오류로 중간에 멈췄습니다. 그 전까지는 저장됐습니다: {e}")),
    };
    // 녹음하는 동안 남긴 메모가 있으므로 다시 읽어 상태만 바꾼다
    state
        .store
        .update(&id, |r| {
            r.status = Status::New;
            r.error = error;
            r.duration = mixed.ok();
        })
        .map_err(err)
}

#[tauri::command]
async fn summary_providers() -> Vec<summarize::ProviderStatus> {
    // 로그인 셸을 띄워 PATH를 알아내므로 메인 스레드를 막지 않게 async로 둔다
    tauri::async_runtime::spawn_blocking(summarize::statuses).await.unwrap_or_default()
}

#[tauri::command]
fn default_summary_prompt() -> &'static str {
    summarize::DEFAULT_PROMPT
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SummaryState {
    id: String,
    running: bool,
}

/// 요약 중인지 알린다. 자동 요약처럼 화면 밖에서 시작한 요약도 요약 화면에 보이게 한다
fn emit_summary_state(app: &AppHandle, id: &str, running: bool) {
    let _ = app.emit("summary-state", SummaryState { id: id.to_string(), running });
}

#[tauri::command]
fn summary_running(state: State<AppState>, id: String) -> bool {
    state.summaries.lock().unwrap().contains_key(&id)
}

/// AI로 녹음을 정리한다: 겹치는 캘린더 일정을 찾고, 제목, 태그, 할 일 후보를 한 번에 받는다.
/// 이미 정리했으면 force일 때만 다시 한다. 사람이 고친 제목과 태그는 그대로 두고, 미리 알림에 넣은 할 일은 남긴다.
/// 건너뛰면 None. 받아 적기가 끝난 녹음만 된다
/// @see docs/organize.md
#[tauri::command]
async fn organize_recording(app: AppHandle, id: String, force: Option<bool>) -> CmdResult<Option<Recording>> {
    let state = app.state::<AppState>();
    let recording = state.store.get(&id).map_err(err)?;
    if recording.auto_tagged_at.is_some() && force != Some(true) {
        return Ok(None);
    }
    let Some(_guard) = tagger::begin(&id) else { return Ok(None) };
    let transcript = state.store.get_transcript(&id).map_err(err)?.ok_or("먼저 전사를 끝내 주세요")?;
    let settings = state.store.get_settings(&state.home);

    // 일정은 한 번 찾으면 남긴다. 처음에는 macOS가 캘린더 허락을 묻는다
    let mut recording = recording;
    if settings.use_calendar && recording.event.is_none() {
        let start = recording.created_at;
        let end = start + chrono::Duration::milliseconds((recording.duration.unwrap_or(0.0) * 1000.0) as i64);
        if let Ok(Some(event)) = tauri::async_runtime::spawn_blocking(move || calendar::event_for(start, end)).await {
            recording = state.store.update(&id, |r| r.event = Some(event)).map_err(err)?;
        }
    }

    let summary = state.store.get_summary(&id).ok().flatten().map(|s| s.markdown);
    let vocab = tagger::vocabulary(&state.store.list().map_err(err)?, &id);
    let provider = match settings.summary_provider {
        Some(p) => p,
        None => tauri::async_runtime::spawn_blocking(summarize::statuses)
            .await
            .unwrap_or_default()
            .into_iter()
            .find(|s| s.path.is_some())
            .map(|s| s.provider)
            .ok_or("claude 또는 codex CLI가 없어 AI로 정리할 수 없습니다")?,
    };
    let prompt = tagger::build_prompt(&recording, &transcript, summary.as_deref(), &vocab);
    // 취소 버튼은 없다. 보내는 쪽을 끝까지 들고 있어야 run이 취소로 읽지 않는다
    let (_cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
    let answer = summarize::run(provider, prompt, cancel_rx).await.map_err(|e| format!("{e:#}"))?;
    let organized = tagger::parse_answer(&answer).map_err(|e| format!("{e:#}"))?;

    // 미리 알림에 넣은 할 일은 남기고, 아직 넣지 않은 후보만 새로 바꾼다
    let mut todos: Vec<store::Todo> = state.store.get_todos(&id).map_err(err)?.into_iter().filter(|t| t.added_at.is_some()).collect();
    for (text, due) in organized.todos {
        if !todos.iter().any(|t| t.text == text) {
            todos.push(store::Todo { id: uuid::Uuid::new_v4().to_string(), text, due, added_at: None });
        }
    }
    state.store.save_todos(&id, &todos).map_err(err)?;

    // AI가 도는 1분 사이 사람이 고쳤으면 그대로 둔다
    let saved = state
        .store
        .update(&id, |r| {
            if r.tags_edited_at.is_none() && !organized.tags.is_empty() {
                r.tags = organized.tags;
            }
            if let Some(title) = organized.title {
                if r.title_edited_at.is_none() && tagger::is_default_title(&r.title) {
                    r.title = title;
                }
            }
            r.auto_tagged_at = Some(chrono::Utc::now());
        })
        .map_err(err)?;
    sync::request(&app);
    Ok(Some(saved))
}

#[tauri::command]
fn get_todos(state: State<AppState>, id: String) -> CmdResult<Vec<store::Todo>> {
    state.store.get_todos(&id).map_err(err)
}

/// 할 일 후보를 고치거나 지운 결과를 저장한다
#[tauri::command]
fn save_todos(state: State<AppState>, id: String, todos: Vec<store::Todo>) -> CmdResult<()> {
    state.store.save_todos(&id, &todos).map_err(err)
}

/// 사용자가 확인한 할 일을 미리 알림에 넣는다. 이미 넣은 것은 건너뛴다. 넣은 개수를 돌려준다
#[tauri::command]
async fn add_todos_to_reminders(app: AppHandle, id: String, todo_ids: Vec<String>) -> CmdResult<usize> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let recording = state.store.get(&id).map_err(err)?;
        let mut todos = state.store.get_todos(&id).map_err(err)?;
        let picked: Vec<store::Todo> = todos.iter().filter(|t| t.added_at.is_none() && todo_ids.contains(&t.id)).cloned().collect();
        if picked.is_empty() {
            return Ok(0);
        }
        let note = format!("sori #{} {}", recording.number, recording.title);
        let added = calendar::add_reminders(&picked, &note).map_err(|e| format!("{e:#}"))?;
        let now = chrono::Utc::now();
        for t in todos.iter_mut().filter(|t| picked.iter().any(|p| p.id == t.id)) {
            t.added_at = Some(now);
        }
        state.store.save_todos(&id, &todos).map_err(err)?;
        Ok(added)
    })
    .await
    .map_err(err)?
}

/// template이 없으면 설정의 기본 템플릿으로 요약한다
#[tauri::command]
async fn summarize_recording(
    app: AppHandle,
    id: String,
    provider: Option<summarize::Provider>,
    template: Option<String>,
) -> CmdResult<summarize::Summary> {
    let state = app.state::<AppState>();
    let recording = state.store.get(&id).map_err(err)?;
    let transcript = state
        .store
        .get_transcript(&id)
        .map_err(err)?
        .ok_or("먼저 전사를 끝내 주세요")?;
    let settings = state.store.get_settings(&state.home);
    // 고른 템플릿이 그사이 설정에서 지워졌으면 다른 템플릿으로 몰래 요약하지 않고 알린다
    let chosen = match &template {
        Some(id) => settings.summary_templates.iter().find(|t| &t.id == id).cloned().ok_or("고른 요약 템플릿이 없습니다. 설정에서 지웠는지 확인해 주세요")?,
        None => settings
            .summary_templates
            .iter()
            .find(|t| Some(&t.id) == settings.default_template.as_ref())
            .or(settings.summary_templates.first())
            .cloned()
            .ok_or("요약 템플릿이 없습니다")?,
    };
    // CLI를 고르지 않았으면(자동 요약) 지난번에 쓴 것, 없으면 설치된 것 중 첫째
    let provider = match provider.or(settings.summary_provider) {
        Some(p) => p,
        None => tauri::async_runtime::spawn_blocking(summarize::statuses)
            .await
            .unwrap_or_default()
            .into_iter()
            .find(|s| s.path.is_some())
            .map(|s| s.provider)
            .ok_or("claude 또는 codex CLI가 없어 요약할 수 없습니다")?,
    };
    let prompt = summarize::build_prompt(&chosen.prompt, &recording, &transcript);

    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
    {
        let mut summaries = state.summaries.lock().unwrap();
        if summaries.contains_key(&id) {
            return Err("이미 요약하는 중입니다".into());
        }
        summaries.insert(id.clone(), cancel_tx);
    }
    emit_summary_state(&app, &id, true);
    let result = summarize::run(provider, prompt, cancel_rx).await;
    state.summaries.lock().unwrap().remove(&id);
    emit_summary_state(&app, &id, false);

    let markdown = result.map_err(|e| format!("{e:#}"))?;
    let summary = summarize::Summary {
        id: uuid::Uuid::new_v4().to_string(),
        template: Some(chosen.name),
        provider,
        created_at: chrono::Utc::now(),
        markdown,
    };
    state.store.save_summary(&id, &summary).map_err(err)?;
    // 다음 요약에서 같은 CLI를 미리 고르도록 기억한다. 요약하는 1분 사이 사용자가 바꾼 설정을
    // 덮지 않도록 저장 직전에 다시 읽고 이 값만 바꾼다
    let mut latest = state.store.get_settings(&state.home);
    latest.summary_provider = Some(provider);
    let _ = state.store.save_settings(&latest);
    Ok(summary)
}

#[tauri::command]
fn delete_summary(state: State<AppState>, id: String, summary_id: String) -> CmdResult<()> {
    state.store.delete_summary(&id, &summary_id).map_err(err)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Answer {
    markdown: String,
    /// 전체 녹음에 물었을 때 [R3 12:34]의 R3가 가리키는 녹음
    sources: Vec<ask::SourceRef>,
}

/// recording_id가 있으면 그 녹음에, 없으면 모든 녹음에 묻는다. 취소 키는 "ask" 또는 "ask:<녹음 id>"
#[tauri::command]
async fn ask_recordings(
    app: AppHandle,
    question: String,
    recording_id: Option<String>,
    // 전체 녹음에 물을 때 이 태그(하위 태그 포함)가 붙은 녹음만 넘긴다. 15만 자 상한에 오래된 녹음이 잘리지 않게 좁힌다
    tag: Option<String>,
    provider: summarize::Provider,
) -> CmdResult<Answer> {
    let state = app.state::<AppState>();
    let worker = app.clone();
    let scope = recording_id.clone();
    let q = question.clone();
    let (prompt, sources) = tauri::async_runtime::spawn_blocking(move || -> eyre::Result<(String, Vec<ask::SourceRef>)> {
        let state = worker.state::<AppState>();
        let store = &state.store;
        match scope {
            Some(id) => {
                let recording = store.get(&id)?;
                let transcript = store.get_transcript(&id)?.ok_or_else(|| eyre::eyre!("먼저 전사를 끝내 주세요"))?;
                let summary = store.get_summary(&id)?;
                Ok((ask::single_prompt(&q, &recording, &transcript, summary.as_ref()), Vec::new()))
            }
            None => {
                let mut recordings = store.list()?;
                if let Some(tag) = tag.as_deref() {
                    recordings.retain(|r| sori_sync::tags::matches(&r.tags, tag));
                    if recordings.is_empty() {
                        eyre::bail!("'{tag}' 태그가 붙은 녹음이 없습니다");
                    }
                }
                let loaded: Vec<_> = recordings
                    .iter()
                    .map(|r| (store.get_transcript(&r.id).ok().flatten(), store.get_summary(&r.id).ok().flatten()))
                    .collect();
                let candidates: Vec<ask::Candidate> = recordings
                    .iter()
                    .zip(&loaded)
                    .map(|(r, (t, s))| ask::Candidate { recording: r, transcript: t.as_ref(), summary: s.as_ref() })
                    .collect();
                Ok(ask::all_prompt(&q, &candidates))
            }
        }
    })
    .await
    .map_err(err)?
    .map_err(|e| format!("{e:#}"))?;

    let key = recording_id.map_or_else(|| "ask".to_string(), |id| format!("ask:{id}"));
    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
    {
        let mut running = state.summaries.lock().unwrap();
        if running.contains_key(&key) {
            return Err("이미 답을 기다리는 중입니다".into());
        }
        running.insert(key.clone(), cancel_tx);
    }
    let result = summarize::run(provider, prompt, cancel_rx).await;
    state.summaries.lock().unwrap().remove(&key);
    Ok(Answer { markdown: result.map_err(|e| format!("{e:#}"))?, sources })
}

#[tauri::command]
fn cancel_summary(state: State<AppState>, id: String) {
    if let Some(tx) = state.summaries.lock().unwrap().remove(&id) {
        let _ = tx.send(());
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecordingChanged {
    pub(crate) id: String,
}

/// 메모나 북마크를 더한다. id가 없으면 지금 녹음 중인 항목, time이 없으면 녹음을 시작한 지 지난 시간
#[tauri::command]
fn add_note(app: AppHandle, state: State<AppState>, id: Option<String>, time: Option<f32>, text: String) -> CmdResult<Recording> {
    let current = state.recorder.lock().unwrap().as_ref().map(|(r, _)| r.clone());
    let id = id.or(current.clone()).ok_or("녹음 중이 아니라 북마크를 남길 곳이 없습니다")?;
    let time = match time {
        Some(t) => t.max(0.0),
        None if current.as_deref() == Some(id.as_str()) => state
            .recording_started
            .lock()
            .unwrap()
            .map_or(0.0, |t| t.elapsed().as_secs_f32()),
        None => 0.0,
    };
    let recording = state
        .store
        .update(&id, |r| {
            let at = r.notes.partition_point(|n| n.time <= time);
            r.notes.insert(at, store::Note { time, text: text.trim().to_string(), id: None, device: None });
        })
        .map_err(err)?;
    let _ = app.emit("recording-changed", RecordingChanged { id });
    sync::request(&app);
    Ok(recording)
}

#[tauri::command]
fn delete_note(app: AppHandle, state: State<AppState>, id: String, index: usize) -> CmdResult<Recording> {
    let recording = state
        .store
        .update(&id, |r| {
            if index < r.notes.len() {
                r.notes.remove(index);
            }
        })
        .map_err(err)?;
    let _ = app.emit("recording-changed", RecordingChanged { id });
    sync::request(&app);
    Ok(recording)
}

/// 화자 이름을 바꾼다. 목소리 기억이 켜져 있으면 이 녹음의 목소리를 그 이름으로 기억하고,
/// 예전에 다른 이름으로 알아본 화자였다면 그 이름 쪽에서 이 녹음의 목소리를 뺀다.
/// 목소리 기억은 덤이라 실패해도 이름은 바꾼다
#[tauri::command]
async fn name_speaker(app: AppHandle, id: String, speaker: i32, name: String) -> CmdResult<Recording> {
    tauri::async_runtime::spawn_blocking(move || -> eyre::Result<Recording> {
        let state = app.state::<AppState>();
        let key = speaker.to_string();
        let name = name.trim().to_string();
        let mut previous_voice = None;
        let recording = state.store.update(&id, |r| {
            previous_voice = r.voice_ids.remove(&key);
            if name.is_empty() {
                r.speaker_names.remove(&key);
            } else {
                r.speaker_names.insert(key.clone(), name.clone());
            }
        })?;
        if !state.store.get_settings(&state.home).remember_voices {
            return Ok(recording);
        }
        match remember_speaker(&state, &recording, speaker, &name, previous_voice.as_deref()) {
            Ok(Some(voice)) => Ok(state.store.update(&id, |r| {
                // 기억하는 사이 이름이 또 바뀌었으면 연결하지 않는다
                if r.speaker_names.get(&key) == Some(&name) {
                    r.voice_ids.insert(key.clone(), voice);
                }
            })?),
            Ok(None) => Ok(recording),
            Err(e) => {
                eprintln!("목소리를 기억하지 못했습니다({id} 화자 {speaker}): {e:#}");
                Ok(recording)
            }
        }
    })
    .await
    .map_err(err)?
    .map_err(|e| format!("{e:#}"))
}

/// 화자의 목소리 특징을 이름에 더한다. 기억한 목소리 id를 돌려준다
fn remember_speaker(state: &AppState, recording: &Recording, speaker: i32, name: &str, previous: Option<&str>) -> eyre::Result<Option<String>> {
    let Some(transcript) = state.store.get_transcript(&recording.id)? else {
        return Ok(None);
    };
    let embedding = match transcript.speaker_embeddings.get(&speaker) {
        Some(e) => e.clone(),
        None if transcript.speaker_embeddings.is_empty() => {
            // 0.3.0 이하에서 처리한 녹음은 목소리 특징이 없어 지금 뽑는다. 그사이 다시 처리했으면 새 전사를 덮지 않는다
            let settings = state.store.get_settings(&state.home);
            let audio = state.store.audio_path(recording)?;
            let embeddings = engine::embeddings_for(&audio, &transcript.turns, &settings.models.embedding, engine::worker::current_exe().as_deref())?;
            let found = embeddings.get(&speaker).cloned();
            if let Some(mut latest) = state.store.get_transcript(&recording.id)? {
                if latest.speaker_embeddings.is_empty() && latest.turns.len() == transcript.turns.len() {
                    latest.speaker_embeddings = embeddings;
                    state.store.save_transcript(&recording.id, &latest)?;
                }
            }
            match found {
                Some(e) => e,
                None => return Ok(None),
            }
        }
        None => return Ok(None),
    };
    if let Some(voice) = previous {
        state.voices.forget_sample(voice, &embedding)?;
    }
    if name.is_empty() {
        return Ok(None);
    }
    Ok(Some(state.voices.remember(name, &embedding)?))
}

#[tauri::command]
fn list_voices(state: State<AppState>) -> Vec<voices::VoiceInfo> {
    state.voices.list()
}

#[tauri::command]
fn rename_voice(state: State<AppState>, id: String, name: String) -> CmdResult<()> {
    let name = name.trim();
    if name.is_empty() {
        return Err("이름을 적어 주세요".into());
    }
    state.voices.rename(&id, name).map_err(err)
}

#[tauri::command]
fn delete_voice(state: State<AppState>, id: String) -> CmdResult<()> {
    state.voices.delete(&id).map_err(err)
}

/// 저장 창을 띄워 내보낸 파일을 쓴다. 경로는 여기서 고르게 해, 화면 코드가 임의 경로에 쓰지 못하게 한다.
/// 본문은 바이트 그대로, 제안할 파일 이름은 x-name 헤더(URL 인코딩)로 받는다. 취소하면 false
#[tauri::command]
async fn save_export(app: AppHandle, request: tauri::ipc::Request<'_>) -> CmdResult<bool> {
    use tauri_plugin_dialog::DialogExt;
    let name = request
        .headers()
        .get("x-name")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| tauri::Url::parse(&format!("sori://export?n={v}")).ok())
        .and_then(|u| u.query_pairs().find(|(k, _)| k == "n").map(|(_, v)| v.into_owned()))
        .unwrap_or_else(|| "sori".into());
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("내보낼 내용이 없습니다".into());
    };
    let bytes = bytes.clone();
    let ext = Path::new(&name).extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
    let (tx, rx) = tokio::sync::oneshot::channel();
    let mut dialog = app.dialog().file().set_file_name(&name);
    if !ext.is_empty() {
        dialog = dialog.add_filter(ext.to_uppercase(), &[ext.as_str()]);
    }
    dialog.save_file(move |path| {
        let _ = tx.send(path);
    });
    let Some(path) = rx.await.map_err(err)? else {
        return Ok(false);
    };
    let path = path.into_path().map_err(err)?;
    std::fs::write(&path, bytes).map_err(|e| format!("{}에 저장하지 못했습니다: {e}", path.display()))?;
    Ok(true)
}

/// 회의 질문 창이 띄울 내용
#[tauri::command]
fn meeting_prompt(state: State<AppState>) -> Option<meeting::Prompt> {
    state.meeting_prompt.lock().unwrap().clone()
}

/// 회의 질문 창의 답. 녹음을 시작하거나 멈추는 일은 메인 화면의 녹음기가 한다
#[tauri::command]
fn meeting_prompt_respond(app: AppHandle, state: State<AppState>, accept: bool) {
    let prompt = state.meeting_prompt.lock().unwrap().take();
    if let Some(window) = app.get_webview_window(MEETING_WINDOW) {
        // close는 이벤트 루프를 거쳐 늦게 닫혀, 바로 같은 이름으로 새 창을 만들면 실패한다
        let _ = window.destroy();
    }
    match prompt {
        Some(meeting::Prompt::Start { app: name }) if accept => {
            state.meeting.lock().unwrap().recording_started_for(name);
            actions::dispatch(&app, actions::Action::StartRecording);
        }
        Some(meeting::Prompt::Stop { .. }) if accept => actions::dispatch(&app, actions::Action::StopRecording),
        _ => {}
    }
}

const MEETING_WINDOW: &str = "meeting-prompt";

/// 화면 오른쪽 위에 작은 창을 띄운다. 회의 앱을 보는 중이라 초점은 가져오지 않는다
fn show_meeting_prompt(app: &AppHandle, prompt: meeting::Prompt) {
    *app.state::<AppState>().meeting_prompt.lock().unwrap() = Some(prompt);
    if let Some(window) = app.get_webview_window(MEETING_WINDOW) {
        let _ = window.destroy();
    }
    let (width, height) = (360.0, 108.0);
    let (x, y) = app
        .primary_monitor()
        .ok()
        .flatten()
        .map(|m| {
            let scale = m.scale_factor();
            let pos = m.position().to_logical::<f64>(scale);
            let size = m.size().to_logical::<f64>(scale);
            (pos.x + size.width - width - 16.0, pos.y + 40.0)
        })
        .unwrap_or((40.0, 40.0));
    let built = tauri::WebviewWindowBuilder::new(app, MEETING_WINDOW, tauri::WebviewUrl::App("index.html?view=meeting".into()))
        .title("sori")
        .inner_size(width, height)
        .position(x, y)
        .resizable(false)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .visible_on_all_workspaces(true)
        .build();
    if let Err(e) = built {
        eprintln!("회의 질문 창을 띄우지 못했습니다: {e}");
    }
}

/// 3초마다 마이크를 쓰는 회의 앱을 살핀다
fn watch_meetings(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(3));
        let state = app.state::<AppState>();
        if !state.store.get_settings(&state.home).meeting_detection {
            continue;
        }
        let apps = meeting::meeting_apps_using_mic();
        let recording = state.recorder.lock().unwrap().is_some();
        let prompt = state.meeting.lock().unwrap().update(&apps, recording, Instant::now());
        if let Some(prompt) = prompt {
            let handle = app.clone();
            // 창은 메인 스레드에서 만든다
            let _ = app.run_on_main_thread(move || show_meeting_prompt(&handle, prompt));
        }
    });
}

#[tauri::command]
async fn process_recording(app: AppHandle, request: ProcessRequest) -> CmdResult<Transcript> {
    run_processing(app, request).await
}

/// 받아 적기와 화자 나누기. 화면의 명령과 동기화(다른 기기에서 온 녹음 자동 처리)가 함께 쓴다
pub(crate) async fn run_processing(app: AppHandle, request: ProcessRequest) -> CmdResult<Transcript> {
    let state = app.state::<AppState>();
    let id = request.id.clone();
    // 모델이 없으면 whisper.cpp의 알아보기 힘든 오류 대신 설치 방법을 알려 준다
    models::check(&state.store.get_settings(&state.home).models)?;

    let diarize = request.diarize.unwrap_or_else(|| state.store.get_settings(&state.home).default_diarize);
    state
        .store
        .update(&id, |r| {
            r.status = Status::Processing;
            r.error = None;
            r.num_speakers = request.num_speakers;
            r.languages = request.languages.clone();
            r.diarize = diarize;
        })
        .map_err(err)?;

    let cancel = Arc::new(AtomicBool::new(false));
    state.cancels.lock().unwrap().insert(id.clone(), cancel.clone());

    // 한 번에 하나만 처리한다. 둘 다 CPU와 GPU를 다 써서 동시에 돌려도 합친 시간은 줄지 않고 각각 느려진다.
    // 기다리는 동안 멈춘 것처럼 보이지 않게 알린다
    let _guard = match state.pipeline.try_lock() {
        Ok(guard) => guard,
        Err(_) => {
            let _ = app.emit("progress", ProgressEvent { id: id.clone(), stage: Stage::Waiting, progress: 0.0 });
            state.pipeline.lock().await
        }
    };
    let worker_app = app.clone();
    let worker_id = id.clone();
    let worker_cancel = cancel.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        // 앞 작업을 기다리는 동안 취소했으면 시작하지 않는다
        if worker_cancel.load(Ordering::Relaxed) {
            eyre::bail!("취소했습니다");
        }
        let state = worker_app.state::<AppState>();
        let settings = state.store.get_settings(&state.home);
        let mut recording = state.store.get(&worker_id)?;
        // 다른 기기에서 온 녹음은 오디오를 이때 받는다
        if !state.store.audio_path(&recording)?.is_file() {
            sync::fetch_audio(&worker_app, &worker_id)?;
            recording = state.store.get(&worker_id)?;
        }
        let audio = state.store.audio_path(&recording)?;
        let whisper = state.whisper(&settings.models.whisper)?;
        let track_paths = state.store.track_paths(&recording)?;
        let find = |name: &str| track_paths.iter().find(|p| p.ends_with(name)).filter(|p| p.exists()).cloned();
        let tracks = find("mic.wav").zip(find("system.wav"));

        let emitter = worker_app.clone();
        let event_id = worker_id.clone();
        engine::process(
            &audio,
            &whisper,
            &settings.models,
            &ProcessOptions {
                num_speakers: request.num_speakers,
                languages: request.languages,
                tracks,
                diarize_worker: engine::worker::current_exe(),
                diarize,
            },
            cancel,
            move |stage, progress| {
                let _ = emitter.emit(
                    "progress",
                    ProgressEvent {
                        id: event_id.clone(),
                        stage,
                        progress,
                    },
                );
            },
        )
    })
    .await
    // 패닉해도 상태를 "실패"로 남기고 취소 목록을 비운다. 여기서 빠져나가면 "처리 중"에 묶여 삭제도 안 된다
    .unwrap_or_else(|e| Err(eyre::eyre!("처리하다 멈췄습니다: {e}")));

    state.cancels.lock().unwrap().remove(&id);
    match result {
        Ok(transcript) => {
            state.store.save_transcript(&id, &transcript).map_err(err)?;
            // 다시 처리하면 화자 번호가 바뀔 수 있어, 예전 번호에 붙인 이름은 지우고 새로 붙인다
            let mut names = std::collections::BTreeMap::new();
            let mut voice_ids = std::collections::BTreeMap::new();
            // 마이크 쪽 화자가 한 명뿐이면 녹음한 본인이다
            let locals: Vec<i32> = transcript
                .origins
                .iter()
                .filter(|(_, o)| **o == engine::origin::Origin::Local)
                .map(|(s, _)| *s)
                .collect();
            if let [me] = locals.as_slice() {
                names.insert(me.to_string(), "나".to_string());
            }
            if state.store.get_settings(&state.home).remember_voices {
                // 마이크 쪽 한 사람은 본인이 확실하므로 "나"의 목소리로 기억해 둔다. 가져온 파일에서도 본인을 알아본다
                if let [me] = locals.as_slice() {
                    if let Some(e) = transcript.speaker_embeddings.get(me) {
                        if let Ok(voice) = state.voices.remember("나", e) {
                            voice_ids.insert(me.to_string(), voice);
                        }
                    }
                }
                let named: std::collections::BTreeSet<i32> = names.keys().filter_map(|k| k.parse().ok()).collect();
                for m in state.voices.match_speakers(&transcript.speaker_embeddings, &named) {
                    names.insert(m.speaker.to_string(), m.name);
                    voice_ids.insert(m.speaker.to_string(), m.voice_id);
                }
            }
            state
                .store
                .update(&id, |r| {
                    r.speaker_names = names;
                    r.voice_ids = voice_ids;
                    r.status = Status::Done;
                    r.duration = Some(transcript.duration);
                })
                .map_err(err)?;
            sync::request(&app);
            Ok(transcript)
        }
        Err(e) => {
            let message = format!("{e:#}");
            state
                .store
                .update(&id, |r| {
                    r.status = Status::Failed;
                    r.error = Some(message.clone());
                })
                .map_err(err)?;
            Err(message)
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // 다른 플러그인보다 먼저 등록해야 두 번째 실행을 가장 먼저 가로챈다
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            // 딥링크는 deep-link 플러그인이 넘겨받는다. 녹음 토글 링크로 창이 앞에 뜨지 않도록 여기서는 건드리지 않는다
            if args.iter().any(|a| a.starts_with("sori:")) {
                return;
            }
            let files = opened_files(args.iter().skip(1).map(PathBuf::from));
            if files.is_empty() {
                actions::show_main(app);
            } else {
                actions::dispatch(app, actions::Action::Import { paths: files, process: false });
            }
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state() == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                        actions::on_shortcut(app, shortcut);
                    }
                })
                .build(),
        )
        .manage(actions::Bus::new())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let home = app.path().home_dir()?;
            let store = Store::new(data_dir)?;
            let voices_path = store.voices_path();

            store.assign_missing_numbers()?;
            // 처리 도중 앱이 꺼졌던 녹음은 다시 시작할 수 있게 실패로 돌려 둔다
            for mut r in store.list()?.into_iter() {
                match r.status {
                    Status::Processing => {
                        r.status = Status::Failed;
                        r.error = Some("처리 도중 앱이 종료됐습니다".into());
                    }
                    // WAV 헤더를 2초마다 갱신하므로 마지막 2초 정도만 빠지고 나머지는 재생된다
                    Status::Recording => {
                        r.status = Status::New;
                        r.duration = finalize_tracks(&store, &r).ok();
                        r.error = Some("녹음 도중 앱이 종료됐습니다. 마지막 몇 초가 빠졌을 수 있습니다".into());
                    }
                    _ => continue,
                }
                store.save(&r)?;
            }

            app.manage(AppState {
                store,
                home,
                whisper: Mutex::new(None),
                cancels: Mutex::new(HashMap::new()),
                pipeline: tokio::sync::Mutex::new(()),
                recorder: Mutex::new(None),
                summaries: Mutex::new(HashMap::new()),
                model_install: Mutex::new(None),
                search: search::Index::default(),
                shortcut_error: Mutex::new(None),
                voices: voices::VoiceBook::new(voices_path),
                recording_started: Mutex::new(None),
                meeting: Mutex::new(meeting::Detector::default()),
                meeting_prompt: Mutex::new(None),
                syncer: sync::Syncer::new(),
            });
            watch_meetings(app.handle().clone());
            sync::start(app.handle().clone());

            let handle = app.handle().clone();
            // 링크를 처리하기 전에 읽어야 링크로 앱을 띄운 경우도 확인할 수 있다
            actions::load_token(&handle, &app.path().app_data_dir()?)?;
            let settings = app.state::<AppState>().store.get_settings(&home_dir(&handle));
            if let Err(e) = actions::apply_shortcuts(&handle, &settings.shortcuts, None) {
                *app.state::<AppState>().shortcut_error.lock().unwrap() = Some(e);
            }

            setup_deep_links(app)?;
            // Windows는 파일을 sori로 열면 경로를 실행 인자로 넘긴다. macOS는 RunEvent::Opened로 온다
            let files = opened_files(std::env::args_os().skip(1).map(PathBuf::from));
            if !files.is_empty() {
                actions::dispatch(&handle, actions::Action::Import { paths: files, process: false });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            model_catalog,
            install_models,
            cancel_model_install,
            list_recordings,
            import_recordings,
            get_recording,
            update_recording,
            set_tags,
            organize_recording,
            get_todos,
            save_todos,
            add_todos_to_reminders,
            delete_recording,
            cancel_processing,
            process_recording,
            list_input_devices,
            start_recording,
            stop_recording,
            summary_providers,
            default_summary_prompt,
            summarize_recording,
            cancel_summary,
            frontend_ready,
            search_recordings,
            suspend_shortcuts,
            resume_shortcuts,
            summary_running,
            delete_summary,
            ask_recordings,
            add_note,
            delete_note,
            name_speaker,
            list_voices,
            rename_voice,
            delete_voice,
            save_export,
            meeting_prompt,
            meeting_prompt_respond,
            reveal_audio,
            sync_status,
            sync_now,
            download_audio,
            default_sync_folder,
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|app, event| {
            // Finder에서 "다음으로 열기 > sori"를 고르거나 Dock 아이콘에 파일을 끌어다 놓은 경우
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Opened { urls } = &event {
                let files = opened_files(urls.iter().filter_map(|u| u.to_file_path().ok()));
                if !files.is_empty() {
                    actions::dispatch(app, actions::Action::Import { paths: files, process: false });
                }
            }
            if let tauri::RunEvent::Exit = &event {
                exit_cleanly(app);
            }
        });
}

/// 앱을 끌 때 비정상 종료("예기치 않게 종료됨")가 나지 않게 정리한다. 원인이 둘이었다.
/// 1. whisper 모델(Metal)을 앱 상태에 들고 있으면, 프로세스 종료 정리에서 ggml Metal 장치가
///    "쓰는 GPU 메모리가 없어야 한다"고 검사하다 abort한다(ggml-metal-device.m GGML_ASSERT([rsets->data count] == 0)).
///    처리 중이 아니어도 한 번이라도 받아 적은 뒤 끄면 났다. 모델을 먼저 해제한다
/// 2. 처리나 요약 도중이면 아직 도는 whisper, onnxruntime 스레드가 종료 정리와 엉킨다.
///    진행 중인 일을 멈추고 잠깐 기다린 뒤 전역 정리를 건너뛰고 끝낸다
/// 녹음 중이면 파일을 마무리한다. 재현: examples/exit_check.rs, docs/processing.md
fn exit_cleanly(app: &AppHandle) {
    let state = app.state::<AppState>();
    if let Some((id, active)) = state.recorder.lock().unwrap().take() {
        let _ = active.stop();
        if let Ok(recording) = state.store.get(&id) {
            let duration = finalize_tracks(&state.store, &recording).ok();
            let _ = state.store.update(&id, |r| {
                r.status = Status::New;
                r.duration = duration;
            });
        }
    }
    let busy = {
        let cancels = state.cancels.lock().unwrap();
        cancels.values().for_each(|c| c.store(true, Ordering::Relaxed));
        !cancels.is_empty()
    };
    let summarizing = {
        let mut summaries = state.summaries.lock().unwrap();
        let any = !summaries.is_empty();
        summaries.drain().for_each(|(_, tx)| {
            let _ = tx.send(());
        });
        any
    };
    // 처리 스레드가 모델을 함께 쥐고 있으면(Arc 둘 이상) 여기서 놓아도 해제되지 않는다
    let whisper = state.whisper.lock().unwrap().take();
    let whisper_in_use = whisper.as_ref().is_some_and(|(_, ctx)| Arc::strong_count(ctx) > 1);
    drop(whisper);

    if busy || summarizing || whisper_in_use {
        // 화자 분리 프로세스는 0.1초마다 취소를 확인해 끝낸다. 요약 CLI도 취소 신호를 받아 끝낸다
        std::thread::sleep(Duration::from_millis(500));
        for id in state.cancels.lock().unwrap().keys() {
            let _ = state.store.update(id, |r| {
                r.status = Status::Failed;
                r.error = Some("처리 도중 앱을 종료했습니다. 다시 처리해 주세요".into());
            });
        }
        unsafe extern "C" {
            fn _exit(code: i32) -> !;
        }
        unsafe { _exit(0) }
    }
}

fn home_dir(app: &AppHandle) -> PathBuf {
    app.path().home_dir().unwrap_or_default()
}

/// 실행 인자나 열기 요청 중 가져올 수 있는 오디오 파일만 고른다
fn opened_files(paths: impl Iterator<Item = PathBuf>) -> Vec<PathBuf> {
    actions::audio_files(paths)
}

fn setup_deep_links(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    use tauri_plugin_deep_link::DeepLinkExt;
    // 설치 파일이 스킴을 등록한다. 개발 중(cargo run)인 Windows와 Linux만 직접 등록한다
    #[cfg(all(debug_assertions, any(windows, target_os = "linux")))]
    app.deep_link().register_all()?;

    let handle = app.handle().clone();
    app.deep_link().on_open_url(move |event| {
        for url in event.urls() {
            actions::handle_url(&handle, &url);
        }
    });
    // 링크로 앱을 처음 띄운 경우. 화면이 준비되면 frontend_ready로 넘긴다
    if let Some(urls) = app.deep_link().get_current()? {
        for url in urls {
            actions::handle_url(app.handle(), &url);
        }
    }
    Ok(())
}

/// 녹음의 오디오 파일을 Finder(Windows는 탐색기)에서 보여 준다
#[tauri::command]
fn sync_status(state: State<AppState>) -> sync::SyncStatus {
    state.syncer.status()
}

#[tauri::command]
fn sync_now(app: AppHandle) {
    sync::request_now(&app);
}

#[tauri::command]
fn default_sync_folder(state: State<AppState>, provider: sync::Provider) -> Option<PathBuf> {
    sync::default_folder(provider, &state.home)
}

/// 다른 기기에서 온 녹음의 오디오를 받는다. 진행은 "audio-download" 이벤트로 알린다
#[tauri::command]
async fn download_audio(app: AppHandle, id: String) -> CmdResult<()> {
    tauri::async_runtime::spawn_blocking(move || sync::fetch_audio(&app, &id).map(|_| ()))
        .await
        .map_err(err)?
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
fn reveal_audio(state: State<AppState>, id: String) -> CmdResult<()> {
    let recording = state.store.get(&id).map_err(err)?;
    let path = state.store.audio_path(&recording).map_err(err)?;
    tauri_plugin_opener::reveal_item_in_dir(path).map_err(err)
}
