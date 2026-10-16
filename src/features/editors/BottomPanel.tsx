import { IconButton } from "../../components/Button";
import { useUi, type BottomTab } from "../../state/ui";
import { Mixer } from "../mixer/Mixer";
import { AudioEditor, PianoRoll } from "./PianoRoll";

const TABS: { id: BottomTab; label: string; note?: string }[] = [
  { id: "mixer", label: "Mixer" },
  { id: "pianoRoll", label: "Piano Roll" },
  { id: "audioEditor", label: "Audio Editor", note: "Phase 2" },
];

export function BottomPanel() {
  const tab = useUi((s) => s.layout.bottomTab) as BottomTab;
  const setLayout = useUi((s) => s.setLayout);
  return (
    <section
      className={`bottom-panel${tab === "mixer" ? " bottom-panel--mixer" : ""}`}
      aria-label="Editors"
    >
      <div className="bottom-panel__bar">
        <div className="segmented" role="tablist" aria-label="Editors">
          {TABS.map((t) => (
            <button
              key={t.id}
              type="button"
              role="tab"
              aria-selected={t.id === tab}
              className={`tabs__tab${t.id === tab ? " is-on" : ""}`}
              title={t.note ? `${t.label} (${t.note})` : t.label}
              onClick={() => setLayout({ bottomTab: t.id })}
            >
              {t.label}
            </button>
          ))}
        </div>
        <IconButton
          icon="close"
          label="Hide editors (E)"
          size={12}
          onClick={() => setLayout({ bottomVisible: false })}
        />
      </div>
      <div className="bottom-panel__content">
        {tab === "mixer" && <Mixer />}
        {tab === "pianoRoll" && <PianoRoll />}
        {tab === "audioEditor" && <AudioEditor />}
      </div>
    </section>
  );
}
