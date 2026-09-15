// AppHandle + AppState + State<T> — a standalone replacement for the Tauri
// type system. Drop-in API: every module that used tauri::AppHandle /
// tauri::State keeps compiling with only import + attribute changes.

use serde::Serialize;
use std::sync::Arc;
use tokio::sync::broadcast;

// The global event bus. emit() pushes here; web.rs subscribes for SSE fan-out.
pub type EventSender = broadcast::Sender<(String, String)>;
pub type EventReceiver = broadcast::Receiver<(String, String)>;

/// Minimal Event wrapper so modules that call app.listen() compile unchanged.
pub struct Event {
    payload_str: String,
}
impl Event {
    pub fn payload(&self) -> &str {
        &self.payload_str
    }
}

/// AppHandle is cloned cheaply (just clones the inner Arc).
#[derive(Clone)]
pub struct AppHandle(pub Arc<AppState>);

impl AppHandle {
    pub fn state<T>(&self) -> State<T>
    where
        AppState: HasState<T>,
    {
        State(<AppState as HasState<T>>::get(&self.0))
    }

    /// Serialize payload to JSON and broadcast to all SSE subscribers.
    pub fn emit<S: Serialize>(&self, event: &str, payload: S) -> Result<(), String> {
        let json = serde_json::to_string(&payload).unwrap_or_else(|_| "null".into());
        let _ = self.0.events_tx.send((event.to_string(), json));
        Ok(())
    }

    /// Subscribe to the raw event stream (used by the web gateway SSE fan-out).
    pub fn subscribe_events(&self) -> EventReceiver {
        self.0.events_tx.subscribe()
    }

    /// Compatibility shim for modules that call app.listen(). Spawns a tokio
    /// task that filters the event bus and calls the handler for each match.
    pub fn listen<F>(&self, event: &str, handler: F)
    where
        F: Fn(Event) + Send + 'static,
    {
        let mut rx = self.0.events_tx.subscribe();
        let name = event.to_string();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok((ev, payload)) if ev == name => {
                        handler(Event { payload_str: payload });
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
        });
    }
}

/// State<T> mimics tauri::State<'_, T>: deref to T, inner() returns &T.
pub struct State<T>(pub(crate) Arc<T>);

impl<T> std::ops::Deref for State<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> State<T> {
    /// Returns &T, matching tauri::State::inner() which also returns &T.
    pub fn inner(&self) -> &T {
        &self.0
    }
}

/// HasState<T>: implemented once per managed state type so
/// AppHandle::state::<T>() can look up the right field.
pub trait HasState<T> {
    fn get(this: &AppState) -> Arc<T>;
}

// ---------------------------------------------------------------------------
// AppState: one field per managed state type
// ---------------------------------------------------------------------------
//
// For state types that are already Arc<X> (e.g. ProPresenterState =
// Arc<AsyncMutex<...>>), we wrap them in a second Arc so HasState<T> can
// return Arc<T>.  The overhead is one extra allocation at startup, which is
// negligible; every subsequent state() call is just an Arc clone.

pub struct AppState {
    // -- types that are plain wrappers (Arc<plain_type>) --
    pub settings: Arc<crate::settings::SettingsState>,      // Arc<Mutex<Settings>>
    pub keep_awake: Arc<crate::keepalive::KeepAwake>,
    pub midi: Arc<crate::midi::MidiState>,
    pub midi_out: Arc<crate::midi::MidiOutState>,

    // -- types that are already Arc<Inner>; wrapped again so HasState works --
    pub propresenter: Arc<crate::propresenter::ProPresenterState>,
    pub ndi: Arc<crate::ndi::NdiState>,
    pub relay: Arc<crate::relay::RelayState>,
    pub obs: Arc<crate::obs::ObsState>,
    pub audio: Arc<crate::audio::AudioState>,
    pub transcription: Arc<crate::transcription::TranscriptionState>,
    pub osc: Arc<crate::osc::OscState>,
    pub pco: Arc<crate::pco::PcoState>,
    pub web: Arc<crate::web::WebState>,
    pub tap: Arc<crate::tap::TapState>,
    pub chat: Arc<crate::chat::ChatState>,
    pub pages: Arc<crate::pages::PagesState>,
    pub push: Arc<crate::push::PushState>,
    pub checkin: Arc<crate::checkin::CheckinState>,
    pub posfiles: Arc<crate::posfiles::PosFilesState>,
    pub identity: Arc<crate::identity::IdentityState>,
    pub avantis: Arc<crate::avantis::AvantisState>,
    pub ga4: Arc<crate::ga4::Ga4State>,

    pub events_tx: EventSender,
}

// ---------------------------------------------------------------------------
// HasState impls
// ---------------------------------------------------------------------------

macro_rules! has_state {
    ($T:ty, $field:ident) => {
        impl HasState<$T> for AppState {
            fn get(this: &AppState) -> Arc<$T> {
                this.$field.clone()
            }
        }
    };
}

has_state!(crate::settings::SettingsState, settings);
has_state!(crate::keepalive::KeepAwake, keep_awake);
has_state!(crate::midi::MidiState, midi);
has_state!(crate::midi::MidiOutState, midi_out);

has_state!(crate::propresenter::ProPresenterState, propresenter);
has_state!(crate::ndi::NdiState, ndi);
has_state!(crate::relay::RelayState, relay);
has_state!(crate::obs::ObsState, obs);
has_state!(crate::audio::AudioState, audio);
has_state!(crate::transcription::TranscriptionState, transcription);
has_state!(crate::osc::OscState, osc);
has_state!(crate::pco::PcoState, pco);
has_state!(crate::web::WebState, web);
has_state!(crate::tap::TapState, tap);
has_state!(crate::chat::ChatState, chat);
has_state!(crate::pages::PagesState, pages);
has_state!(crate::push::PushState, push);
has_state!(crate::checkin::CheckinState, checkin);
has_state!(crate::posfiles::PosFilesState, posfiles);
has_state!(crate::identity::IdentityState, identity);
has_state!(crate::avantis::AvantisState, avantis);
has_state!(crate::ga4::Ga4State, ga4);
