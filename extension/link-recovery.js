/* Paused browser copies are released unless the existing job has a durable
   accepted receipt. Storage and browser effects are injected for regression tests. */
(function(root) {
  "use strict";
  const __xdL = globalThis.IDMI18n || {t: text => text, f: (text, ...values) => text.replace(/\{(\d+)\}/g, (_, n) => String(values[n]))};
  const PREFIX="linkRecovery.";
  const paused=item=>item?.paused===true && (item.state==="in_progress" || (item.state==="interrupted" && item.canResume===true));
  function matches(session,item,file) {
    if(!session || !["waiting","offered"].includes(session.state) || !file?.filename) return false;
    if(!Number.isFinite(Date.parse(item.startTime)) || Date.parse(item.startTime)<(session.expires-1800)*1000-1000) return false;
    const name=value=>value.normalize("NFC").toLowerCase().replace(/ \(\d+\)(?=\.[^.]+$)/,"");
    if(name(file.filename)!==name(session.name) || (session.size!=null && session.size!==file.size)) return false;
    try {if(item.referrer && new URL(item.referrer).origin!==new URL(session.page).origin)return false;}catch{return false;}
    return true;
  }
  class Manager {
    constructor({api,native,report,wait,now=()=>Date.now(),schedule=task=>setTimeout(task,2000)}) {
      Object.assign(this,{api,native,report,now,schedule});this.running=new Set();this.timers=new Set();
      this.lookup=new IDMCapture.Manager({downloads:Object.fromEntries(["search","pause","resume","cancel"].map(method=>[method,(...args)=>api.downloads[method](...args)])),storage:api.storage.local,native,report,
        enabled:()=>api.permissions.contains({permissions:["downloads"]}),uuid:()=>crypto.randomUUID(),...(wait?{wait}:{})});
    }
    key(id){return PREFIX+id;}
    async read(id){return (await this.api.storage.local.get(this.key(id)))[this.key(id)];}
    async save(record){await this.api.storage.local.set({[this.key(record.id)]:record});}
    async item(record){const [item]=await this.api.downloads.search({id:record.id});return item?.url===record.url && item.startTime===record.startTime?item:null;}
    async note(ok,message){try{await this.report(ok,message);}catch(error){console.warn(error);}}
    later(id){if(this.timers.has(id))return;this.timers.add(id);this.schedule(()=>{this.timers.delete(id);void this.recover(id).catch(console.warn);});}
    async terminal(record,phase){
      record.phase=phase;const item=await this.item(record);
      if(!item || item.state==="complete" || (item.state==="interrupted" && !paused(item)))await this.api.storage.local.remove(this.key(record.id));
      else await this.save(record);
    }
    async release(record,message){
      let session;
      try{session=(await this.native({action:"recovery_cancel",id:record.recovery})).recovery;}
      catch{message=__xdL.t("The app could not confirm cancellation. The browser copy was kept; check the app before retrying.");}
      if(session?.state==="accepted" && !record.resumed){await this.accepted(record);return;}
      const item=await this.item(record);
      if(!record.resumed && paused(item))await this.api.downloads.resume(record.id);
      record.resumed=true;await this.terminal(record,"browser");
      await this.note(session?.state!=="accepted",message || __xdL.t("Recovery cancelled. The download stays in the browser."));
    }
    async accepted(record){
      record.phase="accepted";await this.save(record);
      const item=await this.item(record);
      if(!record.resumed && paused(item) && [undefined,"safe"].includes(item.danger)) {
        try {
          await this.api.downloads.cancel(record.id);
          const after=await this.item(record);
          if(after && (after.state==="complete" || after.state==="in_progress" || paused(after)))throw Error(__xdL.t("Browser copy continued"));
          await this.terminal(record,"done");await this.note(true,__xdL.t("Existing download updated in Download Engine."));return;
        } catch {if(paused(await this.item(record)))await this.api.downloads.resume(record.id);record.resumed=true;}
      }
      await this.terminal(record,"browser");
      await this.note(false,__xdL.t("Download Engine updated the existing download. The browser copy also remains; check both download lists."));
    }
    async consider(id){
      if(this.running.has(id))return true;
      this.running.add(id);let record,journalSaved=false,pauseAttempted=false;
      try {
        if(await this.read(id))return true;
        const [first]=await this.api.downloads.search({id});
        if(!first || first.incognito || first.byExtensionId || !await this.api.permissions.contains({permissions:["downloads"]}))return false;
        let session;try{session=(await this.native({action:"recovery_pending"})).recovery;}catch{return false;}
        if(!session)return false;
        const ready=await this.lookup.file(id);
        if(!ready || !matches(session,ready.item,ready.file))return false;
        if(session.state!=="waiting"){await this.note(false,__xdL.t("A fresh link is already awaiting review in Download Engine."));return true;}
        record={id,url:ready.item.url,startTime:ready.item.startTime,recovery:session.id,phase:"pausing",expires:session.expires};
        await this.save(record);
        journalSaved=true;
        const current=await this.item(record);
        if(!IDMCapture.candidate(current)){await this.terminal(record,"browser");return true;}
        pauseAttempted=true;await this.api.downloads.pause(id);
        if(!paused(await this.item(record)))throw Error(__xdL.t("The browser could not pause this download"));
        const {url,filename,size,mime}=ready.file;
        const reply=await this.native({action:"recovery_offer",id:session.id,file:{url,filename,size,mime}});
        if(reply.recovery?.id!==session.id || !["offered","accepted"].includes(reply.recovery.state))throw Error(__xdL.t("The app did not confirm the fresh link"));
        record.phase="waiting";await this.save(record);
        if(reply.recovery.state==="accepted")await this.accepted(record);
        else {await this.note(true,__xdL.t("Fresh link found. Review the saved-data check in Download Engine, then confirm resuming."));this.later(id);}
        return true;
      } catch(error) {
        if(record && journalSaved && pauseAttempted){try{await this.release(record,__xdL.t("Recovery stopped: ")+error.message);}catch{await this.note(false,__xdL.t("Use Resume in the browser Downloads list; recovery could not release its paused copy."));}return true;}
        if(record){await this.note(false,__xdL.t("Recovery could not start: ")+error.message);return true;}
        return false;
      } finally {this.running.delete(id);}
    }
    async recover(id){
      if(this.running.has(id))return;this.running.add(id);
      try {
        const record=await this.read(id);if(!record)return;
        if(["browser","done"].includes(record.phase)){await this.terminal(record,record.phase);return;}
        if(record.phase==="pausing"){await this.release(record,__xdL.t("Interrupted recovery released to the browser."));return;}
        let session;
        try{session=(await this.native({action:"recovery_status",id:record.recovery})).recovery;}
        catch{await this.release(record);return;}
        if(session?.state==="accepted"){await this.accepted(record);return;}
        const item=await this.item(record);
        if(!session || session.state==="cancelled" || this.now()>=record.expires*1000 || !paused(item) || !await this.api.permissions.contains({permissions:["downloads"]})) {await this.release(record);return;}
        this.later(id);
      } catch(error){await this.note(false,__xdL.t("Recovery needs attention. Use Resume in the browser Downloads list if necessary. ")+error.message);}
      finally {this.running.delete(id);}
    }
    async recoverAll(){const saved=await this.api.storage.local.get(null);for(const [key,record] of Object.entries(saved))if(key.startsWith(PREFIX))await this.recover(record.id);}
    async status(){const reply=await this.native({action:"recovery_pending"});return {...reply,allowed:await this.api.permissions.contains({permissions:["downloads"]})};}
  }
  root.IDMLinkRecovery={Manager,matches};
})(globalThis);
