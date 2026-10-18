// Entries of the mixer's "add insert" menu for the native effects, grouped
// by category, built from the catalogue the core provides.

import type { EffectSlot } from "../../bindings/EffectSlot";
import type { MenuEntry } from "../../components/Popover";
import { CATEGORY_LABEL, cachedCatalog, loadCatalog, newSlot } from "../../lib/effects";

// Fetch the catalogue early so the first menu already lists every effect.
void loadCatalog();

export function effectMenuEntries(onPick: (slot: EffectSlot) => void): MenuEntry[] {
  const catalog = cachedCatalog();
  if (!catalog.length) {
    return [{ id: "fx-loading", label: "Efectos nativos: cargando…", disabled: true, onSelect: () => undefined }];
  }
  const entries: MenuEntry[] = [];
  let category = "";
  for (const info of catalog) {
    if (info.category !== category) {
      category = info.category;
      entries.push({ heading: CATEGORY_LABEL[category] ?? category });
    }
    entries.push({ id: `fx-${info.kind}`, label: info.label, onSelect: () => onPick(newSlot(info)) });
  }
  return entries;
}
