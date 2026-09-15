import { useCallback, useEffect, useRef, useState } from "react";
import {
  IS_WEB,
  IS_ADMIN_PANEL,
  on,
  getSettings,
  updateSettings,
  pcoOauthBegin,
  pcoOauthDisconnect,
  pcoOauthStatus,
  type PcoOauthStatus,
} from "../lib/tauri";

/**
 * Connecting ProDeck to Planning Center.
 *
 * Two routes, deliberately unequal in prominence. **Sign in** is the OAuth
 * flow (see src-tauri/src/pcoauth.rs): the operator clicks a button, approves
 * ProDeck on Planning Center's own page, and no password or long-lived secret
 * is ever typed into or stored by this app. **Use a token instead** is the
 * original Application ID + Secret pair — still supported, because every
 * install that predates this is running on one and because a church may not
 * have an Organization Administrator handy to register an application.
 *
 * Shared by the Planning Center page and the first-run walkthrough so the two
 * can't drift into describing different setups, which is how the old
 * eight-step token instructions ended up written out twice.
 */

export function PcoConnect({
  onConnected,
  compact = false,
}: {
  /** Called once a connection is verified, by either route. */
  onConnected: (who: string) => void | Promise<void>;
  /** Drop the surrounding card chrome — the walkthrough draws its own. */
  compact?: boolean;
}) {
  const [st, setSt] = useState<PcoOauthStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState("");
  const [waiting, setWaiting] = useState(false);
  const [showToken, setShowToken] = useState(false);
  const [clientId, setClientId] = useState("");
  const [appId, setAppId] = useState("");
  const [secret, setSecret] = useState("");
  const onConnectedRef = useRef(onConnected);
  onConnectedRef.current = onConnected;

  const refresh = useCallback(async () => {
    try {
      setSt(await pcoOauthStatus());
    } catch {
      // Desktop-only command. On a phone or kiosk it simply isn't there, and
      // the panel below falls back to explaining where to do this instead.
      setSt(null);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // The browser half of the flow finishes out of band: Planning Center
  // redirects to a loopback listener in the Rust side, which emits this.
  useEffect(() => {
    const p = on<{ ok: boolean; who?: string; error?: string }>("pco:oauth", (e) => {
      setWaiting(false);
      setBusy(false);
      if (e.ok) {
        setMsg("");
        void refresh();
        void onConnectedRef.current(e.who || "Connected");
      } else {
        setMsg(e.error || "Sign-in didn't complete.");
      }
    });
    return () => void p.then((f) => f());
  }, [refresh]);

  if (!IS_ADMIN_PANEL) {
    return (
      <p className="hint">
        Planning Center is connected at the booth computer, not from a browser —
        signing in needs ProDeck itself. Open ProDeck on the booth Mac or PC and
        use <strong>Planning Center → Connect</strong>.
      </p>
    );
  }

  async function connect() {
    setBusy(true);
    setMsg("");
    try {
      const { url } = await pcoOauthBegin();
      if (IS_WEB) {
        // Docker/browser mode: open PCO in a new tab; the server catches the callback.
        window.open(url, "_blank", "noopener");
      }
      setWaiting(true);
      setMsg("");
    } catch (e) {
      setBusy(false);
      setMsg(String(e));
    }
  }

  async function disconnect() {
    setBusy(true);
    try {
      await pcoOauthDisconnect();
      await refresh();
      setMsg("");
    } finally {
      setBusy(false);
    }
  }

  async function saveClientId() {
    setBusy(true);
    try {
      const s = await getSettings();
      await updateSettings({ ...s, pco_client_id: clientId.trim() });
      await refresh();
      setMsg("");
    } catch (e) {
      setMsg(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function saveToken() {
    setBusy(true);
    setMsg("Checking…");
    try {
      const s = await getSettings();
      await updateSettings({ ...s, pco_app_id: appId.trim(), pco_secret: secret.trim() });
      await onConnectedRef.current("Connected");
      setMsg("");
    } catch (e) {
      setMsg(String(e));
    } finally {
      setBusy(false);
    }
  }

  const body = (
    <>
      {st?.connected ? (
        <div className="pco-connected">
          <p className="ob-ok" style={{ marginTop: 0 }}>
            ✓ Connected to Planning Center{st.who ? ` as ${st.who}` : ""}.
          </p>
          <p className="hint">
            ProDeck can read your plans and your team, and nothing else. Revoke it
            any time from your Planning Center account, or here.
          </p>
          <button className="btn ghost small" disabled={busy} onClick={disconnect}>
            Disconnect
          </button>
        </div>
      ) : st?.configured ? (
        <>
          <p className="hint" style={{ marginTop: 0 }}>
            You'll sign in on Planning Center's own page and approve ProDeck for
            <strong> Services</strong> and <strong>People</strong>. Your password
            is never typed into ProDeck.
          </p>
          <button className="btn primary lg" disabled={busy} onClick={connect}>
            {waiting ? "Waiting for your browser…" : "Connect Planning Center"}
          </button>
          {waiting && (
            <p className="hint">
              A Planning Center tab should have opened. Finish there and this page
              will catch up on its own.
            </p>
          )}
        </>
      ) : (
        <RegisterApp
          st={st}
          clientId={clientId}
          setClientId={setClientId}
          busy={busy}
          onSave={saveClientId}
        />
      )}

      {msg && <p className={msg === "Checking…" ? "hint" : "error"}>{msg}</p>}

      {!st?.connected && (
        <details className="pco-token-fallback" open={showToken} onToggle={(e) => setShowToken((e.target as HTMLDetailsElement).open)}>
          <summary>Use a Personal Access Token instead</summary>
          <p className="hint">
            The older way in, and still fine: at{" "}
            <code>api.planningcenteronline.com</code> → Developers → Personal
            Access Tokens, create one and paste both halves. Note that a token
            carries that person's full Planning Center access and doesn't expire,
            so signing in above is the safer option where it's available.
          </p>
          <label className="field">
            <span>Application ID</span>
            <input className="input" autoComplete="off" value={appId} onChange={(e) => setAppId(e.target.value)} />
          </label>
          <label className="field">
            <span>Secret</span>
            <input className="input" type="password" autoComplete="off" value={secret} onChange={(e) => setSecret(e.target.value)} />
          </label>
          <button className="btn" disabled={busy || !appId.trim() || !secret.trim()} onClick={saveToken}>
            Save token
          </button>
        </details>
      )}
    </>
  );

  if (compact) return <div className="pco-connect">{body}</div>;
  return (
    <div className="card connect-card pco-connect">
      <div className="card-head">
        <h3>Connect Planning Center</h3>
      </div>
      {body}
    </div>
  );
}

/**
 * Shown when no OAuth application is configured — either ProDeck's own isn't
 * baked into this build, or the church chose to run their own. Registering one
 * is a five-minute job, but only an Organization Administrator can do it
 * (Planning Center's rule since March 2023), so say so up front rather than
 * letting someone get four steps in and hit a wall.
 */
function RegisterApp({
  st,
  clientId,
  setClientId,
  busy,
  onSave,
}: {
  st: PcoOauthStatus | null;
  clientId: string;
  setClientId: (v: string) => void;
  busy: boolean;
  onSave: () => void;
}) {
  const uris = st?.redirect_uris ?? [];
  return (
    <>
      <p className="hint" style={{ marginTop: 0 }}>
        To sign in, ProDeck needs a Planning Center <em>application</em> to sign
        in through. A <strong>Organization Administrator</strong> creates it once,
        in about five minutes:
      </p>
      <ol className="pco-register-steps">
        <li>
          Go to <code>api.planningcenteronline.com</code> → <strong>Developers</strong>{" "}
          → <strong>My Applications</strong> → <strong>New Application</strong>.
        </li>
        <li>
          Name it <strong>ProDeck</strong> and set the type to{" "}
          <strong>Public</strong>. A public application has no secret — that's
          correct and intended.
        </li>
        <li>
          Paste all three of these as its <strong>Redirect URIs</strong>:
          <pre className="pco-redirects">{uris.join("\n")}</pre>
        </li>
        <li>Save, then copy the <strong>Client ID</strong> it shows you.</li>
      </ol>
      <label className="field">
        <span>Client ID</span>
        <input
          className="input"
          autoComplete="off"
          placeholder="Paste the Client ID"
          value={clientId}
          onChange={(e) => setClientId(e.target.value)}
        />
      </label>
      <button className="btn primary" disabled={busy || !clientId.trim()} onClick={onSave}>
        Save
      </button>
    </>
  );
}
