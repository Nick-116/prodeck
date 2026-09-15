import type { Settings } from "./tauri";

/**
 * Shared first-run logic. Two places decide "is this a fresh install?" (the
 * onboarding gate and the open-on-Setup route); they must agree, and they
 * must not key on a field that has a non-empty default.
 *
 * `pp_host` defaults to "localhost", so testing it for emptiness never
 * detected a fresh install. `pp_auto_connect` is false until ProPresenter has
 * actually connected once, which is the signal we want.
 *
 * `pcoConnected` exists because a Planning Center OAuth sign-in leaves NOTHING
 * in settings — its tokens live in their own file, deliberately out of the
 * browser's reach — so `pco_app_id` can't see it. Callers that know the sign-in
 * state pass it; the default keeps the old behaviour for callers that don't.
 * Getting this wrong is not cosmetic: a booth that looks fresh forever reopens
 * the walkthrough on every launch, which is exactly the loop a Windows install
 * hit in 0.9.85.
 */
export function isFreshInstall(s: Settings, pcoConnected = false): boolean {
  return !s.pp_auto_connect && !s.pco_app_id && !pcoConnected && !s.web_enabled;
}

const DONE_KEY = "prodeck.setupDone";

export function readSetupDone(): boolean {
  try {
    return localStorage.getItem(DONE_KEY) === "1";
  } catch {
    return false;
  }
}

export function writeSetupDone(done: boolean) {
  try {
    if (done) localStorage.setItem(DONE_KEY, "1");
    else localStorage.removeItem(DONE_KEY);
  } catch {
    /* storage unavailable — the onboarding just shows again next launch */
  }
}

/** Imperative re-open, so "Re-run the walkthrough" actually re-runs it. */
export const ONBOARDING_EVENT = "prodeck:onboarding";
export function requestOnboarding() {
  writeSetupDone(false);
  window.dispatchEvent(new Event(ONBOARDING_EVENT));
}
