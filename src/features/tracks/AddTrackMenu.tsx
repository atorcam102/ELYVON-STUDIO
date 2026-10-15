import { IconButton } from "../../components/Button";
import { Menu } from "../../components/Popover";
import { useMenu } from "../../components/useMenu";
import { shortcutLabel } from "../../lib/platform";
import { addTrack } from "../project/editActions";

export function AddTrackButton() {
  const menu = useMenu();
  return (
    <>
      <IconButton icon="plus" label="New track" onClick={(e) => menu.openBelow(e.currentTarget)} size={14} />
      {menu.at && (
        <Menu
          at={menu.at}
          onClose={menu.close}
          entries={[
            { heading: "New track" },
            { id: "audio", label: "Audio", icon: "audio", shortcut: shortcutLabel("Mod+Alt+A"), onSelect: () => void addTrack("audio") },
            {
              id: "instrument",
              label: "Instrument",
              icon: "instrument",
              shortcut: shortcutLabel("Mod+Alt+S"),
              onSelect: () => void addTrack("instrument"),
            },
            { id: "midi", label: "External MIDI", icon: "midi", shortcut: shortcutLabel("Mod+Alt+X"), onSelect: () => void addTrack("midi") },
          ]}
        />
      )}
    </>
  );
}
