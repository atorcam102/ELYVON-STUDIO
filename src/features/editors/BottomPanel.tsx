import { IconButton } from "../../components/Button";
import { Tabs } from "../../components/Tabs";
import { useUi, type BottomTab } from "../../state/ui";
import { Mixer } from "../mixer/Mixer";
import { AudioEditor, PianoRoll } from "./PianoRoll";

export function BottomPanel() {
  const tab = useUi((s) => s.layout.bottomTab) as BottomTab;
  const setLayout = useUi((s) => s.setLayout);
  return (
    <section className="bottom-panel" aria-label="Editors">
      <Tabs<BottomTab>
        label="Editors"
        value={tab}
        onChange={(bottomTab) => setLayout({ bottomTab })}
        tabs={[
          { id: "mixer", label: "Mixer", icon: "mixer" },
          { id: "pianoRoll", label: "Piano Roll", icon: "piano", note: "Preview" },
          { id: "audioEditor", label: "Audio Editor", icon: "waveform", note: "Phase 2" },
        ]}
        trailing={<IconButton icon="close" label="Hide editors (E)" size={13} onClick={() => setLayout({ bottomVisible: false })} />}
      />
      <div className="bottom-panel__content">
        {tab === "mixer" && <Mixer />}
        {tab === "pianoRoll" && <PianoRoll />}
        {tab === "audioEditor" && <AudioEditor />}
      </div>
    </section>
  );
}
