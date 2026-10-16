import {open as chooseOpen} from "@tauri-apps/plugin-dialog";
import {useEffect,useState} from "react";
import {Dialog} from "../../components/Dialog";
import {Button} from "../../components/Button";
import {NumberField} from "../../components/Fields";
import {ipc} from "../../lib/ipc";
import type {DeviceInfo} from "../../bindings/DeviceInfo";
import type {LatencyMeasurement} from "../../bindings/LatencyMeasurement";
import type {RecordedTake} from "../../bindings/RecordedTake";
import {useApp} from "../../state/app";
import {useCapture} from "../../state/capture";
import {useProject} from "../../state/project";
import {notifyError} from "../../state/notifications";
export function RecordingDialog({open,onClose}:{open:boolean;onClose:()=>void}){
 const settings=useApp(s=>s.settings);const update=useApp(s=>s.updateSettings);
 const {status,pending,start,stop}=useCapture();const [devices,setDevices]=useState<DeviceInfo[]>([]);const [takes,setTakes]=useState<RecordedTake[]>([]);
 const refresh=()=>{void ipc.recording.devices().then(setDevices).catch(e=>notifyError(e));void ipc.recording.recoverable().then(setTakes).catch(e=>notifyError(e));};
 useEffect(()=>{if(open)refresh();},[open]);
 const [measurement,setMeasurement]=useState<LatencyMeasurement|null>(null);
 const measure=async()=>{try{const reference=await chooseOpen({title:"Choose a short nonperiodic reference (16–8192 samples)",multiple:false,filters:[{name:"Audio reference",extensions:["wav","aiff","flac"]}]});if(!reference||Array.isArray(reference))return;const captured=await chooseOpen({title:"Choose its aligned loopback recording",multiple:false,filters:[{name:"Captured audio",extensions:["wav","aiff","flac"]}]});if(!captured||Array.isArray(captured))return;setMeasurement(await ipc.recording.measure(reference,captured,48000));}catch(e){notifyError(e,"Loopback measurement failed");}};
 const active=!!status?.active;
 return <Dialog open={open} title="Audio recording and input" onClose={onClose} width={560} footer={<Button onClick={onClose}>Close</Button>}>
  <p>Arm audio tracks and select their channels in the inspector. Count-in, pre-roll, cycle and punch follow the project transport.</p>
  <label>Input device <select aria-label="Recording input device" value={settings?.audio.inputDevice??""} disabled={active} onChange={e=>update(s=>({...s,audio:{...s.audio,inputDevice:e.target.value||null}}))}>
   <option value="">System default</option>{devices.map(d=><option key={d.id} value={d.id}>{d.name}</option>)}
  </select></label>
  <label>Recording offset <NumberField label="Recording offset (frames)" value={settings?.audio.recordingOffsetFrames??0} min={-480000} max={480000} onCommit={value=>update(s=>({...s,audio:{...s.audio,recordingOffsetFrames:value}}))}/></label>
  <p>Positive offsets place captured audio earlier. Device timestamps provide an estimate; use a measured loopback offset for calibration. Live input must support the engine sample rate. Direct hardware monitoring is controlled on your interface.</p>
  <p role="status">{active?(status?.monitoring?"Monitoring input":"Recording"):"Input stopped"} · input peak {((status?.inputPeak??0)*100).toFixed(1)}% · clipped frames {status?.clippedFrames??0} · estimated latency: input {status?.estimatedInputLatencyFrames??0}, output {status?.estimatedOutputLatencyFrames??0} frames</p>
  {status?.interrupted&&<p role="alert">Input interrupted. Stop and recover the preserved takes.</p>}
  <div className="insp-buttons"><Button disabled={active||pending} onClick={()=>void start(true)}>Monitor input</Button><Button disabled={active||pending} onClick={()=>void start()}>Record armed tracks</Button><Button disabled={!active||pending} onClick={()=>void stop()}>Stop input</Button><Button disabled={!active||pending} onClick={()=>void stop(true)}>Cancel take</Button></div>
  {status?.last?.error&&<p role="alert">{status.last.error}</p>}
  <Button disabled={active||pending} onClick={()=>void measure()}>Measure aligned loopback files</Button>
  {measurement&&<p role="status">Measured file delay: {measurement.delayFrames} frames at {measurement.sampleRate} Hz · correlation {measurement.correlation.toFixed(3)}. This compares the files; it does not automatically change the recording offset.</p>}
  <h3>Recoverable takes</h3>{takes.length===0?<p>No interrupted takes in this project.</p>:takes.map(t=><div key={t.path}><span>{t.frames} frames · {t.channels} channel(s)</span><Button disabled={active||pending} onClick={()=>{void ipc.recording.recover(t.path).then(r=>{useProject.getState().setView(r.view);refresh();}).catch(e=>notifyError(e));}}>Recover take</Button></div>)}
 </Dialog>;
}
