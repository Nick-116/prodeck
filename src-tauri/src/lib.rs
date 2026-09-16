mod ahmap;
mod app;
mod audio;
mod avantis;
mod backup;
mod chat;
mod checkin;
mod diag;
mod discovery;
mod eb;
mod edge;
mod ga4;
mod gemini;
mod identity;
mod keepalive;
mod midi;
mod ndi;
mod obs;
mod osc;
mod pages;
mod pco;
mod pcoauth;
mod posfiles;
mod propresenter;
mod push;
mod relay;
mod settings;
mod tap;
mod transcription;
mod web;
mod x32;

use std::sync::{Arc, Mutex};
use tokio::sync::Mutex as AsyncMutex;
use tokio::sync::broadcast;
use app::{AppHandle, AppState};

/// One-time migration from the legacy data-folder name.
fn migrate_legacy_data_dir() {
    let Some(base) = dirs::config_dir() else { return };
    let old = base.join("ProdLink");
    let new = base.join("ProDeck");
    if !old.is_dir() {
        return;
    }
    if new.join("settings.json").exists() {
        return;
    }
    if !new.exists() {
        if let Err(e) = std::fs::rename(&old, &new) {
            crate::diag::log(format!("[migrate] could not rename {} -> {}: {e}", old.display(), new.display()));
            return;
        }
    } else {
        let rd = match std::fs::read_dir(&old) {
            Ok(rd) => rd,
            Err(e) => {
                crate::diag::log(format!("[migrate] could not read {}: {e}", old.display()));
                return;
            }
        };
        for ent in rd.flatten() {
            let dst = new.join(ent.file_name());
            if dst.exists() { continue; }
            if let Err(e) = std::fs::rename(ent.path(), &dst) {
                crate::diag::log(format!("[migrate] could not move {}: {e}", ent.path().display()));
            }
        }
    }
    let sp = new.join("settings.json");
    let Ok(txt) = std::fs::read_to_string(&sp) else { return };
    let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&txt) else { return };
    let mut changed = false;
    if let Some(obj) = v.as_object_mut() {
        for val in obj.values_mut() {
            if let Some(sv) = val.as_str() {
                if sv.starts_with('/') && sv.contains("/ProdLink/") {
                    *val = serde_json::Value::String(sv.replace("/ProdLink/", "/ProDeck/"));
                    changed = true;
                }
            }
        }
    }
    if !changed { return; }
    let Ok(out) = serde_json::to_string_pretty(&v) else { return };
    let tmp = sp.with_extension("json.tmp");
    let res = std::fs::write(&tmp, out).and_then(|_| std::fs::rename(&tmp, &sp));
    if let Err(e) = res {
        crate::diag::log(format!("[migrate] could not rewrite settings.json: {e}"));
        let _ = std::fs::remove_file(&tmp);
    }
}

pub fn run() {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
        .block_on(async_run());
}

async fn async_run() {
    migrate_legacy_data_dir();

    let loaded_settings = settings::load();

    let port: u16 = std::env::var("PRODECK_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| {
            let p = loaded_settings.web_port;
            if p > 0 { p } else { 4000 }
        });

    let (events_tx, _) = broadcast::channel::<(String, String)>(4096);

    let app = AppHandle(Arc::new(AppState {
        settings: Arc::new(Mutex::new(loaded_settings)),
        keep_awake: Arc::new(keepalive::KeepAwake(Mutex::new(None))),
        midi: Arc::new(midi::MidiState::new()),
        midi_out: Arc::new(midi::MidiOutState::new()),

        propresenter: Arc::new(Arc::new(AsyncMutex::new(None::<propresenter::ProPresenterConnection>))),
        ndi: Arc::new(Arc::new(AsyncMutex::new(ndi::NdiManager::new()))),
        relay: Arc::new(Arc::new(AsyncMutex::new(relay::RelayManager::new()))),
        obs: Arc::new(obs::new_state()),
        audio: Arc::new(Arc::new(audio::AudioInner::new())),
        transcription: Arc::new(Arc::new(transcription::TranscriptionInner::new())),
        osc: Arc::new(Arc::new(osc::OscInner::new())),
        pco: Arc::new(Arc::new(pco::PcoInner::new())),
        web: Arc::new(Arc::new(web::WebInner::new())),
        tap: Arc::new(Arc::new(AsyncMutex::new(tap::TapInner::new()))),
        chat: Arc::new(Arc::new(chat::ChatInner::new())),
        pages: Arc::new(Arc::new(pages::PagesInner::new())),
        push: Arc::new(Arc::new(push::PushInner::load())),
        checkin: Arc::new(Arc::new(checkin::CheckinInner::load())),
        posfiles: Arc::new(Arc::new(posfiles::PosFilesInner::load())),
        identity: Arc::new(Arc::new(identity::IdentityInner::load())),
        avantis: Arc::new(Arc::new(Mutex::new(avantis::AvantisInner::default()))),
        ga4: Arc::new(ga4::new_state()),

        events_tx,
    }));

    // Enable keep-awake if configured.
    {
        let on = app.state::<settings::SettingsState>()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .keep_awake;
        if on {
            keepalive::set_keep_awake(&app, true);
        }
    }

    // Start PCO OAuth background refresh.
    pcoauth::init(app.clone());

    // Register port so PCO OAuth can use /pco/callback on this server.
    pcoauth::set_admin_port(port);

    // Start the web gateway — injects __PRODECK_ADMIN_PANEL__ for full UI.
    let web_state = app.state::<web::WebState>().inner().clone();
    web::start(app.clone(), web_state, port, true);

    // Background workers.
    tap::spawn_heartbeat(app.clone());
    avantis::spawn_mirror(app.clone());
    avantis::spawn_watch_flush(app.clone());
    obs::spawn_client(app.clone());
    x32::spawn_mirror(app.clone());
    edge::spawn_edge_push(app.clone());
    ga4::spawn_ga4_poll(app.clone());
    propresenter::spawn_lobby_auto(app.clone());
    propresenter::spawn_announcement_poll(app.clone());

    eprintln!("ProDeck listening on 0.0.0.0:{port}");

    // Block until SIGINT/SIGTERM.
    tokio::signal::ctrl_c().await.ok();
}
