import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import brand from "./brand.png";
import "./styles.css";

type Wallpaper = { id: string; name: string; extension: string };
type Monitor = { id: string; name: string; width: number; height: number; x: number; y: number; primary: boolean; fullscreen: boolean };
type Settings = { fullscreenPause: boolean; batteryPause: boolean; autostart: boolean; fps: 15 | 30 | null };
type Status = { monitors: Monitor[]; onBattery: boolean; manualPause: boolean; errors: Record<string, string>; pauseReasons: Record<string, string>; desktopError: string | null };
type Snapshot = { library: { wallpapers: Wallpaper[]; assignments: Record<string, string>; displayNames: Record<string, string>; settings: Settings }; status: Status; previews: Record<string, string> };
type ImportResult = { file: string; error: string | null };
type Page = "Bibliothek" | "Monitore" | "Einstellungen";
type IconName = "library" | "monitor" | "settings" | "plus" | "upload" | "edit" | "trash" | "play" | "close" | "check" | "info";
type Modal = { kind: "assign" | "rename" | "delete"; item: Wallpaper };

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
};

function Icon({ name, size = 20 }: { name: IconName; size?: number }) {
  return <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">{glyphs[name]}</svg>;
}

function errorText(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

const navigation: [Page, IconName][] = [["Bibliothek", "library"], ["Monitore", "monitor"], ["Einstellungen", "settings"]];

function App() {
  const [page, setPage] = useState<Page>("Bibliothek");
  const [snapshot, setSnapshot] = useState<Snapshot>();
  const [startupError, setStartupError] = useState("");
  const [notice, setNotice] = useState<{ text: string; error?: boolean }>();
  const [modal, setModal] = useState<Modal>();
  const [dragging, setDragging] = useState(false);
  const [busy, setBusy] = useState<string>();
  const activeOperation = useRef(false);
  const refreshNumber = useRef(0);

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
    await run("Wallpaper werden importiert …", async () => {
      const results = await invoke<ImportResult[]>("import_wallpapers", { paths });
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
      register(getCurrentWebview().onDragDropEvent(({ payload }) => {
        setDragging(payload.type === "over" || payload.type === "enter");
        if (payload.type === "drop") void importPaths(payload.paths);
      })),
    ]).then(() => { if (alive) void refresh(); })
      .catch((error: unknown) => { if (alive) setStartupError(errorText(error)); });
    return () => {
      alive = false;
      subscriptions.forEach((unlisten) => unlisten());
    };
  }, [importPaths, refresh]);

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
          aria-current={page === name ? "page" : undefined} onClick={() => setPage(name)}>
          <Icon name={icon} size={19} /><span>{name}</span>
        </button>)}
      </nav>
      <div className="sidebar-footer">
        <span className={attached.some(({ id }) => !status?.pauseReasons[id] && !status?.errors[id]) && !status?.desktopError ? "dot active" : "dot"} />
        <span>{status?.manualPause ? "Manuell pausiert" : status?.desktopError ? "Desktop nicht verfügbar"
          : attached.length ? `${attached.length} Bildschirm${attached.length === 1 ? "" : "e"} mit Wallpaper`
            : "Kein Wallpaper aktiv"}</span>
      </div>
    </aside>
    <main className="workspace">
      <header className="page-header">
        <div>
          <h1>{page === "Bibliothek" ? "Meine Wallpaper" : page}</h1>
          <p>{page === "Bibliothek" ? `${items.length} Wallpaper in deiner Bibliothek`
            : page === "Monitore" ? `${monitors.length} verbundene Bildschirme` : "Wiedergabe und Systemintegration"}</p>
        </div>
        {page === "Bibliothek" && <button className="button primary" disabled={!!busy || !snapshot} onClick={() => void chooseFiles()}><Icon name="plus" />Hinzufügen</button>}
      </header>

      {startupError && <div className="banner error" role="alert"><Icon name="info" />{startupError}</div>}
      {status?.desktopError && <div className="banner error" role="status"><Icon name="info" />{status.desktopError}</div>}
      {notice && !modal && <div className={notice.error ? "banner error" : "banner success"} role={notice.error ? "alert" : "status"}>
        <Icon name={notice.error ? "info" : "check"} /><span>{notice.text}</span>
        <button className="icon-button" aria-label="Meldung schließen" onClick={() => setNotice(undefined)}><Icon name="close" size={17} /></button>
      </div>}
      {busy && <div className="banner pending" role="status"><span className="spinner" />{busy}</div>}

      {!snapshot && !startupError && <p className="loading" role="status">JunaDesk wird geladen …</p>}
      {snapshot && page === "Bibliothek" && (items.length === 0 ?
        <section className="empty drop-surface">
          <div className="empty-art"><Icon name="upload" size={30} /></div>
          <h2>Dein erstes Wallpaper</h2>
          <p>Ziehe MP4-, WebM- oder GIF-Dateien hierher.</p>
          <button className="button secondary" disabled={!!busy} onClick={() => void chooseFiles()}>Dateien auswählen</button>
        </section>
        : <section className="cards" aria-label="Wallpaper-Bibliothek">
          {items.map((item) => {
            const displays = monitors.filter(({ id }) => library?.assignments[id] === item.id);
            return <article className="card" key={item.id}>
              <div className="preview">
                <img src={convertFileSrc(snapshot.previews[item.id])} alt="" loading="lazy" />
                {displays.length > 0 && <span className="assigned"><span className="dot active" />Zugewiesen</span>}
              </div>
              <div className="card-body">
                <div><h2 title={item.name}>{item.name}</h2><p>{item.extension.toUpperCase()}</p></div>
                <div className="card-actions">
                  <button className="icon-button activate" title="Aktivieren" aria-label={`${item.name} aktivieren`} disabled={!!busy}
                    onClick={() => { setNotice(undefined); setModal({ kind: "assign", item }); }}><Icon name="play" size={17} /></button>
                  <button className="icon-button" title="Umbenennen" aria-label={`${item.name} umbenennen`} disabled={!!busy}
                    onClick={() => { setNotice(undefined); setModal({ kind: "rename", item }); }}><Icon name="edit" size={17} /></button>
                  <button className="icon-button danger-action" title="Löschen" aria-label={`${item.name} löschen`} disabled={!!busy}
                    onClick={() => { setNotice(undefined); setModal({ kind: "delete", item }); }}><Icon name="trash" size={17} /></button>
                </div>
              </div>
            </article>;
          })}
        </section>)}

      {snapshot && page === "Monitore" && <section className="monitor-grid" aria-label="Monitorzuweisungen">
        {monitors.length === 0 && <p className="muted">Es sind keine Bildschirme verfügbar.</p>}
        {monitors.map((monitor) => {
          const assignment = library?.assignments[monitor.id] ?? "";
          const reason = status?.pauseReasons[monitor.id];
          const error = status?.errors[monitor.id];
          return <article className="monitor-card" key={monitor.id}>
            <div className="monitor-art"><Icon name="monitor" size={31} /></div>
            <div className="monitor-name"><h2>{monitor.name}</h2><p>{monitor.width} × {monitor.height}{monitor.primary && " · Hauptbildschirm"}</p></div>
            <label htmlFor={`monitor-${monitor.id}`}>Wallpaper</label>
            <select id={`monitor-${monitor.id}`} disabled={!!busy} value={assignment} onChange={(event) => {
              const wallpaperId = event.target.value || null;
              void run("Zuweisung wird gespeichert …", () => invoke("assign_wallpaper", { monitorId: monitor.id, wallpaperId }));
            }}>
              <option value="">Systemhintergrund</option>
              {items.map((item) => <option value={item.id} key={item.id}>{item.name}</option>)}
            </select>
            {assignment && <p className={error ? "monitor-status problem" : "monitor-status"}>{error || reason || "Wiedergabe aktiv"}</p>}
          </article>;
        })}
      </section>}

      {snapshot && page === "Einstellungen" && <div className="settings">
        <section className="settings-panel">
          <h2>Wiedergabe</h2>
          {settingRows.slice(0, 2).map(([name, title, description]) => <Setting key={name} title={title} description={description}
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
          <h2>System</h2>
          <Setting title={settingRows[2][1]} description={settingRows[2][2]} checked={snapshot.library.settings.autostart}
            disabled={!!busy} onChange={(enabled) => void run("Autostart wird gespeichert …", () => invoke("set_option", { option: "autostart", enabled }))} />
        </section>
      </div>}

      {modal && <WallpaperDialog key={modal.kind + modal.item.id} modal={modal} monitors={monitors}
        assignedNames={Object.entries(library?.assignments ?? {}).filter(([, id]) => id === modal.item.id)
          .map(([id]) => monitors.find((monitor) => monitor.id === id)?.name ?? `${library?.displayNames[id] ?? "Bildschirm"} (getrennt)`)}
        disabled={!!busy} error={notice?.error ? notice.text : undefined} onClose={() => { setNotice(undefined); setModal(undefined); }} onConfirm={async (value) => {
          const id = modal.item.id;
          const succeeded = await run("Änderung wird gespeichert …", () => {
            if (modal.kind === "rename") return invoke("rename_wallpaper", { id, name: value });
            if (modal.kind === "delete") return invoke("delete_wallpaper", { id });
            return invoke("assign_wallpaper", { monitorId: value, wallpaperId: id });
          });
          if (succeeded) setModal(undefined);
        }} />}
    </main>
    {dragging && <div className="drag-overlay"><div><Icon name="upload" size={34} /><h2>Wallpaper hier ablegen</h2><p>MP4, WebM und GIF</p></div></div>}
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
  modal: Modal; monitors: Monitor[]; assignedNames: string[]; disabled: boolean; error?: string; onClose: () => void;
  onConfirm: (value: string) => Promise<void>;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const [value, setValue] = useState(modal.kind === "rename" ? modal.item.name : monitors.find((monitor) => monitor.primary)?.id ?? monitors[0]?.id ?? "");
  const title = { assign: "Wallpaper aktivieren", rename: "Wallpaper umbenennen", delete: "Wallpaper löschen" }[modal.kind];
  useEffect(() => { ref.current?.showModal(); }, []);
  return <dialog ref={ref} aria-labelledby="modal-title" onCancel={(event) => {
    event.preventDefault();
    if (!disabled) onClose();
  }}>
    <form onSubmit={(event) => {
      event.preventDefault();
      if (!disabled && (modal.kind === "delete" || value.trim())) void onConfirm(value);
    }}>
      <h2 id="modal-title">{title}</h2>
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
        <p>„{modal.item.name}“ und seine verwalteten Dateien werden aus JunaDesk gelöscht.</p>
        {assignedNames.length > 0 && <><p>Die Zuweisung wird auf diesen Bildschirmen aufgehoben:</p><ul>{assignedNames.map((name, index) => <li key={index}>{name}</li>)}</ul></>}
      </div>}
      {error && <p className="banner error" role="alert">{error}</p>}
      <div className="modal-actions">
        <button className="button secondary" type="button" disabled={disabled} onClick={onClose}>Abbrechen</button>
        <button className={modal.kind === "delete" ? "button destructive" : "button primary"} type="submit"
          disabled={disabled || (modal.kind !== "delete" && !value.trim())}>
          {{ assign: "Aktivieren", rename: "Speichern", delete: "Löschen" }[modal.kind]}
        </button>
      </div>
    </form>
  </dialog>;
}

createRoot(document.getElementById("root")!).render(<App />);
