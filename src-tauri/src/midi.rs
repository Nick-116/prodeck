use midir::{MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};
use std::sync::Mutex;
use crate::app::AppHandle;

pub struct MidiState(pub Mutex<Option<MidiInputConnection<()>>>);

impl MidiState {
    pub fn new() -> Self {
        Self(Mutex::new(None))
    }
}

// A persistent MIDI OUTPUT connection — used to push the song key to a backing-
// track / vocal-tune rig (e.g. a Program Change that recalls a per-key snapshot).
pub struct MidiOutState(pub Mutex<Option<MidiOutputConnection>>);

impl MidiOutState {
    pub fn new() -> Self {
        Self(Mutex::new(None))
    }
}

pub fn list_midi_outputs() -> Result<Vec<String>, String> {
    let midi_out = MidiOutput::new("ProDeck-out").map_err(|e| e.to_string())?;
    Ok(midi_out
        .ports()
        .iter()
        .filter_map(|p| midi_out.port_name(p).ok())
        .collect())
}

pub fn connect_midi_out(
    port_name: String,
    state: crate::app::State<MidiOutState>,
) -> Result<(), String> {
    let midi_out = MidiOutput::new("ProDeck-out").map_err(|e| e.to_string())?;
    let ports = midi_out.ports();
    let port = ports
        .iter()
        .find(|p| midi_out.port_name(p).map(|n| n == port_name).unwrap_or(false))
        .cloned()
        .ok_or_else(|| "MIDI output port not found".to_string())?;
    let conn = midi_out
        .connect(&port, "prodeck-out")
        .map_err(|e| e.to_string())?;
    *state.0.lock().unwrap_or_else(|p| p.into_inner()) = Some(conn);
    Ok(())
}

pub fn disconnect_midi_out(state: crate::app::State<MidiOutState>) {
    *state.0.lock().unwrap_or_else(|p| p.into_inner()) = None;
}

/// Send a key (pitch class 0–11) on the MIDI output as a Program Change, plus the
/// same value as a Control Change when cc_num is 0–127 (set cc_num to -1 to skip
/// the CC). Receivers map "program N" / "CC value N" to their per-key state.
pub fn midi_send_key(
    channel: u8,
    value: u8,
    cc_num: i32,
    state: crate::app::State<MidiOutState>,
) -> Result<(), String> {
    let mut guard = state.0.lock().unwrap_or_else(|p| p.into_inner());
    let conn = guard
        .as_mut()
        .ok_or_else(|| "No MIDI output connected".to_string())?;
    // The UI speaks MIDI channels 1–16; the status-byte nibble is 0-based. Using
    // the raw number as the nibble sent everything one channel high (and 16
    // wrapped to channel 1).
    let ch = channel.clamp(1, 16) - 1;
    let v = value & 0x7F;
    conn.send(&[0xC0 | ch, v]).map_err(|e| e.to_string())?; // Program Change
    if (0..=127).contains(&cc_num) {
        conn.send(&[0xB0 | ch, cc_num as u8, v])
            .map_err(|e| e.to_string())?; // Control Change
    }
    Ok(())
}

pub fn list_midi_inputs() -> Result<Vec<String>, String> {
    let midi_in = MidiInput::new("ProDeck").map_err(|e| e.to_string())?;
    Ok(midi_in
        .ports()
        .iter()
        .filter_map(|p| midi_in.port_name(p).ok())
        .collect())
}

pub fn connect_midi(
    port_name: String,
    state: crate::app::State<MidiState>,
    app: AppHandle,
) -> Result<(), String> {
    let midi_in = MidiInput::new("ProDeck-in").map_err(|e| e.to_string())?;
    let ports = midi_in.ports();
    let port = ports
        .iter()
        .find(|p| {
            midi_in
                .port_name(p)
                .map(|n| n == port_name)
                .unwrap_or(false)
        })
        .cloned()
        .ok_or_else(|| "MIDI port not found".to_string())?;

    let app2 = app.clone();
    let conn = midi_in
        .connect(
            &port,
            "prodeck-in",
            move |_stamp, message, _| {
                let status = message.first().copied().unwrap_or(0);
                let kind = match status & 0xF0 {
                    0x90 if message.get(2).copied().unwrap_or(0) > 0 => "note_on",
                    0x90 | 0x80 => "note_off",
                    0xB0 => "cc",
                    0xC0 => "program",
                    _ => "other",
                };
                app2.emit(
                    "midi:message",
                    serde_json::json!({
                        "kind": kind,
                        // 1-based, matching how the UI (and every MIDI device
                        // label) numbers channels.
                        "channel": (status & 0x0F) + 1,
                        "data1": message.get(1).copied().unwrap_or(0),
                        "data2": message.get(2).copied().unwrap_or(0),
                        "raw": message,
                    }),
                )
                .ok();
            },
            (),
        )
        .map_err(|e| e.to_string())?;

    *state.0.lock().unwrap_or_else(|p| p.into_inner()) = Some(conn);
    app.emit("midi:connected", port_name).ok();
    Ok(())
}

pub fn disconnect_midi(state: crate::app::State<MidiState>, app: AppHandle) {
    // Dropping the connection closes the port.
    *state.0.lock().unwrap_or_else(|p| p.into_inner()) = None;
    app.emit("midi:disconnected", ()).ok();
}

// ---------------------------------------------------------------- Core dispatch helpers
// The web dispatch table holds Arc<T> directly rather than going through
// crate::app::State, so these thin wrappers take the inner type by reference.

pub fn list_inputs_core() -> Vec<String> {
    list_midi_inputs().unwrap_or_default()
}

pub fn list_outputs_core() -> Vec<String> {
    list_midi_outputs().unwrap_or_default()
}

pub fn connect_midi_core(
    port_name: String,
    midi: &MidiState,
    app: AppHandle,
) -> Result<(), String> {
    let midi_in = MidiInput::new("ProDeck-in").map_err(|e| e.to_string())?;
    let ports = midi_in.ports();
    let port = ports
        .iter()
        .find(|p| {
            midi_in
                .port_name(p)
                .map(|n| n == port_name)
                .unwrap_or(false)
        })
        .cloned()
        .ok_or_else(|| "MIDI port not found".to_string())?;

    let app2 = app.clone();
    let conn = midi_in
        .connect(
            &port,
            "prodeck-in",
            move |_stamp, message, _| {
                let status = message.first().copied().unwrap_or(0);
                let kind = match status & 0xF0 {
                    0x90 if message.get(2).copied().unwrap_or(0) > 0 => "note_on",
                    0x90 | 0x80 => "note_off",
                    0xB0 => "cc",
                    0xC0 => "program",
                    _ => "other",
                };
                app2.emit(
                    "midi:message",
                    serde_json::json!({
                        "kind": kind,
                        "channel": (status & 0x0F) + 1,
                        "data1": message.get(1).copied().unwrap_or(0),
                        "data2": message.get(2).copied().unwrap_or(0),
                        "raw": message,
                    }),
                )
                .ok();
            },
            (),
        )
        .map_err(|e| e.to_string())?;

    *midi.0.lock().unwrap_or_else(|p| p.into_inner()) = Some(conn);
    app.emit("midi:connected", port_name).ok();
    Ok(())
}

pub fn disconnect_midi_core(midi: &MidiState) {
    *midi.0.lock().unwrap_or_else(|p| p.into_inner()) = None;
}

pub fn connect_midi_out_core(
    port_name: String,
    midi_out: &MidiOutState,
) -> Result<(), String> {
    let mo = MidiOutput::new("ProDeck-out").map_err(|e| e.to_string())?;
    let ports = mo.ports();
    let port = ports
        .iter()
        .find(|p| mo.port_name(p).map(|n| n == port_name).unwrap_or(false))
        .cloned()
        .ok_or_else(|| "MIDI output port not found".to_string())?;
    let conn = mo
        .connect(&port, "prodeck-out")
        .map_err(|e| e.to_string())?;
    *midi_out.0.lock().unwrap_or_else(|p| p.into_inner()) = Some(conn);
    Ok(())
}

pub fn disconnect_midi_out_core(midi_out: &MidiOutState) {
    *midi_out.0.lock().unwrap_or_else(|p| p.into_inner()) = None;
}

/// Send a key via an `&MidiOutState` reference (dispatch-table variant).
pub fn midi_send_key_core(
    channel: u8,
    value: u8,
    cc_num: i32,
    midi_out: &MidiOutState,
) -> Result<(), String> {
    let mut guard = midi_out.0.lock().unwrap_or_else(|p| p.into_inner());
    let conn = guard
        .as_mut()
        .ok_or_else(|| "No MIDI output connected".to_string())?;
    let ch = channel.clamp(1, 16) - 1;
    let v = value & 0x7F;
    conn.send(&[0xC0 | ch, v]).map_err(|e| e.to_string())?;
    if (0..=127).contains(&cc_num) {
        conn.send(&[0xB0 | ch, cc_num as u8, v])
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
