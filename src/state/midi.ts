import {create} from "zustand";
import type {MidiStatus} from "../bindings/MidiStatus";
import {ipc,inTauri} from "../lib/ipc";
import {useProject,enqueue} from "./project";
import {useUi} from "./ui";
import {notifyError} from "./notifications";
interface MidiState{status:MidiStatus|null;pending:boolean;refresh:()=>Promise<void>;start:()=>Promise<void>;stop:()=>Promise<void>}
export const useMidi=create<MidiState>((set,get)=>({status:null,pending:false,
 refresh:async()=>set({status:await ipc.midi.status()}),
 start:async()=>{if(get().pending)return;set({pending:true});try{await enqueue(async()=>set({status:await ipc.midi.recordStart(useUi.getState().cursorTicks)}));}catch(e){notifyError(e,"MIDI recording could not start");}finally{set({pending:false});}},
 stop:async()=>{if(get().pending)return;set({pending:true});try{useProject.getState().setView(await ipc.midi.recordStop());await get().refresh();}catch(e){notifyError(e,"MIDI recording could not stop");}finally{set({pending:false});}},
}));
export function connectMidi(){if(!inTauri)return()=>{};let pending=false;const tick=()=>{if(pending)return;pending=true;void useMidi.getState().refresh().catch(()=>{}).finally(()=>{pending=false;});};tick();const timer=setInterval(tick,250);return()=>clearInterval(timer);}
