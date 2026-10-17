import { getCurrentWindow } from "@tauri-apps/api/window";
import { inTauri } from "../../lib/ipc";
import { useChrome, type SidePanel } from "../../state/chrome";
import { notify } from "../../state/notifications";
import { useUi } from "../../state/ui";
import { useProject } from "../../state/project";
import { runMenuAction } from "../app/useAppEvents";
import { deleteSelection } from "../project/editActions";
import { copySelectedRegions, cutSelectedRegions, pasteRegions } from "../project/regionActions";
import { toggleCountIn } from "../transport/transportActions";

export function pendingControl(label: string) {
  notify("info", `${label}: todavía no disponible.`);
}

export function activateMenuAction(action: string | undefined, label: string) {
  const ui = useUi.getState();
  const tab = action?.replace("view.", "");
  if (["pianoRoll", "score", "smartTempo", "smartControls", "audioEditor"].includes(tab ?? "")) {
    ui.setLayout({ bottomVisible: true, bottomTab: tab!, bottomHeight: Math.round(window.innerHeight * .65) });
    return;
  }
  if (["events", "markers", "tempo", "signature", "files", "notes", "loops"].includes(tab ?? "")) {
    useChrome.getState().toggleSidePanel(tab as SidePanel);
    return;
  }
  switch (action) {
    case "record.settings": return useChrome.getState().setDialog("recording");
    case "transport.settings": return useChrome.getState().setDialog("transport");
    case "edit.cut": return void cutSelectedRegions();
    case "edit.copy": return copySelectedRegions();
    case "edit.paste": return void pasteRegions();
    case "edit.delete": return void deleteSelection();
    case "transport.countIn": return toggleCountIn();
    case "view.controlbar": return useChrome.getState().toggleControls();
    case "view.main": ui.setLayout({ bottomVisible: false }); return useChrome.getState().setSidePanel(null);
    case "navigate.selection": {
      const p = useProject.getState().view?.project;
      const r = p?.tracks.flatMap(t => t.regions).find(r => ui.selection.regionIds.includes(r.id));
      if (r) ui.setCursor(r.start);
      return;
    }
    case "window.minimize": if (inTauri) void getCurrentWindow().minimize(); return;
    case "view.fullscreen":
      if (inTauri) void getCurrentWindow().isFullscreen().then(v => getCurrentWindow().setFullscreen(!v));
      else if (document.fullscreenElement) void document.exitFullscreen();
      else void document.documentElement.requestFullscreen();
      return;
    case "track.rename": document.querySelector<HTMLElement>('.track-header.is-selected .inline-edit')?.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })); return;
  }
  if (action && !["record.settings", "transport.settings"].includes(action)) return runMenuAction(action);
  pendingControl(label);
}

