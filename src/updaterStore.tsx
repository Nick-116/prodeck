import {
  createContext,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { IS_WEB, invoke } from "./lib/tauri";

// Tauri desktop imports — only used when IS_WEB is false.
// Dynamic imports prevent bundling errors in the web/Docker build.
type TauriUpdate = { version: string; body?: string; downloadAndInstall: (cb: (ev: unknown) => void) => Promise<void> };

export type UpdateStatus =
  | "idle"
  | "checking"
  | "uptodate"
  | "available"
  | "downloading"
  | "ready"
  | "error";

interface UpdaterCtx {
  version: string;
  status: UpdateStatus;
  newVersion: string | null;
  notes: string | null;
  progress: number; // 0..100 while downloading
  error: string | null;
  check: () => Promise<void>;
  install: () => Promise<void>;
  dismiss: () => void;
}

const DISMISS_KEY = "prodeck.updateDismissed";
const GITHUB_REPO = "Nick-116/prodeck";
const GITHUB_API = `https://api.github.com/repos/${GITHUB_REPO}/releases/latest`;

const Ctx = createContext<UpdaterCtx | null>(null);

// Compare semver strings: returns true if `latest` is newer than `current`.
function isNewer(current: string, latest: string): boolean {
  const parse = (v: string) =>
    v.replace(/^v/, "").split(".").map((n) => parseInt(n, 10) || 0);
  const [ca, cb, cc] = parse(current);
  const [la, lb, lc] = parse(latest);
  if (la !== ca) return la > ca;
  if (lb !== cb) return lb > cb;
  return lc > cc;
}

export function UpdaterProvider({ children }: { children: ReactNode }) {
  const [version, setVersion] = useState("");
  const [status, setStatus] = useState<UpdateStatus>("idle");
  const [newVersion, setNewVersion] = useState<string | null>(null);
  const [notes, setNotes] = useState<string | null>(null);
  const [progress, setProgress] = useState(0);
  const [error, setError] = useState<string | null>(null);
  // Only used in desktop (Tauri) mode.
  const tauriUpdateRef = useRef<TauriUpdate | null>(null);

  // ── Web / Docker mode: check GitHub releases API ──────────────────────────
  async function doCheckWeb(silent = false) {
    setStatus("checking");
    setError(null);
    try {
      const current = await invoke<string>("get_app_version").catch(() => "");
      const res = await fetch(GITHUB_API, {
        headers: { Accept: "application/vnd.github+json" },
      });
      if (res.status === 404) { setStatus("uptodate"); return; } // no releases yet
      if (!res.ok) throw new Error(`GitHub API ${res.status}`);
      const data = await res.json();
      const latest: string = (data.tag_name ?? "").replace(/^v/, "");
      const body: string = data.body ?? "";
      if (current && isNewer(current, latest)) {
        let dismissed = "";
        try { dismissed = localStorage.getItem(DISMISS_KEY) ?? ""; } catch { /* */ }
        setNewVersion(latest);
        setNotes(body || null);
        setStatus(!silent || dismissed !== latest ? "available" : "idle");
      } else {
        setStatus("uptodate");
      }
    } catch (e) {
      setError(String(e));
      setStatus("error");
    }
  }

  // ── Desktop (Tauri) mode: use plugin-updater ──────────────────────────────
  async function doCheckDesktop(silent = false) {
    setStatus("checking");
    setError(null);
    try {
      const { check } = await import("@tauri-apps/plugin-updater");
      const u = await check();
      if (u) {
        tauriUpdateRef.current = u as unknown as TauriUpdate;
        setNewVersion(u.version);
        setNotes(u.body ?? null);
        let dismissed = "";
        try { dismissed = localStorage.getItem(DISMISS_KEY) ?? ""; } catch { /* */ }
        setStatus(!silent || dismissed !== u.version ? "available" : "idle");
      } else {
        setStatus("uptodate");
      }
    } catch (e) {
      setError(String(e));
      setStatus("error");
    }
  }

  async function doCheck(silent = false) {
    return IS_WEB ? doCheckWeb(silent) : doCheckDesktop(silent);
  }

  // ── Install ───────────────────────────────────────────────────────────────
  async function install() {
    if (IS_WEB) {
      // Docker: user needs to rebuild the image. Show "ready" to trigger the
      // instructions banner — actual rebuild happens on the host, not here.
      setStatus("ready");
      return;
    }
    const u = tauriUpdateRef.current;
    if (!u) return;
    setStatus("downloading");
    setProgress(0);
    try {
      let total = 0;
      let got = 0;
      await u.downloadAndInstall((ev: unknown) => {
        const e = ev as { event: string; data?: { contentLength?: number; chunkLength?: number } };
        if (e.event === "Started") total = e.data?.contentLength ?? 0;
        else if (e.event === "Progress") {
          got += e.data?.chunkLength ?? 0;
          if (total) setProgress(Math.min(99, Math.round((got / total) * 100)));
        } else if (e.event === "Finished") setProgress(100);
      });
      setStatus("ready");
      const { relaunch } = await import("@tauri-apps/plugin-process");
      await relaunch();
    } catch (e) {
      setError(String(e));
      setStatus("error");
    }
  }

  function dismiss() {
    try {
      if (newVersion) localStorage.setItem(DISMISS_KEY, newVersion);
    } catch { /* storage unavailable */ }
    setStatus("idle");
  }

  useEffect(() => {
    if (IS_WEB) {
      // Get running version from the server, then schedule an update check.
      invoke<string>("get_app_version")
        .then((v) => setVersion(v))
        .catch(() => setVersion("unknown"));
      const t = setTimeout(() => doCheckWeb(true), 4000);
      return () => clearTimeout(t);
    } else {
      // Desktop: use Tauri's getVersion.
      import("@tauri-apps/api/app").then(({ getVersion }) =>
        getVersion().then(setVersion).catch(() => {})
      );
      const t = setTimeout(() => doCheckDesktop(true), 4000);
      return () => clearTimeout(t);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const value: UpdaterCtx = {
    version,
    status,
    newVersion,
    notes,
    progress,
    error,
    check: doCheck,
    install,
    dismiss,
  };
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useUpdater(): UpdaterCtx {
  const ctx = useContext(Ctx);
  if (!ctx) throw new Error("useUpdater must be used within UpdaterProvider");
  return ctx;
}
