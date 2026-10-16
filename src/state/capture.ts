import { create } from "zustand";
import type { CaptureStatus } from "../bindings/CaptureStatus";
import { ipc, inTauri } from "../lib/ipc";
import { enqueue, useProject } from "./project";
import { useApp } from "./app";
import { useUi } from "./ui";
import { notify, notifyError } from "./notifications";
interface CaptureStore {
 status: CaptureStatus | null;
 pending: boolean;
 refresh: () => Promise<void>;
 start: (monitorOnly?: boolean) => Promise<void>;
 stop: (cancel?: boolean) => Promise<void>;
}
export const useCapture=create<CaptureStore>((set,get)=>({
 status:null,pending:false,
 refresh:async()=>{set({status:await ipc.recording.status()});},
 start:async(monitorOnly=false)=>{
  if(get().pending)return;set({pending:true});
  try{await enqueue(async()=>{
   const settings=useApp.getState().settings;if(settings)await ipc.settings.set(settings);
   set({status:await ipc.recording.start(useUi.getState().cursorTicks,monitorOnly)});
  });}catch(e){notifyError(e,"Input could not start");}finally{set({pending:false});}
 },
 stop:async(cancel=false)=>{
  if(get().pending)return;set({pending:true});
  try{const r=await ipc.recording.stop(cancel);useProject.getState().setView(r.view);await get().refresh();
   if(r.report.error)notify("warning",r.report.error);else if(r.report.takes.length)notify("success",`${r.report.takes.length} take(s) preserved. Save the project to retain their references.`);
  }catch(e){notifyError(e,"Input could not stop");}finally{set({pending:false});}
 },
}));
export function connectCapture(){
 if(!inTauri)return()=>{};
 let closed=false,loading=false;
 const poll=()=>{if(closed||loading)return;loading=true;void useCapture.getState().refresh().catch(()=>{}).finally(()=>{loading=false;});};
 poll();const timer=setInterval(poll,250);return()=>{closed=true;clearInterval(timer);};
}
