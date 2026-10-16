import {useEffect} from "react";
import {open as chooseOpen,save} from "@tauri-apps/plugin-dialog";
import {Dialog} from "../../components/Dialog";
import {Button,Toggle} from "../../components/Button";
import {ipc} from "../../lib/ipc";
import {useMidi} from "../../state/midi";
import {useProject,currentProject} from "../../state/project";
import {useUi} from "../../state/ui";
import {notify,notifyError} from "../../state/notifications";
export function MidiDialog({open,onClose}:{open:boolean;onClose:()=>void}){
 const midi=useMidi();const project=useProject(s=>s.view?.project);const apply=useProject(s=>s.apply);const selected=useUi(s=>s.selection.trackIds);
 const track=project?.tracks.find(t=>selected.includes(t.id)&&t.kind!=="audio")??project?.tracks.find(t=>t.kind!=="audio");
 const panic=()=>{void ipc.midi.panic().catch(e=>notifyError(e));};
 useEffect(()=>{if(!open)return;window.addEventListener("blur",panic);return()=>{window.removeEventListener("blur",panic);panic();};},[open]);
 const send=(bytes:number[])=>{if(track)void ipc.midi.send(track.id,bytes).catch(e=>notifyError(e));};
 const importFile=async()=>{try{const path=await chooseOpen({multiple:false,filters:[{name:"MIDI SMF",extensions:["mid","midi"]}]});if(!path||Array.isArray(path))return;const p=currentProject();const tick=useUi.getState().cursorTicks;useProject.getState().setView(await ipc.midi.import(path,tick,!!p&&p.tracks.length===0&&tick===0));}catch(e){notifyError(e,"MIDI import failed");}};
 const exportFile=async()=>{try{const path=await save({defaultPath:"Elyvon MIDI.mid",filters:[{name:"MIDI SMF",extensions:["mid"]}]});if(path){await ipc.midi.export(path);notify("success","MIDI sequence exported.");}}catch(e){notifyError(e,"MIDI export failed");}};
 const locked=!!midi.status?.recording;
 return <Dialog open={open} title="MIDI devices and recording" onClose={()=>{panic();onClose();}} width={700} footer={<Button onClick={()=>{panic();onClose();}}>Close</Button>}>
  <p>MIDI 1.0 input, output and monitoring. The keyboard sends to the selected external port, or to the monitor. Internal instruments and MIDI 2.0/UMP are unavailable.</p>
  <div className="insp-buttons"><Button onClick={()=>void ipc.midi.refresh().catch(e=>notifyError(e))}>Refresh ports</Button><Button onClick={panic}>Panic · all notes off</Button><Button disabled={locked} onClick={()=>void importFile()}>Import MIDI</Button><Button onClick={()=>void exportFile()}>Export MIDI</Button></div>
  {midi.status?.ports.error&&<p role="status">{midi.status.ports.error} The virtual keyboard monitor remains available.</p>}
  {track?<>
   <h3>{track.name}</h3>
   <label>Input <select aria-label="MIDI input port" disabled={locked} value={track.io.midiInput??""} onChange={e=>void apply({type:"setTrackIo",id:track.id,io:{...track.io,midiInput:e.target.value||null}})}><option value="">All available inputs</option>{midi.status?.ports.inputs.map(p=><option key={p.id} value={p.id}>{p.name}</option>)}{track.io.midiInput&&!midi.status?.ports.inputs.some(p=>p.id===track.io.midiInput)&&<option value={track.io.midiInput}>Saved input (absent)</option>}</select></label>
   <label>Output <select aria-label="MIDI output port" disabled={locked} value={track.io.midiOutput??""} onChange={e=>void apply({type:"setTrackIo",id:track.id,io:{...track.io,midiOutput:e.target.value||null}})}><option value="">Monitor only · no sound</option>{midi.status?.ports.outputs.map(p=><option key={p.id} value={p.id}>{p.name}</option>)}{track.io.midiOutput&&!midi.status?.ports.outputs.some(p=>p.id===track.io.midiOutput)&&<option value={track.io.midiOutput}>Saved output (absent)</option>}</select></label>
   <label>Input channel <select aria-label="MIDI channel filter" disabled={locked} value={track.io.midiChannel??""} onChange={e=>void apply({type:"setTrackIo",id:track.id,io:{...track.io,midiChannel:e.target.value===""?null:Number(e.target.value)}})}><option value="">All</option>{Array.from({length:16},(_,i)=><option key={i} value={i}>{i+1}</option>)}</select></label>
   <label>Output channel <select aria-label="MIDI output channel" disabled={locked} value={track.io.outputChannel??""} onChange={e=>void apply({type:"setTrackIo",id:track.id,io:{...track.io,outputChannel:e.target.value===""?null:Number(e.target.value)}})}><option value="">Preserve event channels</option>{Array.from({length:16},(_,i)=><option key={i} value={i}>{i+1}</option>)}</select></label>
   <Toggle checked={track.io.midiMonitor} disabled={locked} label="Monitor and forward input" onChange={v=>void apply({type:"setTrackIo",id:track.id,io:{...track.io,midiMonitor:v}})}/>
   <Toggle checked={track.io.armed} disabled={locked} label="Arm this MIDI track" onChange={v=>void apply({type:"setTrackIo",id:track.id,io:{...track.io,armed:v}})}/>
   <div role="group" aria-label="Virtual MIDI keyboard" className="insp-buttons">{[60,62,64,65,67,69,71,72].map((pitch,i)=><button type="button" className="btn" key={pitch} aria-label={`MIDI ${["C4","D4","E4","F4","G4","A4","B4","C5"][i]}`} onPointerDown={e=>{e.currentTarget.setPointerCapture(e.pointerId);send([0x90,pitch,100]);}} onPointerUp={()=>send([0x80,pitch,0])} onPointerCancel={()=>send([0x80,pitch,0])} onKeyDown={e=>{if(!e.repeat&&(e.key===" "||e.key==="Enter")){e.preventDefault();send([0x90,pitch,100]);}}} onKeyUp={e=>{if(e.key===" "||e.key==="Enter"){e.preventDefault();send([0x80,pitch,0]);}}}>{["C","D","E","F","G","A","B","C"][i]}</button>)}</div>
  </>:<p>Add a MIDI or instrument track to use input routing and the keyboard.</p>}
  <div className="insp-buttons"><Button aria-label="Record armed MIDI tracks" disabled={locked||midi.pending} onClick={()=>void midi.start()}>Record armed MIDI tracks</Button><Button aria-label="Stop MIDI recording" disabled={!locked||midi.pending} onClick={()=>void midi.stop()}>Stop MIDI recording</Button></div>
  <p>Maximum observed scheduling lateness: {midi.status?.maxSchedulerJitterUs??0} µs. Native port timing depends on the OS and device; hardware has to be measured separately.</p>
  {midi.status?.errors.slice(-2).map((e,i)=><p role="alert" key={i}>{e}</p>)}
  <h3>Native MIDI monitor</h3><div className="midi-monitor" role="log" aria-label="MIDI monitor" style={{maxHeight:160,overflow:"auto",fontFamily:"monospace"}}>{midi.status?.monitor.slice(-32).map((m,i)=><div key={`${m.timestampUs}-${i}`}>{m.timestampUs} µs · {m.direction} · {m.bytes.map(b=>b.toString(16).padStart(2,"0")).join(" ")||"all notes off"}</div>)}</div>
 </Dialog>;
}
