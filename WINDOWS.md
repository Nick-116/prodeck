# ProDeck for Windows

This branch is the Windows validation track for ProDeck. The first target is a
Windows 11 booth computer using these four features:

- Allen & Heath Avantis mirroring and control over MIDI-TCP
- RTA and SPL metering from a Windows audio input, including Dante Virtual
  Soundcard
- ProPresenter 7 through its network API
- Planning Center Services through its API

The frontend and most of the backend are platform-neutral. Windows-specific
implementations already exist for start-at-login, sleep prevention, NDI runtime
loading, diagnostic commands, file URLs, and opening printable reports. They
must be compiled and tested on real Windows hardware before this build should
be trusted during a service.

## Install

Take `ProDeck_<version>_x64-setup.exe` from the
[latest release](https://github.com/whiteoakmedia/prodeck/releases/latest) and
run it. It installs for the current user, so there is no admin prompt. The
installer is not Authenticode-signed, so Windows SmartScreen will warn once —
**More info → Run anyway**.

For a build of an unreleased commit, open the repository's **Actions** tab, run
**Windows build**, and download the `ProDeck-Windows-x64` artifact instead.

## Updates

Windows installs update themselves, the same way the Mac build does. At launch
ProDeck reads
`releases/latest/download/latest.json`; if its `windows-x86_64` entry names a
newer version, the banner offers it and installing runs the new setup silently
(NSIS passive mode, current-user, no admin prompt).

The installer in that feed is signed with the project's minisign updater key,
and ProDeck refuses any download the key doesn't vouch for. That key only lives
on the release Mac, so the flow is: `scripts/release-public.sh` dispatches the
**Windows build** workflow for the release commit, waits for it, downloads that
exact `.exe`, signs it, and attaches it to the release. CI deliberately does
*not* attach an installer of its own — a second build would produce different
bytes, and every Windows update would then fail signature verification.

Consequence worth knowing: a Windows copy installed from an Actions artifact,
or from a release published before this existed, has no matching feed entry for
its build and simply reports "up to date" until the next release. Download once
more from the Releases page and it is on the update channel from then on.

## Build locally

Use Windows 11 x64 and install:

1. Microsoft C++ Build Tools with **Desktop development with C++** selected
2. Rust using the stable MSVC toolchain
3. Node.js 22 LTS
4. Microsoft Edge WebView2 Runtime, if it is not already installed

Then run PowerShell in the repository:

```powershell
.\scripts\build-windows.ps1
```

Or run the same steps manually:

```powershell
npm ci
npm test
npx tsc --noEmit
Push-Location src-tauri
cargo test --locked
Pop-Location
npm run tauri build -- --bundles nsis --config src-tauri/tauri.windows.conf.json
```

The installer is written under
`src-tauri\target\release\bundle\nsis\`.

## First hardware test

1. Start ProDeck and use Demo Mode. Confirm dashboards render and navigation is
   stable.
2. Connect Planning Center and load a current plan.
3. Enable the ProPresenter network API, connect ProDeck, and verify status,
   thumbnails, next/previous, and clears.
4. Give the Avantis a fixed address. In ProDeck select Avantis, enter its IP,
   TCP port `51325`, and the matching base MIDI channel. Verify mutes, faders,
   scenes, names, and one harmless control action.
5. Select the Dante Virtual Soundcard or local audio input. Verify input channel
   selection, RTA activity, SPL calibration, and ten minutes of uninterrupted
   capture.
6. Enable **Settings > Reliability > Keep awake** and verify the request with
   `powercfg /requests`.
7. Allow ProDeck through Windows Defender Firewall on private networks when
   prompted so phones and kiosks can reach its web gateway.

Do not use console control during a live service until the read-only mirror has
run through at least one rehearsal without disconnects or incorrect state.
