import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { Channel, convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import Markdown from "react-markdown";
import brand from "./brand.png";
import aurora from "./aurora.jpg";
import "./styles.css";

type Wallpaper = { id: string; name: string; extension: string };
type Monitor = { id: string; name: string; width: number; height: number; x: number; y: number; primary: boolean; fullscreen: boolean };
type Settings = { fullscreenPause: boolean; batteryPause: boolean; autostart: boolean; fps: 15 | 30 | null };
type Status = { monitors: Monitor[]; onBattery: boolean; manualPause: boolean; errors: Record<string, string>; pauseReasons: Record<string, string>; desktopError: string | null; hasBattery?: boolean | null; readyWallpapers?: Record<string, string> };
type Snapshot = { library: { wallpapers: Wallpaper[]; assignments: Record<string, string>; displayNames: Record<string, string>; settings: Settings }; status: Status; previews: Record<string, string> };
type UpdateStatus = { installedVersion: string; availableVersion: string | null; releaseNotes: string | null; automaticChecks: boolean; systemNotifications: boolean; checking: boolean; installing: boolean; installable: boolean };
type UpdateProgress = { phase: "preparing" | "verifying" | "installing" }
  | { phase: "downloading"; downloaded: number; total: number | null };
type UpdateOption = "automaticChecks" | "systemNotifications";
type ImportResult = { file: string; error: string | null };
type Page = "Bibliothek" | "Monitore" | "Einstellungen";
type IconName = "library" | "monitor" | "settings" | "plus" | "upload" | "edit" | "trash" | "play" | "close" | "check" | "info" | "search" | "image" | "power" | "download";
type Modal = { kind: "assign" | "rename" | "delete" | "preview"; item: Wallpaper }
  | { kind: "monitor"; monitor: Monitor }
  | { kind: "update"; version: string; notes: string | null };
type DisplayState = { tone: "active" | "paused" | "error" | "idle"; label: string; detail?: string };

function displayState(status: Status, id: string, wallpaperId: string | undefined, pending: string | null): DisplayState {
  if (id === pending) return { tone: "idle", label: "Wird angewendet …" };
  if (!wallpaperId) return { tone: "idle", label: "Systemhintergrund" };
  const error = status.errors[id] || status.desktopError;
  if (error) return { tone: "error", label: "Wiedergabefehler", detail: error };
  if (status.pauseReasons[id]) return { tone: "paused", label: "Pausiert", detail: status.pauseReasons[id] };
  if (status.readyWallpapers?.[id] === wallpaperId) return { tone: "active", label: "Aktiv" };
  return { tone: "idle", label: "Zugewiesen", detail: "Wiedergabestatus nicht bestätigt" };
}

const glyphs: Record<IconName, ReactNode> = {
  library: <><rect x="3" y="3" width="7" height="7" rx="1.3"/><rect x="14" y="3" width="7" height="7" rx="1.3"/><rect x="3" y="14" width="7" height="7" rx="1.3"/><rect x="14" y="14" width="7" height="7" rx="1.3"/></>,
  monitor: <><rect x="3" y="4" width="18" height="13" rx="2"/><path d="M8 21h8m-4-4v4"/></>,
  settings: <><path d="M4 7h16M4 12h16M4 17h16"/><circle cx="9" cy="7" r="2"/><circle cx="15" cy="12" r="2"/><circle cx="9" cy="17" r="2"/></>,
  plus: <path d="M12 5v14M5 12h14"/>,
  upload: <><path d="M12 16V4m-5 5 5-5 5 5M4 16v3a1 1 0 0 0 1 1h14a1 1 0 0 0 1-1v-3"/></>,
  edit: <path d="m15.5 5.5 3 3M4 20l4-.8 11-11a2.1 2.1 0 1 0-3-3l-11 11L4 20Z"/>,
  trash: <path d="M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13m-8 4v5m4-5v5"/>,
  play: <path d="m9 5 11 7-11 7Z"/>,
  close: <path d="m6 6 12 12M6 18 18 6"/>,
  check: <path d="m5 12 4 4L19 6"/>,
  info: <><circle cx="12" cy="12" r="9"/><path d="M12 11v5m0-8v.1"/></>,
  search: <><circle cx="10.5" cy="10.5" r="5.5"/><path d="m15 15 4 4"/></>,
  image: <><rect x="3" y="4" width="18" height="16" rx="2"/><circle cx="8" cy="9" r="1.4"/><path d="m4 17 5-5 3 3 3-4 5 6"/></>,
  power: <path d="M12 3v9m-5-7a8 8 0 1 0 10 0"/>,
  download: <path d="M12 3v12m-5-5 5 5 5-5M4 16v3a1 1 0 0 0 1 1h14a1 1 0 0 0 1-1v-3"/>,
};

function Icon({ name, size = 20 }: { name: IconName; size?: number }) {
  return <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">{glyphs[name]}</svg>;
}

function errorText(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

const navigation: [Page, IconName][] = [["Bibliothek", "library"], ["Monitore", "monitor"], ["Einstellungen", "settings"]];

const reducedMotion = "(prefers-reduced-motion: reduce)";

function consumeStartupIntro(): boolean {
  const launchId = (window as Window & { __JUNADESK_STARTUP__?: string }).__JUNADESK_STARTUP__;
  if (!launchId) return false;
  try {
    if (sessionStorage.getItem("junadesk/startup") === launchId) return false;
    sessionStorage.setItem("junadesk/startup", launchId);
  } catch {
    // The current document still has a single React root.
  }
  return !window.matchMedia(reducedMotion).matches;
}

const showStartupIntro = consumeStartupIntro();

function App() {
  const [page, setPage] = useState<Page>("Bibliothek");
  const [snapshot, setSnapshot] = useState<Snapshot>();
  const [updates, setUpdates] = useState<UpdateStatus>();
  const [updateFeedback, setUpdateFeedback] = useState<{ text: string; error?: boolean }>();
  const [updateBusy, setUpdateBusy] = useState(false);
  const [updateProgress, setUpdateProgress] = useState<UpdateProgress>();
  const [startupError, setStartupError] = useState("");
  const [notice, setNotice] = useState<{ text: string; error?: boolean }>();
  const [modal, setModal] = useState<Modal>();
  const [dragging, setDragging] = useState(false);
  const [query, setQuery] = useState("");
  const [sortBy, setSortBy] = useState<"recent" | "name">("recent");
  const [pendingDisplay, setPendingDisplay] = useState<string | null>(null);
  const [intro, setIntro] = useState(showStartupIntro);
  const [introRunning, setIntroRunning] = useState(false);
  const workspace = useRef<HTMLElement>(null);
  const pageTitle = useRef<HTMLHeadingElement>(null);
  const modalTrigger = useRef<HTMLElement | null>(null);
  const [busy, setBusy] = useState<string>();
  const activeOperation = useRef(false);
  const refreshNumber = useRef(0);
  const updateRefreshNumber = useRef(0);
  const updatesPanel = useRef<HTMLElement>(null);
  const scrollToUpdates = useRef(false);

  useLayoutEffect(() => {
    if (!intro) return;
    const stop = () => setIntro(false);
    let started = false;
    const visibility = () => {
      if (document.hidden) {
        if (started) stop();
      } else if (!started) {
        started = true;
        setIntroRunning(true);
      }
    };
    const motion = window.matchMedia(reducedMotion);
    visibility();
    window.addEventListener("pointerdown", stop, true);
    window.addEventListener("keydown", stop, true);
    document.addEventListener("visibilitychange", visibility);
    motion.addEventListener("change", stop);
    return () => {
      window.removeEventListener("pointerdown", stop, true);
      window.removeEventListener("keydown", stop, true);
      document.removeEventListener("visibilitychange", visibility);
      motion.removeEventListener("change", stop);
    };
  }, [intro]);

  const navigate = (next: Page) => {
    if (next !== page) workspace.current?.scrollTo(0, 0);
    setPage(next);
  };

  const refreshUpdates = useCallback(async () => {
    const request = ++updateRefreshNumber.current;
    try {
      const next = await invoke<UpdateStatus>("get_update_status");
      if (request === updateRefreshNumber.current) setUpdates(next);
    } catch (error) {
      if (request === updateRefreshNumber.current) setUpdateFeedback({ text: errorText(error), error: true });
    }
  }, []);

  useEffect(() => {
    if (page === "Einstellungen" && scrollToUpdates.current && updatesPanel.current) {
      scrollToUpdates.current = false;
      updatesPanel.current.scrollIntoView({ block: "start" });
      updatesPanel.current.focus({ preventScroll: true });
    }
  }, [page, snapshot]);

  const refresh = useCallback(async () => {
    const version = ++refreshNumber.current;
    try {
      const next = await invoke<Snapshot>("get_snapshot");
      if (version === refreshNumber.current) {
        setSnapshot(next);
        setStartupError("");
      }
    } catch (error) {
      if (version === refreshNumber.current) setStartupError(errorText(error));
    }
  }, []);

  const run = useCallback(async (label: string, task: () => Promise<void>) => {
    if (activeOperation.current) return false;
    activeOperation.current = true;
    setBusy(label);
    setNotice(undefined);
    try {
      await task();
      await refresh();
      return true;
    } catch (error) {
      setNotice({ text: errorText(error), error: true });
      return false;
    } finally {
      activeOperation.current = false;
      setBusy(undefined);
    }
  }, [refresh]);

  const importPaths = useCallback(async (paths: string[]) => {
    if (!paths.length) return;
    if (activeOperation.current) {
      setNotice({ text: "Bitte warte, bis der laufende Vorgang abgeschlossen ist.", error: true });
      return;
    }
    setPage("Bibliothek");
    setQuery("");
    await run("Wallpaper werden importiert …", async () => {
      const supported = paths.filter((path) => /\.(mp4|webm|gif)$/i.test(path));
      const results = supported.length ? await invoke<ImportResult[]>("import_wallpapers", { paths: supported }) : [];
      for (const path of paths) {
        if (!/\.(mp4|webm|gif)$/i.test(path)) {
          results.push({ file: path.split(/[/\\]/).at(-1) || path, error: "Unterstützt werden MP4, WebM und GIF." });
        }
      }
      const errors = results.filter((item) => item.error !== null);
      const imported = results.length - errors.length;
      setNotice({
        text: [
          imported === 1 ? "1 Wallpaper importiert." : imported > 1 ? `${imported} Wallpaper importiert.` : "",
          ...errors.map(({ file, error }) => `${file} – ${error}`),
        ].filter(Boolean).join("\n"),
        error: errors.length > 0,
      });
    });
  }, [run]);

  useEffect(() => {
    let alive = true;
    const subscriptions: UnlistenFn[] = [];
    const register = async (promise: Promise<UnlistenFn>) => {
      const unlisten = await promise;
      if (alive) subscriptions.push(unlisten);
      else unlisten();
    };
    void Promise.all([
      register(listen("library-changed", () => { void refresh(); })),
      register(listen<Status>("desktop-changed", () => { void refresh(); })),
      register(listen("updates-changed", () => { void refreshUpdates(); })),
      register(listen("show-update-settings", () => {
        if (updatesPanel.current) {
          updatesPanel.current.scrollIntoView({ block: "start" });
          updatesPanel.current.focus({ preventScroll: true });
        } else {
          scrollToUpdates.current = true;
          setPage("Einstellungen");
        }
      })),
      register(getCurrentWebview().onDragDropEvent(({ payload }) => {
        setDragging(payload.type === "over" || payload.type === "enter");
        if (payload.type === "drop") void importPaths(payload.paths);
      })),
    ]).then(() => {
      if (alive) {
        void refresh();
        void refreshUpdates();
      }
    })
      .catch((error: unknown) => { if (alive) setStartupError(errorText(error)); });
    return () => {
      alive = false;
      subscriptions.forEach((unlisten) => unlisten());
    };
  }, [importPaths, refresh, refreshUpdates]);

  async function changeUpdateSettings(option?: UpdateOption, enabled?: boolean) {
    setUpdateBusy(true);
    setUpdateFeedback(undefined);
    try {
      const result = option
        ? await invoke<UpdateStatus>("set_update_option", { option, enabled })
        : await invoke<UpdateStatus>("check_updates");
      await refreshUpdates();
      if (!option && !result.checking) {
        setUpdateFeedback({ text: result.availableVersion ? "Eine neue Version ist verfügbar." : "Kein Update verfügbar." });
      }
    } catch (error) {
      setUpdateFeedback({ text: errorText(error), error: true });
    } finally {
      setUpdateBusy(false);
    }
  }

  async function openUpdateRelease() {
    try {
      await invoke("open_update_release");
    } catch (error) {
      setNotice({ text: errorText(error), error: true });
    }
  }

  async function installUpdate(version: string) {
    if (updates?.availableVersion !== version || !updates.installable || updateProgress || updates.installing || updates.checking) return;
    setNotice(undefined);
    setUpdateFeedback(undefined);
    setUpdateProgress({ phase: "preparing" });
    const progress = new Channel<UpdateProgress>(setUpdateProgress);
    try {
      await invoke("install_update", { version, progress });
    } catch (error) {
      setNotice({ text: errorText(error), error: true });
      setUpdateProgress(undefined);
      await refreshUpdates();
    }
  }

  async function chooseFiles() {
    try {
      const paths = await open({
        title: "Wallpaper hinzufügen", multiple: true,
        filters: [{ name: "Animierte Wallpaper", extensions: ["mp4", "webm", "gif"] }],
      });
      if (Array.isArray(paths)) await importPaths(paths);
    } catch (error) {
      setNotice({ text: errorText(error), error: true });
    }
  }

  const library = snapshot?.library;
  const status = snapshot?.status;
  const items = library?.wallpapers ?? [];
  const monitors = status?.monitors ?? [];
  const attached = monitors.filter(({ id }) => Boolean(library?.assignments[id]));
  const states = new Map(status && library ? monitors.map(({ id }) =>
    [id, displayState(status, id, library.assignments[id], pendingDisplay)] as const) : []);
  const activeCount = attached.filter(({ id }) => states.get(id)?.tone === "active").length;
  const hasErrors = attached.some(({ id }) => states.get(id)?.tone === "error");
  const desktopStatus = !status || status.desktopError || !monitors.length ? "Status nicht verfügbar"
    : hasErrors ? "Wiedergabefehler"
    : !attached.length ? "Systemhintergrund"
    : attached.every(({ id }) => states.get(id)?.tone === "paused") ? "Alle Wallpaper pausiert"
    : activeCount ? `${activeCount} ${activeCount === 1 ? "aktiver Bildschirm" : "aktive Bildschirme"}`
    : `${attached.length} ${attached.length === 1 ? "Zuweisung" : "Zuweisungen"}`;
  const filter = query.trim().toLocaleLowerCase("de");
  const offeredVersion = updates?.availableVersion;
  const visibleItems = items.filter(({ name, extension }) =>
    name.toLocaleLowerCase("de").includes(filter) || extension.includes(filter)
  );
  if (sortBy === "name") visibleItems.sort((a, b) => a.name.localeCompare(b.name, "de", { numeric: true }));
  else visibleItems.reverse();

  const previewFor = (id: string) => {
    const path = snapshot?.previews[id];
    return path ? convertFileSrc(path) : undefined;
  };
  const showModal = (next: Modal) => {
    if (!modal) modalTrigger.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    setNotice(undefined);
    setModal(next);
  };
  const closeModal = () => {
    setNotice(undefined);
    setModal(undefined);
    const trigger = modalTrigger.current;
    requestAnimationFrame(() => (trigger?.isConnected ? trigger : pageTitle.current)?.focus({ preventScroll: true }));
  };

  async function applyWallpaper(monitorId: string, wallpaperId: string | null) {
    setPendingDisplay(monitorId);
    try {
      if (await run("Wallpaper wird angewendet …", () => invoke("assign_wallpaper", { monitorId, wallpaperId }))) {
        closeModal();
        setNotice({ text: wallpaperId ? "Wallpaper-Zuweisung gespeichert." : "Systemhintergrund ausgewählt." });
      }
    } finally {
      setPendingDisplay(null);
    }
  }
  const settingRows: [keyof Omit<Settings, "fps">, string, string][] = [
    ["fullscreenPause", "Bei Vollbild pausieren", "Pausiert das Wallpaper auf dem Bildschirm mit der Vollbildanwendung."],
    ["batteryPause", "Bei Akkubetrieb pausieren", "Pausiert alle Wallpaper, solange der Computer im Akkubetrieb ist."],
    ["autostart", "Bei der Anmeldung starten", "Stellt deine Wallpaper nach der Systemanmeldung im Hintergrund wieder her."],
  ];

  return <div className="layout">
    <aside className="sidebar">
      <div className="brand"><img src={brand} alt="" /><span>JunaDesk</span></div>
      <nav aria-label="Hauptnavigation">
        {navigation.map(([name, icon]) => <button key={name} className={page === name ? "nav selected" : "nav"}
          aria-current={page === name ? "page" : undefined} onClick={() => navigate(name)}>
          <Icon name={icon} size={19} /><span>{name}</span>
        </button>)}
      </nav>
      <button className="sidebar-footer" onClick={() => navigate("Monitore")} aria-label={`Monitorstatus anzeigen · ${desktopStatus}`}>
        <span className="status-icon"><Icon name="monitor" size={17} /></span>
        <span className="sidebar-status"><strong>Desktop-Status</strong><small>{desktopStatus}</small></span>
        <span className={hasErrors ? "dot error-dot" : activeCount ? "dot active" : "dot"} />
      </button>
    </aside>
    <main className="workspace" ref={workspace}>
      <div className="view" key={page}>
      <header className="page-header">
        <div>
          <h1 ref={pageTitle} tabIndex={-1}>{page === "Bibliothek" ? "Meine Wallpaper" : page}</h1>
          {page === "Bibliothek" && items.length > 0 && <p>{items.length} Wallpaper</p>}
          {page === "Monitore" && snapshot && <p>{monitors.length} {monitors.length === 1 ? "verbundener Bildschirm" : "verbundene Bildschirme"}</p>}
          {page === "Einstellungen" && <p>Wiedergabe und Systemintegration</p>}
        </div>
        {page === "Bibliothek" && items.length > 0 && <button className="button primary header-import" disabled={!!busy}
          onClick={() => void chooseFiles()}><Icon name="plus" size={18} />Hinzufügen</button>}
      </header>

      {updates && offeredVersion && <div className="banner update-banner">
        <Icon name="download" />
        <strong role="status">JunaDesk {offeredVersion} ist verfügbar</strong>
        {updates.installable && <button className="button primary"
          disabled={!!busy || updateBusy || updates.checking || updates.installing || !!updateProgress}
          onClick={() => showModal({ kind: "update", version: offeredVersion, notes: updates.releaseNotes })}>Jetzt aktualisieren</button>}
        <button className="button secondary" onClick={() => void openUpdateRelease()}>Auf GitHub ansehen</button>
        {(updateProgress || updates.installing) && <UpdateInstallationProgress progress={updateProgress} />}
      </div>}
      {startupError && <div className="banner error" role="alert"><Icon name="info" /><span>{startupError}</span>
        <button className="button secondary" onClick={() => void refresh()}>Erneut versuchen</button></div>}
      {status?.desktopError && <div className="banner error" role="status"><Icon name="info" />{status.desktopError}</div>}
      {notice && !modal && <div className={notice.error ? "banner error" : "banner success"} role={notice.error ? "alert" : "status"}>
        <Icon name={notice.error ? "info" : "check"} /><span>{notice.text}</span>
        <button className="icon-button" aria-label="Meldung schließen" onClick={() => setNotice(undefined)}><Icon name="close" size={17} /></button>
      </div>}
      {busy && <div className="banner pending" role="status"><span className="spinner" />{busy}</div>}

      {!snapshot && !startupError && <div className="loading-panel" role="status">
        <span className="loading-line" aria-hidden="true" /><span className="loading-line" aria-hidden="true" />
        {page === "Monitore" ? "Bildschirme werden geladen …" : `${page} wird geladen …`}
      </div>}
      {snapshot && page === "Bibliothek" && (items.length === 0 ?
        <section className="empty-panel">
          <div className="empty-mark" aria-hidden="true"><img src={aurora} alt="" /></div>
          <h2>Dein erstes Wallpaper</h2>
          <p>Importiere ein Video oder GIF und nutze es als animierten Desktophintergrund.</p>
          <button className="button primary" disabled={!!busy} onClick={() => void chooseFiles()}>
            <Icon name="plus" size={18} />Dateien auswählen
          </button>
          <small>Oder Dateien hierher ziehen · MP4, WebM und GIF</small>
        </section>
        : <section className="collection" aria-label="Wallpaper-Bibliothek">
          <div className="collection-header">
            {items.length > 1 && <div className="search">
              <Icon name="search" size={17} />
              <input aria-label="Wallpaper suchen" type="search" placeholder="Wallpaper suchen" value={query} onChange={(event) => setQuery(event.target.value)} />
              {query && <button className="icon-button" aria-label="Suche leeren" onClick={() => setQuery("")}><Icon name="close" size={15} /></button>}
            </div>}
            {items.length > 1 && <label className="sort-control"><span className="sr-only">Wallpaper sortieren</span>
              <select value={sortBy} onChange={(event) => setSortBy(event.target.value === "name" ? "name" : "recent")}>
                <option value="recent">Zuletzt hinzugefügt</option><option value="name">Nach Name</option>
              </select>
            </label>}
            {filter && <small className="result-count" role="status">{visibleItems.length} Treffer</small>}
          </div>
          {!visibleItems.length && <section className="empty-panel no-results">
            <h2>Keine passenden Wallpaper gefunden</h2>
            <button className="button secondary" onClick={() => setQuery("")}>Suche zurücksetzen</button>
          </section>}
          <div className="cards">
            {visibleItems.map((item, index) => {
              const displays = attached.filter(({ id }) => library?.assignments[id] === item.id);
              const active = displays.filter(({ id }) => states.get(id)?.tone === "active").length;
              const hint = active > 0 ? `Auf ${active} ${active === 1 ? "Bildschirm" : "Bildschirmen"} aktiv`
                : displays.some(({ id }) => states.get(id)?.tone === "error") ? "Wiedergabefehler"
                : displays.length && displays.every(({ id }) => states.get(id)?.tone === "paused") ? "Pausiert"
                : displays.length ? `Zugewiesen · ${displays.length}` : undefined;
              return <article className={active ? "card is-assigned" : "card"} data-enter={index < 8 || undefined} key={item.id}>
                <div className="preview"><Poster src={previewFor(item.id)} />
                  {hint && <span className={`status-badge ${active ? "active" : ""}`}>{hint}</span>}
                </div>
                <div className="card-body">
                  <div className="card-title"><h3 title={item.name}>{item.name}</h3><small>{item.extension.toUpperCase()}</small></div>
                  <div className="card-actions">
                    <button className="button secondary" aria-label={`Vorschau · ${item.name}`} onClick={() => showModal({ kind: "preview", item })}>Vorschau</button>
                    <button className="button primary" disabled={!!busy || !monitors.length} aria-label={`Anwenden · ${item.name}`}
                      onClick={() => showModal({ kind: "assign", item })}>Anwenden</button>
                  </div>
                </div>
              </article>;
            })}
          </div>
        </section>)}

      {snapshot && page === "Monitore" && <>
        {monitors.length === 0 && <div className="empty-panel"><Icon name="monitor" size={24} /><h2>Keine Bildschirme verfügbar</h2>
          <p>Verbinde einen Bildschirm oder prüfe die Anzeigeeinstellungen.</p></div>}
        {monitors.length > 0 && items.length === 0 &&
          <div className="banner"><span>Importiere ein Wallpaper, um es einem Bildschirm zuzuweisen.</span>
            <button className="button secondary" disabled={!!busy} onClick={() => void chooseFiles()}>Wallpaper hinzufügen</button></div>}
        <section className={monitors.length === 1 ? "monitor-grid single" : "monitor-grid"} aria-label="Monitorzuweisungen">
          {monitors.map((monitor) => {
            const assignment = library?.assignments[monitor.id];
            const item = items.find(({ id }) => id === assignment);
            const state = states.get(monitor.id);
            return <article className="monitor-card" key={monitor.id}>
              <MonitorPoster assigned={!!assignment} src={assignment ? previewFor(assignment) : undefined} />
              <div className="monitor-name"><h2>{monitor.name}</h2>
                <p>{monitor.width} × {monitor.height}{monitor.primary && " · Hauptbildschirm"}</p></div>
              {item && <p className="assigned-name">{item.name}</p>}
              <div className="monitor-actions">
                {state && <span className={`status-badge ${state.tone}`} role="status">{state.label}</span>}
                <button className="button secondary" disabled={!!busy || (!items.length && !assignment)}
                  aria-label={`Wallpaper ändern · ${monitor.name}`} onClick={() => showModal({ kind: "monitor", monitor })}>Wallpaper ändern</button>
              </div>
              {state?.detail && <p className={`monitor-detail ${state.tone}`}>{state.detail}</p>}
            </article>;
          })}
        </section>
      </>}

      {snapshot && page === "Einstellungen" && <div className="settings">
        <section className="settings-panel">
          <h2><span className="settings-icon"><Icon name="play" size={18} /></span>Wiedergabe</h2>
          {settingRows.slice(0, status?.hasBattery === false ? 1 : 2).map(([name, title, description]) => <Setting key={name} title={title} description={description}
            checked={snapshot.library.settings[name]} disabled={!!busy}
            onChange={(enabled) => void run("Einstellung wird gespeichert …", () => invoke("set_option", { option: name, enabled }))} />)}
          <div className="setting-row">
            <label htmlFor="fps"><strong>Bildrate begrenzen</strong><small>Je nach Medium muss JunaDesk eine neue Wiedergabedatei erzeugen.</small></label>
            <select id="fps" disabled={!!busy} value={snapshot.library.settings.fps ?? ""} onChange={(event) => {
              const fps = event.target.value ? Number(event.target.value) : null;
              void run("Bildrate wird vorbereitet …", () => invoke("set_fps", { fps }));
            }}>
              <option value="">Kein Limit</option><option value="30">30 FPS</option><option value="15">15 FPS</option>
            </select>
          </div>
        </section>
        <section className="settings-panel">
          <h2><span className="settings-icon"><Icon name="power" size={18} /></span>System</h2>
          <Setting title={settingRows[2][1]} description={settingRows[2][2]} checked={snapshot.library.settings.autostart}
            disabled={!!busy} onChange={(enabled) => void run("Autostart wird gespeichert …", () => invoke("set_option", { option: "autostart", enabled }))} />
        </section>
        <section ref={updatesPanel} className="settings-panel" tabIndex={-1} aria-labelledby="updates-heading">
          <h2 id="updates-heading"><span className="settings-icon"><Icon name="download" size={18} /></span>Updates</h2>
          <div className="setting-row update-version">
            <span><strong>Installierte Version</strong><small>{updates?.installedVersion ?? "Wird geladen …"}</small></span>
            <button className="button secondary" disabled={!updates || updateBusy || updates.checking || updates.installing || !!updateProgress}
              onClick={() => void changeUpdateSettings()}>{updates?.checking ? "Wird geprüft …" : "Jetzt prüfen"}</button>
          </div>
          {updateFeedback && <div className={updateFeedback.error ? "banner error" : "banner success"} role={updateFeedback.error ? "alert" : "status"}>
            <Icon name={updateFeedback.error ? "info" : "check"} />{updateFeedback.text}
          </div>}
          <Setting title="Automatisch nach Updates suchen" description="JunaDesk prüft einmal täglich auf GitHub, ob eine stabile Version vorliegt."
            checked={updates?.automaticChecks ?? false} disabled={!updates || updateBusy || updates.installing}
            onChange={(enabled) => void changeUpdateSettings("automaticChecks", enabled)} />
          <Setting title="Systembenachrichtigungen" description="Ein Hinweis pro neuer Version, auch bei geschlossenem JunaDesk-Fenster."
            checked={updates?.systemNotifications ?? false} disabled={!updates || updateBusy || updates.installing}
            onChange={(enabled) => void changeUpdateSettings("systemNotifications", enabled)} />
        </section>
      </div>}

      </div>
      {modal && (modal.kind === "update" ? <UpdateDialog version={modal.version} notes={modal.notes}
        available={updates?.availableVersion === modal.version && updates.installable}
        busy={!!busy || !!updates?.checking || !!updates?.installing}
        onClose={closeModal} onConfirm={() => {
          closeModal();
          void installUpdate(modal.version);
        }} /> :
        modal.kind === "monitor" ? <MonitorDialog key={modal.monitor.id}
        monitor={modal.monitor} connected={monitors.some(({ id }) => id === modal.monitor.id)}
        items={items} previewFor={previewFor} assignment={library?.assignments[modal.monitor.id]}
        disabled={!!busy} error={notice?.error ? notice.text : undefined} onClose={closeModal}
        onConfirm={(id) => void applyWallpaper(modal.monitor.id, id)} /> :
        modal.kind === "preview" ? <PreviewDialog key={modal.item.id} item={modal.item} poster={previewFor(modal.item.id)}
          assignedNames={attached.filter(({ id }) => library?.assignments[id] === modal.item.id).map(({ name }) => name)}
          disabled={!!busy} canAssign={!!monitors.length} onClose={closeModal}
          onAction={(kind) => showModal({ kind, item: modal.item })} /> :
        <WallpaperDialog key={modal.kind + modal.item.id} modal={{ kind: modal.kind, item: modal.item }} monitors={monitors}
          assignedNames={Object.entries(library?.assignments ?? {}).filter(([, id]) => id === modal.item.id)
            .map(([id]) => monitors.find((monitor) => monitor.id === id)?.name ?? `${library?.displayNames[id] ?? "Bildschirm"} (getrennt)`)}
          disabled={!!busy} error={notice?.error ? notice.text : undefined} onClose={closeModal}
          onConfirm={async (value) => {
            const id = modal.item.id;
            if (modal.kind === "assign") return applyWallpaper(value, id);
            if (await run("Änderung wird gespeichert …", () => modal.kind === "rename"
              ? invoke("rename_wallpaper", { id, name: value }) : invoke("delete_wallpaper", { id }))) closeModal();
          }} />)}
    </main>
    {intro && <div className={introRunning ? "startup startup-running" : "startup"} aria-hidden="true" onAnimationEnd={(event) => {
      if (event.target === event.currentTarget) setIntro(false);
    }}>
      <div className="startup-brand"><div className="startup-logo"><img src={brand} alt="" fetchPriority="high" /></div><span>JunaDesk</span></div>
    </div>}
    {dragging && <div className="drag-overlay"><div><Icon name="upload" size={34} /><h2>Wallpaper hier ablegen</h2><p>MP4, WebM und GIF</p></div></div>}
  </div>;
}

function MonitorPoster({ assigned, src }: { assigned: boolean; src?: string }) {
  const [current, setCurrent] = useState({ assigned, src });
  const [previous, setPrevious] = useState<{ assigned: boolean; src?: string }>();
  if (current.assigned !== assigned || current.src !== src) {
    if (!window.matchMedia("(prefers-reduced-motion: reduce)").matches) setPrevious(current);
    else setPrevious(undefined);
    setCurrent({ assigned, src });
  }
  useEffect(() => {
    const motion = window.matchMedia("(prefers-reduced-motion: reduce)");
    const clear = () => { if (motion.matches || document.hidden) setPrevious(undefined); };
    motion.addEventListener("change", clear);
    document.addEventListener("visibilitychange", clear);
    return () => {
      motion.removeEventListener("change", clear);
      document.removeEventListener("visibilitychange", clear);
    };
  }, []);
  const content = (active: boolean, image?: string) => active ? <Poster src={image} /> :
    <div className="poster-fallback"><Icon name="monitor" size={26} /><small>Systemhintergrund</small></div>;
  return <div className="monitor-art">
    {previous && <div className="previous-poster" aria-hidden="true">{content(previous.assigned, previous.src)}</div>}
    <div className={previous ? "current-poster poster-enter" : "current-poster"} key={String(assigned) + src}
      onAnimationEnd={() => setPrevious(undefined)}>{content(assigned, src)}</div>
  </div>;
}

function Poster({ src }: { src?: string }) {
  const [failedSrc, setFailedSrc] = useState<string>();
  if (!src || src === failedSrc) return <div className="poster-fallback">
    <Icon name="image" size={22} /><small>Vorschau nicht verfügbar</small>
  </div>;
  return <img key={src} src={src} alt="" loading="lazy" decoding="async"
    onError={() => setFailedSrc(src)} />;
}

function Dialog({ title, onClose, busy = false, wide = false, children }: {
  title: string; onClose: () => void; busy?: boolean; wide?: boolean; children: ReactNode;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const dialog = ref.current;
    dialog?.showModal();
    return () => dialog?.close();
  }, []);
  return <dialog className={wide ? "dialog-wide" : undefined} ref={ref} aria-labelledby="modal-title" onCancel={(event) => {
    event.preventDefault();
    if (!busy) onClose();
  }}>
    <div className="dialog-heading"><h2 id="modal-title">{title}</h2>
      <button className="icon-button" aria-label="Dialog schließen" disabled={busy} onClick={onClose}><Icon name="close" size={18} /></button></div>
    {children}
  </dialog>;
}

function UpdateDialog({ version, notes, available, busy, onClose, onConfirm }: {
  version: string; notes: string | null; available: boolean; busy: boolean; onClose: () => void; onConfirm: () => void;
}) {
  return <Dialog title={`Neu in JunaDesk ${version}`} onClose={onClose} wide>
    {notes ? <div className="release-notes" tabIndex={0} role="region" aria-label="Versionshinweise">
      <Markdown skipHtml disallowedElements={["a", "img"]} unwrapDisallowed>{notes}</Markdown>
    </div> : <p className="dialog-subtitle">Keine Versionshinweise verfügbar.</p>}
    {!available && <p role="status" className="dialog-subtitle">Das angebotene Update hat sich geändert.</p>}
    <div className="modal-actions">
      <button className="button secondary" onClick={onClose}>Abbrechen</button>
      <button className="button primary" disabled={!available || busy} onClick={onConfirm}>Update installieren</button>
    </div>
  </Dialog>;
}

function PreviewDialog({ item, poster, assignedNames, disabled, canAssign, onClose, onAction }: {
  item: Wallpaper; poster?: string; assignedNames: string[]; disabled: boolean; canAssign: boolean;
  onClose: () => void; onAction: (kind: "assign" | "rename" | "delete") => void;
}) {
  const [path, setPath] = useState<string>();
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const video = useRef<HTMLVideoElement>(null);
  const mounted = useRef(true);

  useEffect(() => {
    mounted.current = true;
    const pause = () => { if (document.hidden) video.current?.pause(); };
    document.addEventListener("visibilitychange", pause);
    return () => {
      mounted.current = false;
      video.current?.pause();
      document.removeEventListener("visibilitychange", pause);
    };
  }, []);

  const play = async () => {
    setError("");
    setLoading(true);
    try {
      const source = await invoke<string>("wallpaper_preview", { id: item.id });
      if (mounted.current && !document.hidden) setPath(convertFileSrc(source));
    } catch (failure) {
      if (mounted.current) setError(errorText(failure));
    } finally {
      if (mounted.current) setLoading(false);
    }
  };

  return <Dialog title={item.name} onClose={onClose} wide>
    <div className="detail-preview">
      {path ? <video ref={video} src={path} poster={poster} controls autoPlay muted playsInline preload="metadata"
        onError={() => { setPath(undefined); setError("Das Video kann in dieser Vorschau nicht abgespielt werden."); }} />
        : <Poster src={poster} />}
    </div>
    <div className="detail-meta"><span>{item.extension.toUpperCase()}</span>
      {assignedNames.length > 0 && <span>Zugewiesen an {assignedNames.join(", ")}</span>}</div>
    {error && <p className="banner error" role="alert">{error}</p>}
    <div className="detail-actions">
      {!path && <button className="button secondary" disabled={loading} onClick={() => void play()}>
        <Icon name="play" size={16} />{loading ? "Vorschau lädt …" : "Abspielen"}</button>}
      <button className="button secondary" onClick={() => void invoke("reveal_wallpaper", { id: item.id }).catch((failure) => {
        if (mounted.current) setError(errorText(failure));
      })}>Datei anzeigen</button>
      <button className="button secondary" disabled={disabled} onClick={() => onAction("rename")}>Umbenennen</button>
      <button className="button ghost" disabled={disabled} onClick={() => onAction("delete")}>Entfernen</button>
    </div>
    <div className="modal-actions">
      <button className="button primary" disabled={disabled || !canAssign} onClick={() => onAction("assign")}>Als Wallpaper verwenden</button>
    </div>
  </Dialog>;
}

function MonitorDialog({ monitor, connected, items, previewFor, assignment, disabled, error, onClose, onConfirm }: {
  monitor: Monitor; connected: boolean; items: Wallpaper[]; previewFor: (id: string) => string | undefined;
  assignment?: string; disabled: boolean; error?: string; onClose: () => void; onConfirm: (id: string | null) => void;
}) {
  const [selected, setSelected] = useState(assignment ?? "");
  const [query, setQuery] = useState("");
  const matching = items.filter(({ name }) => name.toLocaleLowerCase("de").includes(query.trim().toLocaleLowerCase("de")));
  const canApply = connected && selected !== (assignment ?? "") && (!selected || items.some(({ id }) => id === selected));
  return <Dialog title="Wallpaper ändern" onClose={onClose} busy={disabled}>
    <form onSubmit={(event) => {
      event.preventDefault();
      if (!disabled && canApply) onConfirm(selected || null);
    }}>
      <p className="dialog-subtitle">{monitor.name} · {monitor.width} × {monitor.height}</p>
      {items.length > 5 && <input className="choice-search" type="search" value={query} aria-label="Wallpaper suchen"
        placeholder="Wallpaper suchen" onChange={(event) => setQuery(event.target.value)} />}
      <fieldset className="choice-list" disabled={disabled}>
        <legend className="sr-only">Wallpaper für {monitor.name}</legend>
        <label className="choice"><input type="radio" name="wallpaper" value="" checked={!selected} onChange={() => setSelected("")} />
          <span className="choice-preview"><Icon name="monitor" size={18} /></span><span>Systemhintergrund</span></label>
        {matching.map((item) => <label className="choice" key={item.id}>
          <input type="radio" name="wallpaper" value={item.id} checked={selected === item.id} onChange={() => setSelected(item.id)} />
          <span className="choice-preview"><Poster src={previewFor(item.id)} /></span><span>{item.name}</span>
        </label>)}
        {!matching.length && query && <p className="muted">Keine passenden Wallpaper gefunden.</p>}
      </fieldset>
      {(!connected || error) && <p className="banner error" role="alert">{connected ? error : "Der Bildschirm ist nicht mehr verbunden."}</p>}
      <div className="modal-actions">
        <button className="button secondary" type="button" disabled={disabled} onClick={onClose}>Abbrechen</button>
        <button className="button primary" type="submit" disabled={disabled || !canApply}>{disabled ? "Wird angewendet …" : "Anwenden"}</button>
      </div>
    </form>
  </Dialog>;
}

function UpdateInstallationProgress({ progress }: { progress?: UpdateProgress }) {
  const download = progress?.phase === "downloading" ? progress : undefined;
  const fraction = download?.total ? Math.min(1, download.downloaded / download.total) : undefined;
  const label = progress?.phase === "verifying" ? "Signatur wird geprüft …"
    : progress?.phase === "installing" ? "Update wird installiert …"
    : download ? "Update wird heruntergeladen …" : "Update wird vorbereitet …";
  return <div className="update-progress">
    <span role="status">{label}</span>
    {download && <progress aria-label="Downloadfortschritt" max={1} value={fraction} />}
    {fraction !== undefined && <span aria-hidden="true">{Math.round(fraction * 100)} %</span>}
  </div>;
}

function Setting({ title, description, checked, disabled, onChange }: {
  title: string; description: string; checked: boolean; disabled: boolean; onChange: (enabled: boolean) => void;
}) {
  return <label className="setting-row">
    <span><strong>{title}</strong><small>{description}</small></span>
    <input className="switch" type="checkbox" role="switch" checked={checked} disabled={disabled}
      onChange={(event) => onChange(event.target.checked)} />
  </label>;
}

function WallpaperDialog({ modal, monitors, assignedNames, disabled, error, onClose, onConfirm }: {
  modal: { kind: "assign" | "rename" | "delete"; item: Wallpaper }; monitors: Monitor[]; assignedNames: string[]; disabled: boolean; error?: string; onClose: () => void;
  onConfirm: (value: string) => Promise<void>;
}) {
  const [value, setValue] = useState(modal.kind === "rename" ? modal.item.name : monitors.find((monitor) => monitor.primary)?.id ?? monitors[0]?.id ?? "");
  const title = { assign: "Als Wallpaper verwenden", rename: "Wallpaper umbenennen", delete: "Aus Bibliothek entfernen" }[modal.kind];
  return <Dialog title={title} onClose={onClose} busy={disabled}>
    <form onSubmit={(event) => {
      event.preventDefault();
      if (!disabled && (modal.kind === "delete" || (modal.kind === "assign" ? monitors.some(({ id }) => id === value) : value.trim()))) void onConfirm(value);
    }}>
      {modal.kind === "rename" && <label className="field">Name
        <input autoFocus required maxLength={100} value={value} onChange={(event) => setValue(event.target.value)} />
      </label>}
      {modal.kind === "assign" && <label className="field">Bildschirm für „{modal.item.name}“
        <select autoFocus value={value} onChange={(event) => setValue(event.target.value)}>
          {!monitors.length && <option value="">Kein Bildschirm verfügbar</option>}
          {monitors.map((monitor) => <option key={monitor.id} value={monitor.id}>{monitor.name} · {monitor.width} × {monitor.height}</option>)}
        </select>
      </label>}
      {modal.kind === "delete" && <div className="delete-copy">
        <p>„{modal.item.name}“ wird aus der Bibliothek entfernt. JunaDesk löscht seine verwaltete Kopie. Deine ursprüngliche Importdatei bleibt erhalten.</p>
        {assignedNames.length > 0 && <><p>Die Zuweisung wird auf diesen Bildschirmen aufgehoben:</p><ul>{assignedNames.map((name, index) => <li key={index}>{name}</li>)}</ul></>}
      </div>}
      {error && <p className="banner error" role="alert">{error}</p>}
      <div className="modal-actions">
        <button className="button secondary" type="button" disabled={disabled} onClick={onClose}>Abbrechen</button>
        <button className={modal.kind === "delete" ? "button destructive" : "button primary"} type="submit"
          disabled={disabled || (modal.kind === "rename" && !value.trim()) || (modal.kind === "assign" && !monitors.some(({ id }) => id === value))}>
          {{ assign: "Anwenden", rename: "Speichern", delete: "Entfernen" }[modal.kind]}
        </button>
      </div>
    </form>
  </Dialog>;
}

createRoot(document.getElementById("root")!).render(<App />);
